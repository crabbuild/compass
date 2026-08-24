//! Statically linked deterministic language extraction for Compass.

mod apex;
mod bash;
mod builtins;
mod config;
mod cpp;
mod dart_framework;
mod dm;
mod dotnet_project;
mod elixir;
mod engine;
pub mod evidence;
mod evidence_pipeline;
mod facts;
mod fortran;
pub mod frameworks;

/// Version of the extraction contract consumed by graph publication.
pub const EXTRACTION_SEMANTICS_VERSION: &str = "compass.languages.extraction/3";
mod go;
mod html;
mod ids;
mod json_config;
mod julia;
mod markdown;
mod mcp;
mod objc;
mod package_manifest;
mod pascal;
mod pascal_forms;
mod php;
mod powershell;
mod program;
mod project_evidence;
mod r;
mod registry;
mod scip;
mod semantic;
mod sql;
mod templates;
mod terraform;
mod verilog;
mod xaml;
mod zig;

#[doc(hidden)]
pub use builtins::{is_language_builtin_global, is_language_builtin_qualified_target};
pub use evidence::{
    BindingFact, BindingKind, CandidateRelation, DeclarationFact, EvidenceBuilder,
    EvidenceDiagnostic, EvidenceError, EvidenceErrorCode, EvidenceLimits, EvidenceRange,
    HierarchyConstraint, LanguageCapability, OccurrenceFact, ReceiverDispatchStrategy,
    RelationshipCandidate, ResolutionConstraint, ScopeFact, SemanticEvidenceBatch, SemanticRole,
    SymbolNamespace, UNIVERSAL_EVIDENCE_SCHEMA, UniversalEvidenceIdentity, range_for_node,
    validate_evidence,
};
pub use evidence_pipeline::{
    UniversalEvidencePipeline, UniversalEvidenceProducer, UniversalEvidenceQualification,
    UniversalEvidenceRegistry, UniversalEvidenceRegistryError,
};
pub use facts::{Extraction, RawCall, RawEdgeRecord, RawNodeRecord};
pub use frameworks::{
    FRAMEWORK_PACK_SEMANTICS_VERSION, FrameworkCapability, FrameworkLimitError, FrameworkLimits,
    FrameworkManifestPolicy, FrameworkOccurrencePolicy, FrameworkPackDescriptor, FrameworkPackKind,
    FrameworkPackRegistry, FrameworkPackRegistryError, FrameworkRelation, RawDomainFact,
    RawFrameworkAnchor, RawFrameworkAnnotationFact, RawFrameworkConfigurationFact,
    RawFrameworkFact, RawFrameworkFileSetFact, RawFrameworkOrigin, RawFrameworkRelationFact,
    RawFrameworkRoleFact, RawRouteFact, RawRouteStageFact, RawRouteStageRole,
    framework_pack_semantics_version, framework_semantics_digest,
};
pub use html::{HtmlError, HtmlNormalization, normalize_html};
pub use ids::{file_stem, make_id, normalize_id};
pub use json_config::parse_jsonc;
pub use program::{TREE_SITTER_PROGRAM_PROVIDER_VERSION, TreeSitterSyntaxProvider};
pub use project_evidence::{
    ComposerAutoloadRoot, FRAMEWORK_PROJECT_EVIDENCE_EXTENSION, ProjectEvidence,
    ProjectEvidenceDiagnostic, ProjectEvidenceIndex,
};
pub use registry::{ExtractorKind, LanguageSpec, Registry};
pub use scip::{ScipExtraction, ingest_scip_json};

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct CombinedExtraction {
    pub graph: Extraction,
    pub program: Option<compass_program::EvidenceBatch>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("unsupported deterministic source format: {0}")]
    Unsupported(PathBuf),
    #[error("grammar {language} is not statically linked: {detail}")]
    MissingGrammar { language: String, detail: String },
    #[error("parser returned no syntax tree for {0}")]
    ParseCancelled(PathBuf),
    #[error("invalid program evidence for {path}: {detail}")]
    InvalidProgramEvidence { path: PathBuf, detail: String },
    #[error("invalid prepared document evidence for {path}: {detail}")]
    InvalidDocumentEvidence { path: PathBuf, detail: String },
    #[error(transparent)]
    File(#[from] compass_files::FileError),
}
pub use engine::Engine;

/// Internal extraction-quality marker consumed by the v1 publication pipeline.
///
/// This is serialized with cached extraction facts so parser recovery remains
/// truthful on warm builds.
pub const EXTRACTION_QUALITY_EXTENSION: &str = "_compass_extraction_quality";
pub const EXTRACTION_QUALITY_PARTIAL: &str = "partial";
pub const EXTRACTION_QUALITY_REASON_EXTENSION: &str = "_compass_extraction_quality_reason";

/// Extract deterministic SQL facts from in-memory content.
///
/// Live schema introspectors use a virtual path so credentials and temporary
/// files never enter the graph.
#[must_use]
pub fn extract_sql_content(path: &std::path::Path, content: &[u8]) -> Extraction {
    let mut extraction = sql::extract(path, content);
    engine::stamp_producer_metadata(&mut extraction, "sql");
    extraction
}
