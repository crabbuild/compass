mod model;
mod validate;

pub use model::{
    AdapterIdentity, BindingFact, BindingKind, CandidateRelation, DeclarationFact,
    EvidenceDiagnostic, EvidenceRange, LanguageCapability, OccurrenceFact, RelationshipCandidate,
    ResolutionConstraint, ScopeFact, SemanticEvidenceBatch, SemanticRole,
};
pub use validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits, validate_evidence};
