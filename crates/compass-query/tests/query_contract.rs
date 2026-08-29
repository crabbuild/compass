use compass_model::code_graph::{NodeKind, NodeRole};
use compass_model::query_contract::{
    CODE_QUERY_SCHEMA_V1, CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, QueryNode,
    STRUCTURAL_QUERY_SCHEMA_V1, SearchHit, SearchRequest,
};
use std::fs;
use std::path::Path;

#[test]
fn shared_query_contract_is_strict_bounded_and_deterministic()
-> Result<(), Box<dyn std::error::Error>> {
    let limits = CodeQueryLimits::default();
    assert!(limits.is_valid());
    let request = serde_json::from_str::<SearchRequest>(&serde_json::to_string(&SearchRequest {
        query: "show user".to_owned(),
        limits: limits.clone(),
    })?)?;
    assert_eq!(request.query, "show user");
    assert!(serde_json::from_str::<SearchRequest>(
        r#"{"query":"x","limits":{"maxDepth":1,"maxNodes":1,"maxEdges":1,"maxPaths":1,"maxCandidates":1,"maxSourceBytes":1,"maxResponseBytes":1},"unknown":true}"#
    ).is_err());

    let mut response = CodeQueryResponse::empty(CodeQueryOperation::Search, limits);
    for (id, score) in [("b", 1.0), ("a", 1.0), ("c", 2.0)] {
        response.results.push(SearchHit {
            node_id: id.to_owned(),
            score,
            matched_fields: vec!["name".to_owned()],
        });
        response.nodes.push(QueryNode {
            id: id.to_owned(),
            kind: NodeKind::Function,
            roles: vec![NodeRole::RouteHandler],
            name: id.to_owned(),
            qualified_name: id.to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            source: None,
            details: None,
            evidence: Vec::new(),
        });
    }
    response.sort_stable();
    assert_eq!(response.schema, CODE_QUERY_SCHEMA_V1);
    assert_eq!(
        response
            .results
            .iter()
            .map(|result| result.node_id.as_str())
            .collect::<Vec<_>>(),
        ["c", "a", "b"]
    );
    assert_eq!(
        response
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert!(response.edges.is_empty());
    let structural = response.structural_view("repository", "generation");
    assert_eq!(structural.schema, STRUCTURAL_QUERY_SCHEMA_V1);
    assert_eq!(structural.repository_id, "repository");
    assert_eq!(structural.generation_id, "generation");
    assert_eq!(
        structural
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    Ok(())
}

#[test]
fn checked_in_contract_fingerprint_matches_the_enum_and_field_manifest()
-> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/contracts");
    let bytes = fs::read(root.join("compass-query-v1.manifest.json"))?;
    let expected = fs::read_to_string(root.join("compass-query-v1.fingerprint"))?;
    assert_eq!(
        expected.trim(),
        format!("sha256:{}", compass_ir::hex_sha256(&bytes))
    );
    let example = fs::read(root.join("compass-query-v1.example.json"))?;
    let response = serde_json::from_slice::<CodeQueryResponse>(&example)?;
    assert_eq!(response.schema, CODE_QUERY_SCHEMA_V1);
    Ok(())
}
