use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;

pub type EntityId = String;

/// A financial entity — company, person, sector, macro indicator, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityNode {
    pub id: EntityId,
    pub name: String,
    pub entity_type: EntityType,
    pub ticker: Option<String>,
    pub sector: Option<String>,
    pub country: Option<String>,
    /// Free-form metadata (e.g. "market_cap": "2.5T")
    pub metadata: HashMap<String, String>,
    /// Sentiment score injected from swarm_sim or news [-1.0, 1.0]
    pub sentiment: f64,
    pub last_updated_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntityType {
    Company,
    Person, // CEO, analyst, policymaker
    Sector,
    Country,
    Currency,
    Commodity,
    Index,
    MacroIndicator, // GDP, CPI, Fed Funds Rate
    Regulation,
    Event, // Earnings, merger, crisis
}

/// A directed relationship between two entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub kind: RelationshipKind,
    /// Strength of the relationship [0.0, 1.0]
    pub weight: f64,
    /// For AFFECTS: the correlation coefficient
    pub correlation: Option<f64>,
    pub description: String,
    pub source: String, // "doc_ingest" | "manual" | "swarm_sim"
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RelationshipKind {
    Supplies,    // TSMC -> NVIDIA
    Competes,    // NVIDIA <-> AMD
    Owns,        // Berkshire -> AAPL
    RegulatedBy, // Meta -> FTC
    Affects,     // Fed Rate -> NASDAQ (macro)
    LeadBy,      // Apple -> Tim Cook
    PartOf,      // AAPL -> Technology sector
    DependsOn,   // Airlines -> Oil price
    Correlates,  // BTC -> NASDAQ
    Acquires,    // MSFT -> Activision
    SuppliedBy,  // NVIDIA -> TSMC (reverse of Supplies)
    Custom(String),
}

