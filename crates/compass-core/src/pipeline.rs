use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use compass_files::{
    BuildGuard, BuildScope, Cache, CacheKind, CacheOptions, DetectOptions, Detection, IgnorePolicy,
    Manifest, ManifestKind, detect, write_json_atomic, write_text_atomic,
};
use compass_graph::{
    ClusterOptions, EntityTiebreaker, GRAPH_DIAGNOSTICS_EXTENSION, InventoryEvidence,
    PublicationOmissions, PublicationOutcome, build_owned_with_tiebreaker as build_document,
    canonical_edge_kind, canonical_raw_edge_sites, cluster, dedupe_nodes, extraction_from_v1,
    graph_insights, label_communities_by_hub, normalize_document_v1_with_inventory_best_effort,
    normalize_document_v1_with_inventory_best_effort_owned, remap_communities_to_previous,
    score_communities,
};
use compass_languages::{
    EXTRACTION_QUALITY_EXTENSION, EXTRACTION_QUALITY_PARTIAL, EXTRACTION_QUALITY_REASON_EXTENSION,
    Engine, Extraction, ExtractorKind, FRAMEWORK_PROJECT_EVIDENCE_EXTENSION, ProjectEvidenceIndex,
    RawEdgeRecord, RawFrameworkFact, RawNodeRecord, Registry, file_stem, make_id,
};
use compass_model::code_graph::{
    DiagnosticSeverity, ExtractionStatus, GraphDiagnostic, GraphDocument as V1GraphDocument,
    NodeKind,
};
use compass_model::provenance::{
    COALESCED_NODE_EVIDENCE_ATTRIBUTE, CONSUME_INCREMENTAL_ENDPOINT_REMAP_ATTRIBUTE,
    EndpointRewriteEvidence, EndpointRewriteRule, EvidenceOrigin, OCCURRENCE_RULE_ATTRIBUTE,
    Provenance, SEMANTIC_LAYER_EXTRACTOR, SourceAnchor, TRUSTED_EDGE_RECORD_ATTRIBUTE,
    append_endpoint_rewrite_evidence,
};
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use compass_output::{
    DetectionSummary, HtmlOptions, OutputError, ReportOptions, TokenCost, generate_report,
    graph_view_model_document, write_html,
};
use compass_resolve::{
    apply_program_projection, collect_program_projection_sites, merge_decl_def_classes,
    resolve_owned_with_root,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::build_state::{
    ArtifactSeal, BUILD_STATE_FILE, BuildProfile, BuildState, SavedStats, load_verified,
};
use crate::program::{
    PreparedSyntaxInput, ProgramBuild, build_program, load_current_program, program_artifact_count,
    write_program,
};
use crate::raw_guard::enforce_incomplete_raw_guard;

/// Default upper bound for one source file entering the parser pipeline.
///
/// Syntax and semantic extractors can retain several multiples of the source
/// size while building records. Callers may raise this bound explicitly for
/// repositories with intentionally large generated sources.
pub const DEFAULT_MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const SEMANTIC_MARKER_FILE: &str = ".compass_semantic_marker";

#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub root: PathBuf,
    pub scan_filesystem: bool,
    pub output_root: Option<PathBuf>,
    /// Explicit repository-private cache root used by exact history builds.
    pub cache_root: Option<PathBuf>,
    pub force: bool,
    /// Reuse validated AST and Program cache entries while retaining force
    /// authorization for output replacement.
    pub reuse_cache_on_force: bool,
    pub no_cluster: bool,
    pub no_viz: bool,
    pub gitignore: bool,
    pub ignore_policy: IgnorePolicy,
    pub extra_excludes: Vec<String>,
    pub scope: BuildScope,
    pub resolution: f64,
    pub exclude_hubs: Option<f64>,
    pub google_workspace: bool,
    /// Enable deterministic Program IR analysis and `program.json` output.
    pub program_analysis: bool,
    /// Explicit offline program evidence artifacts, in addition to `index.scip`.
    pub program_artifacts: Vec<PathBuf>,
    /// Resource limits for offline program artifacts.
    pub program_artifact_limits: compass_program::ArtifactLimits,
    /// Maximum number of worker threads used by the deterministic AST stages.
    /// `None` uses the host CPU count in a build-local Rayon pool.
    pub max_workers: Option<usize>,
    /// Maximum size of one source file admitted to AST and Program analysis.
    ///
    /// Oversized files remain in the inventory with partial coverage and an
    /// explicit reason; they are not read into memory.
    pub max_source_bytes: u64,
    /// Override the commit recorded in update artifacts.
    ///
    /// This is primarily useful for reproducible builds and compatibility
    /// tests whose oracle and native halves must share one source revision.
    pub built_at_commit: Option<String>,
    pub purpose: BuildPurpose,
    /// Detection already validated by an init preview.
    pub precomputed_detection: Option<Detection>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BuildPurpose {
    #[default]
    Update,
    Extract,
}

const OUTPUT_STATS_FILE: &str = ".compass_output_stats.json";
const GRAPH_OVERVIEW_FILE: &str = "graph-overview.json";
const GRAPH_OVERVIEW_SCHEMA: &str = "compass.graph-overview/1";
const GRAPH_OVERVIEW_NODE_LIMIT: isize = 5_000;
const MAX_INCREMENTAL_REMAP_DIAGNOSTICS: usize = 100;
const INCREMENTAL_REMAP_DROP_DIAGNOSTIC: &str = "dropped_incremental_remap_without_wiring_site";
const INCREMENTAL_REMAP_TRUNCATION_DIAGNOSTIC: &str =
    "incremental_remap_without_wiring_site_truncated";
#[derive(Clone, Debug, Deserialize, Serialize)]
struct OutputStats {
    graph_bytes: u64,
    nodes: usize,
    edges: usize,
    communities: usize,
    clustered: bool,
    #[serde(default)]
    omitted_nodes: usize,
    #[serde(default)]
    omitted_edges: usize,
    #[serde(default)]
    identity_collisions: usize,
}

impl OutputStats {
    const fn omissions(&self) -> PublicationOmissions {
        PublicationOmissions {
            nodes: self.omitted_nodes,
            edges: self.omitted_edges,
            identity_collisions: self.identity_collisions,
            examples_omitted: 0,
        }
    }
}

impl BuildOptions {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            scan_filesystem: true,
            output_root: None,
            cache_root: None,
            force: false,
            reuse_cache_on_force: false,
            no_cluster: false,
            no_viz: false,
            gitignore: true,
            ignore_policy: IgnorePolicy::CurrentCheckout,
            extra_excludes: Vec::new(),
            scope: BuildScope::default(),
            resolution: 1.0,
            exclude_hubs: None,
            google_workspace: false,
            program_analysis: false,
            program_artifacts: Vec::new(),
            program_artifact_limits: compass_program::ArtifactLimits::default(),
            // Large builds use a local host-sized pool. Keeping this unset also
            // lets CLI callers provide an explicit memory/throughput bound.
            max_workers: None,
            max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
            built_at_commit: None,
            purpose: BuildPurpose::Update,
            precomputed_detection: None,
        }
    }
}

const fn cache_reuse_enabled(force: bool, reuse_cache_on_force: bool) -> bool {
    !force || reuse_cache_on_force
}

const fn prior_published_graph_input_enabled(force: bool) -> bool {
    !force
}

fn prior_semantic_layer_required(read_prior_published_graph: bool, output_dir: &Path) -> bool {
    read_prior_published_graph && output_dir.join(SEMANTIC_MARKER_FILE).is_file()
}

