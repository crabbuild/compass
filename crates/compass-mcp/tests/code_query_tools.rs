use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use compass_core::{ClusterExistingOptions, cluster_existing_graph};
use compass_files::BuildGuard;
use compass_graph::{GodNode, SurpriseConnection};
use compass_mcp::CompassMcp;
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, ExtractionStatus, FileRecord, GraphDocument, NodeKind,
    NodeRecord,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
use compass_output::{
    DetectionSummary, ReportOptions, TokenCost, agent_orientation, graph_artifact_identity,
    render_orientation_json,
};
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
        occurrence_rule: None,
        relationship_site: Some(anchor),
        details: None,
        evidence: vec![evidence],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    });
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;
    let legacy = graph.to_legacy_document()?;
    let communities = std::collections::BTreeMap::new();
    let cohesion = std::collections::BTreeMap::new();
    let labels = std::collections::BTreeMap::new();
    let gods = Vec::<GodNode>::new();
    let surprises = Vec::<SurpriseConnection>::new();
    let mut orientation = agent_orientation(
        &legacy,
        &communities,
        &cohesion,
        &labels,
        &gods,
        &surprises,
        &DetectionSummary::default(),
        TokenCost::default(),
        None,
        None,
        &ReportOptions::new("fixture"),
    );
    orientation.evidence_status.artifact_set_identity = Some(graph_artifact_identity(&graph_path)?);
    fs::write(
        root.join("orientation.json"),
        render_orientation_json(&orientation)?,
    )?;
    Ok(graph_path)
}

fn add_parallel_call_edge(graph_path: &Path) -> Result<(), Box<dyn Error>> {
    let mut graph = GraphDocument::load(graph_path)?;
    let mut parallel = graph.links.first().cloned().ok_or("missing call edge")?;
    let anchor = SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 1,
        end_byte: 3,
        start_line: 1,
        start_column: 1,
        end_line: 1,
        end_column: 3,
    };
    let id = edge_id(
        &parallel.source,
        parallel.kind,
        &parallel.target,
        Some(&anchor),
        None,
    );
    parallel.id.clone_from(&id);
    parallel.key = id;
    parallel.relationship_site = Some(anchor.clone());
    for evidence in &mut parallel.evidence {
        evidence.anchors = vec![anchor.clone()];
    }
    graph.links.push(parallel);
    fs::write(graph_path, serde_json::to_vec_pretty(&graph)?)?;
    Ok(())
}

fn invoke(server: &CompassMcp, name: &str, arguments: Value) -> Result<Value, Box<dyn Error>> {
    let output = server.invoke(
        name,
        arguments.as_object().cloned().unwrap_or_else(Map::new),
    );
    let envelope = serde_json::from_str::<Value>(&output)?;
    assert_eq!(envelope["schema"], "compass.mcp.tool-result/1");
    assert_eq!(envelope["transportTruncation"]["truncated"], false);
    Ok(envelope["result"].clone())
}

