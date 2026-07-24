//! Deterministic, evidence-gated semantic differences between Compass realizations.

mod engine;
mod error;
mod model;
mod verification;

pub use engine::{compare, finding_id};
pub use error::SemanticDiffError;
pub use model::{
    AffectedConsumer, ChangeDirection, CollapsedGroup, Comparison, Compatibility, Completeness,
    Confidence, DependencyDelta, EvidenceRef, FindingOrigin, FindingType, MAX_DIRECT_ENTITIES,
    MAX_EVIDENCE_PER_FINDING, MAX_FINDINGS, MAX_IMPACT_DEPTH, MAX_TRAVERSED_CALL_EDGES,
    NoTestEvidence, REPORT_SCHEMA, SemanticDiffInput, SemanticDiffReport, SemanticFinding,
    SnapshotIdentity, SnapshotReader, SnapshotSide, TestEvidence, TestEvidenceProvider,
    Verification, VerificationState, WitnessHop, WitnessPath,
};
pub use verification::StaticTestEvidence;
