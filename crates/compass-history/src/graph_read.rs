use std::collections::{BTreeMap, HashSet};

use compass_model::{EdgeRecord, NodeRecord};
use prolly::decode_segments;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    HistoryError, MAX_AUTHORITATIVE_BYTES, MAX_KEY_BYTES, MAX_RECORD_VALUE_BYTES,
    MAX_RECORDS_PER_TREE, RealizationReader,
};

/// Receives the graph-only projection of a sealed realization.
pub trait GraphRecordSink {
    fn document_metadata(
        &mut self,
        directed: bool,
        multigraph: bool,
        graph: Map<String, Value>,
        extras: BTreeMap<String, Value>,
    ) -> Result<(), HistoryError>;

    fn node_attribute(
        &mut self,
        node_id: String,
        field: String,
        value: Value,
    ) -> Result<(), HistoryError>;

    fn labels(&mut self, labels: Value) -> Result<(), HistoryError>;

    fn node(&mut self, node: NodeRecord) -> Result<(), HistoryError>;

    fn edge(&mut self, edge: EdgeRecord) -> Result<(), HistoryError>;
}

impl RealizationReader<'_> {
    /// Scan only analysis, node, and edge roots for a historical graph view.
    pub fn scan_graph(&self, sink: &mut dyn GraphRecordSink) -> Result<(), HistoryError> {
        let mut total_bytes = 0_u64;
        let metadata = self.published.version.metadata_root.to_tree();
        let document_key = crate::artifacts::metadata_key(&[b"document"]);
        let document_bytes = self.prolly.get(&metadata, &document_key)?.ok_or_else(|| {
            HistoryError::InvalidArtifacts("missing document metadata".to_owned())
        })?;
        account(&document_key, &document_bytes, &mut total_bytes)?;
        let document: GraphDocumentMetadata =
            crate::artifacts::decode_typed(&document_bytes, "compass.metadata.document")?;
        sink.document_metadata(
            document.directed,
            document.multigraph,
            document.graph,
            document.extras,
        )?;

        let mut pending_analysis = HashSet::<String>::new();
        let analysis = self.published.version.analysis_root.to_tree();
        let mut analysis_count = 0_u64;
        for entry in self.store.prolly.range(&analysis, &[], None)? {
            let (key, bytes) = entry?;
            account(&key, &bytes, &mut total_bytes)?;
            analysis_count = analysis_count.saturating_add(1);
            let segments = decode_segments(&key)
                .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
            match segments.as_slice() {
                [_, _, kind, node, field] if kind == b"node" => {
                    let node = text(node, "node")?;
                    let field = text(field, "analysis field")?;
                    let value = crate::artifacts::decode_typed(&bytes, "compass.analysis.node")?;
                    pending_analysis.insert(node.clone());
                    sink.node_attribute(node, field, value)?;
                }
                [_, _, kind, path] if kind == b"sidecar" => {
                    let path = text(path, "analysis sidecar")?;
                    let value = crate::artifacts::decode_typed(&bytes, "compass.analysis.sidecar")?;
                    match path.as_str() {
                        ".compass_labels.json" => sink.labels(value)?,
                        ".compass_analysis.json" => {}
                        _ => {
                            return Err(HistoryError::InvalidArtifacts(format!(
                                "unknown analysis sidecar {path}"
                            )));
                        }
                    }
                }
                _ => {
                    return Err(HistoryError::InvalidArtifacts(
                        "invalid analysis key".to_owned(),
                    ));
                }
            }
        }
        require_count(
            "analysis",
            self.published.version.analysis_count,
            analysis_count,
        )?;

        let nodes = self.published.version.nodes_root.to_tree();
        let mut node_ids = HashSet::new();
        let mut node_count = 0_u64;
        for entry in self.store.prolly.range(&nodes, &[], None)? {
            let (key, bytes) = entry?;
            account(&key, &bytes, &mut total_bytes)?;
            node_count = node_count.saturating_add(1);
            let segments = decode_segments(&key)
                .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
            let [schema, kind, id] = segments.as_slice() else {
                return Err(HistoryError::InvalidArtifacts(
                    "invalid node key".to_owned(),
                ));
            };
            if schema != &[1] || kind != &[1] {
                return Err(HistoryError::InvalidArtifacts(
                    "invalid node key".to_owned(),
                ));
            }
            let id = text(id, "node ID")?;
            let node = crate::artifacts::decode_compatible_node(&bytes)?;
            if node.id != id || !node_ids.insert(id.clone()) {
                return Err(HistoryError::InvalidArtifacts(format!(
                    "node key does not match unique node ID {id}"
                )));
            }
            pending_analysis.remove(&id);
            sink.node(node)?;
        }
        require_count("nodes", self.published.version.node_count, node_count)?;
        if let Some(node) = pending_analysis.iter().next() {
            return Err(HistoryError::InvalidArtifacts(format!(
                "analysis references missing node {node}"
            )));
        }

        let edges = self.published.version.edges_root.to_tree();
        let mut edge_count = 0_u64;
        for entry in self.store.prolly.range(&edges, &[], None)? {
            let (key, bytes) = entry?;
            account(&key, &bytes, &mut total_bytes)?;
            edge_count = edge_count.saturating_add(1);
            let segments = decode_segments(&key)
                .map_err(|error| HistoryError::InvalidArtifacts(error.to_string()))?;
            if segments.len() < 5 || segments[0] != [1] || segments[1] != [2] {
                return Err(HistoryError::InvalidArtifacts(
                    "invalid edge key".to_owned(),
                ));
            }
            let edge = crate::artifacts::decode_compatible_edge(&bytes)?;
            if !node_ids.contains(&edge.source) || !node_ids.contains(&edge.target) {
                return Err(HistoryError::InvalidArtifacts(format!(
                    "edge references a missing endpoint {} -> {}",
                    edge.source, edge.target
                )));
            }
            sink.edge(edge)?;
        }
        require_count("edges", self.published.version.edge_count, edge_count)
    }
}

#[derive(Deserialize)]
struct GraphDocumentMetadata {
    directed: bool,
    multigraph: bool,
    graph: Map<String, Value>,
    extras: BTreeMap<String, Value>,
}

fn account(key: &[u8], value: &[u8], total: &mut u64) -> Result<(), HistoryError> {
    if key.len() > MAX_KEY_BYTES || value.len() > MAX_RECORD_VALUE_BYTES {
        return Err(HistoryError::InvalidArtifacts(
            "historical graph record exceeds byte limit".to_owned(),
        ));
    }
    *total = total
        .saturating_add(key.len() as u64)
        .saturating_add(value.len() as u64);
    if *total > MAX_AUTHORITATIVE_BYTES {
        return Err(HistoryError::InvalidArtifacts(
            "historical graph projection exceeds byte limit".to_owned(),
        ));
    }
    Ok(())
}

fn require_count(kind: &str, expected: u64, actual: u64) -> Result<(), HistoryError> {
    if actual > MAX_RECORDS_PER_TREE || expected != actual {
        return Err(HistoryError::InvalidArtifacts(format!(
            "{kind} record count mismatch: expected {expected}, read {actual}"
        )));
    }
    Ok(())
}

fn text(bytes: &[u8], kind: &str) -> Result<String, HistoryError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|error| HistoryError::InvalidArtifacts(format!("non-UTF-8 {kind}: {error}")))
}
