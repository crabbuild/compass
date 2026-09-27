//! Source evidence for recorded members, independent of language layout.

use std::collections::{BTreeSet, VecDeque};
use std::path::Path;

use compass_model::code_graph::NodeKind;
use compass_model::{EdgeRecord, Graph, NodeRecord};
use serde_json::Value;

use crate::neighbors::bounded_json_size;
use crate::traversal::{
    ExplainedSource, ExplanationSourceError, explanation_source, node_source_anchor,
    resolve_explanation_source_node,
};

const MAX_MEMBERS: usize = 128;
const MAX_CONTAINERS: usize = 128;
const MAX_ADJACENCY: usize = 10_000;
const MAX_DEPTH: usize = 4;
const MAX_METADATA_BYTES: usize = 1_048_576;
const MAX_SOURCE_BYTES: u64 = 1_048_576;
const MAX_VERIFIED_SPAN_BYTES: u64 = 16_777_216;

#[derive(Debug)]
pub struct ExplainedMember<'a> {
    pub node: &'a NodeRecord,
    pub source: Result<ExplainedSource, ExplanationSourceError>,
}

#[derive(Debug)]
pub struct ExplainedMembers<'a> {
    pub root: &'a NodeRecord,
    /// Full recorded membership evidence, retaining parallel records.
    pub membership: Vec<&'a EdgeRecord>,
    pub members: Vec<ExplainedMember<'a>>,
    pub omitted_members: usize,
    pub source_bytes: u64,
    /// Charged span sizes, including failed attempts; not a disk-I/O measurement.
    pub verification_bytes_charged: u64,
    pub truncated: bool,
}

fn error(message: &str) -> ExplanationSourceError {
    ExplanationSourceError::Read(message.to_owned())
}

fn kind(node: &NodeRecord) -> Option<NodeKind> {
    if node.kind_name().len() > 64 {
        return None;
    }
    serde_json::from_value(Value::String(node.kind_name().to_owned())).ok()
}

