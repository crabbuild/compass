//! Typed compatibility model for Graphify/Trail node-link graphs.

mod document;
mod error;
mod graph;
mod validation;

pub use document::{EdgeRecord, GraphDocument, NodeRecord};
pub use error::GraphError;
pub use graph::{EdgeIndex, Graph, NodeIndex};
pub use validation::{ExtractionValidationError, assert_valid_extraction, validate_extraction};
