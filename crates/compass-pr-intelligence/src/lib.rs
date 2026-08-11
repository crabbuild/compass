//! Deterministic, evidence-qualified pull-request intelligence.
//!
//! This crate is deliberately pure: callers provide immutable revision,
//! snapshot, and semantic-diff evidence. It performs no filesystem, Git,
//! network, provider, or presentation work.

mod analyze;
mod canonical;
mod error;
mod model;

pub use analyze::analyze;
pub use canonical::{canonical_json_bytes, evidence_manifest_digest, report_digest};
pub use error::PrIntelligenceError;
pub use model::{
    AdvisoryRisk, ChangeHunk, ChangeRequest, Completeness, Confidence, EvidenceManifest,
    EvidenceRepository, EvidenceSource, Finding, FindingType, Freshness, GateResult, GateState,
    GraphSnapshot, Location, MergeOutcome, Omission, PullRequestReport, REPORT_SCHEMA,
    RUBRIC_VERSION, ReportIdentity, RepositoryIdentity, RevisionSet, RiskBand, RiskFactor,
    RiskFactorKind, SourceRange, VerificationPlan, VerificationState, WitnessHop,
};

/// Stable fingerprint schema for canonical findings.
pub const FINDING_FINGERPRINT_SCHEMA: &str = "cmpprv1";

/// Upper bounds enforced before a report is constructed.
pub const MAX_CHANGE_HUNKS: usize = 100_000;
pub const MAX_EVIDENCE_REPOSITORIES: usize = 10_000;
pub const MAX_EVIDENCE_SOURCES: usize = 50_000;
pub const MAX_ENTITIES_PER_FINDING: usize = 50_000;
pub const MAX_FINDINGS: usize = 5_000;
pub const MAX_LOCATIONS_PER_FINDING: usize = 50_000;
pub const MAX_TESTS_PER_FINDING: usize = 50_000;
pub const MAX_WITNESS_HOPS: usize = 64;
pub const MAX_STRING_BYTES: usize = 16 * 1024;
