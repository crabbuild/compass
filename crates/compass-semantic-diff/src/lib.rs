//! Deterministic, evidence-gated semantic differences between Compass realizations.

mod engine;
mod error;
mod history;
mod logic;
mod model;
mod topology;
mod verification;

pub use engine::{compare, finding_id};
pub use error::SemanticDiffError;
pub use history::compare_history_realizations;
pub use model::{
    AffectedConsumer, CLASSIFIER_VERSION, ChangeDirection, CollapsedGroup, Comparison,
    Compatibility, Completeness, Confidence, DependencyDelta, DependencyTopology, EvidenceRef,
    FeatureGroup, FindingOrigin, FindingType, GraphDelta, GraphEdgeDelta, GraphNodeDelta,
    MAX_DIRECT_ENTITIES, MAX_EVIDENCE_PER_FINDING, MAX_FINDINGS, MAX_IMPACT_DEPTH,
    MAX_TRAVERSED_CALL_EDGES, NoTestEvidence, REPORT_SCHEMA, SemanticDiffInput, SemanticDiffReport,
    SemanticFinding, SnapshotIdentity, SnapshotReader, SnapshotSide, TestEvidence,
    TestEvidenceProvider, Verification, VerificationState, WitnessHop, WitnessPath,
};
pub use topology::{DependencyCycleIndex, dependency_participates_in_cycle};
pub use verification::StaticTestEvidence;

/// Included in derived-cache keys. Increment whenever comparison semantics change.
pub const ENGINE_VERSION: u32 = 2;
