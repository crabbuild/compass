use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::{EdgeRecord, GraphDocument, GraphError, NodeRecord};

pub type NodeIndex = usize;
pub type EdgeIndex = usize;

/// Query-oriented directed graph preserving document insertion order.
#[derive(Clone, Debug)]
pub struct Graph {
    directed: bool,
    multigraph: bool,
    nodes: Arc<Vec<NodeRecord>>,
    edges: Arc<Vec<EdgeRecord>>,
    ids: Arc<HashMap<String, NodeIndex>>,
    outgoing: Vec<Vec<EdgeIndex>>,
    incoming: Vec<Vec<EdgeIndex>>,
}

impl Graph {
    /// Load a graph file with the same default 512 MiB safety cap as Python.
    ///
    /// # Errors
    ///
    /// Returns a [`GraphError`] when the path, size, JSON, or graph structure is invalid.
    pub fn load(path: &Path) -> Result<Self, GraphError> {
        Self::load_with_direction(path, false)
    }

    /// Load a graph while recovering persisted edge direction even when an old
    /// graph declares itself undirected.
    ///
    /// # Errors
    ///
    /// Returns a [`GraphError`] when the path, size, JSON, or graph structure is invalid.
    pub fn load_directed(path: &Path) -> Result<Self, GraphError> {
        Self::load_with_direction(path, true)
    }

    fn load_with_direction(path: &Path, force_directed: bool) -> Result<Self, GraphError> {
        let mut document = GraphDocument::load(path)?;
        if force_directed {
            document.directed = true;
        }
        Self::from_document(document)
    }

    /// Build query indexes from an already decoded node-link document.
    ///
    /// # Errors
    ///
    /// Returns a [`GraphError`] if an edge endpoint cannot be indexed.
    pub fn from_document(document: GraphDocument) -> Result<Self, GraphError> {
        let GraphDocument {
            directed,
            multigraph,
            nodes: source_nodes,
            links: source_edges,
            ..
        } = document;
        let mut nodes: Vec<NodeRecord> = Vec::new();
        let mut ids: HashMap<String, NodeIndex> = HashMap::new();
        for node in source_nodes {
            if let Some(&index) = ids.get(&node.id) {
                nodes[index].attributes.extend(node.attributes);
            } else {
                ids.insert(node.id.clone(), nodes.len());
                nodes.push(node);
            }
        }

        for edge in &source_edges {
            ensure_endpoint(&edge.source, &mut nodes, &mut ids);
            ensure_endpoint(&edge.target, &mut nodes, &mut ids);
        }

        let mut edges: Vec<EdgeRecord> = Vec::new();
        if multigraph {
            edges = source_edges;
        } else {
            let mut positions: HashMap<(String, String), usize> = HashMap::new();
            for edge in source_edges {
                let key = (edge.source.clone(), edge.target.clone());
                if let Some(&position) = positions.get(&key) {
                    edges[position].attributes.extend(edge.attributes);
                } else {
                    positions.insert(key, edges.len());
                    edges.push(edge);
                }
            }
        }

        Self::from_parts(
            directed,
            multigraph,
            Arc::new(nodes),
            Arc::new(edges),
            Arc::new(ids),
        )
    }

    fn from_parts(
        directed: bool,
        multigraph: bool,
        nodes: Arc<Vec<NodeRecord>>,
        edges: Arc<Vec<EdgeRecord>>,
        ids: Arc<HashMap<String, NodeIndex>>,
    ) -> Result<Self, GraphError> {
        let mut outgoing = vec![Vec::new(); nodes.len()];
        let mut incoming = vec![Vec::new(); nodes.len()];
        for (edge_index, edge) in edges.iter().enumerate() {
            let Some(&source) = ids.get(&edge.source) else {
                return Err(GraphError::InvalidEdgeEndpoint);
            };
            let Some(&target) = ids.get(&edge.target) else {
                return Err(GraphError::InvalidEdgeEndpoint);
            };
            outgoing[source].push(edge_index);
            incoming[target].push(edge_index);
            if !directed {
                outgoing[target].push(edge_index);
                incoming[source].push(edge_index);
            }
        }

        Ok(Self {
            directed,
            multigraph,
            nodes,
            edges,
            ids,
            outgoing,
            incoming,
        })
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn nodes(&self) -> impl Iterator<Item = (NodeIndex, &NodeRecord)> {
        self.nodes.iter().enumerate()
    }

    #[must_use]
    pub fn node(&self, index: NodeIndex) -> &NodeRecord {
        &self.nodes[index]
    }

    #[must_use]
    pub fn node_index(&self, id: &str) -> Option<NodeIndex> {
        self.ids.get(id).copied()
    }

    #[must_use]
    pub fn edge(&self, index: EdgeIndex) -> &EdgeRecord {
        &self.edges[index]
    }

    pub fn outgoing_edges(&self, node: NodeIndex) -> impl Iterator<Item = EdgeIndex> + '_ {
        self.outgoing[node].iter().copied()
    }

