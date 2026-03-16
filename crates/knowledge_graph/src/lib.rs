// ============================================================
// knowledge_graph — Financial Entity Knowledge Graph
// Part of RustForge Terminal (rust-finance)
//
// Bridges the MiroFish GraphRAG capability into native Rust.
// Replaces Zep Cloud with an in-process petgraph store that
// is queryable by both the Dexter AI analyst and the ReACT
// report_agent.
//
// Data flow:
//   doc_ingest  ──extract()──►  Ontology
//   Ontology    ──load()────►  FinancialGraph
//   FinancialGraph  ──query()──►  GraphContext  ──►  Dexter AI prompt
// ============================================================

pub mod graph;
pub mod ontology;
pub mod query;
pub mod impact;

pub use graph::{FinancialGraph, EntityId, EntityNode, Relationship, RelationshipKind};
pub use ontology::{Ontology, OntologyEntity, OntologyEdge};
pub use query::{GraphQuery, GraphContext, ImpactPath};
pub use impact::ImpactEngine;
