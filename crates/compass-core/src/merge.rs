use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use compass_files::write_json_atomic;
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use serde_json::{Map, Value};

use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeResult {
    pub graphs: usize,
    pub nodes: usize,
    pub edges: usize,
    pub output_path: PathBuf,
    pub tags: Vec<String>,
    pub naive_tags_collided: bool,
}

pub fn merge_graphs(paths: &[PathBuf], output: &Path) -> Result<MergeResult, CoreError> {
    let tags = distinct_repo_tags(paths);
    let naive = paths
        .iter()
        .map(|path| repo_dir(path).name)
        .collect::<Vec<_>>();
    let naive_tags_collided = unique_count(&naive) != naive.len();
    let mut nodes = Vec::<NodeRecord>::new();
    let mut node_positions = HashMap::<String, usize>::new();
    let mut edges = Vec::<EdgeRecord>::new();
    let mut edge_positions = HashMap::<(String, String), usize>::new();
    let mut graph = Map::new();
    for (path, tag) in paths.iter().zip(&tags) {
        let document = GraphDocument::load(path)?;
        let was_multigraph = document.multigraph;
        graph.extend(document.graph);
        for mut node in document.nodes {
            let local_id = node.id;
            node.id = format!("{tag}::{local_id}");
            node.attributes
                .insert("repo".to_owned(), Value::String(tag.clone()));
            node.attributes
                .entry("local_id".to_owned())
                .or_insert(Value::String(local_id));
            if let Some(position) = node_positions.get(&node.id).copied() {
                nodes[position].attributes.extend(node.attributes);
            } else {
                node_positions.insert(node.id.clone(), nodes.len());
                nodes.push(node);
            }
        }
        for mut edge in document.links {
            if was_multigraph {
                edge.attributes.remove("key");
            }
            edge.source = format!("{tag}::{}", edge.source);
            edge.target = format!("{tag}::{}", edge.target);
            let source_position = node_positions
                .get(&edge.source)
                .copied()
                .unwrap_or(usize::MAX);
            let target_position = node_positions
                .get(&edge.target)
                .copied()
                .unwrap_or(usize::MAX);
            if target_position < source_position {
                std::mem::swap(&mut edge.source, &mut edge.target);
            }
            let key = (edge.source.clone(), edge.target.clone());
            if let Some(position) = edge_positions.get(&key).copied() {
                edges[position].attributes.extend(edge.attributes);
            } else {
                edge_positions.insert(key, edges.len());
                edges.push(edge);
            }
        }
    }
    let document = GraphDocument {
        directed: false,
        multigraph: false,
        graph,
        nodes,
        links: edges,
        extras: BTreeMap::new(),
        used_legacy_edges_key: false,
    };
    write_json_atomic(output, &document, true)?;
    Ok(MergeResult {
        graphs: paths.len(),
        nodes: document.nodes.len(),
        edges: document.links.len(),
        output_path: output.to_path_buf(),
        tags,
        naive_tags_collided,
    })
}

struct RepoDir {
    name: String,
    parent_name: String,
}

fn repo_dir(path: &Path) -> RepoDir {
    let directory = path
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(""));
    RepoDir {
        name: directory
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("repo")
            .to_owned(),
        parent_name: directory
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned(),
    }
}

fn distinct_repo_tags(paths: &[PathBuf]) -> Vec<String> {
    let directories = paths.iter().map(|path| repo_dir(path)).collect::<Vec<_>>();
    let mut tags = directories
        .iter()
        .map(|directory| directory.name.clone())
        .collect::<Vec<_>>();
    if unique_count(&tags) != tags.len() {
        tags = directories
            .iter()
            .map(|directory| {
                if directory.parent_name.is_empty() {
                    directory.name.clone()
                } else {
                    format!("{}_{}", directory.parent_name, directory.name)
                }
            })
            .collect();
    }
    let mut seen = HashMap::<String, usize>::new();
    tags.into_iter()
        .map(|tag| {
            let count = seen.entry(tag.clone()).or_default();
            *count += 1;
            if *count == 1 {
                tag
            } else {
                format!("{tag}-{count}")
            }
        })
        .collect()
}

fn unique_count(values: &[String]) -> usize {
    values
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
}
