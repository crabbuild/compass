mod build;
mod csharp;
mod dart;
mod documentation;
mod groovy;
mod kotlin;
mod model;
mod php;
mod ruby;
mod scala;
mod shared;
mod swift;
mod typescript;
mod validate;

pub const UNIVERSAL_EVIDENCE_SCHEMA: &str = "compass.languages.evidence/2";

pub(crate) use build::extract_tree_evidence;
pub use build::{EvidenceBuilder, range_for_node};
pub use model::{
    BindingFact, BindingKind, CandidateRelation, DECLARATION_DOCUMENTATION_OMISSION_CODE,
    DeclarationDocumentationFact, DeclarationDocumentationKind, DeclarationFact,
    EvidenceDiagnostic, EvidenceRange, HierarchyConstraint, LanguageCapability,
    MAX_BATCH_DOCUMENTATION_BYTES, MAX_DECLARATION_DOCUMENTATION_BYTES, OccurrenceFact,
    ReceiverDispatchStrategy, RelationshipCandidate, ResolutionConstraint, ScopeFact,
    SemanticEvidenceBatch, SemanticRole, SymbolNamespace, UniversalEvidenceIdentity,
};
pub use validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits, validate_evidence};
