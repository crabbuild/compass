use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde_json::json;
use trail_files::{
    BuildGuard, Cache, CacheKind, DetectOptions, Detection, Manifest, ManifestKind, detect,
    write_json_atomic, write_text_atomic,
};
use trail_graph::{
    ClusterOptions, build_from_extraction, cluster, dedupe_edges, dedupe_nodes, god_nodes,
    label_communities_by_hub, remap_communities_to_previous, score_communities, suggest_questions,
    surprising_connections,
};
use trail_languages::{Engine, Extraction, Registry, file_stem, make_id};
use trail_model::{EdgeRecord, GraphDocument, NodeRecord};
use trail_output::{
    DetectionSummary, HtmlOptions, JsonExportOptions, OutputError, ReportOptions, TokenCost,
    generate_report, write_html, write_json,
};
use trail_resolve::resolve_with_root;

#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub root: PathBuf,
    pub scan_filesystem: bool,
    pub output_root: Option<PathBuf>,
    pub force: bool,
    pub no_cluster: bool,
    pub no_viz: bool,
    pub gitignore: bool,
    pub extra_excludes: Vec<String>,
    pub resolution: f64,
    pub exclude_hubs: Option<f64>,
    pub google_workspace: bool,
    /// Maximum number of worker threads used by the deterministic AST stages.
    /// `None` uses Rayon's process-wide default, matching the library default.
    pub max_workers: Option<usize>,
    /// Override the commit recorded in update artifacts.
    ///
    /// This is primarily useful for reproducible builds and compatibility
    /// tests whose oracle and native halves must share one source revision.
    pub built_at_commit: Option<String>,
    pub purpose: BuildPurpose,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BuildPurpose {
    #[default]
    Update,
    Extract,
}

