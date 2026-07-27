//! Deterministic behavior summaries over merged Program IR.

mod call_graph;
mod invalidation;
mod summary;
mod universal_call_graph;

pub use invalidation::affected_summaries;
pub use summary::{AnalysisBundle, AnalysisError, FunctionSummary, analyze, analyze_prevalidated};

pub const ANALYSIS_SCHEMA_VERSION: u32 = 1;
pub const ANALYZER_VERSION: u32 = 1;
pub use call_graph::{
    CALL_GRAPH_SCHEMA, CallContinuation, CallEdge, CallGraphDirection, CallGraphError,
    CallGraphRequest, CallGraphResponse, CallGraphRoot, CallNode, CallResolution, CallSite,
    build_call_graph,
};
pub use universal_call_graph::{
    UNIVERSAL_CALL_GRAPH_SCHEMA, UniversalCallContinuation, UniversalCallCoverage,
    UniversalCallEdge, UniversalCallGraphError, UniversalCallGraphRequest,
    UniversalCallGraphResponse, UniversalCallGraphRoot, UniversalCallNode, UniversalCallSite,
    build_universal_call_graph,
};
