//! Program evidence provider contracts and deterministic reconciliation.

mod evidence;
mod merge;
mod path;
mod provider;

pub use evidence::{
    EvidenceBatch, EvidenceFact, FactKind, Role, coverage_with, evidence_id, evidence_record,
};
pub use merge::{MERGER_VERSION, MergeError, merge_evidence};
pub use path::normalize_source_path;
pub use provider::{
    ArtifactInput, ArtifactLimits, ArtifactManifest, ArtifactProvider, ArtifactReader, FileInput,
    ProjectAnalyzer, ProjectFile, ProjectInput, ProviderError, SyntaxProvider,
};