#[test]
fn code_query_tools_share_the_bounded_versioned_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = write_typed_graph(directory.path())?;
    let server = CompassMcp::new(graph);
    let orientation: Value = serde_json::from_str(&server.read("compass://orientation")?)?;
    assert_eq!(orientation["schema"], "compass.orientation/1");
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

    let reverse = invoke(
        &server,
        "get_node",
        json!({"source":"Target","target":"Caller"}),
    )?;
    assert_eq!(reverse["paths"], json!([]));
    assert_eq!(reverse["diagnostics"][0]["code"], "direction_mismatch");

    let default_discovery = invoke(
        &server,
        "query_graph",
        json!({"question":"who calls Target?"}),
    )?;
    assert_eq!(default_discovery["schema"], "compass.query.discovery/1");

    let discovery = invoke(
        &server,
        "query_graph",
        json!({
            "question":"Target",
            "direction":"incoming",
            "scope":[{"kind":"node","value":"Fixture.Target"}],
            "relation_contexts":["call"],
            "traversal":"bfs"
        }),
    )?;
    assert_eq!(discovery["schema"], "compass.query.discovery/1");
    assert_eq!(discovery["selectedDirection"], "incoming");
    assert_eq!(discovery["directionSource"], "explicit");
    assert_eq!(discovery["scope"][0]["kind"], "node");
    assert_eq!(discovery["scope"][0]["value"], "n:target");
    assert_eq!(discovery["seeds"][0]["nodeId"], "n:target");

    let invalid_scope = server.invoke(
        "query_graph",
        json!({
            "question":"Target",
            "scope":[{"kind":"guessed","value":"Fixture.Target"}]
        })
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new),
    );
    assert!(invalid_scope.contains("unsupported 'kind' value"));

    for (field, value) in [
        ("direction", json!("auto")),
        ("relation_contexts", json!([])),
        ("scope", json!([])),
        ("traversal", json!("bfs")),
        ("include_heuristic", json!(false)),
        ("max_depth", json!(2)),
        ("max_seeds", json!(3)),
        ("max_candidates", json!(256)),
        ("max_nodes", json!(500)),
        ("max_edges", json!(1000)),
        ("max_expanded_relationships", json!(10_000)),
        ("max_response_bytes", json!(8_388_608)),
        ("timeout_ms", json!(30_000)),
    ] {
        let mut arguments = Map::from_iter([("question".to_owned(), json!("Target"))]);
        arguments.insert(field.to_owned(), value);
        let response = invoke(&server, "query_graph", Value::Object(arguments))?;
        assert_eq!(response["schema"], "compass.query.discovery/1", "{field}");
    }

    for (field, value) in [
        ("direction", json!(7)),
        ("relation_contexts", json!("call")),
        ("scope", json!("node:Target")),
        ("traversal", json!(7)),
        ("include_heuristic", json!("false")),
        ("max_depth", json!("2")),
        ("max_seeds", json!(-1)),
        ("max_candidates", json!(false)),
        ("max_nodes", json!(1.5)),
        ("max_edges", Value::Null),
        ("max_expanded_relationships", json!("100")),
        ("max_response_bytes", json!(false)),
        ("timeout_ms", json!(-1)),
    ] {
        let mut arguments = Map::from_iter([("question".to_owned(), json!("Target"))]);
        arguments.insert(field.to_owned(), value);
        let output = server.invoke("query_graph", arguments);
        assert!(
            output.contains("must be") || output.contains("unsupported"),
            "{field}: {output}"
        );
    }
    assert!(
        server
            .invoke(
                "query_graph",
                Map::from_iter([
                    ("question".to_owned(), json!("Target")),
                    ("mode".to_owned(), json!("bfs")),
                    ("direction".to_owned(), json!("incoming")),
                ]),
            )
            .contains("cannot be combined")
    );
    assert!(
        server
            .invoke(
                "query_graph",
                Map::from_iter([
                    ("question".to_owned(), json!("Target")),
                    ("unknown".to_owned(), json!(true)),
                ]),
            )
            .contains("unknown query_graph argument")
    );

    let default = invoke(
        &server,
        "query_graph",
        json!({"question":"authentication flow"}),
    )?;
    assert_eq!(default["schema"], "compass.query.discovery/1");
    let legacy = server.invoke(
        "query_graph",
        Map::from_iter([
            ("question".to_owned(), json!("who calls Target?")),
            ("mode".to_owned(), json!("bfs")),
        ]),
    );
    assert!(serde_json::from_str::<Value>(&legacy).is_err(), "{legacy}");
    Ok(())
}

#[test]
fn cluster_only_output_remains_typed_and_serves_orientation_resources() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    fs::create_dir(&output)?;
    let graph = write_typed_graph(&output)?;
    add_parallel_call_edge(&graph)?;
    cluster_existing_graph(&ClusterExistingOptions {
        graph_path: graph,
        output_dir: output.clone(),
        root: directory.path().to_path_buf(),
        no_viz: true,
        no_label: true,
        resolution: 1.0,
        exclude_hubs: None,
        min_community_size: 1,
    })?;

    let active = BuildGuard::resolve_current_snapshot_directory(&output)?;
    let typed = GraphDocument::load(&active.join("graph.json"))?;
    assert_eq!(typed.graph.schema, "compass.graph/1");
    assert_eq!(typed.links.len(), 2);
    let server = CompassMcp::new(output.join("graph.json"));
    let orientation: Value = serde_json::from_str(&server.read("compass://orientation")?)?;
    assert_eq!(orientation["schema"], "compass.orientation/1");
    assert!(orientation["evidenceStatus"]["buildCommit"].is_null());
    assert_eq!(orientation["graphSummary"]["edges"], 2);
    let report = server.read("compass://report")?;
    assert!(report.contains("# Agent Orientation"));
    assert!(report.contains("· 2 edges ·"));
    Ok(())
}

