#![cfg(feature = "mem")]

use compass_graphdb_surreal::{
    InterruptAfter, ProjectionError, ProjectionLimits, ProjectionPlan, SurrealProjection,
};
use compass_model::code_graph::{BuildMetadata, EdgeKind, GraphDocument, NodeKind};
use compass_model::identity::{edge_id, file_id, symbol_id};
use compass_model::provenance::SourceAnchor;
use serde_json::json;

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn semantic_graph(generation: char) -> Result<GraphDocument, serde_json::Error> {
    let path = "src/lib.rs";
    let source = symbol_id("rust", path, NodeKind::Function, "source", "");
    let target = symbol_id("rust", path, NodeKind::Function, "target", "");
    let anchor = SourceAnchor {
        file: path.to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 4,
    };
    let evidence = json!([{
        "origin": "config",
        "extractor": "projection-test",
        "confidence": "exact",
        "anchors": [anchor]
    }]);
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: digest('1'),
        source_tree_digest: digest('2'),
        configuration_digest: digest('3'),
        generation_id: digest(generation),
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
            "source": anchor,
            "evidence": evidence
        },
        {
            "id": target,
            "kind": "function",
            "name": "target",
            "qualifiedName": "target",
            "language": "rust",
            "source": anchor,
            "evidence": evidence
        }
    ]))?;
    let edge = |rule: &str, edge_source: &str, edge_target: &str| {
        let id = edge_id(
            edge_source,
            EdgeKind::Calls,
            edge_target,
            Some(&anchor),
            Some(rule),
        );
        json!({
            "id": id,
            "key": id,
            "source": edge_source,
            "target": edge_target,
            "kind": "calls",
            "occurrenceRule": rule,
            "relationshipSite": anchor,
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

#[tokio::test(flavor = "multi_thread")]
async fn activation_round_trips_and_interruption_keeps_previous_generation()
-> Result<(), Box<dyn std::error::Error>> {
    let projection = SurrealProjection::memory("compass_test", "projection_test").await?;
    let first = ProjectionPlan::from_graph("repository", &semantic_graph('4')?)?;
    let outcome = projection.activate(&first).await?;
    assert!(!outcome.already_present);
    assert_eq!(outcome.nodes, 2);
    assert_eq!(outcome.relations, 4);
    assert_eq!(
        projection.read_active_projection("repository").await?,
        Some(first.clone())
    );

    let repeated = projection.activate(&first).await?;
    assert!(repeated.already_present);

    let second = ProjectionPlan::from_graph("repository", &semantic_graph('6')?)?;
    assert!(matches!(
        projection
            .activate_with_interrupt(&second, Some(InterruptAfter(1)))
            .await,
        Err(ProjectionError::Interrupted {
            completed_mutations: 1
        })
    ));
    assert_eq!(
        projection.active_generation("repository").await?,
        Some(first.generation_id.clone())
    );
    assert_eq!(
        projection.read_active_projection("repository").await?,
        Some(first)
    );

    projection.activate(&second).await?;
    assert_eq!(
        projection.read_active_projection("repository").await?,
        Some(second)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn activation_rejects_a_plan_over_the_configured_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let limits = ProjectionLimits::new(1, 8, compass_model::DEFAULT_GRAPH_SIZE_CAP_BYTES)?;
    let projection = SurrealProjection::memory_with_limits(
        "compass_limit_test",
        "projection_limit_test",
        limits,
    )
    .await?;
    let plan = ProjectionPlan::from_graph("repository", &semantic_graph('8')?)?;
    assert!(matches!(
        projection.activate(&plan).await,
        Err(ProjectionError::LimitExceeded {
            resource: "nodes",
            actual: 2,
            limit: 1,
        })
    ));
    assert_eq!(projection.active_generation("repository").await?, None);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn zero_interrupt_stops_before_the_first_mutation() -> Result<(), Box<dyn std::error::Error>>
{
    let projection = SurrealProjection::memory("compass_zero_interrupt", "projection_test").await?;
    let plan = ProjectionPlan::from_graph("repository", &semantic_graph('9')?)?;
    assert!(matches!(
        projection
            .activate_with_interrupt(&plan, Some(InterruptAfter(0)))
            .await,
        Err(ProjectionError::Interrupted {
            completed_mutations: 0
        })
    ));
    assert_eq!(projection.active_generation("repository").await?, None);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn idempotent_reactivation_rejects_changed_generation_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let projection = SurrealProjection::memory("compass_metadata_test", "projection_test").await?;
    let plan = ProjectionPlan::from_graph("repository", &semantic_graph('a')?)?;
    projection.activate(&plan).await?;

    let mut changed = plan.clone();
    changed.source_tree_digest = digest('b');
    assert!(matches!(
        projection.activate(&changed).await,
        Err(ProjectionError::InvalidPlan(message))
            if message.contains("immutable generation already exists")
    ));
    assert_eq!(
        projection.read_active_projection("repository").await?,
        Some(plan)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn injection_shaped_repository_identity_round_trips_as_bound_data()
-> Result<(), Box<dyn std::error::Error>> {
    let projection = SurrealProjection::memory("compass_injection_test", "projection_test").await?;
    let repository = "repo:'; DELETE code_node; --";
    let plan = ProjectionPlan::from_graph(repository, &semantic_graph('c')?)?;
    projection.activate(&plan).await?;
    assert_eq!(
        projection.read_active_projection(repository).await?,
        Some(plan)
    );
    Ok(())
}
