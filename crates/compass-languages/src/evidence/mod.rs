mod build;
mod model;
mod validate;

pub(crate) use build::extract_tree_evidence;
pub use build::{EvidenceBuilder, range_for_node};
pub use model::{
    AdapterIdentity, BindingFact, BindingKind, CandidateRelation, DeclarationFact,
    EvidenceDiagnostic, EvidenceRange, LanguageCapability, OccurrenceFact, RelationshipCandidate,
    ResolutionConstraint, ScopeFact, SemanticEvidenceBatch, SemanticRole,
};
pub use validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits, validate_evidence};
