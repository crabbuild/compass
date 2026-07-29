use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use compass_mcp::CompassMcp;
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, ExtractionStatus, FileRecord, GraphDocument, NodeKind,
    NodeRecord,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, Value, json};

fn write_typed_graph(root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let graph_path = root.join("graph.json");
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), "code")?;
    let anchor = SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 4,
    };
    let evidence = Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "mcp-test".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: None,
        anchors: vec![anchor.clone()],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    };
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "sha256:test".to_owned(),
        source_tree_digest: "sha256:test".to_owned(),
        configuration_digest: "sha256:test".to_owned(),
        generation_id: "sha256:test".to_owned(),
        source_commit: None,
    });
    graph.graph.files.push(FileRecord {
        id: file_id("src/lib.rs"),
        path: "src/lib.rs".to_owned(),
        language: Some("rust".to_owned()),
        content_digest: "sha256:5694d08a2e53ffcae0c3103e5ad6f6076abd960eb1f8a56577040bc1028f702b"
            .to_owned(),
        byte_size: 4,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["mcp-test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    graph.nodes = ["Caller", "Target"]
        .into_iter()
        .map(|name| NodeRecord {
            id: format!("n:{}", name.to_ascii_lowercase()),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: name.to_owned(),
            qualified_name: format!("Fixture.{name}"),
            language: Some("rust".to_owned()),
            framework: None,
            source: Some(anchor.clone()),
            details: None,
            evidence: vec![evidence.clone()],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        })
        .collect();
    let id = edge_id("n:caller", EdgeKind::Calls, "n:target", Some(&anchor), None);
    graph.links.push(EdgeRecord {
        id: id.clone(),
        key: id,
        source: "n:caller".to_owned(),
        target: "n:target".to_owned(),
        kind: EdgeKind::Calls,
        relationship_site: Some(anchor),
        details: None,
        evidence: vec![evidence],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    });
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;
    Ok(graph_path)
}

fn invoke(server: &CompassMcp, name: &str, arguments: Value) -> Result<Value, Box<dyn Error>> {
    let output = server.invoke(
        name,
        arguments.as_object().cloned().unwrap_or_else(Map::new),
    );
    Ok(serde_json::from_str(&output)?)
}

#[test]
fn code_query_tools_share_the_bounded_versioned_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = write_typed_graph(directory.path())?;
    let server = CompassMcp::new(graph);
    for (tool, arguments, operation) in [
        ("search_symbols", json!({"query":"Target"}), "search"),
        ("get_callers", json!({"symbol":"Target"}), "callers"),
        ("get_callees", json!({"symbol":"Caller"}), "callees"),
        ("get_impact", json!({"symbol":"Target"}), "impact"),
        (
            "explore_code",
            json!({"symbols":["Caller","Target"],"root":directory.path()}),
            "explore",
        ),
        (
            "get_node",
            json!({"source":"Caller","target":"Target"}),
            "node_trail",
        ),
    ] {
        let response = invoke(&server, tool, arguments)?;
        assert_eq!(response["schema"], "compass.query/1", "{tool}");
        assert_eq!(response["operation"], operation, "{tool}");
        assert!(response["limits"]["maxNodes"].as_u64().is_some(), "{tool}");
    }
    Ok(())
}

#[test]
fn code_query_tool_schemas_are_closed_and_bounded() {
    for tool in CompassMcp::tools().into_iter().filter(|tool| {
        matches!(
            tool.name.as_ref(),
            "search_symbols"
                | "get_callers"
                | "get_callees"
                | "get_impact"
                | "explore_code"
                | "get_node"
        )
    }) {
        assert_eq!(
            tool.input_schema.get("additionalProperties"),
            Some(&Value::Bool(false))
        );
        assert!(
            tool.input_schema
                .get("properties")
                .and_then(Value::as_object)
                .and_then(|properties| properties.get("max_nodes"))
                .and_then(Value::as_object)
                .and_then(|limit| limit.get("default"))
                .and_then(Value::as_u64)
                .is_some()
        );
    }
}

#[tokio::test]
async fn mcp_code_queries_publish_structured_content_and_protocol_errors()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = write_typed_graph(directory.path())?;
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_task = tokio::spawn(async move {
        let running = CompassMcp::new(graph)
            .serve(server_transport)
            .await
            .map_err(|error| error.to_string())?;
        running.waiting().await.map_err(|error| error.to_string())
    });
    let client = ().serve(client_transport).await?;
    let response = client
        .call_tool(
            CallToolRequestParams::new("search_symbols")
                .with_arguments(Map::from_iter([("query".to_owned(), json!("Target"))])),
        )
        .await?;
    assert_eq!(
        response
            .structured_content
            .as_ref()
            .and_then(|value| value.get("schema"))
            .and_then(Value::as_str),
        Some("compass.query/1")
    );
    assert!(!response.content.is_empty());
    assert!(
        client
            .call_tool(CallToolRequestParams::new("search_symbols"))
            .await
            .is_err()
    );
    client.cancel().await?;
    server_task.await?.map_err(std::io::Error::other)?;
    Ok(())
}