#[test]
fn report_resource_is_rendered_from_validated_orientation_and_rejects_missing_or_stale_evidence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = write_typed_graph(directory.path())?;
    let orientation_path = directory.path().join("orientation.json");
    let orientation_bytes = fs::read(&orientation_path)?;
    fs::write(
        directory.path().join("GRAPH_REPORT.md"),
        "# stale sibling report\nUNTRUSTED-SIBLING-CONTENT\n",
    )?;
    let server = CompassMcp::new(&graph);
    let report = server.read("compass://report")?;
    assert!(report.contains("# Agent Orientation"));
    assert!(report.contains("# Bounded Graph Detail"));
    assert!(!report.contains("UNTRUSTED-SIBLING-CONTENT"));

    fs::remove_file(&orientation_path)?;
    let missing = server
        .read("compass://report")
        .err()
        .ok_or("missing orientation unexpectedly succeeded")?;
    assert!(
        missing
            .to_string()
            .contains("coherent orientation artifact is unavailable")
    );
    fs::write(&orientation_path, orientation_bytes)?;

    let mut changed: Value = serde_json::from_slice(&fs::read(&graph)?)?;
    changed["nodes"][0]["community"] = json!({"id":7,"label":"Changed"});
    fs::write(&graph, serde_json::to_vec_pretty(&changed)?)?;
    let changed_server = CompassMcp::new(graph);
    let stale = changed_server
        .read("compass://orientation")
        .err()
        .ok_or("same-size graph summary with changed community unexpectedly succeeded")?;
    assert!(
        stale
            .to_string()
            .contains("orientation artifact-set identity does not match")
    );
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

#[test]
fn query_graph_schema_exposes_typed_discovery_controls() -> Result<(), Box<dyn Error>> {
    let query = CompassMcp::tools()
        .into_iter()
        .find(|tool| tool.name.as_ref() == "query_graph")
        .ok_or("query_graph tool missing")?;
    let properties = query
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("query_graph properties missing")?;
    assert_eq!(
        properties["direction"]["enum"],
        json!(["auto", "incoming", "outgoing", "both"])
    );
    assert_eq!(query.input_schema["additionalProperties"], false);
    assert_eq!(properties["question"]["maxLength"], 4096);
    assert_eq!(properties["relation_contexts"]["maxItems"], 32);
    assert_eq!(properties["scope"]["maxItems"], 32);
    assert_eq!(properties["scope"]["items"]["additionalProperties"], false);
    assert_eq!(
        properties["scope"]["items"]["properties"]["kind"]["enum"],
        json!(["community", "source", "package", "node"])
    );
    for (name, maximum) in [
        ("max_depth", 8_u64),
        ("max_seeds", 3),
        ("max_candidates", 256),
        ("max_nodes", 500),
        ("max_edges", 1000),
        ("max_expanded_relationships", 10_000),
        ("max_response_bytes", 8_388_608),
        ("timeout_ms", 30_000),
    ] {
        assert_eq!(properties[name]["maximum"], maximum, "{name}");
    }
    for name in [
        "mode",
        "depth",
        "token_budget",
        "direction",
        "traversal",
        "include_heuristic",
        "max_depth",
        "max_seeds",
        "max_candidates",
        "max_nodes",
        "max_edges",
        "max_expanded_relationships",
        "max_response_bytes",
        "timeout_ms",
    ] {
        assert!(properties[name].get("default").is_none(), "{name}");
    }
    Ok(())
}

#[test]
fn explicit_discovery_controls_fail_on_legacy_graphs_instead_of_falling_through()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let legacy_path = directory.path().join("legacy.json");
    fs::write(
        &legacy_path,
        serde_json::to_vec_pretty(&json!({
            "directed":true, "multigraph":false, "graph":{},
            "nodes":[{"id":"target","label":"Target"}], "links":[]
        }))?,
    )?;
    let server = CompassMcp::new(legacy_path);

    let plain = server.invoke(
        "query_graph",
        Map::from_iter([("question".to_owned(), json!("Target"))]),
    );
    assert!(plain.contains("Traversal: BFS"), "{plain}");
    assert!(plain.contains("NODE Target"), "{plain}");

    let explicit = server.invoke(
        "query_graph",
        Map::from_iter([
            ("question".to_owned(), json!("Target")),
            ("direction".to_owned(), json!("incoming")),
        ]),
    );
    assert!(
        explicit.contains("discovery controls require a typed compass.graph/1 artifact"),
        "{explicit}"
    );
    Ok(())
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
        Some("compass.mcp.tool-result/1")
    );
    assert_eq!(
        response
            .structured_content
            .as_ref()
            .and_then(|value| value.get("result"))
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
