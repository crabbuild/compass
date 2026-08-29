#![cfg(feature = "mem")]

use std::fs;

use compass_graphdb_surreal::{
    ProjectionError, ProjectionPlan, RelationPageRequest, SurrealProjection,
};
use compass_model::code_graph::{BuildMetadata, EdgeKind, GraphDocument, NodeKind};
use compass_model::identity::{edge_id, file_id, symbol_id};
use compass_model::provenance::SourceAnchor;
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, ExploreRequest, ImpactRequest, NodeTrailRequest,
};
use compass_query::{EngineSelection, open_with_engine};
use serde_json::json;

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn fixture(generation: char) -> Result<GraphDocument, serde_json::Error> {
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
    let exact = json!([{
        "origin": "config",
        "extractor": "native-query-integration",
        "confidence": "exact",
        "anchors": [anchor]
    }]);
    let heuristic = json!([{
        "origin": "heuristic",
        "extractor": "native-query-integration",
        "confidence": "inferred",
        "anchors": [anchor],
        "rule": "heuristic-fixture",
        "wiringSite": anchor
    }]);
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "integration".to_owned(),
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
            "qualifiedName": "fixture::source",
            "language": "rust",
            "source": anchor,
            "evidence": exact
        },
        {
            "id": target,
            "kind": "function",
            "name": "target",
            "qualifiedName": "fixture::target",
            "language": "rust",
            "source": anchor,
            "evidence": exact
        }
    ]))?;
    let relation = |kind: EdgeKind,
                    rule: &str,
                    edge_source: &str,
                    edge_target: &str,
                    evidence: &serde_json::Value| {
        let id = edge_id(edge_source, kind, edge_target, Some(&anchor), Some(rule));
        json!({
            "id": id,
            "key": id,
            "source": edge_source,
            "target": edge_target,
            "kind": kind.as_str(),
            "occurrenceRule": rule,
            "relationshipSite": anchor,
            "evidence": evidence
        })
    };
    graph.links = serde_json::from_value(json!([
        relation(EdgeKind::Calls, "parallel-a", &source, &target, &exact),
        relation(EdgeKind::Calls, "parallel-b", &source, &target, &exact),
        relation(EdgeKind::Calls, "reverse", &target, &source, &exact),
        relation(EdgeKind::Calls, "self", &source, &source, &exact),
        relation(EdgeKind::Calls, "heuristic", &source, &target, &heuristic)
    ]))?;
    Ok(graph)
}

fn limits() -> CodeQueryLimits {
    CodeQueryLimits {
        max_depth: 8,
        max_nodes: 64,
        max_edges: 64,
        max_paths: 32,
        max_candidates: 16,
        max_source_bytes: 1_024,
        max_response_bytes: 1_048_576,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_and_json_structural_operations_are_semantically_identical()
-> Result<(), Box<dyn std::error::Error>> {
    let repository = "repo:'; DELETE code_node; --";
    let graph = fixture('4')?;
    let generation = graph.graph.build.generation_id.clone();
    let source = graph.nodes[0].id.clone();
    let target = graph.nodes[1].id.clone();
    let temporary = tempfile::tempdir()?;
    let graph_path = temporary.path().join("graph.json");
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;
    let json = open_with_engine(
        &graph_path,
        None,
        &temporary.path().join("cache"),
        EngineSelection::Json,
    )?;
    let surreal = SurrealProjection::memory("native_equivalence", "native_equivalence").await?;
    surreal
        .activate(&ProjectionPlan::from_graph(repository, &graph)?)
        .await?;

    for include_heuristic in [false, true] {
        let callers = CallRequest {
            symbol: "TARGET()".to_owned(),
            include_heuristic,
            limits: limits(),
        };
        assert_eq!(
            surreal
                .callers(repository, callers.clone())
                .await?
                .structural_view(repository, &generation),
            json.callers(callers)?
                .structural_view(repository, &generation)
        );

        let callees = CallRequest {
            symbol: source.clone(),
            include_heuristic,
            limits: limits(),
        };
        assert_eq!(
            surreal
                .callees(repository, callees.clone())
                .await?
                .structural_view(repository, &generation),
            json.callees(callees)?
                .structural_view(repository, &generation)
        );

        let impact = ImpactRequest {
            symbol: target.clone(),
            include_heuristic,
            limits: limits(),
        };
        assert_eq!(
            surreal
                .impact(repository, impact.clone())
                .await?
                .structural_view(repository, &generation),
            json.impact(impact)?
                .structural_view(repository, &generation)
        );

        let trail = NodeTrailRequest {
            source: source.clone(),
            target: target.clone(),
            include_heuristic,
            limits: limits(),
        };
        assert_eq!(
            surreal
                .node_trail(repository, trail.clone())
                .await?
                .structural_view(repository, &generation),
            json.node_trail(trail)?
                .structural_view(repository, &generation)
        );

        let subgraph = ExploreRequest {
            symbols: vec![source.clone(), target.clone()],
            root: String::new(),
            include_heuristic,
            limits: limits(),
        };
        assert_eq!(
            surreal
                .structural_subgraph(repository, subgraph.clone())
                .await?
                .structural_view(repository, &generation),
            json.structural_subgraph(subgraph)?
                .structural_view(repository, &generation)
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn relation_pages_are_complete_ordered_and_generation_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let repository = "repository";
    let first_graph = fixture('6')?;
    let first_plan = ProjectionPlan::from_graph(repository, &first_graph)?;
    let projection = SurrealProjection::memory("native_pages", "native_pages").await?;
    projection.activate(&first_plan).await?;
    let mut cursor = None;
    let mut observed = Vec::new();
    loop {
        let page = projection
            .read_relation_page(
                repository,
                RelationPageRequest {
                    max_items: 2,
                    cursor,
                    include_heuristic: true,
                },
            )
            .await?;
        observed.extend(page.relations.into_iter().map(|relation| relation.id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        observed,
        first_plan
            .relations
            .iter()
            .map(|relation| relation.compass_edge_id.clone())
            .collect::<Vec<_>>()
    );

    let first_page = projection
        .read_relation_page(
            repository,
            RelationPageRequest {
                max_items: 1,
                cursor: None,
                include_heuristic: true,
            },
        )
        .await?;
    let old_cursor = first_page
        .next_cursor
        .ok_or("expected another relation page")?;
    projection
        .activate(&ProjectionPlan::from_graph(repository, &fixture('7')?)?)
        .await?;
    assert!(matches!(
        projection
            .read_relation_page(
                repository,
                RelationPageRequest {
                    max_items: 1,
                    cursor: Some(old_cursor),
                    include_heuristic: true,
                },
            )
            .await,
        Err(ProjectionError::InvalidCursor(_))
    ));
    assert!(matches!(
        projection
            .read_relation_page(
                repository,
                RelationPageRequest {
                    max_items: 1,
                    cursor: Some("not-hex".to_owned()),
                    include_heuristic: true,
                },
            )
            .await,
        Err(ProjectionError::InvalidCursor(_))
    ));
    assert!(matches!(
        projection
            .callers(
                "missing",
                CallRequest {
                    symbol: "anything".to_owned(),
                    include_heuristic: false,
                    limits: limits(),
                },
            )
            .await,
        Err(ProjectionError::ActiveGenerationUnavailable { .. })
    ));
    Ok(())
}