#[derive(Clone, Debug)]
pub struct BuildResult {
    pub root: PathBuf,
    pub output_dir: PathBuf,
    pub detection: Detection,
    pub files_considered: usize,
    pub files_extracted: usize,
    pub files_cached: usize,
    pub empty_files: Vec<PathBuf>,
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    pub omitted_nodes: usize,
    pub omitted_edges: usize,
    pub identity_collisions: usize,
    pub partial_graph: bool,
    pub html_written: bool,
    pub outputs_changed: bool,
    pub program_modules: usize,
    pub program_summaries: usize,
    pub program_syntax_analyzed: usize,
    pub program_syntax_reused: usize,
    pub program_artifacts_loaded: usize,
    pub program_artifacts_reused: usize,
    pub program_artifact_documents_analyzed: usize,
    pub program_artifact_documents_reused: usize,
    pub program_conflicts: usize,
    pub timings: BuildTimings,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildFileProgress {
    pub current: usize,
    pub total: usize,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BuildTimings {
    pub detect: Duration,
    pub deterministic_extract: Duration,
    pub graph_assembly: Duration,
    pub program_analysis: Duration,
    pub publish: Duration,
}

/// Validated semantic output to merge into one atomic graph build.
///
/// `refreshed_files` is the exact set dispatched for this run. Existing
/// semantic facts owned by those sources are removed before the replacement
/// fragment is appended. Partial or uncovered files remain unstamped so the
/// next incremental run retries them.
#[derive(Clone, Debug)]
pub struct SemanticLayer {
    pub fragment: serde_json::Value,
    pub refreshed_files: Vec<PathBuf>,
    pub partial_files: Vec<PathBuf>,
    pub allow_partial: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    File(#[from] compass_files::FileError),
    #[error(transparent)]
    Extract(#[from] compass_languages::ExtractError),
    #[error(transparent)]
    Graph(#[from] compass_model::GraphError),
    #[error(transparent)]
    Dedup(#[from] compass_graph::DedupError),
    #[error(transparent)]
    Output(#[from] compass_output::OutputError),
    #[error("invalid cached AST extraction for {path}: {source}")]
    InvalidCache {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not serialize AST extraction for {path}: {source}")]
    SerializeExtraction {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("scan root does not exist: {0}")]
    MissingRoot(PathBuf),
    #[error("graph is empty — deterministic extraction produced no nodes")]
    EmptyGraph,
    #[error("diagnostic input must be a JSON object")]
    InvalidDiagnostic,
    #[error("{0}")]
    DiagnosticFile(String),
    #[error("invalid semantic extraction fragment: {0}")]
    InvalidSemanticFragment(serde_json::Error),
    #[error("invalid supplemental extraction fragment: {0}")]
    InvalidSupplementalFragment(serde_json::Error),
    #[error("could not create an AST worker pool: {0}")]
    WorkerPool(String),
    #[error("build worker panicked during {0}")]
    WorkerPanic(String),
    #[error(
        "semantic extraction was incomplete and would shrink the graph ({new} < {existing} nodes)"
    )]
    IncompleteSemanticShrink { existing: usize, new: usize },
    #[error("semantic extraction was incomplete and the existing graph is unreadable: {0}")]
    IncompleteSemanticExisting(PathBuf),
    #[error(transparent)]
    ProgramProvider(#[from] compass_program::ProviderError),
    #[error(transparent)]
    ProgramMerge(#[from] compass_program::MergeError),
    #[error(transparent)]
    ProgramAnalysis(#[from] compass_analysis::AnalysisError),
    #[error(transparent)]
    ProgramIr(#[from] compass_ir::IrError),
    #[error("invalid Program IR input: {0}")]
    InvalidProgramInput(String),
    #[error("invalid completed-build state: {0}")]
    InvalidBuildState(String),
    #[error("precomputed detection root does not match build root")]
    DetectionRootMismatch,
}

/// Run the complete deterministic local graph pipeline without invoking Python,
/// an LLM, a network service, or a dynamically installed grammar.
pub fn build_local_graph(options: &BuildOptions) -> Result<BuildResult, CoreError> {
    build_graph(options, None, &[], None, None)
}

/// Merge a completed semantic provider result into the native graph pipeline.
pub fn build_graph_with_semantic(
    options: &BuildOptions,
    semantic: &SemanticLayer,
) -> Result<BuildResult, CoreError> {
    build_graph(options, Some(semantic), &[], None, None)
}

/// Merge deterministic supplemental facts, such as Cargo or database schema
/// introspection, into the same atomic native graph build.
pub fn build_graph_with_layers(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[serde_json::Value],
) -> Result<BuildResult, CoreError> {
    let supplemental = supplemental
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<Extraction>, _>>()
        .map_err(CoreError::InvalidSupplementalFragment)?;
    build_graph(options, semantic, &supplemental, None, None)
}

pub fn build_graph_with_layers_and_progress(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[serde_json::Value],
    progress: &(dyn Fn(BuildFileProgress) + Sync),
) -> Result<BuildResult, CoreError> {
    let supplemental = supplemental
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<Extraction>, _>>()
        .map_err(CoreError::InvalidSupplementalFragment)?;
    build_graph(options, semantic, &supplemental, None, Some(progress))
}

pub fn build_graph_with_layers_and_tiebreaker(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[serde_json::Value],
    tiebreaker: &mut dyn EntityTiebreaker,
) -> Result<BuildResult, CoreError> {
    let supplemental = supplemental
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<Extraction>, _>>()
        .map_err(CoreError::InvalidSupplementalFragment)?;
    build_graph(options, semantic, &supplemental, Some(tiebreaker), None)
}

fn build_graph(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[Extraction],
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
) -> Result<BuildResult, CoreError> {
    build_graph_inner(options, semantic, supplemental, tiebreaker, progress)
}

fn build_graph_inner(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[Extraction],
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
) -> Result<BuildResult, CoreError> {
    let mut timings = BuildTimings::default();
    let mut stage_started = Instant::now();
    if !options.root.exists() {
        return Err(CoreError::MissingRoot(options.root.clone()));
    }
    let root = fs::canonicalize(&options.root).map_err(|source| compass_files::FileError::Io {
        path: options.root.clone(),
        source,
    })?;
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let output_root = options
        .output_root
        .as_deref()
        .map_or_else(|| root.clone(), absolutize);
    let reuse_cached_analysis = cache_reuse_enabled(options.force, options.reuse_cache_on_force);
    let read_prior_published_graph = prior_published_graph_input_enabled(options.force);
    let output_container = output_root.join(&output_name);
    fs::create_dir_all(&output_container).map_err(|source| compass_files::FileError::Io {
        path: output_container.clone(),
        source,
    })?;
    let prior_build_complete = BuildGuard::ensure_complete(&output_container).is_ok();
    let guard = BuildGuard::begin(&output_container)?;
    let output_dir = guard.staging_directory().to_path_buf();
    if !options.program_analysis {
        remove_if_exists(&output_dir.join("program.json"))?;
    }
    if options.force || !prior_build_complete {
        remove_if_exists(&output_dir.join(BUILD_STATE_FILE))?;
    }
    let manifest_path = output_dir.join("manifest.json");
    let prior_manifest = Manifest::load(&manifest_path, Some(&root));
    let detect_options = DetectOptions {
        scan_filesystem: options.scan_filesystem,
        gitignore: options.gitignore,
        ignore_policy: options.ignore_policy,
        extra_excludes: options.extra_excludes.clone(),
        scope: options.scope.clone(),
        output_name: output_name.clone(),
        cache_root: Some(output_root.clone()),
        ..DetectOptions::default()
    };
    let mut detection = if let Some(detection) = options.precomputed_detection.clone() {
        let scan_root = fs::canonicalize(&detection.scan_root).map_err(|source| {
            compass_files::FileError::Io {
                path: PathBuf::from(&detection.scan_root),
                source,
            }
        })?;
        if scan_root != root {
            return Err(CoreError::DetectionRootMismatch);
        }
        detection
    } else {
        detect(&root, &detect_options)?
    };
    if options.google_workspace {
        let converted_dir = root.join(&output_name).join("converted");
        let mut sidecars = Vec::new();
        let mut failures = Vec::new();
        for shortcut in &detection.google_workspace_shortcuts {
            match compass_google_workspace::convert_google_workspace_file(shortcut, &converted_dir)
            {
                Ok(Some(sidecar)) => sidecars.push(sidecar),
                Ok(None) => failures.push(format!(
                    "{} [Google Workspace export produced no readable text]",
                    shortcut.display()
                )),
                Err(error) => failures.push(format!(
                    "{} [Google Workspace export failed: {error}]",
                    shortcut.display()
                )),
            }
        }
        detection = detect(
            &root,
            &DetectOptions {
                google_workspace: true,
                additional_files: sidecars,
                ..detect_options
            },
        )?;
        detection.skipped_sensitive.extend(failures);
    }
    timings.detect = stage_started.elapsed();
    stage_started = Instant::now();
    let mut internal_started = Instant::now();
    let preserve_prior_semantic =
        prior_semantic_layer_required(read_prior_published_graph, &output_dir);
    let mut semantic_documents = if preserve_prior_semantic {
        semantic_document_sources(&output_dir.join("graph.json"), &root)
    } else {
        HashSet::new()
    };
    if let Some(layer) = semantic {
        semantic_documents.extend(canonical_source_set(&layer.refreshed_files, &root));
    }
    let mut sources = detection
        .files
        .get("code")
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .filter(|path| Registry::resolve(path).is_some())
        .collect::<Vec<_>>();
    sources.extend(
        detection
            .files
            .get("document")
            .into_iter()
            .flatten()
            .map(PathBuf::from)
            .filter(|path| {
                Registry::resolve(path).is_some()
                    && !semantic_documents.contains(&canonical_identity(path))
            }),
    );

    let reusable_semantic_layer = semantic.is_none()
        || (options.purpose == BuildPurpose::Extract
            && semantic.is_some_and(semantic_layer_is_empty));
    let manifest_unchanged = read_prior_published_graph
        && prior_manifest.is_unchanged(&detection.files, ManifestKind::Ast);
    let build_profile = build_profile(options);
    let has_program_artifacts =
        options.program_analysis && program_artifact_count(&root, options)? != 0;
    let verified_state = if reusable_semantic_layer && supplemental.is_empty() && manifest_unchanged
    {
        load_verified(
            &output_dir,
            &build_profile,
            &manifest_path,
            prior_build_complete,
        )?
    } else {
        None
    };
    let verified_output = verified_state.is_some();
    if !has_program_artifacts {
        let verified = verified_state;
        if let Some(state) = verified.filter(|state| state.stats.files == sources.len()) {
            if options.no_viz {
                remove_if_exists(&output_dir.join("graph.html"))?;
            }
            if options.no_cluster {
                remove_if_exists(&output_dir.join(GRAPH_OVERVIEW_FILE))?;
            }
            remove_if_exists(&output_dir.join("needs_update"))?;
            let published_output_dir = commit_generation(guard, &output_container)?;
            return Ok(BuildResult {
                root,
                output_dir: published_output_dir,
                detection,
                files_considered: state.stats.files,
                files_extracted: 0,
                files_cached: state.stats.files,
                empty_files: Vec::new(),
                nodes: state.stats.nodes,
                edges: state.stats.edges,
                communities: state.stats.communities,
                omitted_nodes: state.stats.omitted_nodes,
                omitted_edges: state.stats.omitted_edges,
                identity_collisions: state.stats.identity_collisions,
                partial_graph: state.stats.omitted_nodes > 0
                    || state.stats.omitted_edges > 0
                    || state.stats.identity_collisions > 0,
                html_written: output_dir.join("graph.html").is_file(),
                outputs_changed: false,
                program_modules: state.stats.program_modules,
                program_summaries: state.stats.program_summaries,
                program_syntax_analyzed: 0,
                program_syntax_reused: state.stats.program_modules,
                program_artifacts_loaded: 0,
                program_artifacts_reused: 0,
                program_artifact_documents_analyzed: 0,
                program_artifact_documents_reused: 0,
                program_conflicts: state.stats.program_conflicts,
                timings,
            });
        }
    }
    let unchanged_program_build = if options.program_analysis
        && reusable_semantic_layer
        && supplemental.is_empty()
        && manifest_unchanged
        && verified_output
    {
        load_current_program(&root, &sources, options, &output_dir)?
    } else {
        None
    };
    let unchanged_program_available = unchanged_program_build.is_some();
    let unchanged_program = unchanged_program_build
        .as_ref()
        .map(ProgramBuildSummary::from_program);
    drop(unchanged_program_build);
    if reusable_semantic_layer
        && supplemental.is_empty()
        && manifest_unchanged
        && verified_output
        && (!options.program_analysis || unchanged_program_available)
        && let Some(stats) = unchanged_output_stats(options, &output_dir)
    {
        if options.no_viz {
            remove_if_exists(&output_dir.join("graph.html"))?;
        }
        if options.no_cluster {
            remove_if_exists(&output_dir.join(GRAPH_OVERVIEW_FILE))?;
        }
        remove_if_exists(&output_dir.join("needs_update"))?;
        publish_build_state(
            options,
            &output_dir,
            &manifest_path,
            sources.len(),
            stats.nodes,
            stats.edges,
            stats.communities,
            stats.omissions(),
            unchanged_program.as_ref(),
        )?;
        let published_output_dir = commit_generation(guard, &output_container)?;
        return Ok(BuildResult {
            root,
            output_dir: published_output_dir,
            detection,
            files_considered: sources.len(),
            files_extracted: 0,
            files_cached: sources.len(),
            empty_files: Vec::new(),
            nodes: stats.nodes,
            edges: stats.edges,
            communities: stats.communities,
            omitted_nodes: stats.omitted_nodes,
            omitted_edges: stats.omitted_edges,
            identity_collisions: stats.identity_collisions,
            partial_graph: stats.omissions().is_partial(),
            html_written: output_dir.join("graph.html").is_file(),
            outputs_changed: false,
            program_modules: program_modules(unchanged_program.as_ref()),
            program_summaries: program_summaries(unchanged_program.as_ref()),
            program_syntax_analyzed: 0,
            program_syntax_reused: unchanged_program
                .as_ref()
                .map_or(0, |program| program.syntax_reused),
            program_artifacts_loaded: 0,
            program_artifacts_reused: unchanged_program
                .as_ref()
                .map_or(0, |program| program.artifacts_reused),
            program_artifact_documents_analyzed: 0,
            program_artifact_documents_reused: unchanged_program
                .as_ref()
                .map_or(0, |program| program.artifact_documents_reused),
            program_conflicts: unchanged_program
                .as_ref()
                .map_or(0, |program| program.conflicts),
            timings,
        });
    }

    let output_cache_root = (output_root != root).then_some(output_root.as_path());
    let cache_options = options.cache_root.as_deref().map_or_else(
        || CacheOptions::output_directory(output_cache_root),
        CacheOptions::shared_history,
    );
    let mut cache = Cache::open(&root, cache_options)?;
    let project_evidence = Arc::new(ProjectEvidenceIndex::build(&root, &sources));
    let mut extractions = BTreeMap::<PathBuf, Extraction>::new();
    let mut missing = Vec::new();
    if reuse_cached_analysis {
        for path in &sources {
            if fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.len() > options.max_source_bytes
            }) {
                extractions.insert(
                    path.clone(),
                    oversized_source_extraction(path, options.max_source_bytes)?,
                );
                continue;
            }
            let cached = cache.load(path, &CacheKind::Ast, None, false)?;
            if let Some(value) = cached {
                let mut extraction =
                    serde_json::from_value(value).map_err(|source| CoreError::InvalidCache {
                        path: path.clone(),
                        source,
                    })?;
                absolutize_cached_framework_fact_sources(&mut extraction, &root);
                if cached_framework_evidence_matches(&extraction, path, &project_evidence)
                    && cached_universal_evidence_matches(&extraction, path)
                {
                    extractions.insert(path.clone(), extraction);
                } else {
                    missing.push(path.clone());
                }
            } else {
                missing.push(path.clone());
            }
        }
    } else {
        for path in &sources {
            if fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.len() > options.max_source_bytes
            }) {
                extractions.insert(
                    path.clone(),
                    oversized_source_extraction(path, options.max_source_bytes)?,
                );
            } else {
                missing.push(path.clone());
            }
        }
    }
    let worker_count = options.max_workers.unwrap_or_else(default_ast_workers);
    let worker_pool = if missing.len() >= 256 || sources.len() >= 256 {
        Some(
            rayon::ThreadPoolBuilder::new()
                .num_threads(worker_count)
                .thread_name(|index| format!("compass-ast-{index}"))
                .build()
                .map_err(|error| CoreError::WorkerPool(error.to_string()))?,
        )
    } else {
        None
    };
    profile_internal("extract setup and cache load", &mut internal_started);
    // A Rayon worker pool costs more resident memory than it saves time on
    // small multilingual projects, where parser-table page residency dominates.
    // Stay sequential below the measured crossover. Larger corpora use an
    // explicit local pool so an embedding application's global Rayon settings
    // cannot silently serialize cold extraction.
    let completed_files = Mutex::new(0_usize);
    let total_files = missing.len();
    let extract_source =
        |engine: &mut Engine, path: &PathBuf| -> Result<_, compass_languages::ExtractError> {
            let metadata = fs::metadata(path).map_err(|source| compass_files::FileError::Io {
                path: path.clone(),
                source,
            })?;
            if metadata.len() > options.max_source_bytes {
                let graph = oversized_source_extraction(path, options.max_source_bytes)?;
                if let Some(progress) = progress {
                    let mut completed = completed_files
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *completed += 1;
                    progress(BuildFileProgress {
                        current: *completed,
                        total: total_files,
                        path: path.clone(),
                    });
                }
                return Ok((
                    path.clone(),
                    graph,
                    (path.to_string_lossy().into_owned(), String::new()),
                    None,
                ));
            }
            let bytes = fs::read(path).map_err(|source| compass_files::FileError::Io {
                path: path.clone(),
                source,
            })?;
            let source_file = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let language = Registry::resolve(path).map_or("", |spec| spec.name);
            let (mut graph, program) = if options.program_analysis {
                let combined = engine.extract_source_combined(path, &source_file, &bytes)?;
                (combined.graph, combined.program)
            } else {
                (
                    engine.extract_source_graph_only(path, &source_file, &bytes)?,
                    None,
                )
            };
            let empty_structured_document = bytes.is_empty()
                && Registry::resolve(path).is_some_and(|spec| {
                    matches!(
                        spec.kind,
                        ExtractorKind::JsonConfig | ExtractorKind::ProjectXml | ExtractorKind::Xaml
                    )
                });
            if empty_structured_document && graph.error.is_none() {
                graph.error = Some(format!("{language} extraction failed: empty document"));
            }
            let prepared = program.map(|batch| PreparedSyntaxInput {
                source_file,
                language: language.to_owned(),
                bytes: bytes.clone(),
                batch,
            });
            if let Some(progress) = progress {
                let mut completed = completed_files
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *completed += 1;
                progress(BuildFileProgress {
                    current: *completed,
                    total: total_files,
                    path: path.clone(),
                });
            }
            let source = (
                path.to_string_lossy().into_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
            );
            Ok((path.clone(), graph, source, prepared))
        };
    let fresh_outcomes = if missing.len() < 256 {
        let mut engine = Engine::with_project_evidence(Arc::clone(&project_evidence));
        missing
            .iter()
            .map(|path| extract_source(&mut engine, path))
            .collect::<Vec<_>>()
    } else {
        let worker_evidence = Arc::clone(&project_evidence);
        let extract = || {
            missing
                .par_iter()
                .map_init(
                    || Engine::with_project_evidence(Arc::clone(&worker_evidence)),
                    extract_source,
                )
                .collect::<Vec<_>>()
        };
        if let Some(pool) = &worker_pool {
            pool.install(extract)
        } else {
            extract()
        }
    };
    let mut extraction_failures = BTreeMap::new();
    let mut fresh = missing
        .iter()
        .cloned()
        .zip(fresh_outcomes)
        .filter_map(|(path, outcome)| match outcome {
            Ok(extracted) => Some(extracted),
            Err(error) => {
                extraction_failures.insert(
                    canonical_identity(&path),
                    portable_diagnostic_reason(&error.to_string(), &path, &root),
                );
                None
            }
        })
        .collect::<Vec<_>>();
    let mut extraction_partials = BTreeMap::new();
    for (path, extraction) in &mut extractions {
        prepare_extraction_for_publication(
            path,
            extraction,
            &root,
            &mut extraction_failures,
            &mut extraction_partials,
        );
    }
    for (path, extraction, _, _) in &mut fresh {
        prepare_extraction_for_publication(
            path,
            extraction,
            &root,
            &mut extraction_failures,
            &mut extraction_partials,
        );
    }
    profile_internal("tree-sitter extraction", &mut internal_started);
    let prepared = if !reuse_cached_analysis {
        fresh
            .iter_mut()
            .filter_map(|(_, _, _, prepared)| prepared.take())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut program_handle = if options.program_analysis {
        let program_root = root.clone();
        let program_sources = sources.clone();
        let mut program_options = options.clone();
        program_options.force = !reuse_cached_analysis;
        let program_cache_root = options.cache_root.clone();
        let program_output_cache_root = output_cache_root.map(Path::to_path_buf);
        let program_output_dir = output_dir.clone();
        Some(
            std::thread::Builder::new()
                .name("compass-program".to_owned())
                .spawn(move || {
                    let started = Instant::now();
                    let program_cache_options = program_cache_root.as_deref().map_or_else(
                        || CacheOptions::output_directory(program_output_cache_root.as_deref()),
                        CacheOptions::shared_history,
                    );
                    let program_cache =
                        Cache::open(&program_root, program_cache_options)?.without_hash_flush();
                    let program = build_program(
                        &program_root,
                        &program_sources,
                        &program_options,
                        &program_cache,
                        &prepared,
                    )?;
                    write_program(&program_output_dir, &program.canonical_bytes)?;
                    let summary = ProgramBuildSummary::from_program(&program);
                    Ok::<_, CoreError>((summary, started.elapsed()))
                })
                .map_err(|error| CoreError::WorkerPool(error.to_string()))?,
        )
    } else {
        None
    };
    let mut empty_files = Vec::new();
    let fresh_paths = fresh
        .iter()
        .map(|(path, _, _, _)| path.clone())
        .collect::<HashSet<_>>();
    let mut fresh_source_text = HashMap::with_capacity(fresh.len());
    for (path, extraction, (source_path, source), _) in fresh {
        if !extraction_has_cacheable_ast_facts(&extraction) {
            empty_files.push(path.clone());
        }
        fresh_source_text.insert(source_path, source);
        extractions.insert(path, extraction);
    }
    profile_internal("AST extraction collation", &mut internal_started);

    let ordered_paths = sources
        .iter()
        .filter(|path| extractions.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>();
    let mut ordered = sources
        .iter()
        .filter_map(|path| extractions.remove(path))
        .collect::<Vec<_>>();
    merge_decl_def_classes(&mut ordered);
    profile_internal("declaration merge", &mut internal_started);
    let ast_root_marker = format!("{}_", make_id(&[&root.to_string_lossy()]));
    let portable_check_started = Instant::now();
    let ast_is_portable = ast_extractions_are_portable(&ordered, &root);
    profile_internal_duration("portable AST precheck", portable_check_started.elapsed());
    let ast_id_remap = if ast_is_portable {
        AHashMap::new()
    } else {
        let identity_map_started = Instant::now();
        let live_id_remap = ast_source_identity_map(&sources, &root);
        profile_internal_duration(
            "portable AST source identity map",
            identity_map_started.elapsed(),
        );
        let remap_collection_started = Instant::now();
        let remap = collect_ast_id_remap(&ordered, &root, &live_id_remap);
        profile_internal_duration(
            "portable AST remap collection",
            remap_collection_started.elapsed(),
        );
        remap
    };
    if !ast_id_remap.is_empty() {
        let remap_application_started = Instant::now();
        if ordered.len() < 256 {
            ordered.iter_mut().for_each(|extraction| {
                apply_ast_id_remap(extraction, &ast_id_remap, &ast_root_marker);
            });
        } else {
            ordered.par_iter_mut().for_each(|extraction| {
                apply_ast_id_remap(extraction, &ast_id_remap, &ast_root_marker);
            });
        }
        profile_internal_duration(
            "portable AST remap application",
            remap_application_started.elapsed(),
        );
    }
    profile_internal("portable AST ID remapping", &mut internal_started);
    let ast_cache_started = Instant::now();
    for (path, extraction) in ordered_paths.iter().zip(&mut ordered) {
        if fresh_paths.contains(path) && extraction_has_cacheable_ast_facts(extraction) {
            let mut cache_entry = extraction.clone();
            prepare_portable_ast_cache_entry(&mut cache_entry, path, &root);
            cache.save_portable_ast_batch(&[(path.clone(), cache_entry)])?;
        }
    }
    cache.flush()?;
    profile_internal_duration("AST cache publication", ast_cache_started.elapsed());
    internal_started = Instant::now();
    let read_source = |path: &PathBuf| read_source_text_with_limit(path, options.max_source_bytes);
    let read_cached_source = |path: &PathBuf| {
        (!fresh_paths.contains(path))
            .then(|| read_source(path))
            .flatten()
    };
    let cached_source_text: HashMap<_, _> = if sources.len() < 256 {
        sources.iter().filter_map(read_cached_source).collect()
    } else if let Some(pool) = &worker_pool {
        pool.install(|| sources.par_iter().filter_map(read_cached_source).collect())
    } else {
        sources.par_iter().filter_map(read_cached_source).collect()
    };
    fresh_source_text.extend(cached_source_text);
    let source_text = fresh_source_text;
    let program_projection_sites = collect_program_projection_sites(&ordered);
    let mut resolved = resolve_owned_with_root(ordered, &source_text, &root);
    profile_internal("cross-file resolution total", &mut internal_started);
    drop(source_text);
    let defer_program_join = options.force && !options.no_cluster && !has_program_artifacts;
    let mut program = if defer_program_join {
        None
    } else {
        join_program_worker(program_handle.take(), &mut timings)?
    };
    profile_internal("wait for Program analysis", &mut internal_started);
    if let Some(program) = program.as_ref() {
        apply_program_projection(
            &mut resolved,
            &program_projection_sites,
            &program.compiler_projection,
        );
    }
    profile_internal("Program graph projection", &mut internal_started);
    finalize_ast_extraction(&mut resolved, &root);
    profile_internal("AST finalization", &mut internal_started);
    timings.deterministic_extract = stage_started.elapsed();
    stage_started = Instant::now();
    if preserve_prior_semantic {
        let refreshed = semantic
            .map(|layer| {
                let mut refreshed = canonical_source_set(&layer.refreshed_files, &root);
                refreshed.extend(stale_semantic_sources(
                    &output_dir.join("graph.json"),
                    &root,
                    &detection.files,
                ));
                refreshed
            })
            .unwrap_or_default();
        preserve_semantic_layer(
            &mut resolved,
            &output_dir.join("graph.json"),
            &root,
            &refreshed,
        );
    }
    if let Some(layer) = semantic {
        let mut extracted: Extraction = serde_json::from_value(layer.fragment.clone())
            .map_err(CoreError::InvalidSemanticFragment)?;
        finalize_semantic_extraction(&mut extracted, &root);
        resolved.nodes.extend(extracted.nodes);
        resolved.edges.extend(extracted.edges);
        resolved.hyperedges.extend(extracted.hyperedges);
    }
    for extracted in supplemental {
        resolved.nodes.extend(extracted.nodes.iter().cloned());
        resolved.edges.extend(extracted.edges.iter().cloned());
        resolved
            .hyperedges
            .extend(extracted.hyperedges.iter().cloned());
    }
    let live_sources = detection
        .files
        .values()
        .flatten()
        .map(|path| canonical_identity(Path::new(path)))
        .collect::<HashSet<_>>();
    let source_removed = prior_manifest
        .entries()
        .keys()
        .map(|path| canonical_identity(Path::new(path)))
        .any(|path| !live_sources.contains(&path));
    if options.no_cluster
        && options.purpose == BuildPurpose::Extract
        && !options.force
        && manifest_unchanged
        && missing.is_empty()
        && !source_removed
        && supplemental.is_empty()
        && semantic.is_some_and(semantic_layer_is_empty)
        && let Ok(document) = GraphDocument::load(&output_dir.join("graph.json"))
    {
        let omissions = saved_publication_omissions(&output_dir);
        let mut manifest = prior_manifest;
        save_build_manifest(
            &mut manifest,
            &detection.files,
            &manifest_path,
            &root,
            semantic,
        )?;
        publish_build_state(
            options,
            &output_dir,
            &manifest_path,
            sources.len(),
            document.nodes.len(),
            document.links.len(),
            0,
            omissions,
            program.as_ref(),
        )?;
        let published_output_dir = commit_generation(guard, &output_container)?;
        return Ok(BuildResult {
            root,
            output_dir: published_output_dir,
            detection,
            files_considered: sources.len(),
            files_extracted: 0,
            files_cached: sources.len(),
            empty_files,
            nodes: document.nodes.len(),
            edges: document.links.len(),
            communities: 0,
            omitted_nodes: omissions.nodes,
            omitted_edges: omissions.edges,
            identity_collisions: omissions.identity_collisions,
            partial_graph: omissions.is_partial(),
            html_written: false,
            outputs_changed: false,
            program_modules: program_modules(program.as_ref()),
            program_summaries: program_summaries(program.as_ref()),
            program_syntax_analyzed: program
                .as_ref()
                .map_or(0, |program| program.syntax_analyzed),
            program_syntax_reused: program.as_ref().map_or(0, |program| program.syntax_reused),
            program_artifacts_loaded: program
                .as_ref()
                .map_or(0, |program| program.artifacts_loaded),
            program_artifacts_reused: program
                .as_ref()
                .map_or(0, |program| program.artifacts_reused),
            program_artifact_documents_analyzed: program
                .as_ref()
                .map_or(0, |program| program.artifact_documents_analyzed),
            program_artifact_documents_reused: program
                .as_ref()
                .map_or(0, |program| program.artifact_documents_reused),
            program_conflicts: program.as_ref().map_or(0, |program| program.conflicts),
            timings,
        });
    }
    if options.no_cluster {
        let nodes = dedupe_nodes(&resolved.nodes);
        enforce_incomplete_raw_guard(semantic, &output_dir.join("graph.json"), &root, nodes.len())?;
        let document = build_document(resolved, true, true, Some(&root), tiebreaker)?;
        let configuration_digest = graph_configuration_digest(options, &output_dir)?;
        let source_commit = options
            .built_at_commit
            .clone()
            .or_else(|| git_commit(&root));
        let published = normalize_document_v1_with_inventory_best_effort_owned(
            document,
            &root,
            configuration_digest,
            source_commit.as_deref(),
            detection_inventory(
                &detection,
                semantic,
                &extraction_failures,
                &extraction_partials,
                &root,
            ),
        )?;
        if published.document.nodes.is_empty() {
            return Err(CoreError::EmptyGraph);
        }
        let omissions = published.omissions;
        let published_nodes = published.document.nodes.len();
        let published_edges = published.document.links.len();
        write_json_atomic(output_dir.join("graph.json"), &published.document, false)?;
        remove_if_exists(&output_dir.join(GRAPH_OVERVIEW_FILE))?;
        save_output_stats(
            &output_dir,
            published_nodes,
            published_edges,
            0,
            false,
            omissions,
        )?;
        write_semantic_marker(&output_dir, semantic)?;
        if options.purpose == BuildPurpose::Update {
            write_text_atomic(
                output_dir.join(".compass_root"),
                &options.root.to_string_lossy(),
            )?;
        }
        let mut manifest = prior_manifest;
        save_build_manifest(
            &mut manifest,
            &detection.files,
            &manifest_path,
            &root,
            semantic,
        )?;
        remove_if_exists(&output_dir.join("needs_update"))?;
        publish_build_state(
            options,
            &output_dir,
            &manifest_path,
            sources.len(),
            published_nodes,
            published_edges,
            0,
            omissions,
            program.as_ref(),
        )?;
        let published_output_dir = commit_generation(guard, &output_container)?;
        timings.publish = stage_started.elapsed();
        return Ok(BuildResult {
            root,
            output_dir: published_output_dir,
            detection,
            files_considered: sources.len(),
            files_extracted: missing.len(),
            files_cached: sources.len().saturating_sub(missing.len()),
            empty_files,
            nodes: published_nodes,
            edges: published_edges,
            communities: 0,
            omitted_nodes: omissions.nodes,
            omitted_edges: omissions.edges,
            identity_collisions: omissions.identity_collisions,
            partial_graph: omissions.is_partial(),
            html_written: false,
            outputs_changed: true,
            program_modules: program_modules(program.as_ref()),
            program_summaries: program_summaries(program.as_ref()),
            program_syntax_analyzed: program
                .as_ref()
                .map_or(0, |program| program.syntax_analyzed),
            program_syntax_reused: program.as_ref().map_or(0, |program| program.syntax_reused),
            program_artifacts_loaded: program
                .as_ref()
                .map_or(0, |program| program.artifacts_loaded),
            program_artifacts_reused: program
                .as_ref()
                .map_or(0, |program| program.artifacts_reused),
            program_artifact_documents_analyzed: program
                .as_ref()
                .map_or(0, |program| program.artifact_documents_analyzed),
            program_artifact_documents_reused: program
                .as_ref()
                .map_or(0, |program| program.artifact_documents_reused),
            program_conflicts: program.as_ref().map_or(0, |program| program.conflicts),
            timings,
        });
    }
    let document = build_document(resolved, true, true, Some(&root), tiebreaker)?;
    profile_internal("graph document build and dedup", &mut internal_started);
    timings.graph_assembly = stage_started.elapsed();
    stage_started = Instant::now();
    if document.nodes.is_empty() {
        return Err(CoreError::EmptyGraph);
    }
    let commit = options.built_at_commit.clone().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|directory| git_commit(&directory))
    });
    let unchanged_artifacts_complete = match options.purpose {
        BuildPurpose::Update => update_artifacts_complete(&output_dir),
        BuildPurpose::Extract => {
            output_dir.join("graph.json").is_file()
                && output_dir.join(".compass_analysis.json").is_file()
        }
    };
    let unchanged_layers = semantic.is_none()
        || (options.purpose == BuildPurpose::Extract
            && semantic.is_some_and(semantic_layer_is_empty));
    if unchanged_layers && supplemental.is_empty() && !options.force && unchanged_artifacts_complete
    {
        let preflight_started = Instant::now();
        let preflight = normalize_document_v1_with_inventory_best_effort(
            &document,
            &root,
            graph_configuration_digest(options, &output_dir)?,
            commit.as_deref(),
            detection_inventory(
                &detection,
                semantic,
                &extraction_failures,
                &extraction_partials,
                &root,
            ),
        )?;
        profile_internal_duration("graph.json v1 preflight", preflight_started.elapsed());
        if !preflight.omissions.is_partial()
            && GraphDocument::load(&output_dir.join("graph.json"))
                .is_ok_and(|existing| topology_is_unchanged(&existing, &document))
        {
            let communities = previous_communities(&output_dir.join("graph.json"))
                .values()
                .copied()
                .collect::<HashSet<_>>()
                .len();
            let mut manifest = prior_manifest;
            save_build_manifest(
                &mut manifest,
                &detection.files,
                &manifest_path,
                &root,
                semantic,
            )?;
            remove_if_exists(&output_dir.join("needs_update"))?;
            publish_build_state(
                options,
                &output_dir,
                &manifest_path,
                sources.len(),
                document.nodes.len(),
                document.links.len(),
                communities,
                PublicationOmissions::default(),
                program.as_ref(),
            )?;
            let published_output_dir = commit_generation(guard, &output_container)?;
            return Ok(BuildResult {
                root,
                output_dir: published_output_dir,
                detection,
                files_considered: sources.len(),
                files_extracted: missing.len(),
                files_cached: sources.len().saturating_sub(missing.len()),
                empty_files,
                nodes: document.nodes.len(),
                edges: document.links.len(),
                communities,
                omitted_nodes: 0,
                omitted_edges: 0,
                identity_collisions: 0,
                partial_graph: false,
                html_written: output_dir.join("graph.html").is_file(),
                outputs_changed: false,
                program_modules: program_modules(program.as_ref()),
                program_summaries: program_summaries(program.as_ref()),
                program_syntax_analyzed: program
                    .as_ref()
                    .map_or(0, |program| program.syntax_analyzed),
                program_syntax_reused: program.as_ref().map_or(0, |program| program.syntax_reused),
                program_artifacts_loaded: program
                    .as_ref()
                    .map_or(0, |program| program.artifacts_loaded),
                program_artifacts_reused: program
                    .as_ref()
                    .map_or(0, |program| program.artifacts_reused),
                program_artifact_documents_analyzed: program
                    .as_ref()
                    .map_or(0, |program| program.artifact_documents_analyzed),
                program_artifact_documents_reused: program
                    .as_ref()
                    .map_or(0, |program| program.artifact_documents_reused),
                program_conflicts: program.as_ref().map_or(0, |program| program.conflicts),
                timings,
            });
        }
    }

    // A history realization must depend only on the target commit and build
    // profile. Prior community numbering is current-worktree operational state
    // and cannot influence the content-addressed result.
    let cluster_options = ClusterOptions {
        resolution: options.resolution,
        exclude_hubs_percentile: options.exclude_hubs,
    };
    let ((previous, previous_elapsed), (current, cluster_elapsed)) = rayon::join(
        || {
            let started = Instant::now();
            let previous = if std::env::var_os("COMPASS_HISTORY_BUILD").is_some() {
                HashMap::new()
            } else {
                previous_communities(&output_dir.join("graph.json"))
            };
            (previous, started.elapsed())
        },
        || {
            let started = Instant::now();
            let current = cluster(&document, cluster_options);
            (current, started.elapsed())
        },
    );
    profile_internal_duration("load previous communities", previous_elapsed);
    profile_internal_duration("Louvain clustering", cluster_elapsed);
    internal_started = Instant::now();
    let communities = if previous.is_empty() {
        current
    } else {
        remap_communities_to_previous(&current, &previous)
    };
    timings.graph_assembly += stage_started.elapsed();
    stage_started = Instant::now();
    let labels = label_communities_by_hub(&document, &communities);
    profile_internal("community labeling", &mut internal_started);

    let graph_output = || -> Result<(Duration, PublicationOmissions, usize, usize), CoreError> {
        let started = Instant::now();
        let mut output_profile_started = Instant::now();
        let configuration_digest = graph_configuration_digest(options, &output_dir)?;
        let normalization_started = Instant::now();
        let published = published_v1_document(
            &document,
            &communities,
            &labels,
            &root,
            PublicationEvidence {
                detection: &detection,
                semantic,
                extraction_failures: &extraction_failures,
                extraction_partials: &extraction_partials,
            },
            configuration_digest,
            commit.as_deref(),
        )?;
        profile_internal_duration(
            "graph.json v1 normalization",
            normalization_started.elapsed(),
        );
        if published.document.nodes.is_empty() {
            return Err(CoreError::EmptyGraph);
        }
        let published_nodes = published.document.nodes.len();
        let published_edges = published.document.links.len();
        let serialization_started = Instant::now();
        write_json_atomic(output_dir.join("graph.json"), &published.document, false)?;
        profile_internal_duration(
            "graph.json v1 serialization",
            serialization_started.elapsed(),
        );
        profile_internal("graph.json v1 publication", &mut output_profile_started);
        if options.purpose == BuildPurpose::Update {
            write_text_atomic(
                output_dir.join(".compass_root"),
                &options.root.to_string_lossy(),
            )?;
            write_graph_overview_artifact(&document, &communities, &labels, &output_dir)?;
            profile_internal(
                "graph root marker and overview publication",
                &mut output_profile_started,
            );
        }
        Ok((
            started.elapsed(),
            published.omissions,
            published_nodes,
            published_edges,
        ))
    };
    let graph_analyses = || -> Result<(bool, Duration), CoreError> {
        let started = Instant::now();
        let analysis_compute_started = Instant::now();
        let (cohesion, (gods, surprises, questions)) = rayon::join(
            || score_communities(&document, &communities),
            || graph_insights(&document, &communities, &labels, 10, 5, 10),
        );
        profile_internal_duration(
            "graph analyses computation",
            analysis_compute_started.elapsed(),
        );
        let analysis_render_started = Instant::now();
        let tokens = semantic_tokens(semantic);
        let analysis = if options.purpose == BuildPurpose::Extract {
            json!({
                "communities": communities.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
                "cohesion": cohesion.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
                "gods": gods,
                "surprises": surprises,
                "tokens": {"input": tokens.0, "output": tokens.1},
            })
        } else {
            json!({
                "communities": communities.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
                "cohesion": cohesion.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
                "gods": gods,
                "surprises": surprises,
                "questions": questions,
            })
        };
        if options.purpose == BuildPurpose::Extract {
            write_json_atomic(output_dir.join(".compass_analysis.json"), &analysis, true)?;
        } else {
            let labels_json = serde_json::to_string_pretty(&labels).map_err(|source| {
                CoreError::SerializeExtraction {
                    path: output_dir.join(".compass_labels.json"),
                    source,
                }
            })?;
            write_text_atomic(
                output_dir.join(".compass_labels.json"),
                &format!("{labels_json}\n"),
            )?;
        }
        let detection_summary = DetectionSummary {
            total_files: detection.total_files,
            total_words: usize::try_from(detection.total_words).unwrap_or(usize::MAX),
            warning: (options.purpose == BuildPurpose::Extract)
                .then(|| detection.warning.clone())
                .flatten(),
        };
        let html_written = if options.purpose == BuildPurpose::Update {
            let report_root = report_root_label(&options.root);
            let mut report_options = ReportOptions::new(&report_root);
            report_options.built_at_commit = commit.as_deref();
            let report = generate_report(
                &document,
                &communities,
                &cohesion,
                &labels,
                &gods,
                &surprises,
                &detection_summary,
                TokenCost::default(),
                Some(&questions),
                None,
                &report_options,
            );
            write_text_atomic(output_dir.join("GRAPH_REPORT.md"), &report)?;
            let html_path = output_dir.join("graph.html");
            if options.no_viz {
                remove_if_exists(&html_path)?;
                false
            } else {
                let rendered = match write_html(
                    &document,
                    &communities,
                    &html_path,
                    &HtmlOptions {
                        community_labels: (!labels.is_empty()).then_some(&labels),
                        node_limit: None,
                        ..HtmlOptions::default()
                    },
                ) {
                    Ok(rendered) => rendered,
                    Err(OutputError::HtmlTooLarge { .. }) => None,
                    Err(error) => return Err(CoreError::Output(error)),
                };
                let html_written = rendered.is_some();
                if !html_written {
                    remove_if_exists(&html_path)?;
                }
                html_written
            }
        } else {
            false
        };
        profile_internal_duration(
            "graph analysis and report rendering",
            analysis_render_started.elapsed(),
        );
        Ok((html_written, started.elapsed()))
    };
    let (graph_output_elapsed, analysis_result) = rayon::join(graph_output, graph_analyses);
    let (graph_output_elapsed, omissions, published_nodes, published_edges) = graph_output_elapsed?;
    let (html_written, analysis_elapsed) = analysis_result?;
    profile_internal_duration("graph and overview publication", graph_output_elapsed);
    profile_internal_duration(
        "parallel graph analyses and report publication",
        analysis_elapsed,
    );
    internal_started = Instant::now();
    timings.graph_assembly += stage_started.elapsed();
    stage_started = Instant::now();

    let mut manifest = prior_manifest;
    let (manifest_result, seals_result) = rayon::join(
        || {
            save_build_manifest(
                &mut manifest,
                &detection.files,
                &manifest_path,
                &root,
                semantic,
            )
        },
        || {
            write_semantic_marker(&output_dir, semantic)?;
            save_output_stats(
                &output_dir,
                published_nodes,
                published_edges,
                communities.len(),
                true,
                omissions,
            )
        },
    );
    manifest_result?;
    seals_result?;
    timings.publish = stage_started.elapsed();
    if program.is_none() {
        program = join_program_worker(program_handle.take(), &mut timings)?;
    }
    publish_build_state(
        options,
        &output_dir,
        &manifest_path,
        sources.len(),
        published_nodes,
        published_edges,
        communities.len(),
        omissions,
        program.as_ref(),
    )?;
    profile_internal("Program output and build seals", &mut internal_started);
    let published_output_dir = commit_generation(guard, &output_container)?;
    Ok(BuildResult {
        root,
        output_dir: published_output_dir,
        detection,
        files_considered: sources.len(),
        files_extracted: missing.len(),
        files_cached: sources.len().saturating_sub(missing.len()),
        empty_files,
        nodes: published_nodes,
        edges: published_edges,
        communities: communities.len(),
        omitted_nodes: omissions.nodes,
        omitted_edges: omissions.edges,
        identity_collisions: omissions.identity_collisions,
        partial_graph: omissions.is_partial(),
        html_written,
        outputs_changed: true,
        program_modules: program_modules(program.as_ref()),
        program_summaries: program_summaries(program.as_ref()),
        program_syntax_analyzed: program
            .as_ref()
            .map_or(0, |program| program.syntax_analyzed),
        program_syntax_reused: program.as_ref().map_or(0, |program| program.syntax_reused),
        program_artifacts_loaded: program
            .as_ref()
            .map_or(0, |program| program.artifacts_loaded),
        program_artifacts_reused: program
            .as_ref()
            .map_or(0, |program| program.artifacts_reused),
        program_artifact_documents_analyzed: program
            .as_ref()
            .map_or(0, |program| program.artifact_documents_analyzed),
        program_artifact_documents_reused: program
            .as_ref()
            .map_or(0, |program| program.artifact_documents_reused),
        program_conflicts: program.as_ref().map_or(0, |program| program.conflicts),
        timings,
    })
}

fn oversized_source_extraction(
    path: &Path,
    max_source_bytes: u64,
) -> Result<Extraction, compass_languages::ExtractError> {
    let metadata = fs::metadata(path).map_err(|source| compass_files::FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut extraction = Extraction::default();
    extraction.extensions.insert(
        EXTRACTION_QUALITY_EXTENSION.to_owned(),
        serde_json::Value::String(EXTRACTION_QUALITY_PARTIAL.to_owned()),
    );
    extraction.extensions.insert(
        EXTRACTION_QUALITY_REASON_EXTENSION.to_owned(),
        serde_json::Value::String(format!(
            "source is {} bytes, exceeding the configured {} byte extraction limit",
            metadata.len(),
            max_source_bytes
        )),
    );
    Ok(extraction)
}

struct ProgramBuildSummary {
    seal: ArtifactSeal,
    modules: usize,
    summaries: usize,
    providers: usize,
    syntax_analyzed: usize,
    syntax_reused: usize,
    artifacts_loaded: usize,
    artifacts_reused: usize,
    artifact_documents_analyzed: usize,
    artifact_documents_reused: usize,
    conflicts: usize,
    compiler_projection: compass_program::CompilerProjection,
}

impl ProgramBuildSummary {
    fn from_program(program: &ProgramBuild) -> Self {
        Self {
            seal: ArtifactSeal::from_bytes(&program.canonical_bytes),
            modules: program.analysis.program.modules.len(),
            summaries: program.analysis.summaries.len(),
            providers: program.analysis.program.providers.len(),
            syntax_analyzed: program.syntax_analyzed,
            syntax_reused: program.syntax_reused,
            artifacts_loaded: program.artifacts_loaded,
            artifacts_reused: program.artifacts_reused,
            artifact_documents_analyzed: program.artifact_documents_analyzed,
            artifact_documents_reused: program.artifact_documents_reused,
            conflicts: program.conflicts,
            compiler_projection: program.compiler_projection.clone(),
        }
    }
}

fn program_modules(program: Option<&ProgramBuildSummary>) -> usize {
    program.map_or(0, |program| program.modules)
}

fn program_summaries(program: Option<&ProgramBuildSummary>) -> usize {
    program.map_or(0, |program| program.summaries)
}

fn program_providers(program: Option<&ProgramBuildSummary>) -> usize {
    program.map_or(0, |program| program.providers)
}

fn build_profile(options: &BuildOptions) -> BuildProfile {
    BuildProfile {
        purpose: match options.purpose {
            BuildPurpose::Update => "update",
            BuildPurpose::Extract => "extract",
        }
        .to_owned(),
        no_cluster: options.no_cluster,
        no_viz: options.no_viz,
        resolution: options.resolution,
        exclude_hubs: options.exclude_hubs,
        program_analysis: options.program_analysis,
        max_source_bytes: options.max_source_bytes,
    }
}

#[allow(clippy::too_many_arguments)]
fn publish_build_state(
    options: &BuildOptions,
    output_dir: &Path,
    manifest_path: &Path,
    files: usize,
    nodes: usize,
    edges: usize,
    communities: usize,
    omissions: PublicationOmissions,
    program: Option<&ProgramBuildSummary>,
) -> Result<(), CoreError> {
    let mut required = vec![output_dir.join(OUTPUT_STATS_FILE)];
    match options.purpose {
        BuildPurpose::Update => {
            required.push(output_dir.join(".compass_root"));
            if !options.no_cluster {
                required.extend([
                    output_dir.join(GRAPH_OVERVIEW_FILE),
                    output_dir.join(".compass_labels.json"),
                    output_dir.join("GRAPH_REPORT.md"),
                ]);
            }
        }
        BuildPurpose::Extract if !options.no_cluster => {
            required.push(output_dir.join(".compass_analysis.json"));
        }
        BuildPurpose::Extract => {}
    }
    for optional in ["graph.html", SEMANTIC_MARKER_FILE] {
        let path = output_dir.join(optional);
        if path.is_file() {
            required.push(path);
        }
    }
    let state = BuildState::capture(
        output_dir,
        build_profile(options),
        manifest_path,
        program.map(|program| program.seal.clone()),
        &required,
        SavedStats {
            files,
            nodes,
            edges,
            communities,
            omitted_nodes: omissions.nodes,
            omitted_edges: omissions.edges,
            identity_collisions: omissions.identity_collisions,
            program_modules: program_modules(program),
            program_summaries: program_summaries(program),
            program_providers: program_providers(program),
            program_conflicts: program.map_or(0, |program| program.conflicts),
        },
    )?;
    state.save(output_dir)
}

fn commit_generation(guard: BuildGuard, output_container: &Path) -> Result<PathBuf, CoreError> {
    let mut artifacts = vec!["graph.json", "manifest.json", BUILD_STATE_FILE];
    if guard.staging_directory().join("program.json").is_file() {
        artifacts.push("program.json");
    }
    guard.commit_with_artifacts(&artifacts)?;
    Ok(BuildGuard::resolve_active_directory(output_container)?)
}

#[cfg(target_os = "macos")]
fn default_ast_workers() -> usize {
    num_cpus::get().max(num_cpus::get_physical())
}

#[cfg(not(target_os = "macos"))]
fn default_ast_workers() -> usize {
    num_cpus::get()
}

fn semantic_layer_is_empty(layer: &SemanticLayer) -> bool {
    layer.refreshed_files.is_empty()
        && layer.partial_files.is_empty()
        && ["nodes", "edges", "hyperedges"].into_iter().all(|key| {
            layer
                .fragment
                .get(key)
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty)
        })
}

fn write_semantic_marker(
    output_dir: &Path,
    semantic: Option<&SemanticLayer>,
) -> Result<(), CoreError> {
    let (_, output_tokens) = semantic_tokens(semantic);
    if output_tokens > 0 || semantic.is_some_and(|layer| !semantic_layer_is_empty(layer)) {
        write_json_atomic(
            output_dir.join(SEMANTIC_MARKER_FILE),
            &json!({"output_tokens": output_tokens}),
            false,
        )?;
    }
    Ok(())
}

fn finalize_ast_extraction(extraction: &mut Extraction, root: &Path) {
    let mut external_id_remap = HashMap::new();
    let mut canonical_sources = HashMap::<String, PathBuf>::new();
    for node in &mut extraction.nodes {
        let Some(source) = node
            .attributes
            .get("source_file")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let source_path = Path::new(&source);
        if !source_path.is_absolute() {
            continue;
        }
        let canonical = canonical_sources
            .entry(source.clone())
            .or_insert_with(|| rooted_source_identity(source_path, root));
        if canonical.starts_with(root) {
            continue;
        }
        let portable = portable_out_of_root_source(source_path, root);
        if node.id == make_id(&[&source]) {
            external_id_remap.insert(node.id.clone(), make_id(&["ext", &portable]));
        }
        node.attributes.insert(
            "source_file".to_owned(),
            serde_json::Value::String(portable),
        );
    }
    if !external_id_remap.is_empty() {
        for node in &mut extraction.nodes {
            if let Some(canonical) = external_id_remap.get(&node.id) {
                node.id.clone_from(canonical);
            }
        }
        for edge in &mut extraction.edges {
            if let Some(canonical) = external_id_remap.get(&edge.source) {
                edge.source.clone_from(canonical);
            }
            if let Some(canonical) = external_id_remap.get(&edge.target) {
                edge.target.clone_from(canonical);
            }
        }
    }
    if extraction
        .nodes
        .len()
        .saturating_add(extraction.edges.len())
        < 100_000
    {
        extraction.nodes.iter_mut().for_each(|node| {
            normalize_source_attribute_cached(&mut node.attributes, root, &canonical_sources);
            node.attributes.remove("_callable");
            node.attributes
                .entry("_origin".to_owned())
                .or_insert_with(|| serde_json::Value::String("ast".to_owned()));
        });
        extraction.edges.iter_mut().for_each(|edge| {
            normalize_source_attribute_cached(&mut edge.attributes, root, &canonical_sources);
            edge.attributes
                .entry("_origin".to_owned())
                .or_insert_with(|| serde_json::Value::String("ast".to_owned()));
        });
    } else {
        rayon::join(
            || {
                extraction.nodes.par_iter_mut().for_each(|node| {
                    normalize_source_attribute_cached(
                        &mut node.attributes,
                        root,
                        &canonical_sources,
                    );
                    node.attributes.remove("_callable");
                    node.attributes
                        .entry("_origin".to_owned())
                        .or_insert_with(|| serde_json::Value::String("ast".to_owned()));
                });
            },
            || {
                extraction.edges.par_iter_mut().for_each(|edge| {
                    normalize_source_attribute_cached(
                        &mut edge.attributes,
                        root,
                        &canonical_sources,
                    );
                    edge.attributes
                        .entry("_origin".to_owned())
                        .or_insert_with(|| serde_json::Value::String("ast".to_owned()));
                });
            },
        );
    }
}

fn prepare_portable_ast_cache_entry(extraction: &mut Extraction, source: &Path, root: &Path) {
    let canonical = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let Ok(relative) = source
        .strip_prefix(root)
        .or_else(|_| canonical.strip_prefix(root))
    else {
        return;
    };
    let portable = relative.to_string_lossy().replace('\\', "/");
    let normalize_path = |value: &str| {
        let path = Path::new(value);
        let canonical = if path.is_absolute() {
            canonicalize_allow_missing(path)
        } else {
            canonicalize_allow_missing(&root.join(path))
        };
        canonical.strip_prefix(root).map_or_else(
            |_| portable_out_of_root_source(&canonical, root),
            |relative| relative.to_string_lossy().replace('\\', "/"),
        )
    };
    let normalize_origin_path = |value: &str| {
        let path = Path::new(value);
        let canonical_value = if path.is_absolute() {
            canonicalize_allow_missing(path)
        } else {
            canonicalize_allow_missing(&root.join(path))
        };
        if path == source || canonical_value == canonical {
            portable.clone()
        } else {
            normalize_path(value)
        }
    };
    let set_portable = |attributes: &mut serde_json::Map<String, serde_json::Value>| {
        for key in ["source_file", "origin_file"] {
            let Some(value) = attributes
                .get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let normalized = normalize_origin_path(value);
            if normalized != value {
                attributes.insert(key.to_owned(), serde_json::Value::String(normalized));
            }
        }
        if let Some(anchor) = attributes
            .get_mut("origin_source_anchor")
            .and_then(serde_json::Value::as_object_mut)
            && let Some(value) = anchor
                .get("file")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        {
            anchor.insert(
                "file".to_owned(),
                serde_json::Value::String(normalize_origin_path(&value)),
            );
        }
        if let Some(target) = attributes
            .get("target_file")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        {
            let normalized = normalize_path(&target);
            attributes.insert(
                "target_file".to_owned(),
                serde_json::Value::String(normalized),
            );
        }
    };
    for node in &mut extraction.nodes {
        set_portable(&mut node.attributes);
    }
    for edge in &mut extraction.edges {
        set_portable(&mut edge.attributes);
    }
    for hyperedge in &mut extraction.hyperedges {
        if let Some(attributes) = hyperedge.as_object_mut() {
            set_portable(attributes);
        }
    }
    for fact in &mut extraction.framework_facts {
        let source_file = match fact {
            RawFrameworkFact::Route(route) => &mut route.anchor.source_file,
            RawFrameworkFact::Domain(domain) => &mut domain.anchor.source_file,
        };
        *source_file = normalize_origin_path(source_file);
    }
    if let Some(calls) = extraction.raw_calls.as_mut() {
        for call in calls {
            if !call.source_file.is_empty() {
                call.source_file.clone_from(&portable);
            }
        }
    }
    if let Some(evidence) = extraction.semantic_evidence.as_mut() {
        for range in evidence
            .declarations
            .iter_mut()
            .map(|fact| &mut fact.range)
            .chain(evidence.scopes.iter_mut().map(|fact| &mut fact.range))
            .chain(evidence.bindings.iter_mut().map(|fact| &mut fact.range))
            .chain(evidence.occurrences.iter_mut().map(|fact| &mut fact.range))
            .chain(
                evidence
                    .diagnostics
                    .iter_mut()
                    .filter_map(|diagnostic| diagnostic.range.as_mut()),
            )
        {
            if Path::new(&range.source_file) == source
                || Path::new(&range.source_file).is_absolute()
            {
                range.source_file.clone_from(&portable);
            }
        }
    }
}

fn normalize_source_attribute_cached(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    root: &Path,
    canonical_sources: &HashMap<String, PathBuf>,
) {
    let Some(source) = attributes
        .get("source_file")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let path = Path::new(source);
    if !path.is_absolute() {
        return;
    }
    let canonical_fallback;
    let canonical_path = if let Some(canonical) = canonical_sources.get(source) {
        canonical
    } else {
        canonical_fallback = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        &canonical_fallback
    };
    let Ok(relative) = canonical_path.strip_prefix(root) else {
        return;
    };
    attributes.insert(
        "source_file".to_owned(),
        serde_json::Value::String(relative.to_string_lossy().replace('\\', "/")),
    );
}

fn portable_out_of_root_source(path: &Path, root: &Path) -> String {
    use std::path::Component;

    let path = canonicalize_allow_missing(path);
    let root = canonicalize_allow_missing(root);
    let path_components = path.components().collect::<Vec<_>>();
    let root_components = root.components().collect::<Vec<_>>();
    let common = path_components
        .iter()
        .zip(&root_components)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0
        || matches!(
            (path_components.first(), root_components.first()),
            (Some(Component::Prefix(left)), Some(Component::Prefix(right))) if left != right
        )
    {
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
    }
    let upward = root_components.len().saturating_sub(common);
    if upward > 3 {
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
    }
    let mut relative = PathBuf::new();
    for _ in 0..upward {
        relative.push("..");
    }
    for component in &path_components[common..] {
        relative.push(component.as_os_str());
    }
    relative.to_string_lossy().replace('\\', "/")
}

fn canonicalize_allow_missing(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    let mut cursor = path;
    let mut suffix = Vec::new();
    while let Some(name) = cursor.file_name() {
        suffix.push(name.to_os_string());
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent;
        if let Ok(mut canonical) = fs::canonicalize(cursor) {
            for component in suffix.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
    }
    path.to_path_buf()
}

fn finalize_semantic_extraction(extraction: &mut Extraction, root: &Path) {
    for node in &mut extraction.nodes {
        normalize_source_attribute(&mut node.attributes, root);
        node.attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("semantic".to_owned()),
        );
        node.attributes.insert(
            "extractor".to_owned(),
            serde_json::Value::String(SEMANTIC_LAYER_EXTRACTOR.to_owned()),
        );
    }
    for edge in &mut extraction.edges {
        normalize_source_attribute(&mut edge.attributes, root);
        edge.attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("semantic".to_owned()),
        );
        edge.attributes.insert(
            "extractor".to_owned(),
            serde_json::Value::String(SEMANTIC_LAYER_EXTRACTOR.to_owned()),
        );
    }
    for hyperedge in &mut extraction.hyperedges {
        let Some(attributes) = hyperedge.as_object_mut() else {
            continue;
        };
        normalize_source_attribute(attributes, root);
        attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("semantic".to_owned()),
        );
        attributes.insert(
            "extractor".to_owned(),
            serde_json::Value::String(SEMANTIC_LAYER_EXTRACTOR.to_owned()),
        );
    }
}

fn ast_source_identity_map(sources: &[PathBuf], root: &Path) -> AHashMap<String, String> {
    let mut aliases = AHashMap::with_capacity(sources.len().saturating_mul(2));
    for source in sources {
        let identity = if source.is_absolute()
            && source.starts_with(root)
            && fs::symlink_metadata(source).is_ok_and(|metadata| !metadata.file_type().is_symlink())
        {
            source.clone()
        } else {
            rooted_source_identity(source, root)
        };
        let relative = identity
            .strip_prefix(root)
            .or_else(|_| source.strip_prefix(root));
        let Ok(relative) = relative else {
            continue;
        };
        let portable = make_id(&[&file_stem(relative)]);
        for path in [source.as_path(), identity.as_path()] {
            let legacy = make_id(&[&path.to_string_lossy()]);
            if legacy != portable {
                aliases.insert(legacy, portable.clone());
            }
        }
    }
    aliases
}

fn ast_extractions_are_portable(extractions: &[Extraction], root: &Path) -> bool {
    let rooted_prefix = format!("{}_", make_id(&[&root.to_string_lossy()]));
    let is_portable = |extraction: &Extraction| {
        extraction.nodes.iter().all(|node| {
            !node.id.starts_with(&rooted_prefix)
                && node
                    .attributes
                    .get("source_file")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|source| !Path::new(source).is_absolute())
        }) && extraction.edges.iter().all(|edge| {
            !edge.source.starts_with(&rooted_prefix) && !edge.target.starts_with(&rooted_prefix)
        }) && extraction
            .raw_calls
            .iter()
            .flatten()
            .all(|call| !call.caller_nid.starts_with(&rooted_prefix))
            && extraction
                .semantic_evidence
                .as_ref()
                .is_none_or(|evidence| {
                    evidence.declarations.iter().all(|fact| {
                        !fact.graph_node_id.starts_with(&rooted_prefix)
                            && !Path::new(&fact.range.source_file).is_absolute()
                    }) && evidence
                        .scopes
                        .iter()
                        .all(|fact| !Path::new(&fact.range.source_file).is_absolute())
                        && evidence
                            .bindings
                            .iter()
                            .all(|fact| !Path::new(&fact.range.source_file).is_absolute())
                        && evidence
                            .occurrences
                            .iter()
                            .all(|fact| !Path::new(&fact.range.source_file).is_absolute())
                })
    };
    if extractions.len() < 256 {
        extractions.iter().all(is_portable)
    } else {
        extractions.par_iter().all(is_portable)
    }
}

fn collect_ast_id_remap(
    extractions: &[Extraction],
    root: &Path,
    live_id_remap: &AHashMap<String, String>,
) -> AHashMap<String, String> {
    let node_ids = extractions
        .iter()
        .flat_map(|extraction| extraction.nodes.iter())
        .map(|node| node.id.as_str())
        .collect::<AHashSet<_>>();
    let root_marker = format!("{}_", make_id(&[&root.to_string_lossy()]));
    if extractions.len() < 256 {
        return collect_ast_id_remap_chunk(
            extractions,
            root,
            live_id_remap,
            &node_ids,
            &root_marker,
        );
    }
    let target_chunks = rayon::current_num_threads().saturating_mul(2).max(1);
    let chunk_size = extractions.len().div_ceil(target_chunks);
    let chunks = extractions
        .par_chunks(chunk_size)
        .map(|chunk| {
            collect_ast_id_remap_chunk(chunk, root, live_id_remap, &node_ids, &root_marker)
        })
        .collect::<Vec<_>>();
    let capacity = chunks.iter().map(|chunk| chunk.len()).sum();
    let mut remap = AHashMap::with_capacity(capacity);
    // Indexed parallel collection preserves source chunk order, so a duplicate
    // ID keeps the same last-extraction-wins behavior as the sequential pass.
    for chunk in chunks {
        remap.extend(chunk);
    }
    remap
}

fn collect_ast_id_remap_chunk(
    extractions: &[Extraction],
    root: &Path,
    live_id_remap: &AHashMap<String, String>,
    node_ids: &AHashSet<&str>,
    root_marker: &str,
) -> AHashMap<String, String> {
    // Python first rewrites every exact ID derived from a file that is really
    // present in the detected corpus. This also catches references emitted by
    // another extractor (for example an .lpk unit pointing at sample.pas), but
    // deliberately leaves IDs for absent project references untouched.
    let node_count = extractions
        .iter()
        .map(|extraction| extraction.nodes.len())
        .sum();
    let mut id_remap = AHashMap::with_capacity(node_count);
    let mut source_identities = HashMap::<String, (PathBuf, String, String)>::new();
    let mut remap_rooted_id = |id: &str| {
        if let Some(portable) = live_id_remap.get(id) {
            id_remap.insert(id.to_owned(), portable.clone());
            return;
        }
        // A rooted endpoint without a node is an unresolved file reference.
        // Root-prefixed IDs that do have nodes may instead be symbols whose
        // file-derived prefix happens to match the checkout path.
        if node_ids.contains(id) {
            return;
        }
        if let Some(relative) = id.strip_prefix(root_marker)
            && !relative.is_empty()
        {
            id_remap
                .entry(id.to_owned())
                .or_insert_with(|| relative.to_owned());
        }
    };
    for extraction in extractions {
        for edge in &extraction.edges {
            remap_rooted_id(&edge.source);
            remap_rooted_id(&edge.target);
        }
    }
    for node in extractions
        .iter()
        .flat_map(|extraction| extraction.nodes.iter())
    {
        // An exact alias for a detected file is stronger than a symbol-prefix
        // interpretation from the referring node's source file.
        if let Some(portable) = live_id_remap.get(&node.id) {
            id_remap.insert(node.id.clone(), portable.clone());
            continue;
        }
        let Some(source) = node
            .attributes
            .get("source_file")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let (absolute, old_prefix, new_prefix) = source_identities
            .entry(source.to_owned())
            .or_insert_with(|| {
                let source_path = Path::new(source);
                let old_prefix = make_id(&[&file_stem(source_path)]);
                if source_path.is_relative() {
                    return (root.join(source_path), old_prefix.clone(), old_prefix);
                }
                let absolute = rooted_source_identity(source_path, root);
                let new_prefix = absolute.strip_prefix(root).map_or_else(
                    |_| String::new(),
                    |relative| make_id(&[&file_stem(relative)]),
                );
                (absolute, old_prefix, new_prefix)
            });
        if absolute.strip_prefix(root).is_err() {
            continue;
        }
        if new_prefix.is_empty() {
            continue;
        }
        if old_prefix == new_prefix {
            continue;
        }
        if node
            .attributes
            .get("type")
            .and_then(serde_json::Value::as_str)
            == Some("package")
        {
            continue;
        }
        if node.id == make_id(&[source]) {
            id_remap.insert(node.id.clone(), new_prefix.clone());
        } else if node.id == *old_prefix
            && node
                .attributes
                .get("symbol_kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind != "file")
        {
            // Permissive grammars can surface punctuation-only constructs
            // such as a TypeScript `[]` type as symbols. Their normalized
            // label is empty, so the extractor falls back to the absolute
            // file-stem ID. Keep the symbol distinct from the file node while
            // replacing that checkout-root identity with a stable location.
            let label = node
                .attributes
                .get("label")
                .and_then(serde_json::Value::as_str)
                .map(|value| make_id(&[value]))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "symbol".to_owned());
            let line = node
                .attributes
                .get("line_start")
                .and_then(serde_json::Value::as_u64)
                .or_else(|| {
                    node.attributes
                        .get("source_location")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| value.strip_prefix('L'))
                        .and_then(|value| value.parse::<u64>().ok())
                })
                .unwrap_or_default();
            id_remap.insert(node.id.clone(), format!("{new_prefix}_{label}_{line}"));
        } else if let Some(suffix) = node.id.strip_prefix(&format!("{old_prefix}_")) {
            id_remap.insert(node.id.clone(), format!("{new_prefix}_{suffix}"));
        }
    }
    id_remap
}

fn apply_ast_id(id: &mut String, id_remap: &AHashMap<String, String>, root_marker: &str) {
    if let Some(canonical) = id_remap.get(id) {
        if id
            .strip_prefix(root_marker)
            .is_some_and(|relative| relative == canonical)
        {
            id.drain(..root_marker.len());
        } else {
            id.clone_from(canonical);
        }
    }
}

fn apply_ast_id_remap(
    extraction: &mut Extraction,
    id_remap: &AHashMap<String, String>,
    root_marker: &str,
) {
    for node in &mut extraction.nodes {
        apply_ast_id(&mut node.id, id_remap, root_marker);
    }
    for edge in &mut extraction.edges {
        apply_ast_id(&mut edge.source, id_remap, root_marker);
        apply_ast_id(&mut edge.target, id_remap, root_marker);
    }
    if let Some(calls) = extraction.raw_calls.as_mut() {
        for call in calls {
            apply_ast_id(&mut call.caller_nid, id_remap, root_marker);
        }
    }
    if let Some(evidence) = extraction.semantic_evidence.as_mut() {
        for declaration in &mut evidence.declarations {
            apply_ast_id(&mut declaration.graph_node_id, id_remap, root_marker);
        }
    }
}

fn rooted_source_identity(path: &Path, root: &Path) -> PathBuf {
    if path.is_absolute() {
        canonical_identity(path)
    } else {
        canonical_identity(&root.join(path))
    }
}

fn normalize_source_attribute(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    root: &Path,
) {
    let Some(source) = attributes
        .get("source_file")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let path = Path::new(source);
    if !path.is_absolute() {
        return;
    }
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let Ok(relative) = canonical_path.strip_prefix(root) else {
        return;
    };
    attributes.insert(
        "source_file".to_owned(),
        serde_json::Value::String(relative.to_string_lossy().replace('\\', "/")),
    );
}

fn preserve_semantic_layer(
    extraction: &mut Extraction,
    graph_path: &Path,
    root: &Path,
    refreshed: &HashSet<PathBuf>,
) {
    let Ok(existing) = V1GraphDocument::load(graph_path) else {
        return;
    };
    let mut existing_raw = extraction_from_v1(&existing);
    let current_ast_by_key = extraction
        .nodes
        .iter()
        .filter_map(|node| raw_node_match_key(node).map(|key| (key, node.id.clone())))
        .collect::<HashMap<_, _>>();
    let typed_ast_remap = existing
        .nodes
        .iter()
        .filter(|node| node_has_origin(node, EvidenceOrigin::Ast))
        .filter_map(|node| {
            typed_node_match_key(node)
                .and_then(|key| current_ast_by_key.get(&key))
                .map(|current| (node.id.clone(), current.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut dropped_edges = HashSet::new();
    let mut remapped_edges = HashSet::new();
    let mut remap_diagnostics = Vec::new();
    let mut dropped_without_site = 0_usize;
    for (index, edge) in existing_raw.edges.iter_mut().enumerate() {
        let source = typed_ast_remap
            .get(&edge.source)
            .cloned()
            .unwrap_or_else(|| edge.source.clone());
        let target = typed_ast_remap
            .get(&edge.target)
            .cloned()
            .unwrap_or_else(|| edge.target.clone());
        if source == edge.source && target == edge.target {
            continue;
        }
        if !has_exact_remap_site(&edge.attributes) {
            dropped_edges.insert(index);
            dropped_without_site = dropped_without_site.saturating_add(1);
            if remap_diagnostics.len() < MAX_INCREMENTAL_REMAP_DIAGNOSTICS {
                let mut related_ids = edge
                    .attributes
                    .get(compass_model::provenance::TRUSTED_EDGE_RECORD_ATTRIBUTE)
                    .and_then(|record| record.get("id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .into_iter()
                    .collect::<Vec<_>>();
                related_ids.extend([edge.source.clone(), edge.target.clone()]);
                remap_diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    code: INCREMENTAL_REMAP_DROP_DIAGNOSTIC.to_owned(),
                    message: "dropped preserved edge whose AST endpoint changed without an authoritative producer wiring site".to_owned(),
                    anchor: None,
                    related_ids,
                });
            }
            continue;
        }
        edge.source = source;
        edge.target = target;
        append_endpoint_rewrite_evidence(
            &mut edge.attributes,
            EndpointRewriteEvidence {
                rule: EndpointRewriteRule::IncrementalAstEndpointRemap,
                score: 1.0,
            },
        );
        remapped_edges.insert(index);
    }
    if dropped_without_site > MAX_INCREMENTAL_REMAP_DIAGNOSTICS {
        remap_diagnostics.push(GraphDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: INCREMENTAL_REMAP_TRUNCATION_DIAGNOSTIC.to_owned(),
            message: format!(
                "omitted {} additional incremental remap diagnostics",
                dropped_without_site - MAX_INCREMENTAL_REMAP_DIAGNOSTICS
            ),
            anchor: None,
            related_ids: Vec::new(),
        });
    }
    let mut fresh_edge_sites = HashMap::<IncrementalEdgeKey, Vec<SourceAnchor>>::new();
    let raw_edge_sites = canonical_raw_edge_sites(&extraction.edges, root);
    for (edge, site) in extraction.edges.iter().zip(raw_edge_sites) {
        let Some(key) = incremental_edge_key(edge) else {
            continue;
        };
        let Some(site) = site else {
            continue;
        };
        fresh_edge_sites.entry(key).or_default().push(site);
    }
    for sites in fresh_edge_sites.values_mut() {
        sites.sort_by(|left, right| {
            incremental_anchor_key(left).cmp(&incremental_anchor_key(right))
        });
    }
    let mut mixed_edges = HashMap::<IncrementalEdgeKey, Vec<(usize, SourceAnchor)>>::new();
    for (index, (raw, typed)) in existing_raw
        .edges
        .iter()
        .zip(existing.links.iter())
        .enumerate()
    {
        if dropped_edges.contains(&index)
            || !typed
                .evidence
                .iter()
                .any(|evidence| evidence.origin == EvidenceOrigin::Ast)
            || !typed.evidence.iter().any(is_semantic_layer_evidence)
        {
            continue;
        }
        let Some(key) = incremental_edge_key(raw) else {
            continue;
        };
        let Some(site) = typed.relationship_site.clone() else {
            continue;
        };
        mixed_edges.entry(key).or_default().push((index, site));
    }
    let mut refreshed_mixed_sites = HashMap::<usize, SourceAnchor>::new();
    let mut semantic_only_mixed_edges = HashSet::<usize>::new();
    for (key, mut prior) in mixed_edges {
        let Some(current) = fresh_edge_sites.get(&key) else {
            semantic_only_mixed_edges.extend(prior.into_iter().map(|(index, _)| index));
            continue;
        };
        prior.sort_by(|left, right| {
            incremental_anchor_key(&left.1)
                .cmp(&incremental_anchor_key(&right.1))
                .then_with(|| left.0.cmp(&right.0))
        });
        // Exact shared sites are authoritative even when occurrence cardinality changed.
        // Equal residual cardinality has one stable sorted bijection (the existing moved-site
        // behavior). Unequal residual cardinality is ambiguous: fresh occurrences stay AST-only,
        // while prior occurrences may survive only through semantic-only revalidation below.
        let mut matched_current = vec![false; current.len()];
        let mut unmatched_prior = Vec::new();
        for (index, prior_site) in prior {
            let exact = current
                .iter()
                .enumerate()
                .position(|(current_index, site)| {
                    !matched_current[current_index]
                        && incremental_anchor_key(&prior_site) == incremental_anchor_key(site)
                });
            if let Some(current_index) = exact {
                matched_current[current_index] = true;
                refreshed_mixed_sites.insert(index, current[current_index].clone());
            } else {
                unmatched_prior.push((index, prior_site));
            }
        }
        let unmatched_current = current
            .iter()
            .enumerate()
            .filter(|(index, _)| !matched_current[*index])
            .map(|(_, site)| site)
            .collect::<Vec<_>>();
        if unmatched_prior.len() == unmatched_current.len() {
            for ((index, _), site) in unmatched_prior.into_iter().zip(unmatched_current) {
                refreshed_mixed_sites.insert(index, site.clone());
            }
        } else {
            semantic_only_mixed_edges.extend(unmatched_prior.into_iter().map(|(index, _)| index));
        }
    }
    let prior_diagnostics = existing_raw.extensions.remove(GRAPH_DIAGNOSTICS_EXTENSION);
    append_graph_diagnostics(extraction, prior_diagnostics, remap_diagnostics);
    for prior in &existing.nodes {
        let Some(current_raw_id) = typed_ast_remap.get(&prior.id) else {
            continue;
        };
        let Some(current) = extraction
            .nodes
            .iter_mut()
            .find(|node| node.id == *current_raw_id)
        else {
            continue;
        };
        let fresh_site = raw_source_anchor(&current.attributes, root);
        let semantic_evidence = prior
            .evidence
            .iter()
            .filter(|evidence| is_semantic_layer_evidence(evidence))
            .cloned()
            .map(|mut evidence| {
                rebind_preserved_node_evidence(
                    &mut evidence,
                    prior.source.as_ref(),
                    fresh_site.as_ref(),
                );
                evidence
            })
            .filter_map(|evidence| serde_json::to_value(evidence).ok())
            .collect::<Vec<_>>();
        if semantic_evidence.is_empty() {
            continue;
        }
        let evidence = current
            .attributes
            .entry(COALESCED_NODE_EVIDENCE_ATTRIBUTE.to_owned())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        let Some(evidence) = evidence.as_array_mut() else {
            continue;
        };
        evidence.extend(semantic_evidence);
        evidence.sort_by_cached_key(serde_json::Value::to_string);
        evidence.dedup();
    }
    let preserved_node_ids = existing
        .nodes
        .iter()
        .filter(|node| {
            !typed_ast_remap.contains_key(&node.id)
                && node.evidence.iter().any(is_semantic_layer_evidence)
        })
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut preserved_nodes = existing_raw
        .nodes
        .into_iter()
        .filter(|node| {
            preserved_node_ids.contains(node.id.as_str())
                && !source_in_set(node.attributes.get("source_file"), root, refreshed)
                && !source_was_deleted(node.attributes.get("source_file"), root)
        })
        .collect::<Vec<_>>();
    for node in &mut preserved_nodes {
        strip_preserved_ast_node_evidence(&mut node.attributes);
    }
    let all_ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .chain(preserved_nodes.iter().map(|node| node.id.clone()))
        .collect::<std::collections::HashSet<_>>();
    let mut preserved_edges = existing_raw
        .edges
        .into_iter()
        .zip(existing.links)
        .enumerate()
        .filter_map(|(index, (mut raw, typed))| {
            if dropped_edges.contains(&index) {
                return None;
            }
            let preserve = typed.evidence.iter().any(is_semantic_layer_evidence)
                && all_ids.contains(&raw.source)
                && all_ids.contains(&raw.target)
                && !source_in_set(raw.attributes.get("source_file"), root, refreshed)
                && !source_was_deleted(raw.attributes.get("source_file"), root);
            if !preserve {
                return None;
            }
            if let Some(site) = refreshed_mixed_sites.get(&index) {
                refresh_preserved_mixed_edge(&mut raw.attributes, &raw.source, &raw.target, site);
            } else if semantic_only_mixed_edges.contains(&index)
                && !retain_preserved_mixed_edge_as_semantic_only(
                    &mut raw.attributes,
                    remapped_edges.contains(&index),
                )
            {
                return None;
            }
            Some(raw)
        })
        .collect::<Vec<_>>();
    extraction
        .nodes
        .extend(preserved_nodes.drain(..).map(|node| RawNodeRecord {
            id: node.id,
            attributes: node.attributes,
        }));
    extraction
        .edges
        .extend(preserved_edges.drain(..).map(|edge| RawEdgeRecord {
            source: edge.source,
            target: edge.target,
            attributes: edge.attributes,
        }));
}

type IncrementalEdgeKey = (String, String, String, Option<String>);

fn incremental_anchor_key(anchor: &SourceAnchor) -> (&str, u64, u64) {
    (&anchor.file, anchor.start_byte, anchor.end_byte)
}

fn incremental_edge_key(edge: &RawEdgeRecord) -> Option<IncrementalEdgeKey> {
    let relation = edge.attributes.get("relation")?.as_str()?;
    Some((
        edge.source.clone(),
        edge.target.clone(),
        canonical_edge_kind(relation)?.as_str().to_owned(),
        edge.attributes
            .get(OCCURRENCE_RULE_ATTRIBUTE)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    ))
}

fn refresh_preserved_mixed_edge(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    source: &str,
    target: &str,
    fresh_site: &SourceAnchor,
) {
    let Some(record) = attributes.get_mut(TRUSTED_EDGE_RECORD_ATTRIBUTE) else {
        return;
    };
    let Ok(mut edge) =
        serde_json::from_value::<compass_model::code_graph::EdgeRecord>(record.clone())
    else {
        return;
    };
    let prior_site = edge.relationship_site.clone();
    edge.evidence.retain(|evidence| {
        is_semantic_layer_evidence(evidence)
            && evidence.rule.as_deref()
                != Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
    });
    for evidence in &mut edge.evidence {
        if evidence.wiring_site.as_ref() == prior_site.as_ref() {
            evidence.wiring_site = Some(fresh_site.clone());
        }
        for anchor in &mut evidence.anchors {
            if Some(&*anchor) == prior_site.as_ref() {
                anchor.clone_from(fresh_site);
            }
        }
    }
    edge.source = source.to_owned();
    edge.target = target.to_owned();
    edge.relationship_site = Some(fresh_site.clone());
    *record = serde_json::to_value(edge).unwrap_or(serde_json::Value::Null);
    rebind_raw_anchor(attributes, fresh_site);
}

fn retain_preserved_mixed_edge_as_semantic_only(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    remapped_in_current_pass: bool,
) -> bool {
    if remapped_in_current_pass
        && !attributes
            .get("_endpoint_rewrite_rules")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|rewrites| {
                rewrites.iter().any(|rewrite| {
                    rewrite.get("rule").and_then(serde_json::Value::as_str)
                        == Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
                })
            })
    {
        return false;
    }
    let Some(record) = attributes.get_mut(TRUSTED_EDGE_RECORD_ATTRIBUTE) else {
        return false;
    };
    let Ok(mut edge) =
        serde_json::from_value::<compass_model::code_graph::EdgeRecord>(record.clone())
    else {
        return false;
    };
    edge.evidence.retain(|evidence| {
        is_semantic_layer_evidence(evidence)
            && evidence.rule.as_deref()
                != Some(EndpointRewriteRule::IncrementalAstEndpointRemap.as_str())
    });
    let Some(primary) = edge
        .evidence
        .iter()
        .find(|evidence| {
            evidence
                .rule
                .as_deref()
                .and_then(EndpointRewriteRule::from_wire_name)
                .is_none()
        })
        .cloned()
    else {
        // Endpoint rewrites describe transport, not an independently justified relationship.
        // Without a non-AST producer assertion, an unmatched prior occurrence is stale.
        return false;
    };
    edge.relationship_site = None;
    *record = serde_json::to_value(edge).unwrap_or(serde_json::Value::Null);
    for key in [
        "source_file",
        "source_location",
        "source_anchor",
        "start_byte",
        "end_byte",
        "line_start",
        "line_end",
        "column_start",
        "column_end",
        "_coalesced_edge_evidence",
    ] {
        attributes.remove(key);
    }
    if !remapped_in_current_pass {
        attributes.remove("_endpoint_rewrite_rules");
        attributes.remove(CONSUME_INCREMENTAL_ENDPOINT_REMAP_ATTRIBUTE);
    } else {
        attributes.insert(
            CONSUME_INCREMENTAL_ENDPOINT_REMAP_ATTRIBUTE.to_owned(),
            serde_json::Value::Bool(true),
        );
    }
    project_preserved_semantic_evidence(attributes, &primary);
    true
}

fn project_preserved_semantic_evidence(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    evidence: &compass_model::provenance::Provenance,
) {
    attributes.insert(
        "_origin".to_owned(),
        serde_json::Value::String(evidence.origin.as_str().to_owned()),
    );
    attributes.insert(
        "confidence".to_owned(),
        serde_json::Value::String(evidence.confidence.legacy_str().to_owned()),
    );
    attributes.insert(
        "extractor".to_owned(),
        serde_json::Value::String(evidence.extractor.clone()),
    );
    if let Some(rule) = &evidence.rule {
        attributes.insert("rule".to_owned(), serde_json::Value::String(rule.clone()));
    } else {
        attributes.remove("rule");
    }
    if let Some(score) = evidence.score {
        attributes.insert("confidence_score".to_owned(), score.into());
    } else {
        attributes.remove("confidence_score");
    }
    if evidence.candidates.is_empty() {
        attributes.remove("candidates");
    } else {
        attributes.insert(
            "candidates".to_owned(),
            serde_json::to_value(&evidence.candidates).unwrap_or(serde_json::Value::Null),
        );
    }
    let semantic_site = evidence
        .wiring_site
        .as_ref()
        .or_else(|| evidence.anchors.first());
    if let Some(site) = semantic_site {
        attributes.insert(
            "source_file".to_owned(),
            serde_json::Value::String(site.file.clone()),
        );
        attributes.insert(
            "source_anchor".to_owned(),
            serde_json::to_value(site).unwrap_or(serde_json::Value::Null),
        );
    }
}

fn rebind_raw_anchor(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    fresh_site: &SourceAnchor,
) {
    attributes.insert(
        "source_file".to_owned(),
        serde_json::Value::String(fresh_site.file.clone()),
    );
    attributes.insert(
        "source_location".to_owned(),
        serde_json::Value::String(format!(
            "L{}:{}-L{}:{}",
            fresh_site.start_line,
            fresh_site.start_column,
            fresh_site.end_line,
            fresh_site.end_column
        )),
    );
    attributes.insert(
        "source_anchor".to_owned(),
        serde_json::to_value(fresh_site).unwrap_or(serde_json::Value::Null),
    );
    for (key, value) in [
        ("start_byte", fresh_site.start_byte.into()),
        ("end_byte", fresh_site.end_byte.into()),
        ("line_start", fresh_site.start_line.into()),
        ("line_end", fresh_site.end_line.into()),
        ("column_start", fresh_site.start_column.into()),
        ("column_end", fresh_site.end_column.into()),
    ] {
        if attributes.contains_key(key) {
            attributes.insert(key.to_owned(), value);
        }
    }
}

fn strip_preserved_ast_node_evidence(attributes: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(record) = attributes.get_mut(compass_model::provenance::TRUSTED_NODE_RECORD_ATTRIBUTE)
    else {
        return;
    };
    let Ok(mut node) =
        serde_json::from_value::<compass_model::code_graph::NodeRecord>(record.clone())
    else {
        return;
    };
    node.evidence.retain(is_semantic_layer_evidence);
    *record = serde_json::to_value(node).unwrap_or(serde_json::Value::Null);
}

fn rebind_preserved_node_evidence(
    evidence: &mut compass_model::provenance::Provenance,
    prior_site: Option<&SourceAnchor>,
    fresh_site: Option<&SourceAnchor>,
) {
    let (Some(prior_site), Some(fresh_site)) = (prior_site, fresh_site) else {
        return;
    };
    if evidence.wiring_site.as_ref() == Some(prior_site) {
        evidence.wiring_site = Some(fresh_site.clone());
    }
    for anchor in &mut evidence.anchors {
        if anchor == prior_site {
            anchor.clone_from(fresh_site);
        }
    }
}

fn raw_source_anchor(
    attributes: &serde_json::Map<String, serde_json::Value>,
    root: &Path,
) -> Option<SourceAnchor> {
    let mut site = attributes
        .get("source_anchor")
        .and_then(|value| serde_json::from_value::<SourceAnchor>(value.clone()).ok())
        .or_else(|| {
            Some(SourceAnchor {
                file: attributes.get("source_file")?.as_str()?.to_owned(),
                start_byte: attributes.get("start_byte")?.as_u64()?,
                end_byte: attributes.get("end_byte")?.as_u64()?,
                start_line: u32::try_from(attributes.get("line_start")?.as_u64()?).ok()?,
                start_column: u32::try_from(attributes.get("column_start")?.as_u64()?).ok()?,
                end_line: u32::try_from(attributes.get("line_end")?.as_u64()?).ok()?,
                end_column: u32::try_from(attributes.get("column_end")?.as_u64()?).ok()?,
            })
        })?;
    let path = Path::new(&site.file);
    if path.is_absolute()
        && let Ok(relative) = path.strip_prefix(root)
    {
        site.file = relative.to_string_lossy().replace('\\', "/");
    }
    Some(site)
}

fn has_exact_remap_site(attributes: &serde_json::Map<String, serde_json::Value>) -> bool {
    attributes
        .get("source_anchor")
        .and_then(|value| serde_json::from_value::<SourceAnchor>(value.clone()).ok())
        .is_some_and(|site| site.is_valid() && site.start_byte < site.end_byte)
}

fn append_graph_diagnostics(
    extraction: &mut Extraction,
    prior: Option<serde_json::Value>,
    diagnostics: Vec<GraphDiagnostic>,
) {
    let mut values = extraction
        .extensions
        .remove(GRAPH_DIAGNOSTICS_EXTENSION)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    values.extend(
        prior
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default(),
    );
    values.extend(
        diagnostics
            .into_iter()
            .filter_map(|diagnostic| serde_json::to_value(diagnostic).ok()),
    );
    let mut remap_drops = Vec::new();
    let mut unrelated = Vec::new();
    let mut omitted = 0_usize;
    for value in values {
        match value
            .get("code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
        {
            INCREMENTAL_REMAP_DROP_DIAGNOSTIC => remap_drops.push(value),
            INCREMENTAL_REMAP_TRUNCATION_DIAGNOSTIC => {
                omitted = omitted.saturating_add(remap_truncation_count(&value));
            }
            _ => unrelated.push(value),
        }
    }
    remap_drops.sort_by_cached_key(serde_json::Value::to_string);
    remap_drops.dedup();
    if remap_drops.len() > MAX_INCREMENTAL_REMAP_DIAGNOSTICS {
        omitted = omitted.saturating_add(
            remap_drops
                .len()
                .saturating_sub(MAX_INCREMENTAL_REMAP_DIAGNOSTICS),
        );
        remap_drops.truncate(MAX_INCREMENTAL_REMAP_DIAGNOSTICS);
    }
    unrelated.append(&mut remap_drops);
    if omitted > 0
        && let Ok(summary) = serde_json::to_value(GraphDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: INCREMENTAL_REMAP_TRUNCATION_DIAGNOSTIC.to_owned(),
            message: format!("omitted {omitted} additional incremental remap diagnostics"),
            anchor: None,
            related_ids: Vec::new(),
        })
    {
        unrelated.push(summary);
    }
    unrelated.sort_by_cached_key(serde_json::Value::to_string);
    unrelated.dedup();
    extraction.extensions.insert(
        GRAPH_DIAGNOSTICS_EXTENSION.to_owned(),
        serde_json::Value::Array(unrelated),
    );
}

fn remap_truncation_count(value: &serde_json::Value) -> usize {
    value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .and_then(|message| {
            message
                .strip_prefix("omitted ")
                .and_then(|message| {
                    message.strip_suffix(" additional incremental remap diagnostics")
                })
                .and_then(|count| count.parse().ok())
        })
        .unwrap_or_default()
}

fn raw_node_match_key(node: &RawNodeRecord) -> Option<(String, String, String)> {
    Some((
        node.attributes.get("source_file")?.as_str()?.to_owned(),
        node.label().to_owned(),
        node.attributes
            .get("symbol_kind")
            .or_else(|| node.attributes.get("type"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("symbol")
            .to_owned(),
    ))
}

fn typed_node_match_key(
    node: &compass_model::code_graph::NodeRecord,
) -> Option<(String, String, String)> {
    Some((
        node.source_file()?.to_owned(),
        node.label().to_owned(),
        node.kind.as_str().to_owned(),
    ))
}

fn node_has_origin(node: &compass_model::code_graph::NodeRecord, origin: EvidenceOrigin) -> bool {
    node.evidence
        .iter()
        .any(|evidence| evidence.origin == origin)
}

fn is_semantic_layer_evidence(evidence: &Provenance) -> bool {
    evidence.extractor == SEMANTIC_LAYER_EXTRACTOR
}

fn node_has_semantic_layer_evidence(node: &compass_model::code_graph::NodeRecord) -> bool {
    node.evidence.iter().any(is_semantic_layer_evidence)
}

fn edge_has_semantic_layer_evidence(edge: &compass_model::code_graph::EdgeRecord) -> bool {
    edge.evidence.iter().any(is_semantic_layer_evidence)
}

fn canonical_source_set(paths: &[PathBuf], root: &Path) -> HashSet<PathBuf> {
    paths
        .iter()
        .map(|path| {
            let absolute = if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            };
            canonical_identity(&absolute)
        })
        .collect()
}

fn source_in_set(
    value: Option<&serde_json::Value>,
    root: &Path,
    sources: &HashSet<PathBuf>,
) -> bool {
    let Some(source) = value.and_then(serde_json::Value::as_str) else {
        return false;
    };
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    sources.contains(&canonical_identity(&absolute))
}

fn semantic_source_set(fragment: &serde_json::Value, root: &Path) -> HashSet<PathBuf> {
    ["nodes", "edges", "hyperedges"]
        .into_iter()
        .filter_map(|bucket| fragment.get(bucket).and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(|item| item.get("source_file").and_then(serde_json::Value::as_str))
        .map(|source| {
            let path = Path::new(source);
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            canonical_identity(&absolute)
        })
        .collect()
}

fn stale_semantic_sources(
    graph_path: &Path,
    root: &Path,
    detected: &BTreeMap<String, Vec<String>>,
) -> HashSet<PathBuf> {
    let Ok(existing) = V1GraphDocument::load(graph_path) else {
        return HashSet::new();
    };
    let live = detected
        .values()
        .flatten()
        .map(|path| canonical_identity(Path::new(path)))
        .collect::<HashSet<_>>();
    let mut stale = existing
        .nodes
        .iter()
        .filter(|node| node_has_semantic_layer_evidence(node))
        .filter_map(|node| semantic_path_under_root(node.source_file(), root))
        .filter(|source| !live.contains(source))
        .collect::<HashSet<_>>();
    stale.extend(
        existing
            .links
            .iter()
            .filter(|edge| edge_has_semantic_layer_evidence(edge))
            .filter_map(|edge| semantic_path_under_root(edge.source_file(), root))
            .filter(|source| !live.contains(source)),
    );
    stale
}

fn semantic_path_under_root(source: Option<&str>, root: &Path) -> Option<PathBuf> {
    let source = source?;
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let identity = canonical_identity(&absolute);
    identity.starts_with(root).then_some(identity)
}

fn semantic_tokens(semantic: Option<&SemanticLayer>) -> (u64, u64) {
    let numeric = |key| {
        semantic
            .and_then(|layer| layer.fragment.get(key))
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
                    .or_else(|| value.as_f64().map(|number| number.max(0.0) as u64))
            })
            .unwrap_or_default()
    };
    (numeric("input_tokens"), numeric("output_tokens"))
}

pub(super) fn semantic_is_incomplete(layer: &SemanticLayer, root: &Path) -> bool {
    if !layer.partial_files.is_empty()
        || layer
            .fragment
            .get("failed_chunks")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|count| count > 0)
    {
        return true;
    }
    let extracted = semantic_source_set(&layer.fragment, root);
    canonical_source_set(&layer.refreshed_files, root)
        .iter()
        .any(|source| !extracted.contains(source))
}

fn save_build_manifest(
    manifest: &mut Manifest,
    files: &BTreeMap<String, Vec<String>>,
    path: &Path,
    root: &Path,
    semantic: Option<&SemanticLayer>,
) -> Result<(), CoreError> {
    let Some(layer) = semantic.filter(|layer| !semantic_layer_is_empty(layer)) else {
        let scan_corpus = files.values().flatten().cloned().collect::<BTreeSet<_>>();
        manifest.save(
            files,
            path,
            ManifestKind::Ast,
            Some(root),
            Some(&scan_corpus),
            None,
        )?;
        return Ok(());
    };

    let extracted = semantic_source_set(&layer.fragment, root);
    let partial = canonical_source_set(&layer.partial_files, root);
    let semantic_types = ["document", "paper", "image"];
    let stamped = files
        .iter()
        .map(|(file_type, bucket)| {
            let retained = bucket
                .iter()
                .filter(|file| {
                    if !semantic_types.contains(&file_type.as_str()) {
                        return true;
                    }
                    let canonical = canonical_identity(Path::new(file));
                    extracted.contains(&canonical) && !partial.contains(&canonical)
                })
                .cloned()
                .collect();
            (file_type.clone(), retained)
        })
        .collect::<BTreeMap<_, _>>();
    let scan_corpus = files.values().flatten().cloned().collect::<BTreeSet<_>>();
    let successfully_stamped = stamped
        .values()
        .flatten()
        .map(|file| canonical_identity(Path::new(file)))
        .collect::<HashSet<_>>();
    let clear_semantic = layer
        .refreshed_files
        .iter()
        .map(|file| {
            let absolute = if file.is_absolute() {
                file.clone()
            } else {
                root.join(file)
            };
            canonical_identity(&absolute)
        })
        .filter(|file| !successfully_stamped.contains(file))
        .map(|file| file.to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    manifest.save(
        &stamped,
        path,
        ManifestKind::Both,
        Some(root),
        Some(&scan_corpus),
        Some(&clear_semantic),
    )?;
    Ok(())
}

fn semantic_document_sources(graph_path: &Path, root: &Path) -> HashSet<PathBuf> {
    let Ok(existing) = V1GraphDocument::load(graph_path) else {
        return HashSet::new();
    };
    existing
        .nodes
        .into_iter()
        .filter(|node| node_has_semantic_layer_evidence(node) && node.kind == NodeKind::Resource)
        .filter_map(|node| {
            node.source_file().map(Path::new).map(|path| {
                if path.is_absolute() {
                    canonical_identity(path)
                } else {
                    canonical_identity(&root.join(path))
                }
            })
        })
        .collect()
}

fn canonical_identity(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn graph_configuration_digest(
    options: &BuildOptions,
    output_dir: &Path,
) -> Result<String, CoreError> {
    let bytes = serde_json::to_vec(&build_profile(options)).map_err(|source| {
        CoreError::SerializeExtraction {
            path: output_dir.join("graph.json"),
            source,
        }
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

struct PublicationEvidence<'a> {
    detection: &'a Detection,
    semantic: Option<&'a SemanticLayer>,
    extraction_failures: &'a BTreeMap<PathBuf, String>,
    extraction_partials: &'a BTreeMap<PathBuf, String>,
}

fn published_v1_document(
    document: &GraphDocument,
    communities: &compass_graph::Communities,
    labels: &BTreeMap<usize, String>,
    root: &Path,
    evidence: PublicationEvidence<'_>,
    configuration_digest: String,
    source_commit: Option<&str>,
) -> Result<PublicationOutcome, CoreError> {
    let mut publication_profile_started = Instant::now();
    let mut publication_source = document.clone();
    profile_internal(
        "graph publication source clone",
        &mut publication_profile_started,
    );
    let node_communities = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<HashMap<_, _>>();
    for node in &mut publication_source.nodes {
        let Some(&community_index) = node_communities.get(node.id.as_str()) else {
            continue;
        };
        let community = u64::try_from(community_index)
            .map_err(|_| CoreError::InvalidBuildState("community ID exceeds u64".to_owned()))?;
        node.attributes
            .insert("community".to_owned(), serde_json::Value::from(community));
        if let Some(label) = labels.get(&community_index) {
            node.attributes.insert(
                "community_name".to_owned(),
                serde_json::Value::String(label.clone()),
            );
        }
    }
    profile_internal(
        "graph publication community projection",
        &mut publication_profile_started,
    );
    let published = normalize_document_v1_with_inventory_best_effort_owned(
        publication_source,
        root,
        configuration_digest,
        source_commit,
        detection_inventory(
            evidence.detection,
            evidence.semantic,
            evidence.extraction_failures,
            evidence.extraction_partials,
            root,
        ),
    )?;
    profile_internal(
        "graph publication v1 boundary",
        &mut publication_profile_started,
    );
    Ok(published)
}

fn detection_inventory(
    detection: &Detection,
    semantic: Option<&SemanticLayer>,
    extraction_failures: &BTreeMap<PathBuf, String>,
    extraction_partials: &BTreeMap<PathBuf, String>,
    root: &Path,
) -> Vec<InventoryEvidence> {
    let mut inventory = detection
        .files
        .iter()
        .flat_map(|(category, paths)| {
            paths.iter().map(move |path| {
                let absolute = if Path::new(path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    root.join(path)
                };
                let identity = canonical_identity(&absolute);
                let spec = Registry::resolve(Path::new(path));
                let status = if extraction_failures.contains_key(&identity) {
                    ExtractionStatus::ParseFailure
                } else if extraction_partials.contains_key(&identity)
                    || semantic.is_some_and(|layer| {
                        layer.partial_files.iter().any(|partial| {
                            canonical_identity(partial) == canonical_identity(&absolute)
                        })
                    })
                {
                    ExtractionStatus::Partial
                } else if source_is_generated(&absolute) {
                    ExtractionStatus::Generated
                } else {
                    match category.as_str() {
                        "image" | "video" => ExtractionStatus::Binary,
                        "code" | "document" if spec.is_some() => ExtractionStatus::Extracted,
                        _ => ExtractionStatus::Unsupported,
                    }
                };
                InventoryEvidence {
                    path: PathBuf::from(path),
                    language: spec.map(|spec| spec.name.to_owned()),
                    producer: spec.map_or_else(
                        || "compass.files.detect".to_owned(),
                        |spec| format!("compass.languages.{}", spec.name),
                    ),
                    status,
                    reason: extraction_failures
                        .get(&identity)
                        .or_else(|| extraction_partials.get(&identity))
                        .cloned()
                        .or_else(|| {
                            (status == ExtractionStatus::Unsupported)
                                .then(|| format!("no extractor for detected {category} file"))
                        }),
                }
            })
        })
        .collect::<Vec<_>>();
    inventory.extend(detection.unclassified.iter().map(|path| InventoryEvidence {
        path: PathBuf::from(path),
        language: None,
        producer: "compass.files.detect".to_owned(),
        status: ExtractionStatus::Unsupported,
        reason: Some("unclassified by detector".to_owned()),
    }));
    inventory.extend(detection.ignored.iter().map(|path| InventoryEvidence {
        path: PathBuf::from(path),
        language: None,
        producer: "compass.files.detect".to_owned(),
        status: ExtractionStatus::Excluded,
        reason: Some("excluded by ignore policy".to_owned()),
    }));
    inventory
}

fn prepare_extraction_for_publication(
    path: &Path,
    extraction: &mut Extraction,
    root: &Path,
    failures: &mut BTreeMap<PathBuf, String>,
    partials: &mut BTreeMap<PathBuf, String>,
) {
    let identity = canonical_identity(path);
    make_framework_fact_sources_portable(extraction, root);
    if let Some(error) = extraction.error.take() {
        failures.insert(identity, portable_diagnostic_reason(&error, path, root));
        *extraction = Extraction::default();
        extraction.raw_calls = None;
        return;
    }
    if extraction
        .extensions
        .get(EXTRACTION_QUALITY_EXTENSION)
        .and_then(serde_json::Value::as_str)
        != Some(EXTRACTION_QUALITY_PARTIAL)
    {
        return;
    }
    let reason = extraction
        .extensions
        .get(EXTRACTION_QUALITY_REASON_EXTENSION)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("extraction completed with parser recovery");
    partials.insert(identity, portable_diagnostic_reason(reason, path, root));
    extraction.edges.clear();
    extraction.hyperedges.clear();
    extraction.framework_facts.clear();
    extraction.raw_calls = None;
}

fn make_framework_fact_sources_portable(extraction: &mut Extraction, root: &Path) {
    if extraction.semantic_evidence.is_none() {
        return;
    }
    for fact in &mut extraction.framework_facts {
        let source_file = match fact {
            RawFrameworkFact::Route(route) => &mut route.anchor.source_file,
            RawFrameworkFact::Domain(domain) => &mut domain.anchor.source_file,
        };
        let path = Path::new(source_file);
        if !path.is_absolute() {
            *source_file = source_file.replace('\\', "/");
            continue;
        }
        let canonical = canonicalize_allow_missing(path);
        *source_file = canonical.strip_prefix(root).map_or_else(
            |_| portable_out_of_root_source(&canonical, root),
            |relative| relative.to_string_lossy().replace('\\', "/"),
        );
    }
}

fn absolutize_cached_framework_fact_sources(extraction: &mut Extraction, root: &Path) {
    for fact in &mut extraction.framework_facts {
        let source_file = match fact {
            RawFrameworkFact::Route(route) => &mut route.anchor.source_file,
            RawFrameworkFact::Domain(domain) => &mut domain.anchor.source_file,
        };
        if !source_file.is_empty() && !Path::new(source_file).is_absolute() {
            *source_file = root.join(&*source_file).to_string_lossy().into_owned();
        }
    }
}

fn portable_diagnostic_reason(reason: &str, path: &Path, root: &Path) -> String {
    let root_text = root.to_string_lossy();
    let path_text = path.to_string_lossy();
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let sanitized = reason
        .replace(root_text.as_ref(), ".")
        .replace(path_text.as_ref(), &relative)
        .replace('\\', "/");
    sanitized.chars().take(512).collect()
}

fn source_is_generated(path: &Path) -> bool {
    fs::read(path).ok().is_some_and(|bytes| {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2_048)]);
        ["DO NOT EDIT", "@generated", "Generated by"]
            .iter()
            .any(|marker| head.contains(marker))
    })
}

fn source_was_deleted(value: Option<&serde_json::Value>, root: &Path) -> bool {
    let Some(source) = value.and_then(serde_json::Value::as_str) else {
        return false;
    };
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    Registry::resolve(path).is_some() && !absolute.exists()
}

fn report_root_label(path: &Path) -> String {
    if path.is_absolute() {
        return path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
    }
    if path == Path::new(".") {
        return std::env::current_dir()
            .ok()
            .and_then(|directory| directory.file_name().map(|value| value.to_owned()))
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_owned());
    }
    path.to_string_lossy().into_owned()
}

fn topology_is_unchanged(existing: &GraphDocument, candidate: &GraphDocument) -> bool {
    if existing.directed != candidate.directed || existing.multigraph != candidate.multigraph {
        return false;
    }
    let mut existing_nodes = existing
        .nodes
        .iter()
        .map(canonical_node)
        .collect::<Vec<_>>();
    let mut candidate_nodes = candidate
        .nodes
        .iter()
        .map(canonical_node)
        .collect::<Vec<_>>();
    existing_nodes.sort();
    candidate_nodes.sort();
    if existing_nodes != candidate_nodes {
        return false;
    }
    let mut existing_edges = existing
        .links
        .iter()
        .map(canonical_edge)
        .collect::<Vec<_>>();
    let mut candidate_edges = candidate
        .links
        .iter()
        .map(canonical_edge)
        .collect::<Vec<_>>();
    existing_edges.sort();
    candidate_edges.sort();
    existing_edges == candidate_edges
        && canonical_hyperedges(existing) == canonical_hyperedges(candidate)
}

fn update_artifacts_complete(output_dir: &Path) -> bool {
    [
        "graph.json",
        "GRAPH_REPORT.md",
        ".compass_labels.json",
        ".compass_root",
        GRAPH_OVERVIEW_FILE,
    ]
    .into_iter()
    .all(|name| output_dir.join(name).is_file())
}

pub(crate) fn write_graph_overview_artifact(
    document: &GraphDocument,
    communities: &compass_graph::Communities,
    labels: &BTreeMap<usize, String>,
    output_dir: &Path,
) -> Result<(), CoreError> {
    let overview_path = output_dir.join(GRAPH_OVERVIEW_FILE);
    let overview = graph_view_model_document(
        document,
        communities,
        output_dir.join("graph.json"),
        &HtmlOptions {
            community_labels: (!labels.is_empty()).then_some(labels),
            node_limit: Some(GRAPH_OVERVIEW_NODE_LIMIT),
            ..HtmlOptions::default()
        },
    )?;
    if let Some(model) = overview {
        let graph_path = output_dir.join("graph.json");
        let source_graph_bytes = fs::metadata(&graph_path)
            .map_err(|source| compass_files::FileError::Io {
                path: graph_path,
                source,
            })?
            .len();
        write_json_atomic(
            &overview_path,
            &json!({
                "schema": GRAPH_OVERVIEW_SCHEMA,
                "sourceGraphBytes": source_graph_bytes,
                "nodeLimit": GRAPH_OVERVIEW_NODE_LIMIT,
                "model": model,
            }),
            false,
        )?;
    } else {
        remove_if_exists(&overview_path)?;
    }
    Ok(())
}

fn unchanged_output_stats(options: &BuildOptions, output_dir: &Path) -> Option<OutputStats> {
    let graph_path = output_dir.join("graph.json");
    let graph_bytes = fs::metadata(&graph_path).ok()?.len();
    let saved = fs::read(output_dir.join(OUTPUT_STATS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<OutputStats>(&bytes).ok());
    if let Some(stats) = saved.filter(|stats| stats.graph_bytes == graph_bytes) {
        if options.no_cluster == stats.clustered
            || !output_dir.join(".compass_root").is_file()
            || (!options.no_cluster
                && (options.resolution != 1.0
                    || options.exclude_hubs.is_some()
                    || !update_artifacts_complete(output_dir)))
            || (!options.no_viz && !output_dir.join("graph.html").is_file() && stats.nodes <= 5_000)
        {
            return None;
        }
        return Some(stats);
    }

    let bytes = fs::read(&graph_path).ok()?;
    let header = &bytes[..bytes.len().min(512)];
    let has_key = |key: &[u8]| header.windows(key.len()).any(|window| window == key);
    let is_clustered = has_key(b"\"directed\"") && has_key(b"\"multigraph\"");
    if options.no_cluster == is_clustered || !output_dir.join(".compass_root").is_file() {
        return None;
    }
    let document: GraphDocument = serde_json::from_slice(&bytes).ok()?;
    if options.no_cluster {
        let stats = OutputStats {
            graph_bytes,
            nodes: document.nodes.len(),
            edges: document.links.len(),
            communities: 0,
            clustered: false,
            omitted_nodes: 0,
            omitted_edges: 0,
            identity_collisions: 0,
        };
        let _ = write_json_atomic(output_dir.join(OUTPUT_STATS_FILE), &stats, true);
        return Some(stats);
    }
    if options.resolution != 1.0
        || options.exclude_hubs.is_some()
        || !update_artifacts_complete(output_dir)
    {
        return None;
    }
    if !options.no_viz && !output_dir.join("graph.html").is_file() && document.nodes.len() <= 5_000
    {
        return None;
    }
    let stats = OutputStats {
        graph_bytes,
        nodes: document.nodes.len(),
        edges: document.links.len(),
        communities: document
            .nodes
            .iter()
            .filter_map(|node| node.attributes.get("community")?.as_u64())
            .collect::<HashSet<_>>()
            .len(),
        clustered: true,
        omitted_nodes: 0,
        omitted_edges: 0,
        identity_collisions: 0,
    };
    let _ = write_json_atomic(output_dir.join(OUTPUT_STATS_FILE), &stats, true);
    Some(stats)
}

fn saved_publication_omissions(output_dir: &Path) -> PublicationOmissions {
    fs::read(output_dir.join(OUTPUT_STATS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<OutputStats>(&bytes).ok())
        .map_or_else(PublicationOmissions::default, |stats| stats.omissions())
}

fn save_output_stats(
    output_dir: &Path,
    nodes: usize,
    edges: usize,
    communities: usize,
    clustered: bool,
    omissions: PublicationOmissions,
) -> Result<(), CoreError> {
    let graph_bytes = fs::metadata(output_dir.join("graph.json"))
        .map_err(|source| compass_files::FileError::Io {
            path: output_dir.join("graph.json"),
            source,
        })?
        .len();
    write_json_atomic(
        output_dir.join(OUTPUT_STATS_FILE),
        &OutputStats {
            graph_bytes,
            nodes,
            edges,
            communities,
            clustered,
            omitted_nodes: omissions.nodes,
            omitted_edges: omissions.edges,
            identity_collisions: omissions.identity_collisions,
        },
        true,
    )?;
    Ok(())
}

fn canonical_node(node: &NodeRecord) -> String {
    let mut value = node.attributes.clone();
    for key in ["community", "community_name", "norm_label"] {
        value.remove(key);
    }
    value.insert("id".to_owned(), serde_json::Value::String(node.id.clone()));
    serde_json::to_string(&value).unwrap_or_default()
}

fn canonical_edge(edge: &EdgeRecord) -> String {
    let mut value = edge.attributes.clone();
    let source = value
        .remove("_src")
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| edge.source.clone());
    let target = value
        .remove("_tgt")
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| edge.target.clone());
    value.remove("confidence_score");
    value.insert("source".to_owned(), serde_json::Value::String(source));
    value.insert("target".to_owned(), serde_json::Value::String(target));
    serde_json::to_string(&value).unwrap_or_default()
}

fn canonical_hyperedges(document: &GraphDocument) -> Vec<String> {
    let mut values = document
        .extras
        .get("hyperedges")
        .or_else(|| document.graph.get("hyperedges"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|value| serde_json::to_string(value).unwrap_or_default())
        .collect::<Vec<_>>();
    values.sort();
    values
}

fn previous_communities(path: &Path) -> HashMap<String, usize> {
    GraphDocument::load(path)
        .ok()
        .map(|document| {
            document
                .nodes
                .into_iter()
                .filter_map(|node| {
                    let community = node
                        .attributes
                        .get("community")?
                        .as_u64()
                        .and_then(|value| usize::try_from(value).ok())?;
                    Some((node.id, community))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn remove_if_exists(path: &Path) -> Result<(), CoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        }
        .into()),
    }
}

fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    }
}

fn profile_internal(label: &str, started: &mut Instant) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!(
            "[compass internal] {label}: {:.3}s",
            started.elapsed().as_secs_f64()
        );
    }
    *started = Instant::now();
}