impl RelationshipKind {
    /// Human readable label for TUI / report rendering
    pub fn label(&self) -> &str {
        match self {
            Self::Supplies => "supplies",
            Self::Competes => "competes with",
            Self::Owns => "owns",
            Self::RegulatedBy => "regulated by",
            Self::Affects => "affects",
            Self::LeadBy => "led by",
            Self::PartOf => "part of",
            Self::DependsOn => "depends on",
            Self::Correlates => "correlates with",
            Self::Acquires => "acquires",
            Self::SuppliedBy => "supplied by",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// Is this relationship direction bearish or bullish for the target?
    /// Used by ImpactEngine to score contagion direction.
    pub fn propagation_sign(&self) -> f64 {
        match self {
            Self::Supplies | Self::SuppliedBy => -0.6, // supply disruption = negative
            Self::Affects => -1.0,
            Self::DependsOn => -0.8,
            Self::Correlates => 0.7,
            Self::Competes => 0.3, // competitor pain = your gain
            _ => 0.0,
        }
    }
}

/// The main knowledge graph — wraps a petgraph DiGraph with a name index
pub struct FinancialGraph {
    pub graph: DiGraph<EntityNode, Relationship>,
    /// Fast lookup: entity ID → NodeIndex
    index: HashMap<EntityId, NodeIndex>,
}

impl FinancialGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            index: HashMap::new(),
        }
    }

    /// Add or update an entity. Returns the NodeIndex.
    pub fn upsert_entity(&mut self, entity: EntityNode) -> NodeIndex {
        if let Some(&idx) = self.index.get(&entity.id) {
            // Update in place
            self.graph[idx] = entity;
            return idx;
        }
        let id = entity.id.clone();
        let idx = self.graph.add_node(entity);
        self.index.insert(id, idx);
        idx
    }

    /// Add a directed relationship between two entities by ID.
    /// Creates entities with minimal data if they don't exist yet.
    pub fn add_relationship(&mut self, from_id: &str, to_id: &str, rel: Relationship) {
        let from = self.ensure_entity(from_id);
        let to = self.ensure_entity(to_id);

        // Avoid duplicate edges of same kind
        let already_exists = self
            .graph
            .edges_connecting(from, to)
            .any(|e| e.weight().kind == rel.kind);

        if !already_exists {
            self.graph.add_edge(from, to, rel);
        }
    }

    /// Get a node by entity ID
    pub fn get_entity(&self, id: &str) -> Option<&EntityNode> {
        self.index.get(id).map(|&idx| &self.graph[idx])
    }

    /// Get all entities of a given type
    pub fn entities_of_type(&self, t: &EntityType) -> Vec<&EntityNode> {
        self.graph
            .node_weights()
            .filter(|n| &n.entity_type == t)
            .collect()
    }

    /// Get all entities this one directly affects (outgoing edges)
    pub fn affected_by(&self, id: &str) -> Vec<(&EntityNode, &Relationship)> {
        let Some(&idx) = self.index.get(id) else {
            return vec![];
        };
        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .map(|e| (&self.graph[e.target()], e.weight()))
            .collect()
    }

    /// Get all entities that directly affect this one (incoming edges)
    pub fn drivers_of(&self, id: &str) -> Vec<(&EntityNode, &Relationship)> {
        let Some(&idx) = self.index.get(id) else {
            return vec![];
        };
        self.graph
            .edges_directed(idx, Direction::Incoming)
            .map(|e| (&self.graph[e.source()], e.weight()))
            .collect()
    }

    /// BFS: find all entities reachable within N hops from `id`
    pub fn reachable(&self, id: &str, max_hops: usize) -> Vec<&EntityNode> {
        use petgraph::visit::Bfs;
        let Some(&start) = self.index.get(id) else {
            return vec![];
        };

        let mut bfs = Bfs::new(&self.graph, start);
        let mut result = Vec::new();
        let mut hops = 0;

        while let Some(nx) = bfs.next(&self.graph) {
            if nx == start {
                continue;
            }
            if hops >= max_hops {
                break;
            }
            result.push(&self.graph[nx]);
            hops += 1;
        }
        result
    }

    /// Total node count
    pub fn entity_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Total edge count
    pub fn relationship_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Serialise to JSON for persistence / export to TUI dashboard
    pub fn to_json(&self) -> anyhow::Result<String> {
        #[derive(Serialize)]
        struct Export<'a> {
            nodes: Vec<&'a EntityNode>,
            edges: Vec<EdgeExport<'a>>,
        }
        #[derive(Serialize)]
        struct EdgeExport<'a> {
            from: &'a str,
            to: &'a str,
            relationship: &'a Relationship,
        }

        let nodes: Vec<&EntityNode> = self.graph.node_weights().collect();
        let edges: Vec<EdgeExport> = self
            .graph
            .edge_indices()
            .map(|e| {
                let (src, tgt) = self.graph.edge_endpoints(e).unwrap();
                EdgeExport {
                    from: &self.graph[src].id,
                    to: &self.graph[tgt].id,
                    relationship: &self.graph[e],
                }
            })
            .collect();

        Ok(serde_json::to_string_pretty(&Export { nodes, edges })?)
    }

    /// Deserialise from persisted JSON
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct Import {
            nodes: Vec<EntityNode>,
            edges: Vec<EdgeImport>,
        }
        #[derive(Deserialize)]
        struct EdgeImport {
            from: String,
            to: String,
            relationship: Relationship,
        }

        let import: Import = serde_json::from_str(json)?;
        let mut graph = Self::new();
        for node in import.nodes {
            graph.upsert_entity(node);
        }
        for edge in import.edges {
            graph.add_relationship(&edge.from, &edge.to, edge.relationship);
        }
        info!(
            "Loaded graph: {} entities, {} relationships",
            graph.entity_count(),
            graph.relationship_count()
        );
        Ok(graph)
    }

    fn ensure_entity(&mut self, id: &str) -> NodeIndex {
        if let Some(&idx) = self.index.get(id) {
            return idx;
        }
        let node = EntityNode {
            id: id.to_string(),
            name: id.to_string(),
            entity_type: EntityType::Company,
            ticker: None,
            sector: None,
            country: None,
            metadata: HashMap::new(),
            sentiment: 0.0,
            last_updated_ms: chrono::Utc::now().timestamp_millis(),
        };
        let idx = self.graph.add_node(node);
        self.index.insert(id.to_string(), idx);
        idx
    }
}

