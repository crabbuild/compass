//! Safe, deterministic output formats for Trail graphs.

mod cypher;
mod json;

pub use cypher::{cypher_document, write_cypher};
pub use json::{JsonExportOptions, export_json_value, write_json};

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error(transparent)]
    File(#[from] trail_files::FileError),
    #[error("existing graph is non-empty but malformed: {0}")]
    MalformedGraph(std::path::PathBuf),
    #[error("refusing to shrink graph from {existing} nodes to {new}; use force to override")]
    ShrinkRefused { existing: usize, new: usize },
}