    pub fn incoming_edges(&self, node: NodeIndex) -> impl Iterator<Item = EdgeIndex> + '_ {
        self.incoming[node].iter().copied()
    }

    pub fn successors(&self, node: NodeIndex) -> impl Iterator<Item = NodeIndex> + '_ {
        self.outgoing_edges(node).filter_map(move |edge| {
            let record = &self.edges[edge];
            if self.directed || self.node_index(&record.source) == Some(node) {
                self.node_index(&record.target)
            } else {
                self.node_index(&record.source)
            }
        })
    }

    pub fn predecessors(&self, node: NodeIndex) -> impl Iterator<Item = NodeIndex> + '_ {
        self.incoming_edges(node).filter_map(move |edge| {
            let record = &self.edges[edge];
            if self.directed || self.node_index(&record.target) == Some(node) {
                self.node_index(&record.source)
            } else {
                self.node_index(&record.target)
            }
        })
    }

    #[must_use]
    pub fn degree(&self, node: NodeIndex) -> usize {
        if self.directed {
            self.outgoing[node].len() + self.incoming[node].len()
        } else {
            self.outgoing[node].len()
        }
    }

    #[must_use]
    pub fn edge_between(&self, source: NodeIndex, target: NodeIndex) -> Option<EdgeIndex> {
        self.outgoing_edges(source).find(|&edge| {
            let record = &self.edges[edge];
            (self.node_index(&record.source) == Some(source)
                && self.node_index(&record.target) == Some(target))
                || (!self.directed
                    && self.node_index(&record.source) == Some(target)
                    && self.node_index(&record.target) == Some(source))
        })
    }

    #[must_use]
    pub fn with_edge_contexts(&self, contexts: &[String]) -> Self {
        if contexts.is_empty() {
            return self.clone();
        }
        let edges = self
            .edges
            .iter()
            .filter(|edge| contexts.iter().any(|item| item == &edge.string("context")))
            .cloned()
            .collect::<Vec<_>>();
        // The nodes and every edge endpoint were sourced from a valid graph.
        match Self::from_parts(
            self.directed,
            self.multigraph,
            Arc::clone(&self.nodes),
            Arc::new(edges),
            Arc::clone(&self.ids),
        ) {
            Ok(graph) => graph,
            Err(_) => self.clone(),
        }
    }

    #[must_use]
    pub fn first_edge_attributes(
        &self,
        source: NodeIndex,
        target: NodeIndex,
    ) -> Option<&Map<String, Value>> {
        self.edge_between(source, target)
            .map(|edge| &self.edges[edge].attributes)
    }
}

fn ensure_endpoint(id: &str, nodes: &mut Vec<NodeRecord>, ids: &mut HashMap<String, NodeIndex>) {
    if ids.contains_key(id) {
        return;
    }
    ids.insert(id.to_owned(), nodes.len());
    nodes.push(NodeRecord {
        id: id.to_owned(),
        attributes: Map::new(),
    });
}

pub(crate) fn graph_size_cap() -> u64 {
    const DEFAULT: u64 = 512 * 1024 * 1024;
    let Ok(raw) = std::env::var("GRAPHIFY_MAX_GRAPH_BYTES") else {
        return DEFAULT;
    };
    let upper = raw.trim().to_uppercase();
    let (number, multiplier) = if let Some(number) = upper.strip_suffix("GB") {
        (number.trim(), 1024_u64 * 1024 * 1024)
    } else if let Some(number) = upper.strip_suffix("MB") {
        (number.trim(), 1024_u64 * 1024)
    } else {
        (upper.as_str(), 1)
    };
    number
        .replace('_', "")
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .and_then(|value| value.checked_mul(multiplier))
        .unwrap_or(DEFAULT)
}

pub(crate) fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_legacy_edges_key_and_forces_direction() {
        let raw = r#"{
            "directed": false,
            "multigraph": false,
            "graph": {},
            "nodes": [{"id":"a","label":"A"},{"id":"b","label":"B"}],
            "edges": [{"source":"a","target":"b","relation":"calls"}]
        }"#;
        let document: GraphDocument = serde_json::from_str(raw).unwrap_or_else(|_error| {
            std::process::abort();
        });
        let graph = Graph::from_document(document).unwrap_or_else(|_| std::process::abort());
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
        assert_eq!(graph.successors(0).collect::<Vec<_>>(), vec![1]);
        assert_eq!(graph.predecessors(1).collect::<Vec<_>>(), vec![0]);
        assert_eq!(graph.successors(1).collect::<Vec<_>>(), vec![0]);
    }
}
