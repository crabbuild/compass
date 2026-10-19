mod build;
mod csharp;
mod dart;
mod documentation;
mod groovy;
mod kotlin;
mod merge_dependencies;
mod model;
mod php;
mod ruby;
mod scala;
mod shared;
mod swift;
mod type_arguments;
mod typescript;
mod validate;

pub const UNIVERSAL_EVIDENCE_SCHEMA: &str = "compass.languages.evidence/2";

pub(crate) use build::extract_tree_evidence;
pub use build::{EvidenceBuilder, range_for_node};
#[doc(hidden)]
pub use merge_dependencies::TypeScriptMergeDependencyUsage;
pub use model::{
    BindingFact, BindingKind, CandidateRelation, DECLARATION_DOCUMENTATION_OMISSION_CODE,
    DeclarationDocumentationFact, DeclarationDocumentationKind, DeclarationFact,
    ECMASCRIPT_EXPORT_INVENTORY_VERSION, EcmaScriptExportInventory, EvidenceDiagnostic,
    EvidenceRange, HierarchyConstraint, LanguageCapability, MAX_BATCH_DOCUMENTATION_BYTES,
    MAX_BATCH_TYPESCRIPT_TYPE_ARGUMENT_BYTES, MAX_BATCH_TYPESCRIPT_TYPE_ARGUMENT_NODES,
    MAX_DECLARATION_DOCUMENTATION_BYTES, MAX_TYPESCRIPT_MERGE_DEPENDENCIES,
    MAX_TYPESCRIPT_MERGE_DEPENDENCY_SPECIFIER_BYTES, MAX_TYPESCRIPT_MERGE_DEPENDENCY_TEXT_BYTES,
    MAX_TYPESCRIPT_TYPE_ARGUMENT_ARITY, MAX_TYPESCRIPT_TYPE_ARGUMENT_BYTES,
    MAX_TYPESCRIPT_TYPE_ARGUMENT_DEPTH, MAX_TYPESCRIPT_TYPE_ARGUMENT_NODES,
    MAX_TYPESCRIPT_TYPE_ARGUMENT_REFERENCE_BYTES, OccurrenceFact, ReceiverDispatchStrategy,
    RelationshipCandidate, ResolutionConstraint, ScopeFact, SemanticEvidenceBatch, SemanticRole,
    SymbolNamespace, TYPESCRIPT_MERGE_INVENTORY_VERSION, TypeScriptMergeDependency,
    TypeScriptMergeDependencyKind, TypeScriptTypeArgument, TypeScriptTypeArguments,
    UniversalEvidenceIdentity,
};
#[doc(hidden)]
pub use type_arguments::TypeScriptArgumentUsage;
pub use validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits, validate_evidence};
