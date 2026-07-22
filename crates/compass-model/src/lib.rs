//! Typed compatibility model for Graphify/Compass node-link graphs.

mod document;
mod error;
mod graph;
mod query_index;
mod validation;

pub use document::{EdgeRecord, GraphDocument, NodeRecord};
pub use error::GraphError;
pub use graph::{EdgeIndex, Graph, NodeIndex};
pub use query_index::{QueryIndex, SchemaFingerprint, cypher_node_label, cypher_relationship_type};
pub use validation::{ExtractionValidationError, assert_valid_extraction, validate_extraction};
