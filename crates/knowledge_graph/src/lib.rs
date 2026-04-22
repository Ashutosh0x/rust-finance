#![forbid(unsafe_code)]
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
pub mod impact;
pub mod ontology;
pub mod query;

pub use graph::{EntityId, EntityNode, FinancialGraph, Relationship, RelationshipKind};
pub use impact::ImpactEngine;
pub use ontology::{Ontology, OntologyEdge, OntologyEntity};
pub use query::{GraphContext, GraphQuery, ImpactPath};
