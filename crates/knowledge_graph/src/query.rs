use serde::{Deserialize, Serialize};
use crate::graph::{FinancialGraph, EntityNode, RelationshipKind};

/// A natural language or structured query against the knowledge graph.
#[derive(Debug, Clone)]
pub enum GraphQuery {
    /// "What does a Fed rate hike affect?"
    WhatDoesAffect { entity_id: String, max_hops: usize },
    /// "What are NVIDIA's key dependencies?"
    KeyDependencies { entity_id: String },
    /// "What entities are correlated with BTC?"
    Correlations { entity_id: String, min_weight: f64 },
    /// "What is the supply chain for AI chips?"
    SupplyChain { downstream_id: String },
    /// "Which sectors are most exposed to oil price risk?"
    SectorExposure { risk_factor_id: String },
    /// "Summarise everything we know about NVIDIA"
    EntitySummary { entity_id: String },
    /// Full context dump for Dexter AI system prompt
    FullContextForSymbol { symbol: String },
}

/// The result of a graph query — structured for LLM prompt injection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphContext {
    pub query_description: String,
    /// Plain-English facts derived from the graph
    pub facts: Vec<String>,
    /// Entities involved
    pub entities: Vec<EntitySummary>,
    /// Narrative paragraph for direct system prompt injection
    pub narrative: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySummary {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub key_relationships: Vec<String>,
}

/// A path through the graph showing how event A impacts entity B
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactPath {
    pub steps: Vec<ImpactStep>,
    pub total_weight: f64,
    pub direction: f64,  // positive = bullish, negative = bearish
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactStep {
    pub from: String,
    pub to: String,
    pub relationship: String,
    pub weight: f64,
}

pub struct GraphQueryEngine<'a> {
    graph: &'a FinancialGraph,
}

impl<'a> GraphQueryEngine<'a> {
    pub fn new(graph: &'a FinancialGraph) -> Self {
        Self { graph }
    }

    pub fn execute(&self, query: GraphQuery) -> GraphContext {
        match query {
            GraphQuery::WhatDoesAffect { entity_id, max_hops } =>
                self.what_does_affect(&entity_id, max_hops),

            GraphQuery::KeyDependencies { entity_id } =>
                self.key_dependencies(&entity_id),

            GraphQuery::Correlations { entity_id, min_weight } =>
                self.correlations(&entity_id, min_weight),

            GraphQuery::SupplyChain { downstream_id } =>
                self.supply_chain(&downstream_id),

            GraphQuery::SectorExposure { risk_factor_id } =>
                self.sector_exposure(&risk_factor_id),

            GraphQuery::EntitySummary { entity_id } =>
                self.entity_summary(&entity_id),

            GraphQuery::FullContextForSymbol { symbol } =>
                self.full_context_for_symbol(&symbol),
        }
    }

    // ── Query implementations ────────────────────────────────────────

