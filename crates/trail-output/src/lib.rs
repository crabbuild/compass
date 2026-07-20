//! Safe, deterministic output formats for Trail graphs.

mod canvas;
mod cypher;
mod graphml;
mod html;
mod json;
mod obsidian;
mod report;

pub use canvas::{CanvasOptions, canvas_document, write_canvas};
pub use cypher::{cypher_document, write_cypher};
pub use graphml::{graphml_document, write_graphml};
pub use html::{HtmlOptions, HtmlRender, html_document, write_html};
pub use json::{JsonExportOptions, export_json_value, write_json};
pub use obsidian::{ObsidianExport, ObsidianOptions, export_obsidian, node_filenames};
pub use report::{DetectionSummary, ReportOptions, TokenCost, generate_report};

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error(transparent)]
    File(#[from] trail_files::FileError),
    #[error("existing graph is non-empty but malformed: {0}")]
    MalformedGraph(std::path::PathBuf),
    #[error("refusing to shrink graph from {existing} nodes to {new}; use force to override")]
    ShrinkRefused { existing: usize, new: usize },
    #[error("invalid Obsidian output path: {0}")]
    InvalidObsidianPath(std::path::PathBuf),
    #[error(
        "graph has {nodes} nodes - too large for HTML viz (limit: {limit}). Use --no-viz, raise GRAPHIFY_VIZ_NODE_LIMIT, or reduce input size."
    )]
    HtmlTooLarge { nodes: usize, limit: isize },
}