fn profile_internal_duration(label: &str, elapsed: Duration) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!("[compass internal] {label}: {:.3}s", elapsed.as_secs_f64());
    }
}

fn join_program_worker(
    handle: Option<std::thread::JoinHandle<Result<(ProgramBuildSummary, Duration), CoreError>>>,
    timings: &mut BuildTimings,
) -> Result<Option<ProgramBuildSummary>, CoreError> {
    let Some(handle) = handle else {
        return Ok(None);
    };
    let (program, elapsed) = handle
        .join()
        .map_err(|_| CoreError::WorkerPanic("program analysis".to_owned()))??;
    timings.program_analysis = elapsed;
    Ok(Some(program))
}

pub(crate) fn git_commit(root: &Path) -> Option<String> {
    let dot_git = root
        .ancestors()
        .map(|directory| directory.join(".git"))
        .find(|candidate| candidate.exists())?;
    let repository = dot_git.parent()?.to_path_buf();
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = fs::read_to_string(&dot_git).ok()?;
        let relative = text.trim().strip_prefix("gitdir:")?.trim();
        absolutize_from(&repository, Path::new(relative))
    };
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        fs::read_to_string(git_dir.join(reference))
            .ok()
            .map(|value| value.trim().to_owned())
    } else if !head.is_empty() {
        Some(head.to_owned())
    } else {
        None
    }
}

