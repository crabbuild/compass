use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde_json::json;
use trail_files::{
    BuildGuard, Cache, CacheKind, DetectOptions, Detection, Manifest, ManifestKind, detect,
    write_json_atomic, write_text_atomic,
};
use trail_graph::{
    ClusterOptions, Communities, build_from_extraction, cluster, community_member_signatures,
    god_nodes, label_communities_by_hub, remap_communities_to_previous, score_communities,
    suggest_questions, surprising_connections,
};
use trail_languages::{Engine, Extraction, Registry};
use trail_model::GraphDocument;
use trail_output::{
    DetectionSummary, HtmlOptions, JsonExportOptions, ReportOptions, TokenCost, generate_report,
    write_html, write_json,
};
use trail_resolve::resolve;

#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub root: PathBuf,
    pub output_root: Option<PathBuf>,
    pub force: bool,
    pub no_cluster: bool,
    pub no_viz: bool,
    pub gitignore: bool,
    pub extra_excludes: Vec<String>,
    pub resolution: f64,
    pub exclude_hubs: Option<f64>,
}

impl BuildOptions {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            output_root: None,
            force: false,
            no_cluster: false,
            no_viz: false,
            gitignore: true,
            extra_excludes: Vec::new(),
            resolution: 1.0,
            exclude_hubs: None,
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
}

/// Run the complete deterministic local graph pipeline without invoking Python,
/// an LLM, a network service, or a dynamically installed grammar.
pub fn build_local_graph(options: &BuildOptions) -> Result<BuildResult, CoreError> {
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

    let detection = detect(
        &root,
        &DetectOptions {
            gitignore: options.gitignore,
            extra_excludes: options.extra_excludes.clone(),
            output_name: output_name.clone(),
            ..DetectOptions::default()
        },
    )?;
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
            .filter(|path| Registry::resolve(path).is_some()),
    );
    sources.sort();
    sources.dedup();

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

    let ordered = sources
        .iter()
        .filter_map(|path| extractions.get(path).cloned())
        .collect::<Vec<_>>();
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
    let resolved = resolve(&ordered, &source_text);
    let document = build_from_extraction(&resolved, false, Some(&root));
    if document.nodes.is_empty() {
        return Err(CoreError::EmptyGraph);
    }

    let previous = previous_communities(&output_dir.join("graph.json"));
    let communities = if options.no_cluster {
        Communities::new()
    } else {
        let current = cluster(
            &document,
            ClusterOptions {
                resolution: options.resolution,
                exclude_hubs_percentile: options.exclude_hubs,
            },
        );
        if previous.is_empty() {
            current
        } else {
            remap_communities_to_previous(&current, &previous)
        }
    };
    let labels = label_communities_by_hub(&document, &communities);
    let commit = git_commit(&root);

    write_json(
        &document,
        &communities,
        output_dir.join("graph.json"),
        &JsonExportOptions {
            force: options.force || has_confirmed_deletion,
            built_at_commit: commit.as_deref(),
            community_labels: (!labels.is_empty()).then_some(&labels),
        },
    )?;
    write_text_atomic(output_dir.join(".graphify_root"), &root.to_string_lossy())?;

    let mut html_written = false;
    if !options.no_cluster {
        let cohesion = score_communities(&document, &communities);
        let gods = god_nodes(&document, 10);
        let surprises = surprising_connections(&document, &communities, 5);
        let questions = suggest_questions(&document, &communities, &labels, 10);
        let analysis = json!({
            "communities": communities.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
            "cohesion": cohesion.iter().map(|(key, value)| (key.to_string(), value)).collect::<BTreeMap<_, _>>(),
            "gods": gods,
            "surprises": surprises,
            "questions": questions,
        });
        write_json_atomic(output_dir.join(".graphify_analysis.json"), &analysis, true)?;
        write_json_atomic(output_dir.join(".graphify_labels.json"), &labels, false)?;
        write_json_atomic(
            output_dir.join(".graphify_labels.json.sig"),
            &community_member_signatures(&communities),
            false,
        )?;
        let detection_summary = DetectionSummary {
            total_files: detection.total_files,
            total_words: usize::try_from(detection.total_words).unwrap_or(usize::MAX),
            warning: detection.warning.clone(),
        };
        let report_root = root.to_string_lossy();
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
            let rendered = write_html(
                &document,
                &communities,
                &html_path,
                &HtmlOptions {
                    community_labels: (!labels.is_empty()).then_some(&labels),
                    node_limit: Some(5_000),
                    ..HtmlOptions::default()
                },
            )?;
            html_written = rendered.is_some();
            if !html_written {
                remove_if_exists(&html_path)?;
            }
        }
    }

    let mut manifest = prior_manifest;
    manifest.save(
        &detection.files,
        &manifest_path,
        ManifestKind::Ast,
        Some(&root),
        None,
        None,
    )?;
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
    })
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

fn remove_if_exists(path: &Path) -> Result<(), CoreError> {
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

fn git_commit(root: &Path) -> Option<String> {
    let dot_git = root.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = fs::read_to_string(dot_git).ok()?;
        let relative = text.trim().strip_prefix("gitdir:")?.trim();
        absolutize_from(root, Path::new(relative))
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

        let cold = build_local_graph(&options)?;
        assert_eq!(cold.files_considered, 2);
        assert_eq!(cold.files_extracted, 2);
        assert!(cold.nodes > 0);
        assert!(cold.output_dir.join("graph.json").is_file());
        assert!(cold.output_dir.join("manifest.json").is_file());
        assert!(!cold.output_dir.join(".graphify_incomplete").exists());

        let warm = build_local_graph(&options)?;
        assert_eq!(warm.files_extracted, 0);
        assert_eq!(warm.files_cached, 2);
        assert_eq!(warm.nodes, cold.nodes);
        assert_eq!(warm.edges, cold.edges);

        fs::write(root.join("helper.py"), "def work():\n    return 2\n")?;
        let changed = build_local_graph(&options)?;
        assert_eq!(changed.files_extracted, 1);
        assert_eq!(changed.files_cached, 1);

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
}
