use std::collections::BTreeSet;

use compass_graphdb_surreal::{
    ProjectedRelation, ProjectionError, ProjectionLimits, ProjectionPlan, RelationFamily,
    relation_family,
};
use compass_model::code_graph::{BuildMetadata, EdgeKind, EdgeRecord, GraphDocument, NodeKind};
use compass_model::identity::{edge_id, file_id, symbol_id};
use compass_model::provenance::SourceAnchor;
use serde_json::json;

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn anchor() -> SourceAnchor {
    SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 4,
    }
}

fn semantic_graph() -> Result<GraphDocument, serde_json::Error> {
    let path = "src/lib.rs";
    let source = symbol_id("rust", path, NodeKind::Function, "source", "");
    let target = symbol_id("rust", path, NodeKind::Function, "target", "");
    let source_anchor = anchor();
    let evidence = json!([{
        "origin": "config",
        "extractor": "projection-test",
        "confidence": "exact",
        "anchors": [source_anchor]
    }]);
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: digest('1'),
        source_tree_digest: digest('2'),
        configuration_digest: digest('3'),
        generation_id: digest('4'),
        source_commit: Some("fixture".to_owned()),
    });
    graph.graph.files = serde_json::from_value(json!([{
        "id": file_id(path),
        "path": path,
        "contentDigest": digest('5'),
        "byteSize": 4,
        "generated": false,
        "extractionStatus": "extracted"
    }]))?;
    graph.nodes = serde_json::from_value(json!([
        {
            "id": source,
            "kind": "function",
            "name": "source",
            "qualifiedName": "source",
            "language": "rust",
            "source": source_anchor,
            "evidence": evidence
        },
        {
            "id": target,
            "kind": "function",
            "name": "target",
            "qualifiedName": "target",
            "language": "rust",
            "source": source_anchor,
            "evidence": evidence
        }
    ]))?;
    let edge = |rule: &str, edge_source: &str, edge_target: &str| {
        let id = edge_id(
            edge_source,
            EdgeKind::Calls,
            edge_target,
            Some(&source_anchor),
            Some(rule),
        );
        json!({
            "id": id,
            "key": id,
            "source": edge_source,
            "target": edge_target,
            "kind": "calls",
            "occurrenceRule": rule,
            "relationshipSite": source_anchor,
            "evidence": evidence
        })
    };
    graph.links = serde_json::from_value(json!([
        edge("parallel-a", &source, &target),
        edge("parallel-b", &source, &target),
        edge("reverse", &target, &source),
        edge("self", &source, &source)
    ]))?;
    Ok(graph)
}

#[test]
fn empty_repository_is_rejected() -> Result<(), serde_json::Error> {
    assert!(matches!(
        ProjectionPlan::from_graph(" ", &semantic_graph()?),
        Err(ProjectionError::EmptyRepositoryId)
    ));
    Ok(())
}

#[test]
fn edge_mapping_is_total_and_uses_every_family() {
    let families = EdgeKind::ALL
        .into_iter()
        .map(relation_family)
        .collect::<BTreeSet<_>>();
    assert_eq!(families, RelationFamily::ALL.into_iter().collect());
}

#[test]
fn plan_is_deterministic_and_semantically_lossless() -> Result<(), Box<dyn std::error::Error>> {
    let graph = semantic_graph()?;
    let repository = "repo:'; DELETE code_node; --";
    let first = ProjectionPlan::from_graph(repository, &graph)?;
    let second = ProjectionPlan::from_graph(repository, &graph)?;
    assert_eq!(first, second);
    assert_eq!(first.nodes.len(), 2);
    assert_eq!(first.relations.len(), 4);
    for node in &first.nodes {
        assert_eq!(node.decode()?.id, node.compass_node_id);
        assert_eq!(node.confidence, "exact");
    }
    let decoded = first
        .relations
        .iter()
        .map(ProjectedRelation::decode)
        .collect::<Result<Vec<EdgeRecord>, _>>()?;
    assert_eq!(decoded, {
        let mut edges = graph.links.clone();
        edges.sort_by(|left, right| left.id.cmp(&right.id));
        edges
    });
    for relation in &first.relations {
        assert_eq!(relation.confidence, "exact");
        assert_eq!(relation.family, RelationFamily::Execution);
    }
    Ok(())
}

#[test]
fn tampered_or_unsorted_plan_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let graph = semantic_graph()?;
    let mut plan = ProjectionPlan::from_graph("repo", &graph)?;
    plan.nodes.reverse();
    assert!(matches!(
        plan.validate(),
        Err(ProjectionError::NonDeterministicOrder {
            record_class: "node"
        })
    ));
    let mut plan = ProjectionPlan::from_graph("repo", &graph)?;
    plan.projection_fingerprint.push('0');
    assert!(matches!(
        plan.validate(),
        Err(ProjectionError::FingerprintMismatch)
    ));
    Ok(())
}

#[test]
fn invalid_graph_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = semantic_graph()?;
    graph.links[0].target = "missing".to_owned();
    assert!(matches!(
        ProjectionPlan::from_graph("repo", &graph),
        Err(ProjectionError::GraphValidation(_))
    ));
    Ok(())
}

#[test]
fn projection_limits_are_positive_and_match_qualification_defaults() {
    assert!(matches!(
        ProjectionLimits::new(0, 1, 1),
        Err(ProjectionError::InvalidPlan(_))
    ));
    let limits = ProjectionLimits::default();
    assert_eq!(limits.max_nodes(), 1_000_000);
    assert_eq!(limits.max_relations(), 2_500_000);
    assert_eq!(
        limits.max_projected_bytes(),
        compass_model::DEFAULT_GRAPH_SIZE_CAP_BYTES
    );
}
