//! Safe, deterministic output formats for Compass graphs.

mod backup;
mod callflow;
mod canvas;
mod cypher;
mod graphml;
mod html;
mod json;
mod obsidian;
mod report;
mod svg;
mod tree;
mod wiki;

pub use backup::{BackupResult, backup_if_protected};
pub use callflow::{
    CallflowExport, CallflowOptions, CallflowSection, callflow_html_document,
    derive_callflow_sections, write_callflow_html,
};
pub use canvas::{CanvasOptions, canvas_document, write_canvas};
pub use cypher::{cypher_document, write_cypher};
pub use graphml::{graphml_document, write_graphml};
pub use html::{HtmlOptions, HtmlRender, html_document, write_html};
pub use json::{JsonExportOptions, export_json_value, write_json};
pub use obsidian::{ObsidianExport, ObsidianOptions, export_obsidian, node_filenames};
pub use report::{DetectionSummary, ReportOptions, TokenCost, generate_report};
pub use svg::{SvgOptions, spring_layout, svg_document, write_svg};
pub use tree::{TreeNode, TreeOptions, build_tree, tree_html_document, write_tree_html};
pub use wiki::{WikiExport, WikiOptions, export_wiki};

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error(transparent)]
    File(#[from] compass_files::FileError),
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
    #[error("graph.json contains 0 nodes")]
    EmptyCallflowGraph,
    #[error("no sections defined")]
    NoCallflowSections,
    #[error(
        "communities dict is empty — refusing to clear wiki/. Run `graphify extract .` or `graphify cluster-only .` first."
    )]
    EmptyWikiCommunities,
    #[error(
        "all community node IDs are stale — none exist in the graph. Re-run `graphify extract .` to regenerate .graphify_analysis.json."
    )]
    StaleWikiCommunities,
    #[error("wiki filesystem error at {path}: {source}")]
    WikiIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}
