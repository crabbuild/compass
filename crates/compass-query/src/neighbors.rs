//! Bounded, lossless incident-record projection for direct navigation.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use compass_model::{EdgeRecord, Graph, NodeIndex, NodeRecord};
use serde::Serialize;
use thiserror::Error;

pub const MAX_NEIGHBOR_ADJACENCY_ENTRIES: usize = 1_000_000;
pub const MAX_NEIGHBOR_RECORDS: usize = 10_000;
pub const MAX_NEIGHBOR_RESPONSE_BYTES: usize = 1_048_576;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NeighborDirection {
    Outgoing,
    Incoming,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighborGroup<'a> {
    pub direction: NeighborDirection,
    pub node: &'a NodeRecord,
    /// Complete records, including parallel occurrences and unknown attributes.
    pub edges: Vec<&'a EdgeRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighborReport<'a> {
    pub schema: &'static str,
    /// Directions describe persisted endpoints, even on undirected artifacts.
    pub direction_basis: &'static str,
    pub graph_directed: bool,
    pub seed: &'a NodeRecord,
    pub relation_filter: String,
    pub neighbors: Vec<NeighborGroup<'a>>,
    pub truncated: bool,
}

#[derive(Debug, Error)]
pub enum NeighborError {
    #[error("neighbor seed is absent from the selected graph")]
    MissingSeed,
    #[error("neighbor relation filter exceeds 4096 bytes")]
    FilterLimit,
    #[error("neighbor lookup exceeds its adjacency-entry limit ({0})")]
    AdjacencyLimit(usize),
    #[error("neighbor lookup exceeds its matching-record limit ({0})")]
    RecordLimit(usize),
    #[error("neighbor result exceeds its byte limit ({0}); narrow relation_filter")]
    ResponseLimit(usize),
    #[error("cannot encode neighbor evidence: {0}")]
    Encoding(#[from] serde_json::Error),
}

struct ByteCounter {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes) {
            self.exceeded = true;
            return Err(std::io::Error::other("neighbor byte limit"));
        }
        self.bytes += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ByteCounter {
    fn count(&mut self, value: &impl Serialize) -> Result<(), NeighborError> {
        let result = serde_json::to_writer(&mut *self, value);
        if self.exceeded {
            return Err(NeighborError::ResponseLimit(self.limit));
        }
        result?;
        Ok(())
    }
}

/// Project exact destinations and every matching incident record from one graph.
///
/// No partial result is returned on exhaustion. Nodes sort by direction then ID;
/// records sort by canonical JSON, preserving identical parallel records. A self
/// loop appears in both directions, but counts once against the record budget.
/// Supply a full-record graph, not a compact traversal projection.
pub fn direct_neighbors<'a>(
    graph: &'a Graph,
    seed: NodeIndex,
    relation_filter: &str,
) -> Result<NeighborReport<'a>, NeighborError> {
    bounded_neighbors(
        graph,
        seed,
        relation_filter,
        MAX_NEIGHBOR_ADJACENCY_ENTRIES,
        MAX_NEIGHBOR_RECORDS,
        MAX_NEIGHBOR_RESPONSE_BYTES,
    )
}

