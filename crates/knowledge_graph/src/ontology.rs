use serde::{Deserialize, Serialize};
use crate::graph::{EntityType, RelationshipKind, EntityNode, Relationship, FinancialGraph};
use std::collections::HashMap;

/// The structured output Claude returns after analysing a document.
/// Designed to be deserialisable directly from Claude's JSON response.
///
/// Prompt template lives in `doc_ingest/src/prompts/ontology.txt`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ontology {
    pub source_document: String,
    pub entities: Vec<OntologyEntity>,
    pub edges: Vec<OntologyEdge>,
    pub extraction_confidence: f64,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyEntity {
    pub id: String,          // Normalised ID: "NVIDIA", "FED_FUNDS_RATE"
    pub name: String,        // Display name: "NVIDIA Corporation"
    pub entity_type: String, // "Company" | "Person" | "MacroIndicator" | etc.
    pub ticker: Option<String>,
    pub sector: Option<String>,
    pub country: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyEdge {
    pub from_id: String,
    pub to_id: String,
    pub relationship: String, // "Supplies" | "Affects" | "Competes" | etc.
    pub weight: f64,          // 0.0–1.0 as estimated by Claude
    pub description: String,
    pub correlation: Option<f64>,
}

impl Ontology {
    /// Merge this ontology into a FinancialGraph.
    /// Called by doc_ingest after each document is processed.
    pub fn merge_into(&self, graph: &mut FinancialGraph) {
        // 1. Upsert all entities
        for e in &self.entities {
            let entity_type = parse_entity_type(&e.entity_type);
            let node = EntityNode {
                id: e.id.clone(),
                name: e.name.clone(),
                entity_type,
                ticker: e.ticker.clone(),
                sector: e.sector.clone(),
                country: e.country.clone(),
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("description".to_string(), e.description.clone());
                    m.insert("source".to_string(), self.source_document.clone());
                    m
                },
                sentiment: 0.0,
                last_updated_ms: chrono::Utc::now().timestamp_millis(),
            };
            graph.upsert_entity(node);
        }

        // 2. Add all edges
        for edge in &self.edges {
            let kind = parse_relationship_kind(&edge.relationship);
            let rel = Relationship {
                kind,
                weight: edge.weight,
                correlation: edge.correlation,
                description: edge.description.clone(),
                source: self.source_document.clone(),
            };
            graph.add_relationship(&edge.from_id, &edge.to_id, rel);
        }

        tracing::info!(
            "Merged ontology from '{}': +{} entities, +{} edges",
            self.source_document,
            self.entities.len(),
            self.edges.len()
        );
    }

    /// Validate that all edge references point to existing entity IDs.
    /// Returns list of dangling edge descriptions.
    pub fn validate(&self) -> Vec<String> {
        let entity_ids: std::collections::HashSet<&str> =
            self.entities.iter().map(|e| e.id.as_str()).collect();
        let mut warnings = Vec::new();

        for edge in &self.edges {
            if !entity_ids.contains(edge.from_id.as_str()) {
                warnings.push(format!(
                    "Edge source '{}' not in entity list", edge.from_id
                ));
            }
            if !entity_ids.contains(edge.to_id.as_str()) {
                warnings.push(format!(
                    "Edge target '{}' not in entity list", edge.to_id
                ));
            }
        }
        warnings
    }
}

fn parse_entity_type(s: &str) -> EntityType {
    match s.to_lowercase().as_str() {
        "company" => EntityType::Company,
        "person" => EntityType::Person,
        "sector" => EntityType::Sector,
        "country" => EntityType::Country,
        "currency" => EntityType::Currency,
        "commodity" => EntityType::Commodity,
        "index" => EntityType::Index,
        "macroindicator" | "macro_indicator" | "macro" => EntityType::MacroIndicator,
        "regulation" | "regulatory" => EntityType::Regulation,
        "event" => EntityType::Event,
        _ => EntityType::Company,
    }
}

fn parse_relationship_kind(s: &str) -> RelationshipKind {
    match s.to_lowercase().as_str() {
        "supplies" => RelationshipKind::Supplies,
        "suppliedby" | "supplied_by" => RelationshipKind::SuppliedBy,
        "competes" | "competes_with" => RelationshipKind::Competes,
        "owns" => RelationshipKind::Owns,
        "regulatedby" | "regulated_by" => RelationshipKind::RegulatedBy,
        "affects" => RelationshipKind::Affects,
        "leadby" | "led_by" | "leaderof" => RelationshipKind::LeadBy,
        "partof" | "part_of" => RelationshipKind::PartOf,
        "dependson" | "depends_on" => RelationshipKind::DependsOn,
        "correlates" | "correlates_with" => RelationshipKind::Correlates,
        "acquires" => RelationshipKind::Acquires,
        other => RelationshipKind::Custom(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ontology() -> Ontology {
        Ontology {
            source_document: "test_doc.md".to_string(),
            entities: vec![
                OntologyEntity {
                    id: "OPENAI".to_string(),
                    name: "OpenAI".to_string(),
                    entity_type: "Company".to_string(),
                    ticker: None,
                    sector: Some("Technology".to_string()),
                    country: Some("USA".to_string()),
                    description: "AI research lab".to_string(),
                },
                OntologyEntity {
                    id: "MICROSOFT".to_string(),
                    name: "Microsoft Corporation".to_string(),
                    entity_type: "Company".to_string(),
                    ticker: Some("MSFT".to_string()),
                    sector: Some("Technology".to_string()),
                    country: Some("USA".to_string()),
                    description: "Cloud and software giant".to_string(),
                },
            ],
            edges: vec![OntologyEdge {
                from_id: "MICROSOFT".to_string(),
                to_id: "OPENAI".to_string(),
                relationship: "Owns".to_string(),
                weight: 0.49,
                description: "Microsoft has invested $13B in OpenAI".to_string(),
                correlation: None,
            }],
            extraction_confidence: 0.92,
            summary: "Describes Microsoft's investment in OpenAI".to_string(),
        }
    }

    #[test]
    fn ontology_merges_into_graph() {
        let mut graph = FinancialGraph::new();
        let ont = sample_ontology();
        ont.merge_into(&mut graph);

        assert_eq!(graph.entity_count(), 2);
        assert_eq!(graph.relationship_count(), 1);
        assert!(graph.get_entity("OPENAI").is_some());
    }

    #[test]
    fn validate_catches_dangling_edge() {
        let mut ont = sample_ontology();
        ont.edges.push(OntologyEdge {
            from_id: "NONEXISTENT".to_string(),
            to_id: "OPENAI".to_string(),
            relationship: "Affects".to_string(),
            weight: 0.5,
            description: "dangling".to_string(),
            correlation: None,
        });
        let warnings = ont.validate();
        assert!(!warnings.is_empty());
    }
}
