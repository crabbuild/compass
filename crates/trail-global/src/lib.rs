//! Persistent cross-project graph management for Trail and Graphify.

use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use trail_files::{FileError, write_json_atomic, write_text_atomic};
use trail_model::{EdgeRecord, GraphDocument, GraphError};

const MAX_GRAPH_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum GlobalError {
    #[error("graph not found: {0}")]
    GraphNotFound(PathBuf),
    #[error("graph file {path} is {size} bytes, exceeds {limit}-byte safety cap")]
    GraphTooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("could not load {path}: {source}")]
    Graph { path: PathBuf, source: GraphError },
    #[error("could not write {path}: {source}")]
    Write { path: PathBuf, source: FileError },
    #[error("repo '{0}' not in global graph")]
    UnknownRepo(String),
    #[error("could not determine the user home directory")]
    MissingHome,
}

#[derive(Clone, Debug)]
pub struct GlobalPaths {
    pub directory: PathBuf,
    pub graph: PathBuf,
    pub manifest: PathBuf,
}

impl GlobalPaths {
    pub fn discover() -> Result<Self, GlobalError> {
        let home = home_directory().ok_or(GlobalError::MissingHome)?;
        let directory = home.join(".graphify");
        Ok(Self {
            graph: directory.join("global-graph.json"),
            manifest: directory.join("global-manifest.json"),
            directory,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct AddResult {
    pub repo_tag: String,
    pub nodes_added: usize,
    pub nodes_removed: usize,
    pub skipped: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ManifestLoad {
    pub value: Value,
    pub warnings: Vec<String>,
}

pub fn global_add(
    paths: &GlobalPaths,
    source_path: &Path,
    repo_tag: &str,
    now: OffsetDateTime,
) -> Result<AddResult, GlobalError> {
    if !source_path.exists() {
        return Err(GlobalError::GraphNotFound(source_path.to_path_buf()));
    }
    let mut loaded_manifest = load_manifest(paths);
    let manifest = manifest_object(&mut loaded_manifest.value);
    check_graph_size(source_path)?;
    let source_hash = file_hash(source_path)?;
    let absolute_source = canonical_or_absolute(source_path);
    let existing = manifest
        .get("repos")
        .and_then(Value::as_object)
        .and_then(|repos| repos.get(repo_tag))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(existing_path) = existing.get("source_path").and_then(Value::as_str)
        && !existing_path.is_empty()
        && existing_path != absolute_source.to_string_lossy()
    {
        loaded_manifest.warnings.push(format!(
            "[graphify global] warning: repo tag '{repo_tag}' previously pointed to {}, now updating to {}. Use --as <tag> to give it a different name.",
            python_repr(existing_path),
            python_repr(&absolute_source.to_string_lossy())
        ));
    }
    if existing.get("source_hash").and_then(Value::as_str) == Some(&source_hash) {
        return Ok(AddResult {
            repo_tag: repo_tag.to_owned(),
            skipped: true,
            warnings: loaded_manifest.warnings,
            ..AddResult::default()
        });
    }

    let source = load_graph(source_path)?;
    let edge_count = source.links.len();
    let prefixed = prefix_graph(source, repo_tag);
    let mut global = load_global_graph(paths)?;
    let removed = prune_repo(&mut global, repo_tag);
    let external_labels = global
        .nodes
        .iter()
        .filter(|node| node.string("source_file").is_empty() && !node.label().is_empty())
        .map(|node| (node.label().to_owned(), node.id.clone()))
        .collect::<HashMap<_, _>>();
    let remap = prefixed
        .nodes
        .iter()
        .filter_map(|node| {
            (node.string("source_file").is_empty())
                .then(|| {
                    external_labels
                        .get(node.label())
                        .map(|id| (node.id.clone(), id.clone()))
                })
                .flatten()
        })
        .collect::<HashMap<_, _>>();
    let added = prefixed.nodes.len().saturating_sub(remap.len());
    compose_into(&mut global, prefixed, &remap);
    save_global_graph(paths, &global)?;

    let repos = manifest_repos_mut(&mut loaded_manifest.value);
    let mut info = Map::new();
    info.insert("added_at".to_owned(), Value::String(iso_timestamp(now)));
    info.insert(
        "source_path".to_owned(),
        Value::String(absolute_source.to_string_lossy().into_owned()),
    );
    info.insert("node_count".to_owned(), Value::from(added));
    info.insert("edge_count".to_owned(), Value::from(edge_count));
    info.insert("source_hash".to_owned(), Value::String(source_hash));
    repos.insert(repo_tag.to_owned(), Value::Object(info));
    save_manifest(paths, &loaded_manifest.value)?;
    Ok(AddResult {
        repo_tag: repo_tag.to_owned(),
        nodes_added: added,
        nodes_removed: removed,
        skipped: false,
        warnings: loaded_manifest.warnings,
    })
}

pub fn global_remove(
    paths: &GlobalPaths,
    repo_tag: &str,
) -> Result<(usize, Vec<String>), GlobalError> {
    let mut loaded = load_manifest(paths);
    if !manifest_repos(&loaded.value).contains_key(repo_tag) {
        return Err(GlobalError::UnknownRepo(repo_tag.to_owned()));
    }
    let mut graph = load_global_graph(paths)?;
    let removed = prune_repo(&mut graph, repo_tag);
    save_global_graph(paths, &graph)?;
    manifest_repos_mut(&mut loaded.value).remove(repo_tag);
    save_manifest(paths, &loaded.value)?;
    Ok((removed, loaded.warnings))
}

#[must_use]
pub fn global_list(paths: &GlobalPaths) -> ManifestLoad {
    load_manifest(paths)
}

fn prefix_graph(mut graph: GraphDocument, repo_tag: &str) -> GraphDocument {
    for node in &mut graph.nodes {
        let local = node.id.clone();
        node.id = format!("{repo_tag}::{local}");
        node.attributes
            .insert("repo".to_owned(), Value::String(repo_tag.to_owned()));
        node.attributes
            .entry("local_id".to_owned())
            .or_insert(Value::String(local));
    }
    for edge in &mut graph.links {
        edge.source = format!("{repo_tag}::{}", edge.source);
        edge.target = format!("{repo_tag}::{}", edge.target);
        edge.attributes.remove("key");
    }
    graph
}

fn prune_repo(graph: &mut GraphDocument, repo_tag: &str) -> usize {
    let mut removed = graph
        .nodes
        .iter()
        .filter(|node| node.string("repo") == repo_tag && !node.string("source_file").is_empty())
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    graph.nodes.retain(|node| !removed.contains(&node.id));
    graph
        .links
        .retain(|edge| !removed.contains(&edge.source) && !removed.contains(&edge.target));

    let referenced = graph
        .links
        .iter()
        .flat_map(|edge| [&edge.source, &edge.target])
        .collect::<HashSet<_>>();
    let orphaned_external = graph
        .nodes
        .iter()
        .filter(|node| node.string("source_file").is_empty() && !referenced.contains(&node.id))
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    graph
        .nodes
        .retain(|node| !orphaned_external.contains(&node.id));
    removed.extend(orphaned_external);
    removed.len()
}

fn compose_into(
    global: &mut GraphDocument,
    prefixed: GraphDocument,
    remap: &HashMap<String, String>,
) {
    let mut ids = global
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    for node in prefixed.nodes {
        if !remap.contains_key(&node.id) && ids.insert(node.id.clone()) {
            global.nodes.push(node);
        }
    }
    let positions = global
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut edge_positions = global
        .links
        .iter()
        .enumerate()
        .map(|(index, edge)| {
            (
                undirected_pair(&edge.source, &edge.target, &positions),
                index,
            )
        })
        .collect::<HashMap<_, _>>();
    for mut edge in prefixed.links {
        edge.source = remap.get(&edge.source).cloned().unwrap_or(edge.source);
        edge.target = remap.get(&edge.target).cloned().unwrap_or(edge.target);
        if edge.source == edge.target {
            continue;
        }
        orient_edge(&mut edge, &positions);
        let pair = undirected_pair(&edge.source, &edge.target, &positions);
        if let Some(index) = edge_positions.get(&pair).copied() {
            merge_attributes(&mut global.links[index].attributes, edge.attributes);
        } else {
            edge_positions.insert(pair, global.links.len());
            global.links.push(edge);
        }
    }
}

fn orient_edge(edge: &mut EdgeRecord, positions: &HashMap<String, usize>) {
    let source = positions.get(&edge.source).copied().unwrap_or(usize::MAX);
    let target = positions.get(&edge.target).copied().unwrap_or(usize::MAX);
    if target < source {
        std::mem::swap(&mut edge.source, &mut edge.target);
    }
}

fn undirected_pair(
    source: &str,
    target: &str,
    positions: &HashMap<String, usize>,
) -> (String, String) {
    let source_position = positions.get(source).copied().unwrap_or(usize::MAX);
    let target_position = positions.get(target).copied().unwrap_or(usize::MAX);
    if source_position <= target_position {
        (source.to_owned(), target.to_owned())
    } else {
        (target.to_owned(), source.to_owned())
    }
}

fn merge_attributes(target: &mut Map<String, Value>, incoming: Map<String, Value>) {
    for (key, value) in incoming {
        if let Some(existing) = target.get_mut(&key) {
            *existing = value;
        } else {
            target.insert(key, value);
        }
    }
}

fn load_global_graph(paths: &GlobalPaths) -> Result<GraphDocument, GlobalError> {
    if !paths.graph.exists() {
        return Ok(empty_global_graph());
    }
    let mut graph = load_graph(&paths.graph)?;
    graph.multigraph = false;
    for edge in &mut graph.links {
        edge.attributes.remove("key");
    }
    Ok(graph)
}

fn empty_global_graph() -> GraphDocument {
    GraphDocument {
        directed: false,
        multigraph: false,
        graph: Map::new(),
        nodes: Vec::new(),
        links: Vec::new(),
        extras: Default::default(),
        used_legacy_edges_key: false,
    }
}

fn load_graph(path: &Path) -> Result<GraphDocument, GlobalError> {
    check_graph_size(path)?;
    GraphDocument::load(path).map_err(|source| GlobalError::Graph {
        path: path.to_path_buf(),
        source,
    })
}

fn check_graph_size(path: &Path) -> Result<(), GlobalError> {
    let metadata = path.metadata().map_err(|source| GlobalError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let limit = effective_graph_cap();
    if metadata.len() > limit {
        return Err(GlobalError::GraphTooLarge {
            path: path.to_path_buf(),
            size: metadata.len(),
            limit,
        });
    }
    Ok(())
}

fn effective_graph_cap() -> u64 {
    let raw = env::var("GRAPHIFY_MAX_GRAPH_BYTES").unwrap_or_default();
    let text = raw.trim().to_ascii_uppercase();
    if text.is_empty() {
        return MAX_GRAPH_BYTES;
    }
    let (number, multiplier) = if let Some(number) = text.strip_suffix("GB") {
        (number.trim(), 1024_u64 * 1024 * 1024)
    } else if let Some(number) = text.strip_suffix("MB") {
        (number.trim(), 1024_u64 * 1024)
    } else {
        (text.as_str(), 1)
    };
    number
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .and_then(|value| value.checked_mul(multiplier))
        .unwrap_or(MAX_GRAPH_BYTES)
}

fn save_global_graph(paths: &GlobalPaths, graph: &GraphDocument) -> Result<(), GlobalError> {
    fs::create_dir_all(&paths.directory).map_err(|source| GlobalError::Read {
        path: paths.directory.clone(),
        source,
    })?;
    let value = networkx_document(graph);
    let encoded = serde_json::to_string_pretty(&value).map_err(|source| GlobalError::Parse {
        path: paths.graph.clone(),
        source,
    })?;
    write_text_atomic(&paths.graph, &escape_non_ascii(&encoded)).map_err(|source| {
        GlobalError::Write {
            path: paths.graph.clone(),
            source,
        }
    })
}

fn networkx_document(graph: &GraphDocument) -> Value {
    let nodes = graph
        .nodes
        .iter()
        .map(|node| {
            let mut object = node.attributes.clone();
            object.insert("id".to_owned(), Value::String(node.id.clone()));
            Value::Object(object)
        })
        .collect::<Vec<_>>();
    let links = graph
        .links
        .iter()
        .map(|edge| {
            let mut object = edge.attributes.clone();
            object.insert("source".to_owned(), Value::String(edge.source.clone()));
            object.insert("target".to_owned(), Value::String(edge.target.clone()));
            Value::Object(object)
        })
        .collect::<Vec<_>>();
    let mut document = Map::new();
    document.insert("directed".to_owned(), Value::Bool(graph.directed));
    document.insert("multigraph".to_owned(), Value::Bool(graph.multigraph));
    document.insert("graph".to_owned(), Value::Object(graph.graph.clone()));
    document.insert("nodes".to_owned(), Value::Array(nodes));
    document.insert("links".to_owned(), Value::Array(links));
    Value::Object(document)
}

fn escape_non_ascii(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii() {
            output.push(character);
        } else {
            use std::fmt::Write as _;
            let point = u32::from(character);
            if point <= 0xffff {
                let _ = write!(output, "\\u{point:04x}");
            } else {
                let adjusted = point - 0x1_0000;
                let high = 0xd800 + (adjusted >> 10);
                let low = 0xdc00 + (adjusted & 0x3ff);
                let _ = write!(output, "\\u{high:04x}\\u{low:04x}");
            }
        }
    }
    output
}

fn load_manifest(paths: &GlobalPaths) -> ManifestLoad {
    if paths.manifest.exists() {
        let parsed = paths
            .manifest
            .metadata()
            .ok()
            .filter(|metadata| metadata.len() <= MAX_MANIFEST_BYTES)
            .and_then(|_| fs::read(&paths.manifest).ok())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        if let Some(value @ Value::Object(_)) = parsed {
            return ManifestLoad {
                value,
                warnings: Vec::new(),
            };
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let backup = paths
            .manifest
            .with_extension(format!("json.corrupt.{timestamp}"));
        let warning = match fs::rename(&paths.manifest, &backup) {
            Ok(()) => format!(
                "[graphify global] manifest at {} failed to parse; moved to {} and starting fresh. Restore from the backup if this was unexpected.",
                paths.manifest.display(),
                backup.display()
            ),
            Err(error) => format!(
                "[graphify global] manifest at {} failed to parse and could not be backed up ({error}). Starting fresh.",
                paths.manifest.display()
            ),
        };
        return ManifestLoad {
            value: default_manifest(),
            warnings: vec![warning],
        };
    }
    ManifestLoad {
        value: default_manifest(),
        warnings: Vec::new(),
    }
}

fn save_manifest(paths: &GlobalPaths, manifest: &Value) -> Result<(), GlobalError> {
    fs::create_dir_all(&paths.directory).map_err(|source| GlobalError::Read {
        path: paths.directory.clone(),
        source,
    })?;
    write_json_atomic(&paths.manifest, manifest, true).map_err(|source| GlobalError::Write {
        path: paths.manifest.clone(),
        source,
    })
}

fn default_manifest() -> Value {
    json!({"version":1,"repos":{}})
}

fn manifest_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = default_manifest();
    }
    value
        .as_object_mut()
        .unwrap_or_else(|| std::process::abort())
}

fn manifest_repos(value: &Value) -> Map<String, Value> {
    value
        .get("repos")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn manifest_repos_mut(value: &mut Value) -> &mut Map<String, Value> {
    let object = manifest_object(value);
    if !object.get("repos").is_some_and(Value::is_object) {
        object.insert("repos".to_owned(), Value::Object(Map::new()));
    }
    object
        .get_mut("repos")
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| std::process::abort())
}

fn file_hash(path: &Path) -> Result<String, GlobalError> {
    let mut file = fs::File::open(path).map_err(|source| GlobalError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| GlobalError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize())[..16].to_owned())
}

fn canonical_or_absolute(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir().map_or_else(|_| path.to_path_buf(), |current| current.join(path))
        }
    })
}

