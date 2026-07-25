//! Deterministic behavior summaries over merged Program IR.

mod invalidation;
mod summary;

pub use invalidation::affected_summaries;
pub use summary::{AnalysisBundle, AnalysisError, FunctionSummary, analyze};

pub const ANALYSIS_SCHEMA_VERSION: u32 = 2;
pub const ANALYZER_VERSION: u32 = 2;