fn bounded_neighbors<'a>(
    graph: &'a Graph,
    seed: NodeIndex,
    relation_filter: &str,
    max_adjacency: usize,
    max_records: usize,
    max_bytes: usize,
) -> Result<NeighborReport<'a>, NeighborError> {
    if seed >= graph.node_count() {
        return Err(NeighborError::MissingSeed);
    }
    if relation_filter.len() > 4096 {
        return Err(NeighborError::FilterLimit);
    }
    let filter = relation_filter.to_lowercase();
    let seed_node = graph.node(seed);
    let mut bytes = ByteCounter {
        bytes: 0,
        limit: max_bytes,
        exceeded: false,
    };
    bytes.count(seed_node)?;
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    let mut groups = BTreeMap::<(NeighborDirection, &str), Vec<(Vec<u8>, &EdgeRecord)>>::new();
    for (examined, index) in graph
        .outgoing_edges(seed)
        .chain(graph.incoming_edges(seed))
        .enumerate()
    {
        if examined >= max_adjacency {
            return Err(NeighborError::AdjacencyLimit(max_adjacency));
        }
        if !seen.insert(index) {
            continue;
        }
        let edge = graph.edge(index);
        // Bound encoding and normalization even for a nonmatching record.
        let mut edge_bytes = ByteCounter {
            bytes: 0,
            limit: max_bytes,
            exceeded: false,
        };
        edge_bytes.count(edge)?;
        if !filter.is_empty() && !edge.string("relation").to_lowercase().contains(&filter) {
            continue;
        }
        if matched >= max_records {
            return Err(NeighborError::RecordLimit(max_records));
        }
        matched += 1;
        let mut canonical = serde_json::to_value(edge)?;
        canonical.sort_all_objects();
        let key = serde_json::to_vec(&canonical)?;
        for (direction, applies, other) in [
            (
                NeighborDirection::Outgoing,
                edge.source == seed_node.id,
                edge.target.as_str(),
            ),
            (
                NeighborDirection::Incoming,
                edge.target == seed_node.id,
                edge.source.as_str(),
            ),
        ] {
            if !applies {
                continue;
            }
            let node = graph.node_index(other).ok_or(NeighborError::MissingSeed)?;
            if !groups.contains_key(&(direction, other)) {
                bytes.count(graph.node(node))?;
            }
            bytes.count(edge)?;
            groups
                .entry((direction, other))
                .or_default()
                .push((key.clone(), edge));
        }
    }
    let neighbors = groups
        .into_iter()
        .map(|((direction, id), mut edges)| {
            edges.sort_by(|left, right| left.0.cmp(&right.0));
            let index = graph.node_index(id).ok_or(NeighborError::MissingSeed)?;
            Ok(NeighborGroup {
                direction,
                node: graph.node(index),
                edges: edges.into_iter().map(|(_, edge)| edge).collect(),
            })
        })
        .collect::<Result<Vec<_>, NeighborError>>()?;
    let report = NeighborReport {
        schema: "compass.query.neighbors/1",
        direction_basis: "stored-endpoints",
        graph_directed: graph.is_directed(),
        seed: seed_node,
        relation_filter: filter,
        neighbors,
        truncated: false,
    };
    // Include envelope keys and repeated direction/node fields in the final cap.
    ByteCounter {
        bytes: 0,
        limit: max_bytes,
        exceeded: false,
    }
    .count(&report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::GraphDocument;
    use serde_json::{Value, json};

    fn graph(directed: bool, reverse: bool) -> Result<Graph, Box<dyn std::error::Error>> {
        let mut nodes = vec![
            json!({"id":"seed","label":"run"}),
            json!({"id":"b","label":"close","source":{"file":"b.rs","startLine":5}}),
            json!({"id":"c","label":"close","source_file":"c.rs","source_location":"L9"}),
        ];
        let mut links = vec![
            json!({"id":"edge-b","source":"seed","target":"b","relation":"calls","relationshipSite":{"file":"a.rs","startLine":1},"evidence":[{"origin":"ast","custom":{"x":1}}]}),
            json!({"id":"edge-b2","source":"seed","target":"b","relation":"calls","relationshipSite":{"file":"a.rs","startLine":2}}),
            json!({"source":"seed","target":"b","relation":"contains"}),
            json!({"id":"edge-c","source":"c","target":"seed","relation":"calls"}),
            json!({"id":"loop","source":"seed","target":"seed","relation":"calls"}),
        ];
        links.push(links[0].clone());
        if reverse {
            nodes.reverse();
            links.reverse();
        }
        let document: GraphDocument = serde_json::from_value(
            json!({"directed":directed,"multigraph":true,"nodes":nodes,"links":links}),
        )?;
        Ok(Graph::from_traversal_document(document)?)
    }

    #[test]
    fn neighbors_preserve_direction_parallel_records_self_loops_and_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        for directed in [false, true] {
            let graph = graph(directed, false)?;
            let report =
                direct_neighbors(&graph, graph.node_index("seed").ok_or("seed")?, "CALLS")?;
            assert_eq!(report.neighbors.len(), 4);
            assert_eq!(report.graph_directed, directed);
            assert_eq!(report.direction_basis, "stored-endpoints");
            let group = &report.neighbors[0];
            assert_eq!(group.node.id, "b");
            assert_eq!(group.direction, NeighborDirection::Outgoing);
            assert_eq!(group.edges.len(), 3);
            assert_eq!(group.node.attributes["source"]["startLine"], 5);
            assert_eq!(group.edges[0].attributes["evidence"][0]["custom"]["x"], 1);
            assert_eq!(group.edges[0], group.edges[1]);
            assert_eq!(report.neighbors[1].node.id, "seed");
            assert_eq!(report.neighbors[2].node.id, "c");
            assert_eq!(report.neighbors[2].direction, NeighborDirection::Incoming);
            assert_eq!(report.neighbors[3].node.id, "seed");
        }
        Ok(())
    }

    #[test]
    fn neighbors_are_stable_under_graph_order_and_preserve_same_label_ids()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut outputs = Vec::<Value>::new();
        for reverse in [false, true] {
            let graph = graph(true, reverse)?;
            outputs.push(serde_json::to_value(direct_neighbors(
                &graph,
                graph.node_index("seed").ok_or("seed")?,
                "",
            )?)?);
        }
        assert_eq!(outputs[0], outputs[1]);
        assert_eq!(
            outputs[0]["neighbors"][0]["edges"]
                .as_array()
                .ok_or("edges")?
                .len(),
            4
        );
        Ok(())
    }

    #[test]
    fn neighbors_fail_explicitly_on_every_bound_and_distinguish_empty_result()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = graph(true, false)?;
        let seed = graph.node_index("seed").ok_or("seed")?;
        assert!(matches!(
            bounded_neighbors(&graph, seed, "", 0, 100, 100000),
            Err(NeighborError::AdjacencyLimit(0))
        ));
        assert!(matches!(
            bounded_neighbors(&graph, seed, "absent", 0, 100, 100000),
            Err(NeighborError::AdjacencyLimit(0))
        ));
        assert!(matches!(
            bounded_neighbors(&graph, seed, "calls", 100, 1, 100000),
            Err(NeighborError::RecordLimit(1))
        ));
        assert!(matches!(
            bounded_neighbors(&graph, seed, "calls", 100, 100, 1),
            Err(NeighborError::ResponseLimit(1))
        ));
        assert!(matches!(
            direct_neighbors(&graph, seed, &"x".repeat(4097)),
            Err(NeighborError::FilterLimit)
        ));
        assert!(matches!(
            direct_neighbors(&graph, graph.node_count(), ""),
            Err(NeighborError::MissingSeed)
        ));
        let empty = direct_neighbors(&graph, seed, "absent")?;
        assert!(empty.neighbors.is_empty());
        assert!(!empty.truncated);
        Ok(())
    }
}