fn iso_timestamp(now: OffsetDateTime) -> String {
    let base = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let fraction = if now.microsecond() == 0 {
        String::new()
    } else {
        format!(".{:06}", now.microsecond())
    };
    format!("{base}{fraction}+00:00")
}

fn home_directory() -> Option<PathBuf> {
    #[cfg(not(windows))]
    {
        env::var_os("HOME").filter(non_empty).map(PathBuf::from)
    }
    #[cfg(windows)]
    env::var_os("USERPROFILE")
        .filter(non_empty)
        .or_else(|| {
            let drive = env::var_os("HOMEDRIVE").filter(non_empty)?;
            let path = env::var_os("HOMEPATH").filter(non_empty)?;
            let mut home = PathBuf::from(drive);
            home.push(path);
            Some(home.into_os_string())
        })
        .map(PathBuf::from)
}

fn non_empty(value: &OsString) -> bool {
    value != OsStr::new("")
}

fn python_repr(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_deduplicates_external_nodes_and_rewires_edges() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let paths = GlobalPaths {
            directory: directory.path().join("global"),
            graph: directory.path().join("global/global-graph.json"),
            manifest: directory.path().join("global/global-manifest.json"),
        };
        for (name, module) in [("a", "ModA"), ("b", "ModB")] {
            let graph = directory.path().join(format!("{name}.json"));
            fs::write(
                &graph,
                serde_json::to_vec(&json!({
                    "directed":false,"multigraph":false,"graph":{},
                    "nodes":[{"id":name,"label":module,"source_file":format!("src/{name}.py")},{"id":"requests","label":"requests"}],
                    "links":[{"source":name,"target":"requests","relation":"imports"}]
                }))?,
            )?;
            global_add(&paths, &graph, name, OffsetDateTime::UNIX_EPOCH)?;
        }
        let global = load_global_graph(&paths)?;
        assert!(global.nodes.iter().any(|node| node.id == "a::requests"));
        assert!(!global.nodes.iter().any(|node| node.id == "b::requests"));
        assert!(global.links.iter().any(|edge| {
            edge.source == "a::requests" && edge.target == "b::b"
                || edge.source == "b::b" && edge.target == "a::requests"
        }));
        Ok(())
    }
}