impl Default for FinancialGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-built seed graph with well-known financial relationships.
/// Gives Dexter AI meaningful context immediately, before any doc ingestion.
pub fn seed_graph() -> FinancialGraph {
    let mut g = FinancialGraph::new();

    let rel = |kind: RelationshipKind, w: f64, desc: &str| Relationship {
        kind,
        weight: w,
        correlation: None,
        description: desc.to_string(),
        source: "seed".to_string(),
    };

    // ── Supply chain ─────────────────────────────────────────────────
    g.add_relationship(
        "TSMC",
        "NVIDIA",
        rel(
            RelationshipKind::Supplies,
            0.95,
            "TSMC manufactures NVIDIA GPUs on its advanced nodes (3nm, 5nm)",
        ),
    );
    g.add_relationship(
        "TSMC",
        "APPLE",
        rel(
            RelationshipKind::Supplies,
            0.95,
            "TSMC manufactures Apple A-series and M-series chips exclusively",
        ),
    );
    g.add_relationship(
        "TSMC",
        "AMD",
        rel(
            RelationshipKind::Supplies,
            0.90,
            "TSMC manufactures AMD Zen CPUs and RDNA GPUs",
        ),
    );
    g.add_relationship(
        "SAMSUNG",
        "QUALCOMM",
        rel(
            RelationshipKind::Supplies,
            0.70,
            "Samsung Foundry manufactures some Snapdragon chips",
        ),
    );

    // ── Competition ───────────────────────────────────────────────────
    g.add_relationship(
        "NVIDIA",
        "AMD",
        rel(
            RelationshipKind::Competes,
            0.85,
            "Direct GPU competition across data center and gaming",
        ),
    );
    g.add_relationship(
        "NVIDIA",
        "INTEL",
        rel(
            RelationshipKind::Competes,
            0.60,
            "Competing in AI accelerator and data center markets",
        ),
    );
    g.add_relationship(
        "APPLE",
        "GOOGLE",
        rel(
            RelationshipKind::Competes,
            0.80,
            "Mobile OS and consumer device competition",
        ),
    );
    g.add_relationship(
        "APPLE",
        "MICROSOFT",
        rel(
            RelationshipKind::Competes,
            0.65,
            "Enterprise and productivity software competition",
        ),
    );

    // ── Macro → Market impacts ────────────────────────────────────────
    g.add_relationship(
        "FED_FUNDS_RATE",
        "NASDAQ",
        Relationship {
            kind: RelationshipKind::Affects,
            weight: 0.90,
            correlation: Some(-0.75),
            description:
                "Rising rates increase discount rate on growth stocks, compressing multiples"
                    .to_string(),
            source: "seed".to_string(),
        },
    );
    g.add_relationship(
        "FED_FUNDS_RATE",
        "TREASURY_10Y",
        Relationship {
            kind: RelationshipKind::Affects,
            weight: 0.95,
            correlation: Some(0.85),
            description:
                "Fed rate directly drives short end; long end influenced by inflation expectations"
                    .to_string(),
            source: "seed".to_string(),
        },
    );
    g.add_relationship(
        "TREASURY_10Y",
        "SP500",
        Relationship {
            kind: RelationshipKind::Affects,
            weight: 0.80,
            correlation: Some(-0.65),
            description:
                "10Y yield is the risk-free rate; higher yields compress equity valuations"
                    .to_string(),
            source: "seed".to_string(),
        },
    );
    g.add_relationship(
        "OIL_WTI",
        "AIRLINES",
        Relationship {
            kind: RelationshipKind::Affects,
            weight: 0.88,
            correlation: Some(-0.80),
            description:
                "Jet fuel is ~25% of airline operating costs; oil price directly hits margins"
                    .to_string(),
            source: "seed".to_string(),
        },
    );
    g.add_relationship(
        "OIL_WTI",
        "ENERGY_SECTOR",
        Relationship {
            kind: RelationshipKind::Affects,
            weight: 0.92,
            correlation: Some(0.88),
            description: "Oil price is the primary revenue driver for upstream energy companies"
                .to_string(),
            source: "seed".to_string(),
        },
    );
    g.add_relationship(
        "DXY",
        "GOLD",
        Relationship {
            kind: RelationshipKind::Affects,
            weight: 0.85,
            correlation: Some(-0.72),
            description: "Dollar strength makes gold more expensive for non-USD buyers".to_string(),
            source: "seed".to_string(),
        },
    );
    g.add_relationship(
        "DXY",
        "EMERGING_MARKETS",
        Relationship {
            kind: RelationshipKind::Affects,
            weight: 0.80,
            correlation: Some(-0.70),
            description:
                "Strong dollar pressures EM currencies, tightens USD-denominated debt conditions"
                    .to_string(),
            source: "seed".to_string(),
        },
    );

    // ── Sector membership ─────────────────────────────────────────────
    for ticker in &[
        "NVIDIA",
        "AMD",
        "INTEL",
        "TSMC",
        "APPLE",
        "MICROSOFT",
        "GOOGLE",
        "META",
    ] {
        g.add_relationship(
            ticker,
            "TECHNOLOGY_SECTOR",
            rel(
                RelationshipKind::PartOf,
                1.0,
                "Technology sector constituent",
            ),
        );
    }
    for ticker in &["JPMORGAN", "GOLDMAN_SACHS", "MORGAN_STANLEY", "WELLS_FARGO"] {
        g.add_relationship(
            ticker,
            "FINANCIALS_SECTOR",
            rel(
                RelationshipKind::PartOf,
                1.0,
                "Financials sector constituent",
            ),
        );
    }

    // ── Regulatory ────────────────────────────────────────────────────
    g.add_relationship("META", "FTC", rel(RelationshipKind::RegulatedBy, 0.90,
        "FTC has brought antitrust cases against Meta; Instagram/WhatsApp acquisitions under scrutiny"));
    g.add_relationship(
        "GOOGLE",
        "EU_COMPETITION",
        rel(
            RelationshipKind::RegulatedBy,
            0.90,
            "EU has fined Google multiple times for search and Android antitrust violations",
        ),
    );
    g.add_relationship(
        "NVIDIA",
        "BIS_EXPORT",
        rel(
            RelationshipKind::RegulatedBy,
            0.85,
            "BIS export controls restrict sale of H100/H200 chips to China",
        ),
    );

    // ── Crypto / digital assets ───────────────────────────────────────
    g.add_relationship(
        "BTC",
        "NASDAQ",
        Relationship {
            kind: RelationshipKind::Correlates,
            weight: 0.65,
            correlation: Some(0.55),
            description:
                "Bitcoin has shown positive correlation with risk-on tech assets since 2020"
                    .to_string(),
            source: "seed".to_string(),
        },
    );
    g.add_relationship(
        "BTC",
        "GOLD",
        Relationship {
            kind: RelationshipKind::Correlates,
            weight: 0.40,
            correlation: Some(0.30),
            description: "Both positioned as inflation hedges; correlation is inconsistent"
                .to_string(),
            source: "seed".to_string(),
        },
    );

    // Update entity types where we can infer them
    macro_rules! set_type {
        ($g:expr, $id:expr, $t:expr, $name:expr) => {
            if let Some(&idx) = $g.index.get($id) {
                $g.graph[idx].entity_type = $t;
                $g.graph[idx].name = $name.to_string();
            }
        };
    }

    set_type!(
        g,
        "FED_FUNDS_RATE",
        EntityType::MacroIndicator,
        "Fed Funds Rate"
    );
    set_type!(
        g,
        "TREASURY_10Y",
        EntityType::MacroIndicator,
        "10Y Treasury Yield"
    );
    set_type!(g, "DXY", EntityType::Index, "US Dollar Index");
    set_type!(g, "SP500", EntityType::Index, "S&P 500");
    set_type!(g, "NASDAQ", EntityType::Index, "NASDAQ Composite");
    set_type!(g, "OIL_WTI", EntityType::Commodity, "WTI Crude Oil");
    set_type!(g, "GOLD", EntityType::Commodity, "Gold");
    set_type!(g, "BTC", EntityType::Currency, "Bitcoin");
    set_type!(
        g,
        "TECHNOLOGY_SECTOR",
        EntityType::Sector,
        "Technology Sector"
    );
    set_type!(
        g,
        "FINANCIALS_SECTOR",
        EntityType::Sector,
        "Financials Sector"
    );
    set_type!(g, "ENERGY_SECTOR", EntityType::Sector, "Energy Sector");
    set_type!(g, "AIRLINES", EntityType::Sector, "Airlines Sector");
    set_type!(
        g,
        "EMERGING_MARKETS",
        EntityType::Sector,
        "Emerging Markets"
    );
    set_type!(g, "FTC", EntityType::Regulation, "Federal Trade Commission");
    set_type!(
        g,
        "EU_COMPETITION",
        EntityType::Regulation,
        "EU Competition Authority"
    );
    set_type!(
        g,
        "BIS_EXPORT",
        EntityType::Regulation,
        "BIS Export Controls"
    );

    info!(
        "Seeded knowledge graph: {} entities, {} relationships",
        g.entity_count(),
        g.relationship_count()
    );
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_graph_has_expected_relationships() {
        let g = seed_graph();
        assert!(g.entity_count() > 20);
        assert!(g.relationship_count() > 20);

        // TSMC should affect NVIDIA
        let affected = g.affected_by("TSMC");
        let targets: Vec<&str> = affected.iter().map(|(e, _)| e.id.as_str()).collect();
        assert!(targets.contains(&"NVIDIA"));
    }

    #[test]
    fn graph_serialisation_roundtrip() {
        let g = seed_graph();
        let json = g.to_json().unwrap();
        let g2 = FinancialGraph::from_json(&json).unwrap();
        assert_eq!(g.entity_count(), g2.entity_count());
        assert_eq!(g.relationship_count(), g2.relationship_count());
    }

    #[test]
    fn reachable_finds_transitive_nodes() {
        let g = seed_graph();
        // FED_FUNDS_RATE -> NASDAQ, NASDAQ is reachable in 1 hop
        let reachable = g.reachable("FED_FUNDS_RATE", 2);
        let ids: Vec<&str> = reachable.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"NASDAQ") || ids.contains(&"TREASURY_10Y"));
    }
}
