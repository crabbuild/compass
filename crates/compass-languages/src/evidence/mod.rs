mod build;
mod csharp;
mod kotlin;
mod model;
mod php;
mod ruby;
mod typescript;
mod validate;

pub const UNIVERSAL_EVIDENCE_SCHEMA: &str = "compass.languages.evidence/2";

pub(crate) use build::extract_tree_evidence;
pub use build::{EvidenceBuilder, range_for_node};
pub use model::{
    BindingFact, BindingKind, CandidateRelation, DeclarationFact, EvidenceDiagnostic,
    EvidenceRange, HierarchyConstraint, LanguageCapability, OccurrenceFact,
    ReceiverDispatchStrategy, RelationshipCandidate, ResolutionConstraint, ScopeFact,
    SemanticEvidenceBatch, SemanticRole, SymbolNamespace, UniversalEvidenceIdentity,
};
pub use validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits, validate_evidence};
