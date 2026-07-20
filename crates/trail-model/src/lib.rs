//! Typed compatibility model for Graphify/Trail node-link graphs.

mod document;
mod error;
mod graph;

pub use document::{EdgeRecord, GraphDocument, NodeRecord};
pub use error::GraphError;
pub use graph::{EdgeIndex, Graph, NodeIndex};
