use compass_model::GraphDocument;
use prolly::{Prolly, Tree, VersionedValue};
use prolly_store_sqlite::SqliteStore;
use serde_json::Value;
use std::sync::Arc;

use crate::{
    GraphArtifacts, GraphVersion, HistoryError, PartitionedGraph, RealizationId,
    canonical_json_bytes, node_key,
};

type Records = Vec<(Vec<u8>, Vec<u8>)>;

pub const MAX_AUTHORITATIVE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_KEY_BYTES: usize = 1024 * 1024;
pub const MAX_RECORD_VALUE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_RECORDS_PER_TREE: u64 = 10_000_000;
pub const MAX_JSON_DEPTH: usize = 128;
pub const MAX_JOB_BYTES: usize = 1024 * 1024;
pub const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;

pub(crate) const fn exceeds_limit(actual: u64, limit: u64) -> bool {
    actual > limit
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ValidationReport {
    pub nodes: u64,
    pub edges: u64,
    pub hyperedges: u64,
    pub analysis_records: u64,
    pub metadata_records: u64,
    pub program_fact_records: u64,
    pub program_summary_records: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationProblem {
    RealizationDigest,
    MissingRoot(&'static str),
    Count {
        kind: &'static str,
        expected: u64,
        actual: u64,
    },
    KeyMismatch {
        kind: &'static str,
        key: Vec<u8>,
    },
    MissingEdgeEndpoint {
        edge: Vec<u8>,
        endpoint: String,
    },
    MissingHyperedgeMember {
        hyperedge: Vec<u8>,
        member: String,
    },
    MissingAnalysisNode(String),
    MissingOrderRecord {
        kind: &'static str,
        key: Vec<u8>,
    },
    InvalidMultigraphDiscriminator(Vec<u8>),
    DuplicateExplicitHyperedgeId(String),
    ArtifactRegistry(String),
    IncompleteSemanticState,
    ResourceLimit {
        kind: &'static str,
        limit: u64,
        actual: u64,
    },
}

pub(crate) struct RealizationTrees<'a> {
    pub nodes: &'a Tree,
    pub edges: &'a Tree,
    pub hyperedges: &'a Tree,
    pub analysis: &'a Tree,
    pub metadata: &'a Tree,
    pub program_facts: &'a Tree,
    pub program_summaries: &'a Tree,
}

pub(crate) fn validate_trees(
    manager: &Prolly<Arc<SqliteStore>>,
    id: &RealizationId,
    version: &GraphVersion,
    trees: RealizationTrees<'_>,
) -> Result<ValidationReport, HistoryError> {
    let mut problems = Vec::new();
    if RealizationId::for_version(version)? != *id {
        problems.push(ValidationProblem::RealizationDigest);
    }
    let mut total_bytes = 0_u64;
    let nodes = scan_tree(
        manager,
        trees.nodes,
        "nodes",
        &mut total_bytes,
        &mut problems,
    )?;
    let edges = scan_tree(
        manager,
        trees.edges,
        "edges",
        &mut total_bytes,
        &mut problems,
    )?;
    let hyperedges = scan_tree(
        manager,
        trees.hyperedges,
        "hyperedges",
        &mut total_bytes,
        &mut problems,
    )?;
    let analysis = scan_tree(
        manager,
        trees.analysis,
        "analysis",
        &mut total_bytes,
        &mut problems,
    )?;
    let metadata = scan_tree(
        manager,
        trees.metadata,
        "metadata",
        &mut total_bytes,
        &mut problems,
    )?;
    let program_facts = scan_tree(
        manager,
        trees.program_facts,
        "program facts",
        &mut total_bytes,
        &mut problems,
    )?;
    let program_summaries = scan_tree(
        manager,
        trees.program_summaries,
        "program summaries",
        &mut total_bytes,
        &mut problems,
    )?;
    for (kind, expected, actual) in [
        ("nodes", version.node_count, nodes.len() as u64),
        ("edges", version.edge_count, edges.len() as u64),
        (
            "hyperedges",
            version.hyperedge_count,
            hyperedges.len() as u64,
        ),
        ("analysis", version.analysis_count, analysis.len() as u64),
        ("metadata", version.metadata_count, metadata.len() as u64),
        (
            "program facts",
            version.program_fact_count,
            program_facts.len() as u64,
        ),
        (
            "program summaries",
            version.program_summary_count,
            program_summaries.len() as u64,
        ),
    ] {
        if expected != actual {
            problems.push(ValidationProblem::Count {
                kind,
                expected,
                actual,
            });
        }
    }
    let partitioned = PartitionedGraph {
        nodes,
        edges,
        hyperedges,
        analysis,
        metadata,
        program_facts,
        program_summaries,
    };
    match GraphArtifacts::reconstruct(&partitioned) {
        Ok(artifacts) => {
            validate_references(manager, trees.nodes, &artifacts.document, &mut problems)?
        }
        Err(error) => problems.push(ValidationProblem::ArtifactRegistry(error.to_string())),
    }
    if problems.is_empty() {
        Ok(ValidationReport {
            nodes: version.node_count,
            edges: version.edge_count,
            hyperedges: version.hyperedge_count,
            analysis_records: version.analysis_count,
            metadata_records: version.metadata_count,
            program_fact_records: version.program_fact_count,
            program_summary_records: version.program_summary_count,
        })
    } else {
        Err(HistoryError::InvalidRealization(problems))
    }
}

fn scan_tree(
    manager: &Prolly<Arc<SqliteStore>>,
    tree: &Tree,
    kind: &'static str,
    total_bytes: &mut u64,
    problems: &mut Vec<ValidationProblem>,
) -> Result<Records, HistoryError> {
    let mut entries = Vec::new();
    for entry in manager.range(tree, &[], None)? {
        let (key, value) = entry?;
        if exceeds_limit(key.len() as u64, MAX_KEY_BYTES as u64) {
            problems.push(ValidationProblem::ResourceLimit {
                kind: "key bytes",
                limit: MAX_KEY_BYTES as u64,
                actual: key.len() as u64,
            });
        }
        if exceeds_limit(value.len() as u64, MAX_RECORD_VALUE_BYTES as u64) {
            problems.push(ValidationProblem::ResourceLimit {
                kind: "record value bytes",
                limit: MAX_RECORD_VALUE_BYTES as u64,
                actual: value.len() as u64,
            });
        }
        *total_bytes = total_bytes
            .saturating_add(key.len() as u64)
            .saturating_add(value.len() as u64);
        if exceeds_limit(*total_bytes, MAX_AUTHORITATIVE_BYTES) {
            problems.push(ValidationProblem::ResourceLimit {
                kind: "authoritative bytes",
                limit: MAX_AUTHORITATIVE_BYTES,
                actual: *total_bytes,
            });
        }
        if let Ok(envelope) = VersionedValue::from_bytes(&value)
            && let Ok(json) = serde_json::from_slice::<Value>(&envelope.payload)
        {
            let depth = json_depth(&json);
            if exceeds_limit(depth as u64, MAX_JSON_DEPTH as u64) {
                problems.push(ValidationProblem::ResourceLimit {
                    kind: "JSON depth",
                    limit: MAX_JSON_DEPTH as u64,
                    actual: depth as u64,
                });
            }
            if canonical_json_bytes(&json)? != envelope.payload {
                problems.push(ValidationProblem::KeyMismatch {
                    kind,
                    key: key.clone(),
                });
            }
        }
        entries.push((key, value));
        if exceeds_limit(entries.len() as u64, MAX_RECORDS_PER_TREE) {
            problems.push(ValidationProblem::ResourceLimit {
                kind: "records per tree",
                limit: MAX_RECORDS_PER_TREE,
                actual: entries.len() as u64,
            });
            break;
        }
    }
    Ok(entries)
}

fn validate_references(
    manager: &Prolly<Arc<SqliteStore>>,
    nodes: &Tree,
    document: &GraphDocument,
    problems: &mut Vec<ValidationProblem>,
) -> Result<(), HistoryError> {
    for edge in &document.links {
        for endpoint in [&edge.source, &edge.target] {
            if manager.get(nodes, &node_key(endpoint))?.is_none() {
                problems.push(ValidationProblem::MissingEdgeEndpoint {
                    edge: canonical_json_bytes(&serde_json::to_value(edge)?)?,
                    endpoint: endpoint.clone(),
                });
            }
        }
    }
    for value in document
        .graph
        .get("hyperedges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            document
                .extras
                .get("hyperedges")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
    {
        for member in ["nodes", "members"]
            .into_iter()
            .filter_map(|field| value.get(field).and_then(Value::as_array))
            .flatten()
            .filter_map(Value::as_str)
        {
            if manager.get(nodes, &node_key(member))?.is_none() {
                problems.push(ValidationProblem::MissingHyperedgeMember {
                    hyperedge: canonical_json_bytes(value)?,
                    member: member.to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_AUTHORITATIVE_BYTES, MAX_DIAGNOSTIC_BYTES, MAX_JOB_BYTES, MAX_JSON_DEPTH,
        MAX_KEY_BYTES, MAX_RECORD_VALUE_BYTES, MAX_RECORDS_PER_TREE, exceeds_limit,
    };

    #[test]
    fn every_v1_resource_limit_accepts_the_boundary_and_rejects_the_next_unit() {
        for limit in [
            MAX_AUTHORITATIVE_BYTES,
            MAX_KEY_BYTES as u64,
            MAX_RECORD_VALUE_BYTES as u64,
            MAX_RECORDS_PER_TREE,
            MAX_JSON_DEPTH as u64,
            MAX_JOB_BYTES as u64,
            MAX_DIAGNOSTIC_BYTES as u64,
        ] {
            assert!(!exceeds_limit(limit, limit));
            assert!(exceeds_limit(limit + 1, limit));
        }
    }
}
