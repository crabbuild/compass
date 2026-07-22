//! Application services shared by the Compass and Graphify command frontends.

mod cluster_existing;
mod diagnostics;
mod merge;
mod pipeline;
mod raw_guard;
mod watch;

pub use cluster_existing::{
    ClusterExistingOptions, ClusterExistingResult, ClusterExistingTimings, ClusterLabelContext,
    ClusterLabelSelection, cluster_existing_graph, cluster_existing_graph_with_labeler,
};
pub use diagnostics::{diagnose_graph_file, format_diagnostic_json, format_diagnostic_report};
pub use merge::{MergeResult, merge_graphs};
pub use pipeline::{
    BuildOptions, BuildPurpose, BuildResult, BuildTimings, CoreError, SemanticLayer,
    build_graph_with_layers, build_graph_with_layers_and_tiebreaker, build_graph_with_semantic,
    build_local_graph,
};
pub use watch::{WatchError, WatchOptions, WatchStatus, watch_local_graph};

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use compass_graph::{Communities, GodNode};
use compass_model::{Graph, GraphDocument, GraphError};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub struct ExportInputs {
    pub document: GraphDocument,
    pub communities: Communities,
    pub labels: std::collections::BTreeMap<usize, String>,
    pub cohesion: std::collections::BTreeMap<usize, f64>,
    pub gods: Vec<GodNode>,
    pub report: String,
}

impl ExportInputs {
    pub fn load(graph_path: &Path) -> Result<Self, GraphError> {
        let document = GraphDocument::load(graph_path)?;
        let output_dir = graph_path.parent().unwrap_or_else(|| Path::new("."));
        let analysis = read_json_value(&output_dir.join(".graphify_analysis.json"));
        let mut communities = analysis
            .as_ref()
            .and_then(|value| value.get("communities"))
            .and_then(Value::as_object)
            .map(parse_communities)
            .unwrap_or_default();
        if communities.is_empty() {
            for node in &document.nodes {
                let community = node
                    .attributes
                    .get("community")
                    .and_then(|value| {
                        value
                            .as_u64()
                            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                    })
                    .and_then(|value| usize::try_from(value).ok());
                if let Some(community) = community {
                    communities
                        .entry(community)
                        .or_default()
                        .push(node.id.clone());
                }
            }
        }
        let cohesion = analysis
            .as_ref()
            .and_then(|value| value.get("cohesion"))
            .and_then(Value::as_object)
            .map(parse_float_map)
            .unwrap_or_default();
        let gods = analysis
            .as_ref()
            .and_then(|value| value.get("gods"))
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let labels = read_json_value(&output_dir.join(".graphify_labels.json"))
            .and_then(|value| value.as_object().map(parse_string_map))
            .unwrap_or_default();
        let report = fs::read_to_string(output_dir.join("GRAPH_REPORT.md")).unwrap_or_default();
        Ok(Self {
            document,
            communities,
            labels,
            cohesion,
            gods,
            report,
        })
    }
}

fn read_json_value(path: &Path) -> Option<Value> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn parse_communities(object: &Map<String, Value>) -> Communities {
    object
        .iter()
        .filter_map(|(key, value)| {
            let key = key.parse::<usize>().ok()?;
            let members = value
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            Some((key, members))
        })
        .collect()
}

fn parse_float_map(object: &Map<String, Value>) -> std::collections::BTreeMap<usize, f64> {
    object
        .iter()
        .filter_map(|(key, value)| Some((key.parse().ok()?, value.as_f64()?)))
        .collect()
}

fn parse_string_map(object: &Map<String, Value>) -> std::collections::BTreeMap<usize, String> {
    object
        .iter()
        .filter_map(|(key, value)| Some((key.parse().ok()?, value.as_str()?.to_owned())))
        .collect()
}

pub struct LoadedGraph {
    pub graph: Graph,
    pub overlay: HashMap<String, Map<String, Value>>,
}

impl LoadedGraph {
    pub fn from_document(
        mut document: GraphDocument,
        force_directed: bool,
    ) -> Result<Self, GraphError> {
        if force_directed {
            document.directed = true;
        }
        Ok(Self {
            graph: Graph::from_document(document)?,
            overlay: HashMap::new(),
        })
    }

