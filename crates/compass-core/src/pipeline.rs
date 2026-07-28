use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ahash::{AHashMap, AHashSet};
use compass_files::{
    BuildGuard, BuildScope, Cache, CacheKind, CacheOptions, DetectOptions, Detection, IgnorePolicy,
    Manifest, ManifestKind, detect, write_json_atomic, write_text_atomic,
};
use compass_graph::{
    ClusterOptions, EntityTiebreaker, build_owned_with_tiebreaker as build_document, cluster,
    dedupe_edges, dedupe_nodes, extraction_from_v1, graph_insights, label_communities_by_hub,
    normalize_document_v1, remap_communities_to_previous, score_communities,
};
use compass_languages::{
    Engine, Extraction, RawEdgeRecord, RawNodeRecord, Registry, file_stem, make_id,
};
use compass_model::code_graph::{GraphDocument as V1GraphDocument, NodeKind};
use compass_model::provenance::EvidenceOrigin;
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use compass_output::{
    DetectionSummary, HtmlOptions, OutputError, ReportOptions, TokenCost, generate_report,
    graph_view_model_document, write_html,
};
use compass_resolve::{merge_decl_def_classes, resolve_owned_with_root};
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

#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub root: PathBuf,
    pub scan_filesystem: bool,
    pub output_root: Option<PathBuf>,
    /// Explicit repository-private cache root used by exact history builds.
    pub cache_root: Option<PathBuf>,
    pub force: bool,
    /// Preserve validated extraction and completed-output fast paths while
    /// retaining force authorization for output replacement.
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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OutputStats {
    graph_bytes: u64,
    nodes: usize,
    edges: usize,
    communities: usize,
    clustered: bool,
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
            built_at_commit: None,
            purpose: BuildPurpose::Update,
            precomputed_detection: None,
        }
    }
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
    let output_dir = output_root.join(&output_name);
    fs::create_dir_all(&output_dir).map_err(|source| compass_files::FileError::Io {
        path: output_dir.clone(),
        source,
    })?;
    let prior_build_complete = BuildGuard::ensure_complete(&output_dir).is_ok();
    let guard = BuildGuard::begin(&output_dir)?;
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
    let mut semantic_documents = if options.purpose == BuildPurpose::Update
        || (options.purpose == BuildPurpose::Extract && !options.force)
    {
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

    let manifest_unchanged = options.purpose == BuildPurpose::Update
        && (!options.force || options.reuse_cache_on_force)
        && prior_manifest.is_unchanged(&detection.files, ManifestKind::Ast);
    let build_profile = build_profile(options);
    let has_program_artifacts =
        options.program_analysis && program_artifact_count(&root, options)? != 0;
    let verified_state = if semantic.is_none() && supplemental.is_empty() && manifest_unchanged {
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
            guard.commit()?;
            return Ok(BuildResult {
                root,
                output_dir: output_dir.clone(),
                detection,
                files_considered: state.stats.files,
                files_extracted: 0,
                files_cached: state.stats.files,
                empty_files: Vec::new(),
                nodes: state.stats.nodes,
                edges: state.stats.edges,
                communities: state.stats.communities,
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
    let unchanged_program = if options.program_analysis
        && semantic.is_none()
        && supplemental.is_empty()
        && manifest_unchanged
        && verified_output
    {
        load_current_program(&root, &sources, options, &output_dir)?
    } else {
        None
    };
    if semantic.is_none()
        && supplemental.is_empty()
        && manifest_unchanged
        && verified_output
        && (!options.program_analysis || unchanged_program.is_some())
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
            unchanged_program.as_ref(),
        )?;
        guard.commit()?;
        return Ok(BuildResult {
            root,
            output_dir: output_dir.clone(),
            detection,
            files_considered: sources.len(),
            files_extracted: 0,
            files_cached: sources.len(),
            empty_files: Vec::new(),
            nodes: stats.nodes,
            edges: stats.edges,
            communities: stats.communities,
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
    let mut extractions = BTreeMap::<PathBuf, Extraction>::new();
    let mut missing = Vec::new();
    if !options.force || options.reuse_cache_on_force {
        for path in &sources {
            let cached = cache.load(path, &CacheKind::Ast, None, false)?;
            if let Some(value) = cached {
                let extraction =
                    serde_json::from_value(value).map_err(|source| CoreError::InvalidCache {
                        path: path.clone(),
                        source,
                    })?;
                extractions.insert(path.clone(), extraction);
            } else {
                missing.push(path.clone());
            }
        }
    } else {
        missing.clone_from(&sources);
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
            let combined = engine.extract_source_combined(path, &source_file, &bytes)?;
            let prepared = combined.program.map(|batch| PreparedSyntaxInput {
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
            Ok((path.clone(), combined.graph, source, prepared))
        };
    let fresh = if missing.len() < 256 {
        let mut engine = Engine::default();
        missing
            .iter()
            .map(|path| extract_source(&mut engine, path))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let extract = || {
            missing
                .par_iter()
                .map_init(Engine::default, extract_source)
                .collect::<Result<Vec<_>, _>>()
        };
        if let Some(pool) = &worker_pool {
            pool.install(extract)?
        } else {
            extract()?
        }
    };
    profile_internal("tree-sitter combined extraction", &mut internal_started);
    let prepared = if options.force && !options.reuse_cache_on_force {
        fresh
            .iter()
            .filter_map(|(_, _, _, prepared)| prepared.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut program_handle = if options.program_analysis {
        let program_root = root.clone();
        let program_sources = sources.clone();
        let mut program_options = options.clone();
        program_options.force = options.force && !options.reuse_cache_on_force;
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
                    Ok::<_, CoreError>((program, started.elapsed()))
                })
                .map_err(|error| CoreError::WorkerPool(error.to_string()))?,
        )
    } else {
        None
    };
    let mut ast_cache_entries = fresh
        .par_iter()
        .filter(|(_, extraction, _, _)| !extraction.nodes.is_empty())
        .map(|(path, extraction, _, _)| (path.clone(), extraction.clone()))
        .collect::<Vec<_>>();
    let mut empty_files = Vec::new();
    let fresh_paths = fresh
        .iter()
        .map(|(path, _, _, _)| path.clone())
        .collect::<HashSet<_>>();
    let mut fresh_source_text = HashMap::with_capacity(fresh.len());
    for (path, extraction, (source_path, source), _) in fresh {
        if extraction.nodes.is_empty() {
            empty_files.push(path.clone());
        }
        fresh_source_text.insert(source_path, source);
        extractions.insert(path, extraction);
    }
    profile_internal("AST cache snapshot and dispatch", &mut internal_started);

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
        ordered.par_iter_mut().for_each(|extraction| {
            apply_ast_id_remap(extraction, &ast_id_remap, &ast_root_marker);
        });
        ast_cache_entries
            .par_iter_mut()
            .for_each(|(_, extraction)| {
                apply_ast_id_remap(extraction, &ast_id_remap, &ast_root_marker);
            });
        profile_internal_duration(
            "portable AST remap application",
            remap_application_started.elapsed(),
        );
    }
    ast_cache_entries
        .par_iter_mut()
        .for_each(|(path, extraction)| prepare_portable_ast_cache_entry(extraction, path, &root));
    let ast_cache_handle = std::thread::Builder::new()
        .name("compass-ast-cache".to_owned())
        .spawn(move || {
            let started = Instant::now();
            cache.save_portable_ast_batch(&ast_cache_entries)?;
            cache.flush()?;
            Ok::<_, CoreError>(started.elapsed())
        })
        .map_err(|error| CoreError::WorkerPool(error.to_string()))?;
    profile_internal("portable AST ID remapping", &mut internal_started);
    let read_source = |path: &PathBuf| {
        fs::read(path).ok().map(|bytes| {
            (
                path.to_string_lossy().into_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
            )
        })
    };
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
    let mut resolved = resolve_owned_with_root(ordered, &source_text, &root);
    profile_internal("cross-file resolution total", &mut internal_started);
    drop(source_text);
    finalize_ast_extraction(&mut resolved, &root);
    profile_internal("AST finalization", &mut internal_started);
    let ast_cache_elapsed = ast_cache_handle
        .join()
        .map_err(|_| CoreError::WorkerPanic("AST cache publication".to_owned()))??;
    profile_internal_duration("AST cache publication worker", ast_cache_elapsed);
    internal_started = Instant::now();
    timings.deterministic_extract = stage_started.elapsed();
    let defer_program_join = options.force && !options.no_cluster;
    let mut program = if defer_program_join {
        None
    } else {
        join_program_worker(program_handle.take(), &mut timings)?
    };
    profile_internal("wait for Program analysis", &mut internal_started);
    stage_started = Instant::now();
    if options.purpose == BuildPurpose::Update
        || (options.purpose == BuildPurpose::Extract && !options.force)
    {
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
        && missing.is_empty()
        && !source_removed
        && supplemental.is_empty()
        && semantic.is_some_and(semantic_layer_is_empty)
        && let Ok(document) = GraphDocument::load(&output_dir.join("graph.json"))
    {
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
            program.as_ref(),
        )?;
        guard.commit()?;
        return Ok(BuildResult {
            root,
            output_dir,
            detection,
            files_considered: sources.len(),
            files_extracted: 0,
            files_cached: sources.len(),
            empty_files,
            nodes: document.nodes.len(),
            edges: document.links.len(),
            communities: 0,
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
        let (nodes, edges) = (dedupe_nodes(&resolved.nodes), dedupe_edges(&resolved.edges));
        enforce_incomplete_raw_guard(semantic, &output_dir.join("graph.json"), &root, nodes.len())?;
        let document = build_document(resolved, true, true, Some(&root), tiebreaker)?;
        let configuration_digest = graph_configuration_digest(options, &output_dir)?;
        let published = normalize_document_v1(&document, &root, configuration_digest)?;
        write_json_atomic(output_dir.join("graph.json"), &published, false)?;
        remove_if_exists(&output_dir.join(GRAPH_OVERVIEW_FILE))?;
        save_output_stats(&output_dir, nodes.len(), edges.len(), 0, false)?;
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
            nodes.len(),
            edges.len(),
            0,
            program.as_ref(),
        )?;
        guard.commit()?;
        timings.publish = stage_started.elapsed();
        return Ok(BuildResult {
            root,
            output_dir,
            detection,
            files_considered: sources.len(),
            files_extracted: missing.len(),
            files_cached: sources.len().saturating_sub(missing.len()),
            empty_files,
            nodes: nodes.len(),
            edges: edges.len(),
            communities: 0,
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
    let document = build_document(resolved, false, true, Some(&root), tiebreaker)?;
    profile_internal("graph document build and dedup", &mut internal_started);
    timings.graph_assembly = stage_started.elapsed();
    stage_started = Instant::now();
    if document.nodes.is_empty() {
        return Err(CoreError::EmptyGraph);
    }

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
    if unchanged_layers
        && supplemental.is_empty()
        && !options.force
        && unchanged_artifacts_complete
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
            program.as_ref(),
        )?;
        guard.commit()?;
        return Ok(BuildResult {
            root,
            output_dir: output_dir.clone(),
            detection,
            files_considered: sources.len(),
            files_extracted: missing.len(),
            files_cached: sources.len().saturating_sub(missing.len()),
            empty_files,
            nodes: document.nodes.len(),
            edges: document.links.len(),
            communities,
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
    let commit = options.built_at_commit.clone().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|directory| git_commit(&directory))
    });

    let graph_output = || -> Result<Duration, CoreError> {
        let started = Instant::now();
        let mut output_profile_started = Instant::now();
        let configuration_digest = graph_configuration_digest(options, &output_dir)?;
        let published = published_v1_document(
            &document,
            &communities,
            &labels,
            &root,
            configuration_digest,
        )?;
        write_json_atomic(output_dir.join("graph.json"), &published, false)?;
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
        Ok(started.elapsed())
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
    let graph_output_elapsed = graph_output_elapsed?;
    let (html_written, analysis_elapsed) = analysis_result?;
    profile_internal_duration("graph and overview publication", graph_output_elapsed);
    profile_internal_duration(
        "parallel graph analyses and report publication",
        analysis_elapsed,
    );
    internal_started = Instant::now();
    timings.graph_assembly += stage_started.elapsed();
    stage_started = Instant::now();

    write_semantic_marker(&output_dir, semantic)?;

    let mut manifest = prior_manifest;
    save_build_manifest(
        &mut manifest,
        &detection.files,
        &manifest_path,
        &root,
        semantic,
    )?;
    timings.publish = stage_started.elapsed();
    if program.is_none() {
        program = join_program_worker(program_handle.take(), &mut timings)?;
    }
    save_output_stats(
        &output_dir,
        document.nodes.len(),
        document.links.len(),
        communities.len(),
        true,
    )?;
    publish_build_state(
        options,
        &output_dir,
        &manifest_path,
        sources.len(),
        document.nodes.len(),
        document.links.len(),
        communities.len(),
        program.as_ref(),
    )?;
    profile_internal("Program output and build seals", &mut internal_started);
    guard.commit()?;
    Ok(BuildResult {
        root,
        output_dir,
        detection,
        files_considered: sources.len(),
        files_extracted: missing.len(),
        files_cached: sources.len().saturating_sub(missing.len()),
        empty_files,
        nodes: document.nodes.len(),
        edges: document.links.len(),
        communities: communities.len(),
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

fn program_modules(program: Option<&ProgramBuild>) -> usize {
    program.map_or(0, |program| program.analysis.program.modules.len())
}

fn program_summaries(program: Option<&ProgramBuild>) -> usize {
    program.map_or(0, |program| program.analysis.summaries.len())
}

fn program_providers(program: Option<&ProgramBuild>) -> usize {
    program.map_or(0, |program| program.analysis.program.providers.len())
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
    program: Option<&ProgramBuild>,
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
    for optional in ["graph.html", ".compass_semantic_marker"] {
        let path = output_dir.join(optional);
        if path.is_file() {
            required.push(path);
        }
    }
    let state = BuildState::capture(
        output_dir,
        build_profile(options),
        manifest_path,
        program.map(|program| ArtifactSeal::from_bytes(&program.canonical_bytes)),
        &required,
        SavedStats {
            files,
            nodes,
            edges,
            communities,
            program_modules: program_modules(program),
            program_summaries: program_summaries(program),
            program_providers: program_providers(program),
            program_conflicts: program.map_or(0, |program| program.conflicts),
        },
    )?;
    state.save(output_dir)
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
    if output_tokens > 0 {
        write_json_atomic(
            output_dir.join(".compass_semantic_marker"),
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
    rayon::join(
        || {
            extraction.nodes.par_iter_mut().for_each(|node| {
                normalize_source_attribute_cached(&mut node.attributes, root, &canonical_sources);
                node.attributes.remove("origin_file");
                node.attributes.remove("_callable");
                node.attributes.insert(
                    "_origin".to_owned(),
                    serde_json::Value::String("ast".to_owned()),
                );
            });
        },
        || {
            extraction.edges.par_iter_mut().for_each(|edge| {
                normalize_source_attribute_cached(&mut edge.attributes, root, &canonical_sources);
                edge.attributes.insert(
                    "_origin".to_owned(),
                    serde_json::Value::String("ast".to_owned()),
                );
            });
        },
    );
}

fn prepare_portable_ast_cache_entry(extraction: &mut Extraction, source: &Path, root: &Path) {
    let canonical = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let Ok(relative) = canonical
        .strip_prefix(root)
        .or_else(|_| source.strip_prefix(root))
    else {
        return;
    };
    let portable = relative.to_string_lossy().replace('\\', "/");
    let set_portable = |attributes: &mut serde_json::Map<String, serde_json::Value>| {
        for key in ["source_file", "origin_file"] {
            if attributes
                .get(key)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.is_empty())
            {
                attributes.insert(key.to_owned(), serde_json::Value::String(portable.clone()));
            }
        }
        if let Some(target) = attributes
            .get("target_file")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        {
            let target = Path::new(&target);
            let canonical = fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
            if let Ok(relative) = canonical.strip_prefix(root) {
                attributes.insert(
                    "target_file".to_owned(),
                    serde_json::Value::String(relative.to_string_lossy().replace('\\', "/")),
                );
            }
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
    if let Some(calls) = extraction.raw_calls.as_mut() {
        for call in calls {
            if !call.source_file.is_empty() {
                call.source_file.clone_from(&portable);
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
    }
    for edge in &mut extraction.edges {
        normalize_source_attribute(&mut edge.attributes, root);
        edge.attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("semantic".to_owned()),
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
    extractions.par_iter().all(|extraction| {
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
    })
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
    for edge in &mut existing_raw.edges {
        if let Some(current) = typed_ast_remap.get(&edge.source) {
            edge.source.clone_from(current);
        }
        if let Some(current) = typed_ast_remap.get(&edge.target) {
            edge.target.clone_from(current);
        }
    }
    let ast_ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let preserved_node_ids = existing
        .nodes
        .iter()
        .filter(|node| !node_has_origin(node, EvidenceOrigin::Ast))
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut preserved_nodes = existing_raw
        .nodes
        .into_iter()
        .filter(|node| {
            !ast_ids.contains(node.id.as_str())
                && preserved_node_ids.contains(node.id.as_str())
                && !source_in_set(node.attributes.get("source_file"), root, refreshed)
                && !source_was_deleted(node.attributes.get("source_file"), root)
        })
        .collect::<Vec<_>>();
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
        .filter_map(|(raw, typed)| {
            (!edge_has_origin(&typed, EvidenceOrigin::Ast)
                && all_ids.contains(&raw.source)
                && all_ids.contains(&raw.target)
                && !source_in_set(raw.attributes.get("source_file"), root, refreshed)
                && !source_was_deleted(raw.attributes.get("source_file"), root))
            .then_some(raw)
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

fn edge_has_origin(edge: &compass_model::code_graph::EdgeRecord, origin: EvidenceOrigin) -> bool {
    edge.evidence
        .iter()
        .any(|evidence| evidence.origin == origin)
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
        .filter(|node| !node_has_origin(node, EvidenceOrigin::Ast))
        .filter_map(|node| semantic_path_under_root(node.source_file(), root))
        .filter(|source| !live.contains(source))
        .collect::<HashSet<_>>();
    stale.extend(
        existing
            .links
            .iter()
            .filter(|edge| !edge_has_origin(edge, EvidenceOrigin::Ast))
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
    let Some(layer) = semantic else {
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
        .filter(|node| {
            !node_has_origin(node, EvidenceOrigin::Ast) && node.kind == NodeKind::Resource
        })
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

fn published_v1_document(
    document: &GraphDocument,
    communities: &compass_graph::Communities,
    labels: &BTreeMap<usize, String>,
    root: &Path,
    configuration_digest: String,
) -> Result<compass_model::code_graph::GraphDocument, CoreError> {
    let mut publication_source = document.clone();
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
    Ok(normalize_document_v1(
        &publication_source,
        root,
        configuration_digest,
    )?)
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
    };
    let _ = write_json_atomic(output_dir.join(OUTPUT_STATS_FILE), &stats, true);
    Some(stats)
}

fn save_output_stats(
    output_dir: &Path,
    nodes: usize,
    edges: usize,
    communities: usize,
    clustered: bool,
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
    handle: Option<std::thread::JoinHandle<Result<(ProgramBuild, Duration), CoreError>>>,
    timings: &mut BuildTimings,
) -> Result<Option<ProgramBuild>, CoreError> {
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

#[cfg(test)]
mod tests {
    use std::error::Error;

    use compass_model::code_graph::GraphDocument as V1GraphDocument;
    use serde_json::{Map, Value};

    use super::*;

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
            build_local_graph(&options)?;
            Ok(V1GraphDocument::load(
                &root.join("compass-out").join("graph.json"),
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
            !punctuation_symbols.is_empty(),
            "fixture must exercise the punctuation-symbol identity collision"
        );
        assert!(
            punctuation_symbols
                .iter()
                .all(|node| node.id.starts_with("sha256:"))
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
        let first = build_graph_with_semantic(&options, &semantic)?;
        let graph_path = first.output_dir.join("graph.json");

        fs::write(root.join("main.py"), "def after():\n    return 2\n")?;
        build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&graph_path)?;
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
        let first = build_graph_with_semantic(&options, &semantic)?;
        let graph_path = first.output_dir.join("graph.json");

        build_local_graph(&options)?;
        let graph = V1GraphDocument::load(&graph_path)?;
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
        fs::remove_dir_all(cold.output_dir.join("cache"))?;

        let warm = build_local_graph(&options)?;
        assert_eq!(warm.files_extracted, 0);
        assert_eq!(warm.files_cached, 1);
        assert_eq!(fs::read(graph_path)?, graph_bytes);
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
        build_graph_with_semantic(&options, &second_layer)?;
        let graph = V1GraphDocument::load(&graph_path)?;
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
        build_graph_with_semantic(&options, &incomplete)?;
        let graph = V1GraphDocument::load(&graph_path)?;
        assert!(graph.nodes.iter().any(|node| node.label() == "Concept A"));
        assert!(!graph.nodes.iter().any(|node| node.label() == "Concept B"));
        let manifest: Value =
            serde_json::from_slice(&fs::read(first.output_dir.join("manifest.json"))?)?;
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
        let first = build_graph_with_semantic(&options, &complete)?;

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
        build_graph_with_semantic(&options, &smaller)?;
        let graph_path = first.output_dir.join("graph.json");
        let graph = V1GraphDocument::load(&graph_path)?;
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
        build_graph_with_semantic(&options, &empty)?;
        let graph = V1GraphDocument::load(&graph_path)?;
        assert!(!graph.nodes.iter().any(|node| node.label() == "Concept A"));
        let manifest: Value =
            serde_json::from_slice(&fs::read(first.output_dir.join("manifest.json"))?)?;
        assert!(manifest.get("diagram.png").is_none());
        Ok(())
    }
}