impl BuildOptions {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            scan_filesystem: true,
            output_root: None,
            force: false,
            no_cluster: false,
            no_viz: false,
            gitignore: true,
            extra_excludes: Vec::new(),
            resolution: 1.0,
            exclude_hubs: None,
            google_workspace: false,
            max_workers: None,
            built_at_commit: None,
            purpose: BuildPurpose::Update,
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
    pub timings: BuildTimings,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BuildTimings {
    pub detect: Duration,
    pub ast_extract: Duration,
    pub build: Duration,
    pub cluster: Duration,
    pub analyze: Duration,
    pub export: Duration,
    pub write: Duration,
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
    File(#[from] trail_files::FileError),
    #[error(transparent)]
    Extract(#[from] trail_languages::ExtractError),
    #[error(transparent)]
    Graph(#[from] trail_model::GraphError),
    #[error(transparent)]
    Output(#[from] trail_output::OutputError),
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
    #[error("invalid semantic extraction fragment: {0}")]
    InvalidSemanticFragment(serde_json::Error),
    #[error("invalid supplemental extraction fragment: {0}")]
    InvalidSupplementalFragment(serde_json::Error),
    #[error("could not create an AST worker pool: {0}")]
    WorkerPool(String),
    #[error(
        "semantic extraction was incomplete and would shrink the graph ({new} < {existing} nodes)"
    )]
    IncompleteSemanticShrink { existing: usize, new: usize },
    #[error("semantic extraction was incomplete and the existing graph is unreadable: {0}")]
    IncompleteSemanticExisting(PathBuf),
}

/// Run the complete deterministic local graph pipeline without invoking Python,
/// an LLM, a network service, or a dynamically installed grammar.
pub fn build_local_graph(options: &BuildOptions) -> Result<BuildResult, CoreError> {
    build_graph(options, None, &[])
}

/// Merge a completed semantic provider result into the native graph pipeline.
pub fn build_graph_with_semantic(
    options: &BuildOptions,
    semantic: &SemanticLayer,
) -> Result<BuildResult, CoreError> {
    build_graph(options, Some(semantic), &[])
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
    build_graph(options, semantic, &supplemental)
}

fn build_graph(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[Extraction],
) -> Result<BuildResult, CoreError> {
    if let Some(max_workers) = options.max_workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(max_workers)
            .thread_name(|index| format!("trail-ast-{index}"))
            .build()
            .map_err(|error| CoreError::WorkerPool(error.to_string()))?;
        return pool.install(|| build_graph_inner(options, semantic, supplemental));
    }
    build_graph_inner(options, semantic, supplemental)
}

fn build_graph_inner(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[Extraction],
) -> Result<BuildResult, CoreError> {
    let mut timings = BuildTimings::default();
    let mut stage_started = Instant::now();
    if !options.root.exists() {
        return Err(CoreError::MissingRoot(options.root.clone()));
    }
    let root = fs::canonicalize(&options.root).map_err(|source| trail_files::FileError::Io {
        path: options.root.clone(),
        source,
    })?;
    let output_name = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let output_root = options
        .output_root
        .as_deref()
        .map_or_else(|| root.clone(), absolutize);
    let output_dir = output_root.join(&output_name);
    fs::create_dir_all(&output_dir).map_err(|source| trail_files::FileError::Io {
        path: output_dir.clone(),
        source,
    })?;
    let guard = BuildGuard::begin(&output_dir)?;
    let manifest_path = output_dir.join("manifest.json");
    let prior_manifest = Manifest::load(&manifest_path, Some(&root));
    let has_confirmed_deletion = prior_manifest
        .entries()
        .keys()
        .any(|path| !Path::new(path).exists());

    let detect_options = DetectOptions {
        scan_filesystem: options.scan_filesystem,
        gitignore: options.gitignore,
        extra_excludes: options.extra_excludes.clone(),
        output_name: output_name.clone(),
        ..DetectOptions::default()
    };
    let mut detection = detect(&root, &detect_options)?;
    if options.google_workspace {
        let converted_dir = root.join(&output_name).join("converted");
        let mut sidecars = Vec::new();
        let mut failures = Vec::new();
        for shortcut in &detection.google_workspace_shortcuts {
            match trail_google_workspace::convert_google_workspace_file(shortcut, &converted_dir) {
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
    sources.sort();
    sources.dedup();

    if semantic.is_none()
        && supplemental.is_empty()
        && options.purpose == BuildPurpose::Update
        && !options.force
        && prior_manifest.is_unchanged(&detection.files, ManifestKind::Ast)
        && let Some(document) = unchanged_output_document(options, &output_dir)
    {
        if options.no_viz {
            remove_if_exists(&output_dir.join("graph.html"))?;
        }
        remove_if_exists(&output_dir.join("needs_update"))?;
        let communities = document
            .nodes
            .iter()
            .filter_map(|node| node.attributes.get("community")?.as_u64())
            .collect::<HashSet<_>>()
            .len();
        guard.commit()?;
        return Ok(BuildResult {
            root,
            output_dir: output_dir.clone(),
            detection,
            files_considered: sources.len(),
            files_extracted: 0,
            files_cached: sources.len(),
            empty_files: Vec::new(),
            nodes: document.nodes.len(),
            edges: document.links.len(),
            communities,
            html_written: output_dir.join("graph.html").is_file(),
            outputs_changed: false,
            timings,
        });
    }

    let cache_root = (output_root != root).then_some(output_root.as_path());
    let mut cache = Cache::new(&root, cache_root)?;
    let mut extractions = BTreeMap::<PathBuf, Extraction>::new();
    let mut missing = Vec::new();
    if !options.force {
        for path in &sources {
            let cached = cache.load(path, &CacheKind::Ast, None, false, false)?;
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
    let fresh = missing
        .par_iter()
        .map_init(Engine::default, |engine, path| {
            engine
                .extract(path)
                .map(|extraction| (path.clone(), extraction))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut empty_files = Vec::new();
    for (path, extraction) in fresh {
        if extraction.nodes.is_empty() {
            empty_files.push(path.clone());
        } else {
            let value = serde_json::to_value(&extraction).map_err(|source| {
                CoreError::SerializeExtraction {
                    path: path.clone(),
                    source,
                }
            })?;
            cache.save(&path, &value, &CacheKind::Ast, None)?;
        }
        extractions.insert(path, extraction);
    }
    cache.flush()?;

    let mut ordered = sources
        .iter()
        .filter_map(|path| extractions.get(path).cloned())
        .collect::<Vec<_>>();
    for extraction in &mut ordered {
        remap_ast_ids(extraction, &root);
    }
    let source_text = sources
        .par_iter()
        .filter_map(|path| {
            fs::read(path).ok().map(|bytes| {
                (
                    path.to_string_lossy().into_owned(),
                    String::from_utf8_lossy(&bytes).into_owned(),
                )
            })
        })
        .collect::<HashMap<_, _>>();
    let mut resolved = resolve_with_root(&ordered, &source_text, &root);
    finalize_ast_extraction(&mut resolved, &root);
    timings.ast_extract = stage_started.elapsed();
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
    if options.no_cluster {
        let (nodes, edges) = (dedupe_nodes(&resolved.nodes), dedupe_edges(&resolved.edges));
        enforce_incomplete_raw_guard(semantic, &output_dir.join("graph.json"), &root, nodes.len())?;
        write_raw_graph(
            &output_dir.join("graph.json"),
            &resolved,
            &nodes,
            &edges,
            options.purpose,
            semantic_tokens(semantic),
        )?;
        write_semantic_marker(&output_dir, semantic)?;
        if options.purpose == BuildPurpose::Update {
            write_text_atomic(
                output_dir.join(".graphify_root"),
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
        guard.commit()?;
        timings.write = stage_started.elapsed();
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
            timings,
        });
    }
    let document = build_from_extraction(&resolved, false, Some(&root));
    timings.build = stage_started.elapsed();
    stage_started = Instant::now();
    if document.nodes.is_empty() {
        return Err(CoreError::EmptyGraph);
    }

    if semantic.is_none()
        && supplemental.is_empty()
        && options.purpose == BuildPurpose::Update
        && update_artifacts_complete(&output_dir)
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
            timings,
        });
    }

    let previous = previous_communities(&output_dir.join("graph.json"));
    let current = cluster(
        &document,
        ClusterOptions {
            resolution: options.resolution,
            exclude_hubs_percentile: options.exclude_hubs,
        },
    );
    let communities = if previous.is_empty() {
        current
    } else {
        remap_communities_to_previous(&current, &previous)
    };
    timings.cluster = stage_started.elapsed();
    stage_started = Instant::now();
    let labels = label_communities_by_hub(&document, &communities);
    let commit = options.built_at_commit.clone().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|directory| git_commit(&directory))
    });

    let incomplete_semantic = semantic.is_some_and(|layer| semantic_is_incomplete(layer, &root));
    write_json(
        &document,
        &communities,
        output_dir.join("graph.json"),
        &JsonExportOptions {
            force: semantic.map_or(options.force || has_confirmed_deletion, |layer| {
                !incomplete_semantic || layer.allow_partial
            }),
            built_at_commit: (options.purpose == BuildPurpose::Update)
                .then_some(commit.as_deref())
                .flatten(),
            community_labels: (options.purpose == BuildPurpose::Update && !labels.is_empty())
                .then_some(&labels),
        },
    )?;
    if options.purpose == BuildPurpose::Update {
        write_text_atomic(
            output_dir.join(".graphify_root"),
            &options.root.to_string_lossy(),
        )?;
    }

    let mut html_written = false;
    {
        let cohesion = score_communities(&document, &communities);
        let gods = god_nodes(&document, 10);
        let surprises = surprising_connections(&document, &communities, 5);
        let questions = suggest_questions(&document, &communities, &labels, 10);
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
        timings.analyze = stage_started.elapsed();
        stage_started = Instant::now();
        if options.purpose == BuildPurpose::Extract {
            write_json_atomic(output_dir.join(".graphify_analysis.json"), &analysis, true)?;
        } else {
            let labels_json = serde_json::to_string_pretty(&labels).map_err(|source| {
                CoreError::SerializeExtraction {
                    path: output_dir.join(".graphify_labels.json"),
                    source,
                }
            })?;
            write_text_atomic(
                output_dir.join(".graphify_labels.json"),
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
        if options.purpose == BuildPurpose::Update {
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
                html_written = rendered.is_some();
                if !html_written {
                    remove_if_exists(&html_path)?;
                }
            }
        }
    }

    write_semantic_marker(&output_dir, semantic)?;

    let mut manifest = prior_manifest;
    save_build_manifest(
        &mut manifest,
        &detection.files,
        &manifest_path,
        &root,
        semantic,
    )?;
    timings.export = stage_started.elapsed();
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
        timings,
    })
}

fn write_semantic_marker(
    output_dir: &Path,
    semantic: Option<&SemanticLayer>,
) -> Result<(), CoreError> {
    let (_, output_tokens) = semantic_tokens(semantic);
    if output_tokens > 0 {
        write_json_atomic(
            output_dir.join(".graphify_semantic_marker"),
            &json!({"output_tokens": output_tokens}),
            false,
        )?;
    }
    Ok(())
}

fn finalize_ast_extraction(extraction: &mut Extraction, root: &Path) {
    remap_ast_ids(extraction, root);
    for node in &mut extraction.nodes {
        normalize_source_attribute(&mut node.attributes, root);
        node.attributes.remove("origin_file");
        node.attributes.remove("_callable");
        node.attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("ast".to_owned()),
        );
    }
    for edge in &mut extraction.edges {
        normalize_source_attribute(&mut edge.attributes, root);
        edge.attributes.insert(
            "_origin".to_owned(),
            serde_json::Value::String("ast".to_owned()),
        );
    }
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

fn remap_ast_ids(extraction: &mut Extraction, root: &Path) {
    let mut id_remap = HashMap::new();
    for node in &extraction.nodes {
        let Some(source) = node
            .attributes
            .get("source_file")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let source_path = Path::new(source);
        let Ok(relative) = source_path.strip_prefix(root) else {
            continue;
        };
        let old_prefix = make_id(&[&file_stem(source_path)]);
        let new_prefix = make_id(&[&file_stem(relative)]);
        if node
            .attributes
            .get("type")
            .and_then(serde_json::Value::as_str)
            == Some("package")
        {
            continue;
        }
        if node.id == make_id(&[source]) || node.id == old_prefix {
            id_remap.insert(node.id.clone(), new_prefix);
        } else if let Some(suffix) = node.id.strip_prefix(&format!("{old_prefix}_")) {
            id_remap.insert(node.id.clone(), format!("{new_prefix}_{suffix}"));
        }
    }
    for node in &mut extraction.nodes {
        if let Some(canonical) = id_remap.get(&node.id) {
            node.id.clone_from(canonical);
        }
    }
    for edge in &mut extraction.edges {
        if let Some(canonical) = id_remap.get(&edge.source) {
            edge.source.clone_from(canonical);
        }
        if let Some(canonical) = id_remap.get(&edge.target) {
            edge.target.clone_from(canonical);
        }
    }
    if let Some(calls) = extraction.raw_calls.as_mut() {
        for call in calls {
            if let Some(canonical) = id_remap.get(&call.caller_nid) {
                call.caller_nid.clone_from(canonical);
            }
        }
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
    let Ok(relative) = path.strip_prefix(root) else {
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
    let Ok(existing) = GraphDocument::load(graph_path) else {
        return;
    };
    let ast_ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut preserved_nodes = existing
        .nodes
        .into_iter()
        .filter(|node| {
            !ast_ids.contains(node.id.as_str())
                && node
                    .attributes
                    .get("_origin")
                    .and_then(serde_json::Value::as_str)
                    != Some("ast")
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
    let mut preserved_edges = existing
        .links
        .into_iter()
        .filter(|edge| {
            all_ids.contains(&edge.source)
                && all_ids.contains(&edge.target)
                && edge
                    .attributes
                    .get("_origin")
                    .and_then(serde_json::Value::as_str)
                    != Some("ast")
                && !source_in_set(edge.attributes.get("source_file"), root, refreshed)
                && !source_was_deleted(edge.attributes.get("source_file"), root)
        })
        .collect::<Vec<_>>();
    extraction.nodes.append(&mut preserved_nodes);
    extraction.edges.append(&mut preserved_edges);
    let new_hyperedge_ids = extraction
        .hyperedges
        .iter()
        .filter_map(|value| value.get("id").and_then(serde_json::Value::as_str))
        .collect::<HashSet<_>>();
    let existing_hyperedges = existing
        .extras
        .get("hyperedges")
        .or_else(|| existing.graph.get("hyperedges"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|hyperedge| {
            let Some(object) = hyperedge.as_object() else {
                return false;
            };
            if object
                .get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| new_hyperedge_ids.contains(id))
                || source_in_set(object.get("source_file"), root, refreshed)
                || source_was_deleted(object.get("source_file"), root)
            {
                return false;
            }
            ["nodes", "members", "node_ids"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(serde_json::Value::as_array))
                .is_none_or(|members| {
                    members.iter().all(|member| {
                        member
                            .as_str()
                            .is_some_and(|member| all_ids.contains(member))
                    })
                })
        })
        .collect::<Vec<_>>();
    extraction.hyperedges.extend(existing_hyperedges);
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
    let Ok(existing) = GraphDocument::load(graph_path) else {
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
        .filter(|node| {
            node.attributes
                .get("_origin")
                .and_then(serde_json::Value::as_str)
                != Some("ast")
        })
        .filter_map(|node| semantic_source_under_root(node.attributes.get("source_file"), root))
        .filter(|source| !live.contains(source))
        .collect::<HashSet<_>>();
    stale.extend(
        existing
            .links
            .iter()
            .filter(|edge| {
                edge.attributes
                    .get("_origin")
                    .and_then(serde_json::Value::as_str)
                    != Some("ast")
            })
            .filter_map(|edge| semantic_source_under_root(edge.attributes.get("source_file"), root))
            .filter(|source| !live.contains(source)),
    );
    let hyperedges = existing
        .extras
        .get("hyperedges")
        .or_else(|| existing.graph.get("hyperedges"))
        .and_then(serde_json::Value::as_array);
    stale.extend(
        hyperedges
            .into_iter()
            .flatten()
            .filter_map(|hyperedge| semantic_source_under_root(hyperedge.get("source_file"), root))
            .filter(|source| !live.contains(source)),
    );
    stale
}

fn semantic_source_under_root(value: Option<&serde_json::Value>, root: &Path) -> Option<PathBuf> {
    let source = value.and_then(serde_json::Value::as_str)?;
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

fn semantic_is_incomplete(layer: &SemanticLayer, root: &Path) -> bool {
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
        manifest.save(files, path, ManifestKind::Ast, Some(root), None, None)?;
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

fn enforce_incomplete_raw_guard(
    semantic: Option<&SemanticLayer>,
    graph_path: &Path,
    root: &Path,
    new_count: usize,
) -> Result<(), CoreError> {
    let Some(layer) = semantic else {
        return Ok(());
    };
    if layer.allow_partial || !semantic_is_incomplete(layer, root) || !graph_path.exists() {
        return Ok(());
    }
    let existing = GraphDocument::load(graph_path)
        .map_err(|_| CoreError::IncompleteSemanticExisting(graph_path.to_path_buf()))?
        .nodes
        .len();
    if new_count < existing {
        return Err(CoreError::IncompleteSemanticShrink {
            existing,
            new: new_count,
        });
    }
    Ok(())
}

fn semantic_document_sources(graph_path: &Path, root: &Path) -> HashSet<PathBuf> {
    let Ok(existing) = GraphDocument::load(graph_path) else {
        return HashSet::new();
    };
    existing
        .nodes
        .into_iter()
        .filter(|node| {
            node.attributes
                .get("_origin")
                .and_then(serde_json::Value::as_str)
                != Some("ast")
                && matches!(
                    node.attributes
                        .get("file_type")
                        .and_then(serde_json::Value::as_str),
                    Some("document" | "concept" | "rationale" | "paper")
                )
        })
        .filter_map(|node| {
            node.attributes
                .get("source_file")
                .and_then(serde_json::Value::as_str)
                .map(Path::new)
                .map(|path| {
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

fn write_raw_graph(
    path: &Path,
    extraction: &Extraction,
    nodes: &[NodeRecord],
    edges: &[EdgeRecord],
    purpose: BuildPurpose,
    tokens: (u64, u64),
) -> Result<(), CoreError> {
    let mut output = serde_json::Map::new();
    let nodes = serde_json::to_value(nodes).map_err(|source| CoreError::SerializeExtraction {
        path: path.to_path_buf(),
        source,
    })?;
    let edges = serde_json::to_value(edges).map_err(|source| CoreError::SerializeExtraction {
        path: path.to_path_buf(),
        source,
    })?;
    let hyperedges = serde_json::Value::Array(extraction.hyperedges.clone());
    if purpose == BuildPurpose::Extract {
        output.insert("nodes".to_owned(), nodes);
        output.insert("edges".to_owned(), edges);
        output.insert("hyperedges".to_owned(), hyperedges);
        output.insert("input_tokens".to_owned(), serde_json::Value::from(tokens.0));
        output.insert(
            "output_tokens".to_owned(),
            serde_json::Value::from(tokens.1),
        );
    } else {
        output.insert("input_tokens".to_owned(), serde_json::Value::from(0));
        output.insert("output_tokens".to_owned(), serde_json::Value::from(0));
        output.insert("nodes".to_owned(), nodes);
        output.insert("links".to_owned(), edges);
    }
    let encoded =
        serde_json::to_string_pretty(&serde_json::Value::Object(output)).map_err(|source| {
            CoreError::SerializeExtraction {
                path: path.to_path_buf(),
                source,
            }
        })?;
    let encoded = escape_non_ascii(&encoded);
    let encoded = if purpose == BuildPurpose::Update {
        format!("{encoded}\n")
    } else {
        encoded
    };
    write_text_atomic(path, &encoded)?;
    Ok(())
}

fn escape_non_ascii(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        let code = character as u32;
        if code <= 0x7f {
            output.push(character);
        } else if code <= 0xffff {
            output.push_str(&format!("\\u{code:04x}"));
        } else {
            let scalar = code - 0x1_0000;
            output.push_str(&format!(
                "\\u{:04x}\\u{:04x}",
                0xd800 + (scalar >> 10),
                0xdc00 + (scalar & 0x3ff)
            ));
        }
    }
    output
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
        ".graphify_labels.json",
        ".graphify_root",
    ]
    .into_iter()
    .all(|name| output_dir.join(name).is_file())
}

fn unchanged_output_document(options: &BuildOptions, output_dir: &Path) -> Option<GraphDocument> {
    let graph_path = output_dir.join("graph.json");
    let bytes = fs::read(&graph_path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let is_clustered = value.get("directed").is_some() && value.get("multigraph").is_some();
    if options.no_cluster == is_clustered || !output_dir.join(".graphify_root").is_file() {
        return None;
    }
    let document: GraphDocument = serde_json::from_value(value).ok()?;
    if options.no_cluster {
        return Some(document);
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
    Some(document)
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
        Err(source) => Err(trail_files::FileError::Io {
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

    use serde_json::{Map, Value};

    use super::*;

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
        assert!(cold.timings.ast_extract > Duration::ZERO);
        assert!(cold.timings.build > Duration::ZERO);
        assert!(cold.timings.cluster > Duration::ZERO);
        assert!(cold.timings.analyze > Duration::ZERO);
        assert!(cold.timings.export > Duration::ZERO);
        assert!(cold.nodes > 0);
        assert!(cold.output_dir.join("graph.json").is_file());
        assert!(cold.output_dir.join("manifest.json").is_file());
        assert!(!cold.output_dir.join(".graphify_incomplete").exists());
        let cold_graph = GraphDocument::load(&cold.output_dir.join("graph.json"))?;
        assert!(cold_graph.nodes.iter().all(|node| {
            node.attributes.get("_origin").and_then(Value::as_str) == Some("ast")
                && !Path::new(&node.string("source_file")).is_absolute()
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
        assert_eq!(
            fs::read(changed.output_dir.join("graph.json"))?,
            cold_graph_bytes,
            "a body-only incremental edit must preserve cold-build topology"
        );

        fs::remove_file(root.join("helper.py"))?;
        let deleted = build_local_graph(&options)?;
        assert_eq!(deleted.files_considered, 1);
        let graph = GraphDocument::load(&deleted.output_dir.join("graph.json"))?;
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.string("source_file") != "helper.py")
        );
        Ok(())
    }

    #[test]
    fn update_preserves_semantic_layer_but_replaces_ast_layer() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::write(root.join("main.py"), "def before():\n    return 1\n")?;
        let mut options = BuildOptions::new(root);
        options.no_viz = true;
        let first = build_local_graph(&options)?;
        let graph_path = first.output_dir.join("graph.json");
        let mut graph = GraphDocument::load(&graph_path)?;
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String("Domain rule".to_owned()));
        attributes.insert("file_type".to_owned(), Value::String("concept".to_owned()));
        graph.nodes.push(NodeRecord {
            id: "semantic_domain_rule".to_owned(),
            attributes,
        });
        write_json_atomic(&graph_path, &graph, true)?;

        fs::write(root.join("main.py"), "def after():\n    return 2\n")?;
        build_local_graph(&options)?;
        let graph = GraphDocument::load(&graph_path)?;
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "semantic_domain_rule")
        );
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
        let first = build_local_graph(&options)?;
        let graph_path = first.output_dir.join("graph.json");
        let semantic = GraphDocument {
            directed: false,
            multigraph: false,
            graph: Map::new(),
            nodes: vec![NodeRecord {
                id: "semantic_guide".to_owned(),
                attributes: Map::from_iter([
                    (
                        "label".to_owned(),
                        Value::String("Guide concept".to_owned()),
                    ),
                    ("file_type".to_owned(), Value::String("concept".to_owned())),
                    (
                        "source_file".to_owned(),
                        Value::String("guide.md".to_owned()),
                    ),
                ]),
            }],
            links: Vec::new(),
            extras: BTreeMap::new(),
            used_legacy_edges_key: false,
        };
        write_json_atomic(&graph_path, &semantic, true)?;

        build_local_graph(&options)?;
        let graph = GraphDocument::load(&graph_path)?;
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].id, "semantic_guide");
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.attributes.get("_origin").is_none())
        );
        Ok(())
    }

    #[test]
    fn no_cluster_schema_tracks_command_purpose() -> Result<(), Box<dyn Error>> {
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
        assert!(value.get("edges").is_some());
        assert!(value.get("links").is_none());
        assert!(value.get("directed").is_none());
        assert!(!result.output_dir.join("GRAPH_REPORT.md").exists());
        assert!(!result.output_dir.join(".graphify_analysis.json").exists());

        let update_dir = tempfile::tempdir()?;
        fs::write(update_dir.path().join("main.py"), "def main():\n    pass\n")?;
        let mut update = BuildOptions::new(update_dir.path());
        update.no_cluster = true;
        let result = build_local_graph(&update)?;
        let bytes = fs::read(result.output_dir.join("graph.json"))?;
        assert_eq!(bytes.last(), Some(&b'\n'));
        let value: Value = serde_json::from_slice(&bytes)?;
        assert!(value.get("links").is_some());
        assert!(value.get("edges").is_none());
        assert!(result.output_dir.join(".graphify_root").is_file());
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
        let graph = GraphDocument::load(&graph_path)?;
        assert!(graph.nodes.iter().any(|node| node.id == "old_concept"));
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
            serde_json::from_slice(&fs::read(first.output_dir.join(".graphify_analysis.json"))?)?;
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
        let graph = GraphDocument::load(&graph_path)?;
        assert!(!graph.nodes.iter().any(|node| node.id == "old_concept"));
        assert!(graph.nodes.iter().any(|node| node.id == "new_concept"));
        let Some(semantic) = graph.nodes.iter().find(|node| node.id == "new_concept") else {
            return Err("new semantic node was not written".into());
        };
        assert_eq!(semantic.string("source_file"), "diagram.png");
        assert_eq!(semantic.string("_origin"), "semantic");
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
                    {"id":"concept_a", "source_file":"diagram.png"},
                    {"id":"concept_b", "source_file":"diagram.png"}
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
                "nodes": [{"id":"concept_a", "source_file":"diagram.png"}],
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
        let graph = GraphDocument::load(&graph_path)?;
        assert!(graph.nodes.iter().any(|node| node.id == "concept_a"));
        assert!(!graph.nodes.iter().any(|node| node.id == "concept_b"));
        let raw: Value = serde_json::from_slice(&fs::read(&graph_path)?)?;
        assert_eq!(raw["input_tokens"], 2);
        assert_eq!(raw["output_tokens"], 1);
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
                    {"id":"concept_a", "source_file":"diagram.png"},
                    {"id":"concept_b", "source_file":"diagram.png"}
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
                "nodes": [{"id":"concept_a", "source_file":"diagram.png"}],
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
        let graph = GraphDocument::load(&graph_path)?;
        assert!(graph.nodes.iter().any(|node| node.id == "concept_a"));
        assert!(!graph.nodes.iter().any(|node| node.id == "concept_b"));

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
        let graph = GraphDocument::load(&graph_path)?;
        assert!(!graph.nodes.iter().any(|node| node.id == "concept_a"));
        let manifest: Value =
            serde_json::from_slice(&fs::read(first.output_dir.join("manifest.json"))?)?;
        assert!(manifest.get("diagram.png").is_none());
        Ok(())
    }
}
