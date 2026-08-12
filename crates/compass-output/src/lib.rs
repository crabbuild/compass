//! Safe, deterministic output formats for Compass graphs.

mod backup;
mod callflow;
mod callflow_model;
mod canvas;
mod cql;
mod cypher;
mod graphml;
mod history_bundle;
mod history_viewer;
mod html;
mod json;
mod obsidian;
mod report;
mod review;
mod svg;
mod tree;
mod viewer_model;
mod wiki;

pub use backup::{BackupResult, backup_if_protected, backup_if_protected_to};
pub use callflow::{
    CallflowExport, CallflowOptions, CallflowSection, callflow_html_document,
    derive_callflow_sections, write_callflow_html,
};
pub use callflow_model::{
    CALLFLOW_VIEWER_SCHEMA, CallflowCoverage, CallflowCrossSectionCall, CallflowProvenance,
    CallflowSourceScope, CallflowStatistics, CallflowViewEdge, CallflowViewLink, CallflowViewModel,
    CallflowViewNode, CallflowViewSection, callflow_view_model,
};
pub use canvas::{CanvasOptions, canvas_document, write_canvas};
pub use cql::{render_cql_json, render_cql_jsonl, render_cql_table};
pub use cypher::{cypher_document, write_cypher};
pub use graphml::{graphml_document, write_graphml};
pub use history_bundle::{
    DerivedArtifactRequest, HistoricalPublicationEvidence, HistoryBundleInput,
    SUPPORTED_HISTORY_RENDERER, publish_history_bundle,
};
pub use history_viewer::{HistoricalViewError, historical_graph_document, historical_view_model};
pub use html::{
    HtmlOptions, HtmlRender, graph_community_view_model_document, graph_view_model_document,
    html_document, write_html,
};
pub use json::{JsonExportOptions, export_json_value, write_json};
pub use obsidian::{ObsidianExport, ObsidianOptions, export_obsidian, node_filenames};
pub use report::{
    AgentOrientation, BoundedCoverage, DetectionSummary, FreshnessBasis, FreshnessStatus,
    ORIENTATION_MARKDOWN_MAX_CHARS, ORIENTATION_SCHEMA, OrientationAmbiguousEdge,
    OrientationCommunity, OrientationCommunityLink, OrientationConnection, OrientationCycle,
    OrientationDetails, OrientationEvidenceStatus, OrientationGraphSummary, OrientationHealth,
    OrientationHub, OrientationHyperedge, OrientationLearnedQuestion, OrientationNodeReference,
    OrientationOmissions, OrientationPublicationDiagnostic, OrientationQuery, OrientationRisk,
    OrientationSourceAnchor, OrientationWorkMemory, PublicationStatus, REPORT_MARKDOWN_MAX_CHARS,
    ReportOptions, SectionOmission, TokenCost, WorkingTreeState, agent_orientation,
    generate_report, graph_artifact_identity, render_agent_report_markdown,
    render_orientation_json, render_orientation_markdown, validate_orientation_graph_identity,
};
pub use review::{
    MAX_REVIEW_RENDER_BYTES, RenderedReview, render_readiness_json, render_readiness_markdown,
    render_review_json, render_review_markdown, render_review_markdown_bounded,
    render_review_sarif, render_review_text,
};
pub use svg::{SvgOptions, spring_layout, svg_document, write_svg};
pub use tree::{TreeNode, TreeOptions, build_tree, tree_html_document, write_tree_html};
pub use viewer_model::{
    GRAPH_VIEWER_SCHEMA, GraphViewCommunity, GraphViewEdge, GraphViewModel, GraphViewNode,
    GraphViewSource, GraphViewStats, graph_view_model, shared_viewer_html,
    shared_viewer_html_with_communities,
};
pub use wiki::{WikiExport, WikiOptions, export_wiki};

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error("could not serialize output: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("orientation Markdown is {rendered_chars} characters; limit is {limit}")]
    OrientationBudgetExceeded { rendered_chars: usize, limit: usize },
    #[error("graph report Markdown is {rendered_chars} characters; limit is {limit}")]
    ReportBudgetExceeded { rendered_chars: usize, limit: usize },
    #[error("PR review output is {rendered_bytes} bytes; limit is {limit}")]
    ReviewBudgetExceeded { rendered_bytes: usize, limit: usize },
    #[error("invalid PR review output: {0}")]
    InvalidReview(String),
    #[error(transparent)]
    Review(#[from] compass_pr_intelligence::PrIntelligenceError),
    #[error("invalid orientation model: {reason}")]
    InvalidOrientationModel { reason: &'static str },
    #[error(transparent)]
    File(#[from] compass_files::FileError),
    #[error("existing graph is non-empty but malformed: {0}")]
    MalformedGraph(std::path::PathBuf),
    #[error("refusing to shrink graph from {existing} nodes to {new}; use force to override")]
    ShrinkRefused { existing: usize, new: usize },
    #[error("invalid Obsidian output path: {0}")]
    InvalidObsidianPath(std::path::PathBuf),
    #[error(
        "graph has {nodes} nodes - too large for HTML viz (limit: {limit}). Use --no-viz, raise COMPASS_VIZ_NODE_LIMIT, or reduce input size."
    )]
    HtmlTooLarge { nodes: usize, limit: isize },
    #[error("community {community} does not exist in this graph")]
    UnknownCommunity { community: usize },
    #[error(
        "community {community} is incomplete because {missing} declared member nodes are missing from the graph; rebuild the graph before exporting this community"
    )]
    IncompleteCommunity { community: usize, missing: usize },
    #[error(
        "community {community} has {nodes} nodes, exceeding the detail limit of {limit}; increase the graph node limit to explore it"
    )]
    CommunityTooLarge {
        community: usize,
        nodes: usize,
        limit: isize,
    },
    #[error("graph.json contains 0 nodes")]
    EmptyCallflowGraph,
    #[error("no sections defined")]
    NoCallflowSections,
    #[error(
        "communities dict is empty — refusing to clear wiki/. Run `compass extract .` or `compass cluster-only .` first."
    )]
    EmptyWikiCommunities,
    #[error(
        "all community node IDs are stale — none exist in the graph. Re-run `compass extract .` to regenerate analysis.json."
    )]
    StaleWikiCommunities,
    #[error("unsupported history renderer {version} for {path}")]
    UnsupportedHistoryRenderer { path: String, version: String },
    #[error("history bundle destination already exists: {0}")]
    HistoryBundleExists(std::path::PathBuf),
    #[error("history bundle contains an unsafe path: {0}")]
    UnsafeHistoryPath(String),
    #[error("history bundle I/O failed at {path}: {source}")]
    HistoryBundleIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("history bundle validation failed: {0}")]
    InvalidHistoryBundle(String),
    #[error("wiki filesystem error at {path}: {source}")]
    WikiIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}