    fn what_does_affect(&self, entity_id: &str, max_hops: usize) -> GraphContext {
        let affected = self.graph.affected_by(entity_id);
        let entity_name = self.entity_name(entity_id);
        let mut facts = Vec::new();
        let mut entities = Vec::new();

        for (target, rel) in &affected {
            let direction = if rel.correlation.unwrap_or(0.0) < 0.0 { "negatively" } else { "positively" };
            facts.push(format!(
                "{} {} {} ({}) — weight: {:.2}",
                entity_name, rel.kind.label(), target.name, direction, rel.weight
            ));
            if let Some(corr) = rel.correlation {
                facts.push(format!(
                    "  Historical correlation: {:.2}", corr
                ));
            }
            facts.push(format!("  Detail: {}", rel.description));
            entities.push(self.summarise_entity(target));
        }

        // Transitive effects (2nd hop)
        if max_hops >= 2 {
            for (first, _) in &affected {
                for (second, rel2) in self.graph.affected_by(&first.id) {
                    if second.id != entity_id {
                        facts.push(format!(
                            "  Indirect: {} → {} → {} ({})",
                            entity_name, first.name, second.name, rel2.kind.label()
                        ));
                    }
                }
            }
        }

        let narrative = format!(
            "Knowledge graph analysis: {} directly affects {} entities. {}",
            entity_name,
            affected.len(),
            if affected.is_empty() {
                "No downstream relationships found in the graph.".to_string()
            } else {
                format!(
                    "Primary impacts: {}.",
                    affected
                        .iter()
                        .take(3)
                        .map(|(e, r)| format!("{} ({})", e.name, r.kind.label()))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        );

        GraphContext {
            query_description: format!("What does {} affect?", entity_name),
            facts,
            entities,
            narrative,
        }
    }

    fn key_dependencies(&self, entity_id: &str) -> GraphContext {
        let drivers = self.graph.drivers_of(entity_id);
        let entity_name = self.entity_name(entity_id);
        let mut facts = Vec::new();

        for (driver, rel) in &drivers {
            facts.push(format!(
                "{} {} {} (weight: {:.2}): {}",
                driver.name, rel.kind.label(), entity_name, rel.weight, rel.description
            ));
        }

        // Also look at DependsOn edges outgoing
        let deps = self.graph.affected_by(entity_id);
        for (dep, rel) in deps.iter().filter(|(_, r)| r.kind == RelationshipKind::DependsOn) {
            facts.push(format!(
                "{} depends on {} — {}", entity_name, dep.name, rel.description
            ));
        }

        let narrative = format!(
            "{} has {} incoming dependencies. {}",
            entity_name,
            drivers.len(),
            facts.first().cloned().unwrap_or_default()
        );

        GraphContext {
            query_description: format!("Key dependencies of {}", entity_name),
            facts,
            entities: drivers
                .iter()
                .map(|(e, _)| self.summarise_entity(e))
                .collect(),
            narrative,
        }
    }

    fn correlations(&self, entity_id: &str, min_weight: f64) -> GraphContext {
        let entity_name = self.entity_name(entity_id);
        let correlated = self.graph.affected_by(entity_id)
            .into_iter()
            .chain(self.graph.drivers_of(entity_id))
            .filter(|(_, r)| {
                r.kind == RelationshipKind::Correlates && r.weight >= min_weight
            })
            .collect::<Vec<_>>();

        let facts: Vec<String> = correlated.iter().map(|(e, r)| {
            let corr = r.correlation.map(|c| format!(", r={:.2}", c)).unwrap_or_default();
            format!("{} ↔ {} (weight: {:.2}{}): {}", entity_name, e.name, r.weight, corr, r.description)
        }).collect();

        let narrative = format!(
            "{} shows notable correlations with {} assets in the knowledge graph.",
            entity_name, correlated.len()
        );

        GraphContext {
            query_description: format!("Correlations with {}", entity_name),
            facts,
            entities: correlated.iter().map(|(e, _)| self.summarise_entity(e)).collect(),
            narrative,
        }
    }

    fn supply_chain(&self, downstream_id: &str) -> GraphContext {
        let entity_name = self.entity_name(downstream_id);
        let suppliers = self.graph.drivers_of(downstream_id)
            .into_iter()
            .filter(|(_, r)| matches!(r.kind, RelationshipKind::Supplies | RelationshipKind::SuppliedBy))
            .collect::<Vec<_>>();

        let facts: Vec<String> = suppliers.iter().map(|(e, r)| {
            format!("{} → {} (weight: {:.2}): {}", e.name, entity_name, r.weight, r.description)
        }).collect();

        let narrative = format!(
            "{} has {} known suppliers in the graph. {}",
            entity_name,
            suppliers.len(),
            facts.first().cloned().unwrap_or("No supply chain data available.".to_string())
        );

        GraphContext {
            query_description: format!("Supply chain for {}", entity_name),
            facts,
            entities: suppliers.iter().map(|(e, _)| self.summarise_entity(e)).collect(),
            narrative,
        }
    }

    fn sector_exposure(&self, risk_factor_id: &str) -> GraphContext {
        let factor_name = self.entity_name(risk_factor_id);
        let directly_affected = self.graph.affected_by(risk_factor_id);
        let mut facts = Vec::new();

        for (target, rel) in &directly_affected {
            if target.entity_type == crate::graph::EntityType::Sector {
                let dir = rel.correlation.unwrap_or(0.0);
                let impact = if dir < -0.3 { "bearish" } else if dir > 0.3 { "bullish" } else { "mixed" };
                facts.push(format!(
                    "{} is {} for {} sector (correlation: {:.2}): {}",
                    factor_name, impact, target.name,
                    dir, rel.description
                ));
            }
        }

        GraphContext {
            query_description: format!("Sector exposure to {}", factor_name),
            facts: facts.clone(),
            entities: directly_affected.iter().map(|(e, _)| self.summarise_entity(e)).collect(),
            narrative: if facts.is_empty() {
                format!("No sector-level relationships found for {} in the graph.", factor_name)
            } else {
                facts.join(" | ")
            },
        }
    }

    fn entity_summary(&self, entity_id: &str) -> GraphContext {
        let Some(entity) = self.graph.get_entity(entity_id) else {
            return GraphContext {
                query_description: format!("Summary of {}", entity_id),
                facts: vec![format!("Entity '{}' not found in knowledge graph", entity_id)],
                entities: vec![],
                narrative: format!("No data available for '{}'", entity_id),
            };
        };

        let outgoing = self.graph.affected_by(entity_id);
        let incoming = self.graph.drivers_of(entity_id);
        let mut facts = Vec::new();

        if let Some(sector) = &entity.sector {
            facts.push(format!("Sector: {}", sector));
        }
        if let Some(ticker) = &entity.ticker {
            facts.push(format!("Ticker: {}", ticker));
        }
        if let Some(country) = &entity.country {
            facts.push(format!("Country: {}", country));
        }
        if let Some(desc) = entity.metadata.get("description") {
            facts.push(format!("Description: {}", desc));
        }
        for (target, rel) in &outgoing {
            facts.push(format!("{} → {} ({})", entity.name, target.name, rel.kind.label()));
        }
        for (source, rel) in &incoming {
            facts.push(format!("{} → {} ({})", source.name, entity.name, rel.kind.label()));
        }

        let narrative = format!(
            "{} ({:?}) has {} outgoing relationships and {} incoming. \
            It is connected to: {}.",
            entity.name,
            entity.entity_type,
            outgoing.len(),
            incoming.len(),
            outgoing
                .iter()
                .take(3)
                .map(|(e, _)| e.name.as_str())
                .chain(incoming.iter().take(2).map(|(e, _)| e.name.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        );

        GraphContext {
            query_description: format!("Full summary of {}", entity.name),
            facts,
            entities: vec![self.summarise_entity(entity)],
            narrative,
        }
    }

    /// Build a comprehensive context block for a trading symbol.
    /// This is what gets injected into Dexter AI's system prompt.
    pub fn full_context_for_symbol(&self, symbol: &str) -> GraphContext {
        let mut all_facts = Vec::new();
        let mut all_entities = Vec::new();

        // 1. Supply chain
        let supply = self.supply_chain(symbol);
        all_facts.extend(supply.facts);
        all_entities.extend(supply.entities);

        // 2. What drives it
        let deps = self.key_dependencies(symbol);
        all_facts.extend(deps.facts);

        // 3. What it affects
        let affects = self.what_does_affect(symbol, 1);
        all_facts.extend(affects.facts);

        // 4. Correlations
        let corr = self.correlations(symbol, 0.5);
        all_facts.extend(corr.facts);

        // Deduplicate
        all_facts.dedup();

        let narrative = format!(
            "Knowledge graph context for {}: {}",
            symbol,
            all_facts.iter().take(5).cloned().collect::<Vec<_>>().join(". ")
        );

        GraphContext {
            query_description: format!("Full knowledge graph context for {}", symbol),
            facts: all_facts,
            entities: all_entities,
            narrative,
        }
    }

    fn entity_name(&self, id: &str) -> String {
        self.graph
            .get_entity(id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    fn summarise_entity(&self, entity: &EntityNode) -> EntitySummary {
        let out = self.graph.affected_by(&entity.id);
        let key_rels: Vec<String> = out
            .iter()
            .take(3)
            .map(|(e, r)| format!("{} {}", r.kind.label(), e.name))
            .collect();

        EntitySummary {
            id: entity.id.clone(),
            name: entity.name.clone(),
            entity_type: format!("{:?}", entity.entity_type),
            key_relationships: key_rels,
        }
    }
}

impl GraphContext {
    /// Format as a concise system prompt block for Claude
    pub fn to_prompt_block(&self) -> String {
        let facts_block = self
            .facts
            .iter()
            .take(10)
            .map(|f| format!("• {}", f))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "[KNOWLEDGE GRAPH: {}]\n{}\n\nNarrative: {}",
            self.query_description, facts_block, self.narrative
        )
    }
}