fn absolutize_from(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn read_source_text_with_limit(path: &Path, max_source_bytes: u64) -> Option<(String, String)> {
    fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file() && metadata.len() <= max_source_bytes)
        .and_then(|_| fs::read(path).ok())
        .map(|bytes| {
            (
                path.to_string_lossy().into_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
            )
        })
}

fn cached_framework_evidence_matches(
    extraction: &Extraction,
    path: &Path,
    project_evidence: &ProjectEvidenceIndex,
) -> bool {
    extraction
        .extensions
        .get(FRAMEWORK_PROJECT_EVIDENCE_EXTENSION)
        .and_then(serde_json::Value::as_str)
        == Some(project_evidence.fingerprint_for(path))
}

fn cached_universal_evidence_matches(extraction: &Extraction, path: &Path) -> bool {
    let Some(profile) = Registry::universal_adapter(path) else {
        return true;
    };
    extraction.raw_calls.is_none()
        && extraction.semantic_evidence.as_ref().is_some_and(|batch| {
            batch.adapter.id == profile.id
                && batch.adapter.language == profile.language
                && batch.adapter.version == profile.version
                && batch.adapter.evidence_schema == profile.evidence_schema
                && batch.adapter.profile == profile.profile
                && batch.adapter.capabilities.as_slice() == profile.capabilities
                && compass_languages::validate_evidence(
                    batch,
                    compass_languages::EvidenceLimits::default(),
                )
                .is_ok()
        })
}

