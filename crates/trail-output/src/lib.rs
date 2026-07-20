//! Safe, deterministic output formats for Trail graphs.

mod cypher;
mod graphml;
mod json;
mod report;

pub use cypher::{cypher_document, write_cypher};
pub use graphml::{graphml_document, write_graphml};
pub use json::{JsonExportOptions, export_json_value, write_json};
pub use report::{DetectionSummary, ReportOptions, TokenCost, generate_report};

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error(transparent)]
    File(#[from] trail_files::FileError),
    #[error("existing graph is non-empty but malformed: {0}")]
    MalformedGraph(std::path::PathBuf),
    #[error("refusing to shrink graph from {existing} nodes to {new}; use force to override")]
    ShrinkRefused { existing: usize, new: usize },
}
