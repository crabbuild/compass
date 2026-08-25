//! Program evidence provider contracts and deterministic reconciliation.

mod evidence;
mod manifest;
mod merge;
mod path;
mod projection;
mod provider;
mod scip;
mod scip_stream;

pub use evidence::{
    EvidenceBatch, EvidenceFact, FactKind, Role, coverage_with, evidence_id, evidence_record,
};
pub use manifest::{
    MANAGED_ANALYZER_PROFILE_SCHEMA, SCIP_MANIFEST_SCHEMA, managed_analyzer_profile_digest,
    parse_artifact_manifest, source_inventory_digest,
};
pub use merge::{MERGER_VERSION, MergeError, merge_evidence};
pub use path::normalize_source_path;
pub use projection::{CompilerCall, CompilerDefinition, CompilerProjection, compiler_projection};
pub use provider::{
    ArtifactInput, ArtifactLimits, ArtifactManifest, ArtifactProvider, ArtifactReader, FileInput,
    ManagedAnalyzerPermissions, ManagedAnalyzerProfile, ManagedAnalyzerState,
    ManagedPythonEnvironment, ManagedPythonPackage, ProjectAnalyzer, ProjectFile, ProjectInput,
    ProviderError, PythonNamespacePolicy, SyntaxProvider,
};
pub use scip::{
    DecodedScipArtifact, DecodedScipDocument, OfficialScipProvider, SCIP_PROVIDER_VERSION,
};
