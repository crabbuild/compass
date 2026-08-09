//! Typed model for Compass node-link graphs.

/// Default bounded size accepted by current graph readers.
pub const DEFAULT_GRAPH_SIZE_CAP_BYTES: u64 = 1024 * 1024 * 1024;

pub mod code_graph;
mod document;
mod error;
mod graph;
pub mod identity;
pub mod provenance;
pub mod query_contract;
mod query_index;
pub mod search;
mod validation;

pub use document::{EdgeRecord, GraphDocument, NodeRecord};
pub use error::GraphError;
pub use graph::{EdgeIndex, Graph, NodeIndex};
pub use query_index::{QueryIndex, SchemaFingerprint, cypher_node_label, cypher_relationship_type};
pub use validation::{
    CodeGraphValidationError, CodeGraphValidationReport, ExtractionValidationError,
    RecordValidationErrors, assert_valid_extraction, validate_code_graph,
    validate_code_graph_records, validate_extraction,
};