    pub fn load(path: &Path) -> Result<Self, GraphError> {
        let graph = Graph::load(path)?;
        let overlay = load_learning_overlay(path);
        Ok(Self { graph, overlay })
    }

    pub fn load_directed(path: &Path) -> Result<Self, GraphError> {
        let graph = Graph::load_directed(path)?;
        let overlay = load_learning_overlay(path);
        Ok(Self { graph, overlay })
    }

    pub fn load_for_affected(path: &Path) -> Result<Self, GraphError> {
        let graph = Graph::load_for_affected(path)?;
        Ok(Self {
            graph,
            overlay: HashMap::new(),
        })
    }
}

#[must_use]
pub fn default_graph_path() -> PathBuf {
    PathBuf::from(std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned()))
        .join("graph.json")
}

fn load_learning_overlay(graph_path: &Path) -> HashMap<String, Map<String, Value>> {
    let sidecar = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".graphify_learning.json");
    let Ok(bytes) = fs::read(sidecar) else {
        return HashMap::new();
    };
    let Ok(document) = serde_json::from_slice::<Value>(&bytes) else {
        return HashMap::new();
    };
    let Some(nodes) = document.get("nodes").and_then(Value::as_object) else {
        return HashMap::new();
    };
    nodes
        .iter()
        .filter_map(|(id, value)| {
            let mut entry = value.as_object()?.clone();
            entry.insert(
                "stale".to_owned(),
                Value::Bool(is_stale(&entry, graph_path)),
            );
            Some((id.clone(), entry))
        })
        .collect()
}

fn load_learning_for_report(graph_path: &Path) -> Option<Value> {
    let overlay = load_learning_overlay(graph_path);
    let memory_dir = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("memory");
    let docs = compass_reflect::load_memory_docs(&memory_dir);
    let aggregate = compass_reflect::aggregate_lessons(
        &docs,
        None,
        None,
        time::OffsetDateTime::now_utc(),
        compass_reflect::DEFAULT_HALF_LIFE_DAYS,
        compass_reflect::DEFAULT_MIN_CORROBORATION,
    );
    if overlay.is_empty() && aggregate.dead_ends.is_empty() {
        return None;
    }
    let dead_ends = aggregate
        .dead_ends
        .into_iter()
        .map(|dead_end| {
            serde_json::json!({
                "question": dead_end.question,
                "nodes": dead_end.nodes,
                "date": dead_end.date,
            })
        })
        .collect::<Vec<_>>();
    Some(serde_json::json!({
        "overlay": overlay,
        "dead_ends": dead_ends,
    }))
}

fn is_stale(entry: &Map<String, Value>, graph_path: &Path) -> bool {
    let source = entry
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if source.is_empty() {
        return false;
    }
    let Some(path) = resolve_source_path(source, graph_path) else {
        return true;
    };
    let stored = entry
        .get("code_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if stored.is_empty() {
        return true;
    }
    fs::read(path)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)) != stored)
        .unwrap_or(true)
}

fn resolve_source_path(source: &str, graph_path: &Path) -> Option<PathBuf> {
    let source_path = Path::new(source);
    if source_path.is_absolute() {
        return source_path.is_file().then(|| source_path.to_path_buf());
    }
    let output_dir = graph_path.parent().unwrap_or_else(|| Path::new("."));
    let output_name = std::env::var("GRAPHIFY_OUT")
        .ok()
        .and_then(|value| {
            PathBuf::from(value)
                .file_name()
                .map(|name| name.to_os_string())
        })
        .unwrap_or_else(|| "graphify-out".into());
    let mut roots = Vec::new();
    if let Ok(recorded) = fs::read_to_string(output_dir.join(".graphify_root")) {
        let recorded = recorded.trim();
        if !recorded.is_empty() {
            roots.push(PathBuf::from(recorded));
        }
    }
    if output_dir.file_name() == Some(output_name.as_os_str()) {
        if let Some(parent) = output_dir.parent() {
            roots.push(parent.to_path_buf());
        }
        roots.push(output_dir.to_path_buf());
    } else {
        roots.push(output_dir.to_path_buf());
        if let Some(parent) = output_dir.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    if let Ok(current) = std::env::current_dir() {
        roots.push(current);
    }
    roots
        .into_iter()
        .map(|root| root.join(source_path))
        .find(|candidate| candidate.is_file())
}