fn extraction_has_cacheable_ast_facts(extraction: &Extraction) -> bool {
    !extraction.nodes.is_empty()
        || !extraction.edges.is_empty()
        || !extraction.hyperedges.is_empty()
        || extraction
            .raw_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        || !extraction.framework_facts.is_empty()
        || extraction.semantic_evidence.as_ref().is_some_and(|batch| {
            !batch.declarations.is_empty()
                || !batch.scopes.is_empty()
                || !batch.bindings.is_empty()
                || !batch.occurrences.is_empty()
                || !batch.candidates.is_empty()
                || !batch.diagnostics.is_empty()
        })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use compass_model::code_graph::GraphDocument as V1GraphDocument;
    use serde_json::{Map, Value};

    use super::*;

    #[test]
    fn force_cache_reuse_never_authorizes_prior_published_graph_input() {
        assert!(cache_reuse_enabled(false, false));
        assert!(cache_reuse_enabled(false, true));
        assert!(!cache_reuse_enabled(true, false));
        assert!(cache_reuse_enabled(true, true));
        assert!(prior_published_graph_input_enabled(false));
        assert!(!prior_published_graph_input_enabled(true));
    }

    #[test]
    fn prior_semantic_preservation_requires_an_owned_marker() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        assert!(!prior_semantic_layer_required(true, directory.path()));
        fs::write(directory.path().join(SEMANTIC_MARKER_FILE), b"{}")?;
        assert!(prior_semantic_layer_required(true, directory.path()));
        assert!(!prior_semantic_layer_required(false, directory.path()));
        Ok(())
    }

    #[test]
    fn nonempty_zero_token_semantic_layers_publish_a_marker() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let semantic = SemanticLayer {
            fragment: json!({
                "nodes": [{"id": "semantic", "label": "Semantic", "file_type": "concept"}],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 0,
                "output_tokens": 0,
                "failed_chunks": 0,
            }),
            refreshed_files: Vec::new(),
            partial_files: Vec::new(),
            allow_partial: false,
        };

        write_semantic_marker(directory.path(), Some(&semantic))?;

        assert!(directory.path().join(SEMANTIC_MARKER_FILE).is_file());
        Ok(())
    }

    #[test]
    fn resolver_source_text_enforces_the_pre_read_byte_limit() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("oversized.py");
        fs::write(&source, b"0123456789")?;

        assert!(read_source_text_with_limit(&source, 9).is_none());
        assert_eq!(
            read_source_text_with_limit(&source, 10)
                .map(|(_, body)| body)
                .as_deref(),
            Some("0123456789")
        );
        Ok(())
    }

    #[test]
    fn universal_cache_entries_reject_any_replaced_raw_call_payload() -> Result<(), Box<dyn Error>>
    {
        let path = Path::new("cached.py");
        let mut extraction = compass_languages::Engine::default()
            .extract_source(path, b"def cached():\n    pass\n")?;
        assert!(cached_universal_evidence_matches(&extraction, path));

        extraction.raw_calls = Some(Vec::new());
        assert!(!cached_universal_evidence_matches(&extraction, path));
        Ok(())
    }

    #[test]
    fn evidence_only_universal_extractions_are_cacheable_ast_facts() -> Result<(), Box<dyn Error>> {
        let extraction = compass_languages::Engine::default()
            .extract_source(Path::new("cached.go"), b"package cached\n\nfunc Run() {}\n")?;
        assert!(extraction.nodes.is_empty());
        assert!(extraction.edges.is_empty());
        assert!(extraction.semantic_evidence.is_some());
        assert!(extraction_has_cacheable_ast_facts(&extraction));
        assert!(!extraction_has_cacheable_ast_facts(&Extraction::default()));
        Ok(())
    }

    #[test]
    fn framework_cache_reuse_is_scoped_to_project_evidence() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("src/routes/+page.svelte");
        fs::create_dir_all(source.parent().ok_or("source has no parent")?)?;
        fs::write(&source, "<h1>Home</h1>")?;
        fs::write(
            directory.path().join("package.json"),
            r#"{"dependencies":{}}"#,
        )?;

        let initial = ProjectEvidenceIndex::build(directory.path(), std::slice::from_ref(&source));
        let mut extraction = Extraction::default();
        assert!(!cached_framework_evidence_matches(
            &extraction,
            &source,
            &initial
        ));
        extraction.extensions.insert(
            FRAMEWORK_PROJECT_EVIDENCE_EXTENSION.to_owned(),
            Value::String(initial.fingerprint_for(&source).to_owned()),
        );
        assert!(cached_framework_evidence_matches(
            &extraction,
            &source,
            &initial
        ));

        fs::write(
            directory.path().join("package.json"),
            r#"{"dependencies":{"@sveltejs/kit":"2.0.0"}}"#,
        )?;
        let changed = ProjectEvidenceIndex::build(directory.path(), std::slice::from_ref(&source));
        assert!(!cached_framework_evidence_matches(
            &extraction,
            &source,
            &changed
        ));
        Ok(())
    }

    #[test]
    fn universal_adapter_cache_requires_current_valid_evidence() -> Result<(), Box<dyn Error>> {
        let python = Path::new("src/example.py");
        let rust = Path::new("src/example.rs");
        assert!(!cached_universal_evidence_matches(
            &Extraction::default(),
            python
        ));
        assert!(!cached_universal_evidence_matches(
            &Extraction::default(),
            rust
        ));

        let source = b"def example():\n    return 1\n";
        let mut engine = Engine::default();
        let extracted = engine
            .extract_source_combined(python, "src/example.py", source)?
            .graph;
        assert!(cached_universal_evidence_matches(&extracted, python));
        let extracted_rust = engine
            .extract_source_combined(rust, "src/example.rs", b"fn example() {}\n")?
            .graph;
        assert!(cached_universal_evidence_matches(&extracted_rust, rust));

        let mut invalid = extracted;
        invalid
            .semantic_evidence
            .as_mut()
            .ok_or_else(|| std::io::Error::other("universal evidence is missing"))?
            .adapter
            .capabilities
            .clear();
        assert!(!cached_universal_evidence_matches(&invalid, python));
        Ok(())
    }

    #[test]
    fn precomputed_detection_cannot_cross_repository_roots() -> Result<(), Box<dyn Error>> {
        let detected_root = tempfile::tempdir()?;
        fs::write(
            detected_root.path().join("main.py"),
            "def main():\n    pass\n",
        )?;
        let detection = detect(detected_root.path(), &DetectOptions::default())?;

        let build_root = tempfile::tempdir()?;
        fs::write(
            build_root.path().join("main.py"),
            "def other():\n    pass\n",
        )?;
        let mut options = BuildOptions::new(build_root.path());
        options.precomputed_detection = Some(detection);

        let error = match build_local_graph(&options) {
            Ok(_) => return Err("mismatched detection unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert!(matches!(error, CoreError::DetectionRootMismatch));
        Ok(())
    }

    #[test]
    fn ast_id_remap_does_not_conflate_prefix_named_symbol_with_file() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let source = root.join("internal/timeformattype_string.go");
        fs::create_dir_all(source.parent().ok_or("source path has no parent")?)?;
        fs::write(&source, "package internal\n\nfunc _() {}\n")?;
        let source_text = source.to_string_lossy().into_owned();
        let file_id = make_id(&[&source_text]);
        let prefix_symbol_id = make_id(&[&file_stem(&source)]);
        let mut extraction = Extraction {
            nodes: vec![
                RawNodeRecord {
                    id: file_id,
                    attributes: Map::from_iter([(
                        "source_file".to_owned(),
                        Value::String(source_text.clone()),
                    )]),
                },
                RawNodeRecord {
                    id: prefix_symbol_id.clone(),
                    attributes: Map::from_iter([(
                        "source_file".to_owned(),
                        Value::String(source_text),
                    )]),
                },
            ],
            edges: vec![RawEdgeRecord {
                source: "caller".to_owned(),
                target: prefix_symbol_id.clone(),
                attributes: Map::new(),
            }],
            ..Extraction::default()
        };
        let live_id_remap = ast_source_identity_map(std::slice::from_ref(&source), root);
        let id_remap =
            collect_ast_id_remap(std::slice::from_ref(&extraction), root, &live_id_remap);
        apply_ast_id_remap(
            &mut extraction,
            &id_remap,
            &format!("{}_", make_id(&[&root.to_string_lossy()])),
        );

        assert_eq!(extraction.nodes[0].id, "internal_timeformattype_string");
        assert_eq!(extraction.nodes[1].id, prefix_symbol_id);
        assert_eq!(extraction.edges[0].target, extraction.nodes[1].id);
        Ok(())
    }

    #[test]
    fn out_of_root_ast_sources_get_portable_ext_ids() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("WebApi");
        fs::create_dir(&root)?;
        let external = directory.path().join("Core/Core.csproj");
        let external_text = external.to_string_lossy().into_owned();
        let old_id = make_id(&[&external_text]);
        let source_id = "webapi".to_owned();
        let mut extraction = Extraction {
            nodes: vec![RawNodeRecord {
                id: old_id.clone(),
                attributes: Map::from_iter([(
                    "source_file".to_owned(),
                    Value::String(external_text),
                )]),
            }],
            edges: vec![RawEdgeRecord {
                source: source_id.clone(),
                target: old_id,
                attributes: Map::from_iter([(
                    "source_file".to_owned(),
                    Value::String(root.join("WebApi.csproj").to_string_lossy().into_owned()),
                )]),
            }],
            ..Extraction::default()
        };
        finalize_ast_extraction(&mut extraction, &root);
        assert_eq!(extraction.nodes[0].id, "ext_core_core_csproj");
        assert_eq!(
            extraction.nodes[0].string("source_file"),
            "../Core/Core.csproj"
        );
        assert_eq!(extraction.edges[0].target, "ext_core_core_csproj");
        assert_eq!(extraction.edges[0].source, source_id);
        Ok(())
    }

    #[test]
    fn portable_ast_cache_removes_worktree_relative_source_paths() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let worktrees = directory.path().join("tmp");
        let root = worktrees.join("worktree-current/checkout");
        let source = root.join("fixtures/code-graph/routes/csharp/AspNetController.cs");
        fs::create_dir_all(source.parent().ok_or("source path has no parent")?)?;
        fs::write(&source, "class AspNetController {}\n")?;
        let escaped =
            "../../worktree-current/checkout/fixtures/code-graph/routes/csharp/AspNetController.cs";
        let mut extraction = Extraction {
            nodes: vec![RawNodeRecord {
                id: "controller".to_owned(),
                attributes: Map::from_iter([
                    ("source_file".to_owned(), Value::String(escaped.to_owned())),
                    ("origin_file".to_owned(), Value::String(escaped.to_owned())),
                ]),
            }],
            framework_facts: vec![serde_json::from_value(json!({
                "type": "domain",
                "fact": {
                    "framework": "aspnet",
                    "kind": "controller",
                    "name": "AspNetController",
                    "declaringScope": "AspNetController",
                    "anchor": {
                        "sourceFile": escaped,
                        "startByte": 0,
                        "endByte": 5,
                        "startLine": 1,
                        "startColumn": 0,
                        "endLine": 1,
                        "endColumn": 5
                    },
                    "origin": "ast"
                }
            }))?],
            ..Extraction::default()
        };

        prepare_portable_ast_cache_entry(&mut extraction, &source, &root);

        let expected = "fixtures/code-graph/routes/csharp/AspNetController.cs";
        assert_eq!(extraction.nodes[0].string("source_file"), expected);
        assert_eq!(extraction.nodes[0].string("origin_file"), expected);
        let framework_source = match &extraction.framework_facts[0] {
            RawFrameworkFact::Route(route) => &route.anchor.source_file,
            RawFrameworkFact::Domain(domain) => &domain.anchor.source_file,
        };
        assert_eq!(framework_source, expected);
        Ok(())
    }

    #[test]
    fn astro_import_identities_do_not_include_checkout_root() -> Result<(), Box<dyn Error>> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let build = |root: &Path| -> Result<V1GraphDocument, Box<dyn Error>> {
            let source = root.join("src");
            fs::create_dir_all(&source)?;
            fs::create_dir_all(root.join(".hidden"))?;
            fs::write(
                source.join("Page.astro"),
                "---\nimport Layout from '../.hidden/Layout.astro';\nconst values: string[] = [];\n---\n<Layout />\n",
            )?;
            fs::write(root.join(".hidden/Layout.astro"), "<slot />\n")?;
            let mut options = BuildOptions::new(root);
            options.no_viz = true;
            let result = build_local_graph(&options)?;
            Ok(V1GraphDocument::load(
                &result.output_dir.join("graph.json"),
            )?)
        };
        let first_graph = build(first.path())?;
        let second_graph = build(second.path())?;
        let identities = |document: &V1GraphDocument| {
            let mut nodes = document
                .nodes
                .iter()
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            let mut edges = document
                .links
                .iter()
                .map(|edge| {
                    (
                        edge.source.clone(),
                        edge.target.clone(),
                        edge.relation().to_owned(),
                    )
                })
                .collect::<Vec<_>>();
            nodes.sort();
            edges.sort();
            (nodes, edges)
        };
        assert_eq!(identities(&first_graph), identities(&second_graph));
        let encoded = first_graph
            .nodes
            .iter()
            .chain(&second_graph.nodes)
            .flat_map(|node| {
                [
                    node.id.clone(),
                    node.source_file().unwrap_or_default().to_owned(),
                ]
            })
            .chain(
                first_graph
                    .links
                    .iter()
                    .chain(&second_graph.links)
                    .flat_map(|edge| {
                        [
                            edge.source.clone(),
                            edge.target.clone(),
                            edge.source_file().unwrap_or_default().to_owned(),
                        ]
                    }),
            )
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!encoded.contains(&make_id(&[&first.path().to_string_lossy()])));
        assert!(!encoded.contains(&make_id(&[&second.path().to_string_lossy()])));
        assert!(!encoded.contains(&first.path().to_string_lossy().to_string()));
        assert!(!encoded.contains(&second.path().to_string_lossy().to_string()));
        let punctuation_symbols = first_graph
            .nodes
            .iter()
            .filter(|node| node.label() == "[]")
            .collect::<Vec<_>>();
        assert!(
            punctuation_symbols.is_empty(),
            "generic punctuation symbols must not be guessed into the typed graph"
        );
        assert!(
            first_graph
                .graph
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "unresolved_node_kind"),
            "generic punctuation must be rejected during extraction instead of reaching publication: {:#?}",
            first_graph.graph.diagnostics
        );
        Ok(())
    }

    #[test]
    fn markdown_documents_edges_survive_portable_ast_remapping() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("guide.md"),
            "# Guide\n[Implementation](documented.rs)\n",
        )?;
        fs::write(
            directory.path().join("documented.rs"),
            "pub fn documented() {}\n",
        )?;
        let mut options = BuildOptions::new(directory.path());
        options.no_cluster = true;
        options.no_viz = true;
        options.force = true;
        let result = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&result.output_dir.join("graph.json"))?;
        assert!(
            graph
                .links
                .iter()
                .any(|edge| edge.kind.as_str() == "documents"),
            "graph={graph:#?}"
        );
        Ok(())
    }

    #[test]
    fn cold_warm_change_and_delete_builds_are_consistent() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(
            root.join("main.py"),
            "from helper import work\n\ndef main():\n    return work()\n",
        )?;
        fs::write(root.join("helper.py"), "def work():\n    return 1\n")?;
        let mut options = BuildOptions::new(root);
        options.no_viz = true;
        options.max_workers = Some(2);

        let cold = build_local_graph(&options)?;
        assert_eq!(cold.files_considered, 2);
        assert_eq!(cold.files_extracted, 2);
        assert!(cold.timings.detect > Duration::ZERO);
        assert!(cold.timings.deterministic_extract > Duration::ZERO);
        assert!(cold.timings.graph_assembly > Duration::ZERO);
        assert!(cold.timings.publish > Duration::ZERO);
        assert!(cold.nodes > 0);
        assert!(cold.output_dir.join("graph.json").is_file());
        let overview_path = cold.output_dir.join("graph-overview.json");
        assert!(
            overview_path.is_file(),
            "clustered updates should prepare the VS Code graph overview even with --no-viz"
        );
        let overview: Value = serde_json::from_slice(&fs::read(&overview_path)?)?;
        assert_eq!(overview["schema"], "compass.graph-overview/1");
        assert_eq!(overview["nodeLimit"], 5_000);
        assert_eq!(
            overview["sourceGraphBytes"],
            fs::metadata(cold.output_dir.join("graph.json"))?.len()
        );
        assert_eq!(overview["model"]["schema"], "compass.viewer.graph/1");
        assert!(cold.output_dir.join("manifest.json").is_file());
        assert!(!cold.output_dir.join(".compass_incomplete").exists());
        let cold_graph = V1GraphDocument::load(&cold.output_dir.join("graph.json"))?;
        assert!(cold_graph.nodes.iter().all(|node| {
            node.evidence
                .first()
                .is_some_and(|evidence| evidence.origin == EvidenceOrigin::Ast)
                && node
                    .source_file()
                    .is_none_or(|source| !Path::new(source).is_absolute())
        }));
        let cold_graph_bytes = fs::read(cold.output_dir.join("graph.json"))?;
        let cold_report_bytes = fs::read(cold.output_dir.join("GRAPH_REPORT.md"))?;

        let warm = build_local_graph(&options)?;
        assert_eq!(warm.files_extracted, 0);
        assert_eq!(warm.files_cached, 2);
        assert_eq!(warm.nodes, cold.nodes);
        assert_eq!(warm.edges, cold.edges);
        assert_eq!(
            fs::read(warm.output_dir.join("graph.json"))?,
            cold_graph_bytes
        );
        assert_eq!(
            fs::read(warm.output_dir.join("GRAPH_REPORT.md"))?,
            cold_report_bytes
        );

        fs::write(root.join("helper.py"), "def work():\n    return 2\n")?;
        let changed = build_local_graph(&options)?;
        assert_eq!(changed.files_extracted, 1);
        assert_eq!(changed.files_cached, 1);
        let changed_graph = V1GraphDocument::load(&changed.output_dir.join("graph.json"))?;
        let changed_graph_bytes = fs::read(changed.output_dir.join("graph.json"))?;
        assert_ne!(
            changed_graph_bytes, cold_graph_bytes,
            "a body-only edit must update definition hashes"
        );
        let identities = |document: &V1GraphDocument| {
            (
                document
                    .nodes
                    .iter()
                    .map(|node| node.id.clone())
                    .collect::<HashSet<_>>(),
                document
                    .links
                    .iter()
                    .map(|edge| (edge.source.clone(), edge.target.clone(), edge.relation()))
                    .collect::<HashSet<_>>(),
            )
        };
        assert_eq!(identities(&changed_graph), identities(&cold_graph));
        let implementation_hash = |document: &V1GraphDocument| {
            document
                .nodes
                .iter()
                .find(|node| node.label() == "work()")
                .and_then(|node| node.digest("implementation_hash"))
                .map(str::to_owned)
        };
        assert_ne!(
            implementation_hash(&changed_graph),
            implementation_hash(&cold_graph)
        );
        let warm_changed = build_local_graph(&options)?;
        assert_eq!(warm_changed.files_extracted, 0);
        assert_eq!(
            fs::read(warm_changed.output_dir.join("graph.json"))?,
            changed_graph_bytes,
            "cached and freshly extracted definition hashes must agree"
        );

        fs::remove_file(root.join("helper.py"))?;
        let deleted = build_local_graph(&options)?;
        assert_eq!(deleted.files_considered, 1);
        let graph = V1GraphDocument::load(&deleted.output_dir.join("graph.json"))?;
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.source_file() != Some("helper.py"))
        );
        Ok(())
    }

    #[test]
    fn update_preserves_semantic_layer_but_replaces_ast_layer() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def before():\n    return 1\n")?;
        fs::write(root.join("domain.md"), "# Domain rule\n")?;
        let mut options = BuildOptions::new(root);
        options.no_viz = true;
        let semantic = SemanticLayer {
            fragment: json!({
                "nodes": [{
                    "id": "semantic_domain_rule",
                    "label": "Domain rule",
                    "file_type": "concept",
                    "source_file": "domain.md",
                }],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: vec![PathBuf::from("domain.md")],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        build_graph_with_semantic(&options, &semantic)?;

        fs::write(root.join("main.py"), "def after():\n    return 2\n")?;
        let updated = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&updated.output_dir.join("graph.json"))?;
        assert!(graph.nodes.iter().any(|node| node.label() == "Domain rule"));
        assert!(graph.nodes.iter().any(|node| node.label() == "after()"));
        assert!(!graph.nodes.iter().any(|node| node.label() == "before()"));
        Ok(())
    }

    #[test]
    fn update_does_not_duplicate_semantic_backed_documents() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("guide.md"), "# Guide\n\nLocal structure.\n")?;
        let mut options = BuildOptions::new(root);
        options.no_viz = true;
        let semantic = SemanticLayer {
            fragment: json!({
                "nodes": [{
                    "id": "semantic_guide",
                    "label": "Guide concept",
                    "file_type": "concept",
                    "source_file": "guide.md",
                }],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: vec![PathBuf::from("guide.md")],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        build_graph_with_semantic(&options, &semantic)?;

        let updated = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&updated.output_dir.join("graph.json"))?;
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.label() == "Guide concept")
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn no_cluster_always_publishes_the_v1_contract() -> Result<(), Box<dyn Error>> {
        let extract_dir = tempfile::tempdir()?;
        fs::write(
            extract_dir.path().join("main.py"),
            "def main():\n    pass\n",
        )?;
        let mut extract = BuildOptions::new(extract_dir.path());
        extract.no_cluster = true;
        extract.purpose = BuildPurpose::Extract;
        let result = build_local_graph(&extract)?;
        let value: Value =
            serde_json::from_slice(&fs::read(result.output_dir.join("graph.json"))?)?;
        assert_eq!(value["graph"]["schema"], "compass.graph/1");
        assert_eq!(value["directed"], true);
        assert_eq!(value["multigraph"], true);
        assert!(value.get("links").is_some());
        assert!(value.get("edges").is_none());
        assert!(!result.output_dir.join("GRAPH_REPORT.md").exists());
        assert!(!result.output_dir.join(".compass_analysis.json").exists());

        let update_dir = tempfile::tempdir()?;
        fs::write(update_dir.path().join("main.py"), "def main():\n    pass\n")?;
        let mut update = BuildOptions::new(update_dir.path());
        update.no_cluster = true;
        let result = build_local_graph(&update)?;
        let bytes = fs::read(result.output_dir.join("graph.json"))?;
        let value: Value = serde_json::from_slice(&bytes)?;
        assert_eq!(value["graph"]["schema"], "compass.graph/1");
        assert_eq!(value["directed"], true);
        assert_eq!(value["multigraph"], true);
        assert!(value.get("links").is_some());
        assert!(value.get("edges").is_none());
        assert!(result.output_dir.join(".compass_root").is_file());
        Ok(())
    }

    #[test]
    fn code_and_deterministic_document_sources_are_both_published() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("zzz.py"),
            "def code_symbol():\n    return 1\n",
        )?;
        fs::write(
            directory.path().join("aaa.md"),
            "# Document heading\n\nDocument body.\n",
        )?;
        let mut options = BuildOptions::new(directory.path());
        options.no_cluster = true;
        options.no_viz = true;

        let result = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&result.output_dir.join("graph.json"))?;
        let code_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.source_file() == Some("zzz.py"))
            .collect::<Vec<_>>();
        let document_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.source_file() == Some("aaa.md"))
            .collect::<Vec<_>>();

        assert!(!code_nodes.is_empty());
        assert!(!document_nodes.is_empty());
        Ok(())
    }

    #[test]
    fn unsupported_extensionless_shebang_is_skipped() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("main.py"),
            "def supported_symbol():\n    return 1\n",
        )?;
        fs::write(
            directory.path().join("vendor-treadmill"),
            "#!/usr/bin/fish\nfunction unsupported_symbol; echo 1; end\n",
        )?;
        let mut options = BuildOptions::new(directory.path());
        options.no_cluster = true;
        options.no_viz = true;

        let result = build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&result.output_dir.join("graph.json"))?;
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.label() == "supported_symbol()")
        );
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.source_file() != Some("vendor-treadmill"))
        );
        Ok(())
    }

    #[test]
    fn unchanged_no_cluster_update_uses_manifest_without_loading_cache()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("main.py"),
            "def main():\n    return 1\n",
        )?;
        let mut options = BuildOptions::new(directory.path());
        options.no_cluster = true;
        options.no_viz = true;
        let cold = build_local_graph(&options)?;
        let graph_path = cold.output_dir.join("graph.json");
        let graph_bytes = fs::read(&graph_path)?;
        let manifest_bytes = fs::read(cold.output_dir.join("manifest.json"))?;
        fs::remove_dir_all(directory.path().join("compass-out").join("cache"))?;

        let warm = build_local_graph(&options)?;
        assert_eq!(warm.files_extracted, 0);
        assert_eq!(warm.files_cached, 1);
        assert_eq!(fs::read(warm.output_dir.join("graph.json"))?, graph_bytes);
        assert_eq!(
            fs::read(warm.output_dir.join("manifest.json"))?,
            manifest_bytes
        );
        Ok(())
    }

    #[test]
    fn ast_manifest_prunes_existing_files_removed_from_scope() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def main():\n    return 1\n")?;
        fs::create_dir(root.join("generated"))?;
        fs::write(
            root.join("generated/copied.py"),
            "def generated():\n    return 2\n",
        )?;
        let mut options = BuildOptions::new(root);
        options.no_cluster = true;
        options.no_viz = true;

        let initial = build_local_graph(&options)?;
        assert_eq!(initial.files_considered, 2);

        options.extra_excludes = vec!["generated/**".to_owned()];
        let scoped = build_local_graph(&options)?;
        assert_eq!(scoped.files_considered, 1);
        let manifest: Value =
            serde_json::from_slice(&fs::read(scoped.output_dir.join("manifest.json"))?)?;
        assert_eq!(manifest.as_object().map(serde_json::Map::len), Some(1));

        let unchanged = build_local_graph(&options)?;
        assert_eq!(unchanged.files_extracted, 0);
        assert_eq!(unchanged.files_cached, 1);
        assert!(!unchanged.outputs_changed);
        Ok(())
    }

    #[test]
    fn semantic_layer_replaces_owned_facts_and_stamps_manifest() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def main():\n    return 1\n")?;
        fs::write(root.join("diagram.png"), b"not-decoded-by-core")?;
        let mut options = BuildOptions::new(root);
        options.purpose = BuildPurpose::Extract;
        options.no_viz = true;
        let source = root.join("diagram.png");
        let first_layer = SemanticLayer {
            fragment: json!({
                "nodes": [{
                    "id": "old_concept",
                    "label": "Old concept",
                    "file_type": "concept",
                    "source_file": source,
                }],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 13,
                "output_tokens": 7,
                "failed_chunks": 0,
            }),
            refreshed_files: vec![source.clone()],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let first = build_graph_with_semantic(&options, &first_layer)?;
        let graph_path = first.output_dir.join("graph.json");
        let graph = V1GraphDocument::load(&graph_path)?;
        assert!(graph.nodes.iter().any(|node| node.label() == "Old concept"));
        let manifest: Value =
            serde_json::from_slice(&fs::read(first.output_dir.join("manifest.json"))?)?;
        assert!(
            manifest["diagram.png"]["ast_hash"]
                .as_str()
                .is_some_and(|hash| !hash.is_empty())
        );
        assert!(
            manifest["diagram.png"]["semantic_hash"]
                .as_str()
                .is_some_and(|hash| !hash.is_empty())
        );
        let analysis: Value =
            serde_json::from_slice(&fs::read(first.output_dir.join(".compass_analysis.json"))?)?;
        assert_eq!(analysis["tokens"], json!({"input": 13, "output": 7}));

        let second_layer = SemanticLayer {
            fragment: json!({
                "nodes": [{
                    "id": "new_concept",
                    "label": "New concept",
                    "file_type": "concept",
                    "source_file": "diagram.png",
                }],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 3,
                "output_tokens": 2,
                "failed_chunks": 0,
            }),
            refreshed_files: vec![source],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let second = build_graph_with_semantic(&options, &second_layer)?;
        let graph = V1GraphDocument::load(&second.output_dir.join("graph.json"))?;
        assert!(!graph.nodes.iter().any(|node| node.label() == "Old concept"));
        assert!(graph.nodes.iter().any(|node| node.label() == "New concept"));
        let Some(semantic) = graph
            .nodes
            .iter()
            .find(|node| node.label() == "New concept")
        else {
            return Err("new semantic node was not written".into());
        };
        assert_eq!(semantic.source_file(), Some("diagram.png"));
        assert!(node_has_origin(semantic, EvidenceOrigin::Heuristic));
        Ok(())
    }

    #[test]
    fn incomplete_raw_semantic_shrink_requires_explicit_override() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def main():\n    return 1\n")?;
        fs::write(root.join("diagram.png"), b"not-decoded-by-core")?;
        let mut options = BuildOptions::new(root);
        options.purpose = BuildPurpose::Extract;
        options.no_cluster = true;
        options.no_viz = true;
        let source = root.join("diagram.png");
        let complete = SemanticLayer {
            fragment: json!({
                "nodes": [
                    {"id":"concept_a", "label":"Concept A", "file_type":"concept", "source_file":"diagram.png"},
                    {"id":"concept_b", "label":"Concept B", "file_type":"concept", "source_file":"diagram.png"}
                ],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 5,
                "output_tokens": 4,
                "failed_chunks": 0,
            }),
            refreshed_files: vec![source.clone()],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let first = build_graph_with_semantic(&options, &complete)?;
        let graph_path = first.output_dir.join("graph.json");
        let original = fs::read(&graph_path)?;
        let mut incomplete = SemanticLayer {
            fragment: json!({
                "nodes": [{"id":"concept_a", "label":"Concept A", "file_type":"concept", "source_file":"diagram.png"}],
                "edges": [],
                "hyperedges": [],
                "input_tokens": 2,
                "output_tokens": 1,
                "failed_chunks": 1,
            }),
            refreshed_files: vec![source],
            partial_files: vec![PathBuf::from("diagram.png")],
            allow_partial: false,
        };
        let error = match build_graph_with_semantic(&options, &incomplete) {
            Ok(_) => return Err("incomplete semantic shrink unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert!(matches!(error, CoreError::IncompleteSemanticShrink { .. }));
        assert_eq!(fs::read(&graph_path)?, original);

        incomplete.allow_partial = true;
        let partial = build_graph_with_semantic(&options, &incomplete)?;
        let graph = V1GraphDocument::load(&partial.output_dir.join("graph.json"))?;
        assert!(graph.nodes.iter().any(|node| node.label() == "Concept A"));
        assert!(!graph.nodes.iter().any(|node| node.label() == "Concept B"));
        let manifest: Value =
            serde_json::from_slice(&fs::read(partial.output_dir.join("manifest.json"))?)?;
        assert_eq!(manifest["diagram.png"]["semantic_hash"], "");
        Ok(())
    }

    #[test]
    fn complete_semantic_run_may_shrink_and_prunes_retired_sources() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def main():\n    return 1\n")?;
        let image = root.join("diagram.png");
        fs::write(&image, b"not-decoded-by-core")?;
        let mut options = BuildOptions::new(root);
        options.purpose = BuildPurpose::Extract;
        options.no_viz = true;
        let complete = SemanticLayer {
            fragment: json!({
                "nodes": [
                    {"id":"concept_a", "label":"Concept A", "file_type":"concept", "source_file":"diagram.png"},
                    {"id":"concept_b", "label":"Concept B", "file_type":"concept", "source_file":"diagram.png"}
                ],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: vec![image.clone()],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        build_graph_with_semantic(&options, &complete)?;

        let smaller = SemanticLayer {
            fragment: json!({
                "nodes": [{"id":"concept_a", "label":"Concept A", "file_type":"concept", "source_file":"diagram.png"}],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: vec![image.clone()],
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let smaller = build_graph_with_semantic(&options, &smaller)?;
        let graph = V1GraphDocument::load(&smaller.output_dir.join("graph.json"))?;
        assert!(graph.nodes.iter().any(|node| node.label() == "Concept A"));
        assert!(!graph.nodes.iter().any(|node| node.label() == "Concept B"));

        fs::remove_file(&image)?;
        let empty = SemanticLayer {
            fragment: json!({
                "nodes": [],
                "edges": [],
                "hyperedges": [],
                "failed_chunks": 0,
            }),
            refreshed_files: Vec::new(),
            partial_files: Vec::new(),
            allow_partial: false,
        };
        let empty = build_graph_with_semantic(&options, &empty)?;
        let graph = V1GraphDocument::load(&empty.output_dir.join("graph.json"))?;
        assert!(!graph.nodes.iter().any(|node| node.label() == "Concept A"));
        let manifest: Value =
            serde_json::from_slice(&fs::read(empty.output_dir.join("manifest.json"))?)?;
        assert!(manifest.get("diagram.png").is_none());
        Ok(())
    }
}