/// Read callable members reached through recorded outgoing containment.
///
/// Nested type containers are traversed; callable bodies already include their
/// lexical contents, so their nested declarations are not read a second time.
/// Source order is deterministic and all excerpts share one retained-byte cap.
/// Bounds on traversal, metadata and verification work apply independently.
/// This reports graph membership, not proof of excessive responsibility.
pub fn explanation_member_sources<'a>(
    graph: &'a Graph,
    label: &str,
    root: &Path,
    max_source_bytes: u64,
) -> Result<ExplainedMembers<'a>, ExplanationSourceError> {
    if label.len() > 4096 || !(1..=MAX_SOURCE_BYTES).contains(&max_source_bytes) {
        return Err(error(
            "member source requires a selector up to 4096 bytes and a source budget from 1 to 1048576 bytes",
        ));
    }
    if !graph.is_directed() {
        return Err(error(
            "member source requires directed recorded containment",
        ));
    }
    let seed = resolve_explanation_source_node(graph, label)?;
    let mut metadata_bytes = bounded_json_size(graph.node(seed), MAX_METADATA_BYTES)
        .map_err(|_| error("member metadata exceeds its 1048576-byte limit"))?;
    let mut queue = VecDeque::from([(seed, 0_usize)]);
    let mut containers = BTreeSet::from([seed]);
    let mut members = BTreeSet::new();
    let mut membership = Vec::new();
    let mut adjacency = 0_usize;
    while let Some((owner, depth)) = queue.pop_front() {
        for index in graph.outgoing_edges(owner) {
            adjacency += 1;
            if adjacency > MAX_ADJACENCY {
                return Err(error("member discovery exceeds its 10000-adjacency limit"));
            }
            let edge = graph.edge(index);
            if !matches!(edge.relation(), "contains" | "method")
                || edge.source != graph.node(owner).id
            {
                continue;
            }
            // Bound normalization even for records later excluded by confidence.
            metadata_bytes +=
                bounded_json_size(edge, MAX_METADATA_BYTES.saturating_sub(metadata_bytes))
                    .map_err(|_| error("member metadata exceeds its 1048576-byte limit"))?;
            if edge.attributes.get("deferred").and_then(Value::as_bool) == Some(true)
                || !matches!(edge.string("confidence").as_str(), "" | "EXTRACTED")
            {
                continue;
            }
            let Some(target) = graph.node_index(&edge.target) else {
                continue;
            };
            let node = graph.node(target);
            let Some(kind) = kind(node) else {
                continue;
            };
            if !kind.is_callable() && !kind.is_type() {
                continue;
            }
            // Canonical order retains every parallel record and unknown attribute.
            let mut canonical = serde_json::to_value(edge).map_err(|e| error(&e.to_string()))?;
            canonical.sort_all_objects();
            let key = serde_json::to_vec(&canonical).map_err(|e| error(&e.to_string()))?;
            membership.push((key, edge));
            if target == seed || containers.contains(&target) || members.contains(&target) {
                continue;
            }
            if depth >= MAX_DEPTH {
                return Err(error(
                    "member discovery exceeds its containment depth limit (4)",
                ));
            }
            metadata_bytes +=
                bounded_json_size(node, MAX_METADATA_BYTES.saturating_sub(metadata_bytes))
                    .map_err(|_| error("member metadata exceeds its 1048576-byte limit"))?;
            if kind.is_callable() {
                members.insert(target);
                if members.len() > MAX_MEMBERS {
                    return Err(error("member discovery exceeds its 128-callable limit"));
                }
            } else {
                containers.insert(target);
                if containers.len() > MAX_CONTAINERS {
                    return Err(error("member discovery exceeds its 128-container limit"));
                }
                queue.push_back((target, depth + 1));
            }
        }
    }
    membership.sort_by(|a, b| a.0.cmp(&b.0));
    let mut ordered = members
        .into_iter()
        .map(|index| {
            let node = graph.node(index);
            let anchor = node_source_anchor(node);
            let key = anchor
                .as_ref()
                .map(|a| (a.file.clone(), a.start_byte, a.end_byte));
            (key, node)
        })
        .collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let total = ordered.len();
    let mut report = ExplainedMembers {
        root: graph.node(seed),
        membership: membership.into_iter().map(|(_, edge)| edge).collect(),
        members: Vec::new(),
        omitted_members: 0,
        source_bytes: 0,
        verification_bytes_charged: 0,
        truncated: false,
    };
    for (_, node) in ordered {
        let remaining = max_source_bytes.saturating_sub(report.source_bytes);
        if remaining == 0 {
            report.truncated = true;
            break;
        }
        let Some(anchor) = node_source_anchor(node) else {
            report.members.push(ExplainedMember {
                node,
                source: Err(ExplanationSourceError::Unsourced {
                    label: node.id.clone(),
                }),
            });
            continue;
        };
        let Some(span_bytes) = anchor.end_byte.checked_sub(anchor.start_byte) else {
            report.members.push(ExplainedMember {
                node,
                source: Err(error("recorded member source span is inverted")),
            });
            continue;
        };
        if span_bytes > MAX_VERIFIED_SPAN_BYTES.saturating_sub(report.verification_bytes_charged) {
            report.truncated = true;
            break;
        }
        // Charge attempted complete spans too: failed digest checks consume work.
        report.verification_bytes_charged += span_bytes;
        let mut source = explanation_source(graph, &node.id, root, remaining);
        if let Ok(excerpt) = &mut source {
            // Lossy UTF-8 decoding can expand a clipped sequence. The retained
            // string itself must still fit the shared byte cap.
            let cap = usize::try_from(remaining).unwrap_or(usize::MAX);
            if excerpt.source.len() > cap {
                let mut end = cap;
                while !excerpt.source.is_char_boundary(end) {
                    end -= 1;
                }
                excerpt.source.truncate(end);
                excerpt.truncated = true;
            }
            report.source_bytes += excerpt.source.len() as u64;
            report.truncated |= excerpt.truncated;
        }
        report.members.push(ExplainedMember { node, source });
    }
    report.omitted_members = total.saturating_sub(report.members.len());
    Ok(report)
}
