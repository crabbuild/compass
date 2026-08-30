use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use compass_core::{ClusterExistingOptions, cluster_existing_graph};
use compass_files::BuildGuard;
use compass_graph::{GodNode, GraphSnapshotBuilder, SurpriseConnection};
use compass_mcp::CompassMcp;
use compass_model::code_graph::{
    BuildMetadata, CallDispatch, CallEdgeDetails, ComponentNodeDetails, ConfigNodeDetails,
    DatabaseNodeDetails, EdgeDetails, EdgeKind, EdgeRecord, ExtractionStatus, FileNodeDetails,
    FileRecord, GraphDocument, ImportExportNodeDetails, JobNodeDetails, MappingEdgeDetails,
    MessagingEdgeDetails, MessagingNodeDetails, NodeDetails, NodeKind, NodeRecord, NodeRole,
    QueryNodeDetails, ResourceKind, ResourceNodeDetails, RouteEdgeDetails, RouteNodeDetails,
    RouteStage, RouteStageDetails, ScheduleEdgeDetails, SchemaNodeDetails, SymbolNodeDetails,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, Provenance, ResolutionCandidate, ResolutionState,
    SourceAnchor,
};
use compass_output::{
    DetectionSummary, ReportOptions, TokenCost, agent_orientation, graph_artifact_identity,
    render_orientation_json,
};
use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation, ProtocolVersion,
    ResultType,
};
use rmcp::{ClientLifecycleMode, ClientServiceExt, ServiceExt};
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

fn write_parallel_call_graph(root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let graph_path = write_typed_graph(root)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let anchor = SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 1,
        end_byte: 4,
        start_line: 1,
        start_column: 1,
        end_line: 1,
        end_column: 4,
    };
    let evidence = Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "mcp-test-parallel".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: Some("second-call-site".to_owned()),
        anchors: vec![anchor.clone()],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    };
    let id = edge_id(
        "n:caller",
        EdgeKind::Calls,
        "n:target",
        Some(&anchor),
        Some("second-call-site"),
    );
    graph.links.push(EdgeRecord {
        id: id.clone(),
        key: id,
        source: "n:caller".to_owned(),
        target: "n:target".to_owned(),
        kind: EdgeKind::Calls,
        occurrence_rule: compass_model::provenance::OccurrenceRule::new("second-call-site"),
        relationship_site: Some(anchor),
        details: Some(EdgeDetails::Call(CallEdgeDetails {
            dispatch: CallDispatch::Static,
            receiver_type: Some("Fixture".to_owned()),
            argument_count: Some(1),
        })),
        evidence: vec![evidence],
        weight: None,
        context: Some("parallel fixture".to_owned()),
        deferred: false,
        diagnostics: Vec::new(),
    });
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;
    Ok(graph_path)
}

fn publish_store(root: &Path, graph_path: &Path) -> Result<(), Box<dyn Error>> {
    let store = SqliteStore::open(root.join(STORE_FILE_NAME))?;
    let graph = GraphDocument::load(graph_path)?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    fs::write(
        root.join(STORE_REF_FILE_NAME),
        serde_json::to_vec(&store.snapshot_reference()?)?,
    )?;
    store.checkpoint()?;
    Ok(())
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
    if envelope["schema"] == "compass.code_context.v1" {
        return Ok(envelope);
    }
    assert_eq!(envelope["schema"], "compass.mcp.tool-result/1");
    assert_eq!(envelope["transportTruncation"]["truncated"], false);
    Ok(envelope["result"].clone())
}

fn golden_result(tool: &str) -> Result<Value, Box<dyn Error>> {
    let contents = match tool {
        "search_symbols" => include_str!("fixtures/search_symbols-result.json"),
        "get_callers" => include_str!("fixtures/get_callers-result.json"),
        "get_callees" => include_str!("fixtures/get_callees-result.json"),
        "get_impact" => include_str!("fixtures/get_impact-result.json"),
        _ => return Err(format!("no golden result for {tool}").into()),
    };
    Ok(serde_json::from_str(contents)?)
}

fn pre_envelope_golden(tool: &str) -> Result<Value, Box<dyn Error>> {
    let contents = match tool {
        "search_symbols" => include_str!("fixtures/search_symbols-query-v1.json"),
        "get_callers" => include_str!("fixtures/get_callers-query-v1.json"),
        "get_callees" => include_str!("fixtures/get_callees-query-v1.json"),
        "get_impact" => include_str!("fixtures/get_impact-query-v1.json"),
        _ => return Err(format!("no pre-envelope golden result for {tool}").into()),
    };
    Ok(serde_json::from_str(contents)?)
}

fn parallel_callers_golden() -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_str(include_str!(
        "fixtures/get_callers-parallel-query-v1.json"
    ))?)
}

fn validate_schema(value: &Value, schema: &Value, root: &Value, path: &str) -> Result<(), String> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let pointer = reference
            .strip_prefix('#')
            .ok_or_else(|| format!("{path}: external schema reference is unsupported"))?;
        let resolved = root
            .pointer(pointer)
            .ok_or_else(|| format!("{path}: unresolved schema reference {reference}"))?;
        return validate_schema(value, resolved, root, path);
    }
    if let Some(expected) = schema.get("const")
        && value != expected
    {
        return Err(format!(
            "{path}: expected constant {expected}, found {value}"
        ));
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        return Err(format!("{path}: value {value} is outside the enum"));
    }
    if let Some(alternatives) = schema.get("anyOf").and_then(Value::as_array)
        && !alternatives
            .iter()
            .any(|alternative| validate_schema(value, alternative, root, path).is_ok())
    {
        return Err(format!("{path}: no anyOf alternative matched"));
    }
    if let Some(alternatives) = schema.get("oneOf").and_then(Value::as_array) {
        let matches = alternatives
            .iter()
            .filter(|alternative| validate_schema(value, alternative, root, path).is_ok())
            .count();
        if matches != 1 {
            return Err(format!("{path}: expected one oneOf match, found {matches}"));
        }
    }
    if let Some(expected) = schema.get("type") {
        let matches_type = |name: &str| match name {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        };
        let valid = expected.as_str().is_some_and(matches_type)
            || expected
                .as_array()
                .is_some_and(|names| names.iter().filter_map(Value::as_str).any(matches_type));
        if !valid {
            return Err(format!("{path}: value {value} has the wrong type"));
        }
    }
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
        && value.as_f64().is_some_and(|number| number < minimum)
    {
        return Err(format!("{path}: numeric value is below {minimum}"));
    }
    if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64)
        && value
            .as_str()
            .is_some_and(|text| text.len() < minimum as usize)
    {
        return Err(format!("{path}: string is shorter than {minimum}"));
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(name) {
                    return Err(format!("{path}: required property {name} is absent"));
                }
            }
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            for name in object.keys() {
                if !properties.is_some_and(|known| known.contains_key(name)) {
                    return Err(format!("{path}: unexpected property {name}"));
                }
            }
        }
        if let Some(properties) = properties {
            for (name, child_schema) in properties {
                if let Some(child) = object.get(name) {
                    validate_schema(child, child_schema, root, &format!("{path}.{name}"))?;
                }
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array()) {
        for (index, item) in values.iter().enumerate() {
            validate_schema(item, items, root, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

#[test]
fn code_query_tools_share_the_bounded_versioned_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = write_typed_graph(directory.path())?;
    let server = CompassMcp::new(graph);
    let orientation: Value = serde_json::from_str(&server.read("compass://orientation")?)?;
    assert_eq!(orientation["schema"], "compass.orientation/2");
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
        let data = if matches!(
            tool,
            "search_symbols" | "get_callers" | "get_callees" | "get_impact"
        ) {
            assert_eq!(response["schema"], "compass.code_context.v1", "{tool}");
            assert_eq!(response["repository"], "sha256:test", "{tool}");
            assert_eq!(response["generation"], "sha256:test", "{tool}");
            assert_eq!(response["freshness"]["status"], "unknown", "{tool}");
            assert_eq!(response["truncation"]["next"], Value::Null, "{tool}");
            assert!(response.get("resultType").is_none(), "{tool}");
            assert_eq!(response, golden_result(tool)?, "{tool} golden result");
            assert_eq!(
                response["data"],
                pre_envelope_golden(tool)?,
                "{tool} pre-envelope payload"
            );
            &response["data"]
        } else {
            &response
        };
        assert_eq!(data["schema"], "compass.query/1", "{tool}");
        assert_eq!(data["operation"], operation, "{tool}");
        assert!(data["limits"]["maxNodes"].as_u64().is_some(), "{tool}");
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
    assert_eq!(orientation["schema"], "compass.orientation/2");
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
fn code_query_tool_schemas_are_closed_and_bounded() -> Result<(), Box<dyn Error>> {
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
        if matches!(
            tool.name.as_ref(),
            "search_symbols" | "get_callers" | "get_callees" | "get_impact"
        ) {
            let output = tool.output_schema.as_ref().ok_or("missing output schema")?;
            assert_eq!(
                output.get("additionalProperties"),
                Some(&Value::Bool(false))
            );
            assert_eq!(
                output
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|properties| properties.get("schema"))
                    .and_then(Value::as_object)
                    .and_then(|schema| schema.get("const")),
                Some(&json!("compass.code_context.v1"))
            );
            let golden = golden_result(tool.name.as_ref())?;
            validate_schema(
                &golden,
                &Value::Object(output.as_ref().clone()),
                &Value::Object(output.as_ref().clone()),
                "$",
            )?;
            let mut invalid = golden;
            invalid["data"]["nodes"] = json!([17]);
            assert!(
                validate_schema(
                    &invalid,
                    &Value::Object(output.as_ref().clone()),
                    &Value::Object(output.as_ref().clone()),
                    "$"
                )
                .is_err()
            );

            let mut detailed = golden_result(tool.name.as_ref())?;
            detailed["data"]["nodes"][0]["details"] = json!({
                "type": "symbol",
                "data": {"signature": "fn()", "modifiers": ["public"]}
            });
            if detailed["data"]["edges"]
                .as_array()
                .is_some_and(|edges| !edges.is_empty())
            {
                detailed["data"]["edges"][0]["details"] = json!({
                    "type": "call",
                    "data": {"dispatch": "static", "argumentCount": 0}
                });
            }
            validate_schema(
                &detailed,
                &Value::Object(output.as_ref().clone()),
                &Value::Object(output.as_ref().clone()),
                "$",
            )?;
            for invalid_details in [
                json!({"type":"unknown","data":{}}),
                json!({"type":"symbol","data":{"unexpected":true}}),
            ] {
                detailed["data"]["nodes"][0]["details"] = invalid_details;
                assert!(
                    validate_schema(
                        &detailed,
                        &Value::Object(output.as_ref().clone()),
                        &Value::Object(output.as_ref().clone()),
                        "$"
                    )
                    .is_err()
                );
            }
            detailed["data"]["nodes"][0]["details"] = Value::Null;
            detailed["data"]["nodes"][0]["kind"] = json!("unknown");
            assert!(
                validate_schema(
                    &detailed,
                    &Value::Object(output.as_ref().clone()),
                    &Value::Object(output.as_ref().clone()),
                    "$"
                )
                .is_err()
            );
            detailed["data"]["nodes"][0]["kind"] = json!("function");
            detailed["data"]["nodes"][0]["roles"] = json!(["unknown"]);
            assert!(
                validate_schema(
                    &detailed,
                    &Value::Object(output.as_ref().clone()),
                    &Value::Object(output.as_ref().clone()),
                    "$"
                )
                .is_err()
            );
            if detailed["data"]["edges"]
                .as_array()
                .is_some_and(|edges| !edges.is_empty())
            {
                detailed["data"]["nodes"][0]["roles"] = json!([]);
                for invalid_details in [
                    json!({"type":"unknown","data":{}}),
                    json!({"type":"call","data":{"dispatch":"static","unexpected":true}}),
                ] {
                    detailed["data"]["edges"][0]["details"] = invalid_details;
                    assert!(
                        validate_schema(
                            &detailed,
                            &Value::Object(output.as_ref().clone()),
                            &Value::Object(output.as_ref().clone()),
                            "$"
                        )
                        .is_err()
                    );
                }
                detailed["data"]["edges"][0]["details"] = Value::Null;
                detailed["data"]["edges"][0]["kind"] = json!("unknown");
                assert!(
                    validate_schema(
                        &detailed,
                        &Value::Object(output.as_ref().clone()),
                        &Value::Object(output.as_ref().clone()),
                        "$"
                    )
                    .is_err()
                );
            }
        } else {
            assert!(tool.output_schema.is_none());
        }
    }
    Ok(())
}

fn complete_node_details(anchor: &SourceAnchor) -> Vec<NodeDetails> {
    vec![
        NodeDetails::File(FileNodeDetails {
            content_digest: "sha256:file".to_owned(),
            byte_size: 42,
            generated: true,
        }),
        NodeDetails::Symbol(SymbolNodeDetails {
            signature: Some("fn example(value: usize)".to_owned()),
            modifiers: vec!["public".to_owned()],
            overload_discriminator: Some("usize".to_owned()),
            declaring_type: Some("Fixture".to_owned()),
            signature_digest: Some("sha256:signature".to_owned()),
            implementation_digest: Some("sha256:implementation".to_owned()),
            source_digest: Some("sha256:source".to_owned()),
        }),
        NodeDetails::ImportExport(ImportExportNodeDetails {
            specifier: "crate::fixture".to_owned(),
            imported_name: Some("Fixture".to_owned()),
            local_name: Some("LocalFixture".to_owned()),
            type_only: true,
        }),
        NodeDetails::Route(RouteNodeDetails {
            operation: "GET".to_owned(),
            path: "/fixture".to_owned(),
            original_path: Some("/fixture/:id".to_owned()),
            declaring_scope: "Fixture".to_owned(),
            resolution: ResolutionState::Ambiguous,
            middleware_count: 1,
            stages: vec![RouteStageDetails {
                stage: RouteStage::Middleware,
                position: 0,
                reference: "authenticate".to_owned(),
                resolution: ResolutionState::Ambiguous,
                source_anchor: Some(anchor.clone()),
                target: Some("n:middleware".to_owned()),
                candidates: vec![ResolutionCandidate {
                    node_id: "n:middleware".to_owned(),
                    reason: "fixture".to_owned(),
                    confidence: EvidenceConfidence::Ambiguous,
                    score: Some(0.5),
                    anchor: Some(anchor.clone()),
                }],
            }],
        }),
        NodeDetails::Component(ComponentNodeDetails {
            component_type: "view".to_owned(),
        }),
        NodeDetails::Resource(ResourceNodeDetails {
            resource_kind: ResourceKind::Document,
            uri: Some("https://example.invalid/fixture".to_owned()),
            media_type: Some("text/plain".to_owned()),
        }),
        NodeDetails::Messaging(MessagingNodeDetails {
            transport: "nats".to_owned(),
            subject: "fixture.created".to_owned(),
            declaring_scope: "Fixture".to_owned(),
        }),
        NodeDetails::Job(JobNodeDetails {
            schedule: Some("0 * * * *".to_owned()),
            queue: Some("fixture".to_owned()),
        }),
        NodeDetails::Schema(SchemaNodeDetails {
            dialect: Some("postgres".to_owned()),
            logical_database: Some("fixture".to_owned()),
            namespace: Some("public".to_owned()),
        }),
        NodeDetails::Query(QueryNodeDetails {
            dialect: Some("sql".to_owned()),
            operation: Some("select".to_owned()),
            text_digest: Some("sha256:query".to_owned()),
        }),
        NodeDetails::Config(ConfigNodeDetails {
            format: "toml".to_owned(),
            key_path: "fixture.enabled".to_owned(),
        }),
        NodeDetails::Database(DatabaseNodeDetails {
            logical_database: "fixture".to_owned(),
            database_schema: Some("public".to_owned()),
        }),
    ]
}

fn complete_edge_details() -> Vec<EdgeDetails> {
    vec![
        EdgeDetails::Call(CallEdgeDetails {
            dispatch: CallDispatch::Virtual,
            receiver_type: Some("Fixture".to_owned()),
            argument_count: Some(2),
        }),
        EdgeDetails::Route(RouteEdgeDetails {
            stage: RouteStage::Handler,
            position: Some(1),
            operation: Some("POST".to_owned()),
        }),
        EdgeDetails::Messaging(MessagingEdgeDetails {
            transport: "nats".to_owned(),
            subject: "fixture.created".to_owned(),
        }),
        EdgeDetails::Schedule(ScheduleEdgeDetails {
            expression: Some("0 * * * *".to_owned()),
        }),
        EdgeDetails::Mapping(MappingEdgeDetails {
            mapping_kind: "column".to_owned(),
        }),
    ]
}

#[test]
fn output_schema_covers_every_graph_enum_and_detail_variant() -> Result<(), Box<dyn Error>> {
    let tool = CompassMcp::tools()
        .into_iter()
        .find(|tool| tool.name == "get_callers")
        .ok_or("get_callers missing")?;
    let root = Value::Object(
        tool.output_schema
            .as_ref()
            .ok_or("get_callers output schema missing")?
            .as_ref()
            .clone(),
    );
    let mut result = golden_result("get_callers")?;

    for kind in NodeKind::ALL {
        result["data"]["nodes"][0]["kind"] = serde_json::to_value(kind)?;
        validate_schema(&result, &root, &root, "$")?;
    }
    result["data"]["nodes"][0]["kind"] = json!("function");
    for role in NodeRole::ALL {
        result["data"]["nodes"][0]["roles"] = json!([serde_json::to_value(role)?]);
        validate_schema(&result, &root, &root, "$")?;
    }
    result["data"]["nodes"][0]["roles"] = json!([]);
    for kind in EdgeKind::ALL {
        result["data"]["edges"][0]["kind"] = serde_json::to_value(kind)?;
        validate_schema(&result, &root, &root, "$")?;
    }
    result["data"]["edges"][0]["kind"] = json!("calls");

    let anchor = SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 4,
    };
    for details in complete_node_details(&anchor) {
        let serialized = serde_json::to_value(details)?;
        result["data"]["nodes"][0]["details"] = serialized.clone();
        validate_schema(&result, &root, &root, "$")?;

        let mut missing_data = serialized.clone();
        missing_data
            .as_object_mut()
            .ok_or("serialized node details are not an object")?
            .remove("data");
        result["data"]["nodes"][0]["details"] = missing_data;
        assert!(validate_schema(&result, &root, &root, "$").is_err());

        let mut unknown_field = serialized.clone();
        unknown_field["data"]["unexpected"] = Value::Bool(true);
        result["data"]["nodes"][0]["details"] = unknown_field;
        assert!(validate_schema(&result, &root, &root, "$").is_err());

        let mut unknown_tag = serialized;
        unknown_tag["type"] = json!("unknown");
        result["data"]["nodes"][0]["details"] = unknown_tag;
        assert!(validate_schema(&result, &root, &root, "$").is_err());
    }
    result["data"]["nodes"][0]["details"] = Value::Null;
    for details in complete_edge_details() {
        let serialized = serde_json::to_value(details)?;
        result["data"]["edges"][0]["details"] = serialized.clone();
        validate_schema(&result, &root, &root, "$")?;

        let mut missing_data = serialized.clone();
        missing_data
            .as_object_mut()
            .ok_or("serialized edge details are not an object")?
            .remove("data");
        result["data"]["edges"][0]["details"] = missing_data;
        assert!(validate_schema(&result, &root, &root, "$").is_err());

        let mut unknown_field = serialized.clone();
        unknown_field["data"]["unexpected"] = Value::Bool(true);
        result["data"]["edges"][0]["details"] = unknown_field;
        assert!(validate_schema(&result, &root, &root, "$").is_err());

        let mut unknown_tag = serialized;
        unknown_tag["type"] = json!("unknown");
        result["data"]["edges"][0]["details"] = unknown_tag;
        assert!(validate_schema(&result, &root, &root, "$").is_err());
    }
    Ok(())
}

#[test]
fn envelope_preserves_bounds_warnings_and_deterministic_discovery() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = write_typed_graph(directory.path())?;
    let server = CompassMcp::new(graph);
    let bounded = invoke(
        &server,
        "get_impact",
        json!({"symbol":"Target","max_nodes":1}),
    )?;
    assert_eq!(bounded["data"]["truncated"], true);
    assert_eq!(bounded["truncation"]["truncated"], true);
    assert_eq!(bounded["truncation"]["next"], Value::Null);
    assert_eq!(bounded["warnings"], bounded["data"]["diagnostics"]);
    assert!(bounded["warnings"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["code"] == "bounded_truncation")
    }));

    let first = CompassMcp::tools();
    let second = CompassMcp::tools();
    assert_eq!(
        first.iter().map(|tool| &tool.name).collect::<Vec<_>>(),
        second.iter().map(|tool| &tool.name).collect::<Vec<_>>()
    );
    assert_eq!(
        first
            .iter()
            .take(4)
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>(),
        vec!["search_symbols", "get_callers", "get_callees", "get_impact"]
    );
    for tool in &first {
        if matches!(
            tool.name.as_ref(),
            "get_neighbors"
                | "get_community"
                | "god_nodes"
                | "graph_stats"
                | "shortest_path"
                | "list_prs"
                | "get_pr_impact"
                | "triage_prs"
        ) {
            assert!(
                tool.description
                    .as_deref()
                    .is_some_and(|description| description.starts_with("DEPRECATED text result:"))
            );
            assert_eq!(
                tool.meta
                    .as_ref()
                    .and_then(|meta| meta.0.get("compass/deprecated")),
                Some(&Value::Bool(true))
            );
        }
    }
    let query_graph = first
        .iter()
        .find(|tool| tool.name == "query_graph")
        .ok_or("query_graph missing")?;
    assert_eq!(
        query_graph
            .meta
            .as_ref()
            .and_then(|meta| meta.0.get("compass/deprecatedTextMode")),
        Some(&Value::Bool(true))
    );

    let oversized = server.invoke(
        "search_symbols",
        json!({"query":"Target","max_response_bytes":1000})
            .as_object()
            .cloned()
            .ok_or("request is not an object")?,
    );
    assert!(
        oversized.contains("after MCP envelope encoding"),
        "{oversized}"
    );
    assert!(
        oversized.contains("query_response_too_large"),
        "{oversized}"
    );
    Ok(())
}

#[test]
fn envelope_preserves_parallel_edge_occurrences_against_pre_envelope_golden()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = write_parallel_call_graph(directory.path())?;
    let response = invoke(
        &CompassMcp::new(graph),
        "get_callers",
        json!({"symbol":"Target"}),
    )?;
    assert_eq!(response["data"], parallel_callers_golden()?);
    let edges = response["data"]["edges"]
        .as_array()
        .ok_or("parallel edge result is not an array")?;
    assert_eq!(edges.len(), 2);
    assert_ne!(edges[0]["id"], edges[1]["id"]);
    assert_ne!(edges[0]["relationshipSite"], edges[1]["relationshipSite"]);
    assert_ne!(edges[0]["evidence"], edges[1]["evidence"]);
    Ok(())
}

#[test]
fn envelope_preserves_ambiguity_without_inventing_a_target() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = write_typed_graph(directory.path())?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let mut duplicate = graph
        .nodes
        .iter()
        .find(|node| node.name == "Target")
        .cloned()
        .ok_or("fixture target missing")?;
    duplicate.id = "n:other-target".to_owned();
    duplicate.qualified_name = "Other.Target".to_owned();
    graph.nodes.push(duplicate);
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let response = invoke(
        &CompassMcp::new(graph_path),
        "get_callers",
        json!({"symbol":"Target"}),
    )?;
    assert!(
        response["data"]["nodes"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(
        response["data"]["edges"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert_eq!(response["warnings"], response["data"]["diagnostics"]);
    assert!(
        response["warnings"]
            .as_array()
            .is_some_and(|items| { items.iter().any(|item| item["code"] == "ambiguous_match") })
    );
    Ok(())
}

#[test]
fn envelope_rejects_an_empty_graph_identity() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = write_typed_graph(directory.path())?;
    let mut graph = GraphDocument::load(&graph_path)?;
    graph.graph.build.source_tree_digest.clear();
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let output = CompassMcp::new(graph_path).invoke(
        "search_symbols",
        json!({"query":"Target"})
            .as_object()
            .cloned()
            .ok_or("request is not an object")?,
    );
    assert!(
        output.contains("graph.build.sourceTreeDigest must not be empty"),
        "{output}"
    );
    assert!(serde_json::from_str::<Value>(&output).is_err(), "{output}");
    Ok(())
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

#[test]
fn discovery_with_a_store_reference_bypasses_eager_json_graph_loading() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    fs::write(&graph, b"not a graph")?;
    fs::write(directory.path().join("store.ref"), b"not a store reference")?;

    let output = CompassMcp::new(graph).invoke(
        "query_graph",
        Map::from_iter([("question".to_owned(), json!("where is Target"))]),
    );

    assert!(output.contains("store_ref_decode_failed"), "{output}");
    Ok(())
}

#[test]
fn typed_store_tools_do_not_load_the_compatibility_json_graph() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = write_typed_graph(directory.path())?;
    publish_store(directory.path(), &graph)?;
    fs::write(&graph, b"not a graph")?;
    let server = CompassMcp::new(graph);

    let search: Value = serde_json::from_str(&server.invoke(
        "search_symbols",
        Map::from_iter([("query".to_owned(), json!("Target"))]),
    ))?;
    assert_eq!(search["data"]["operation"], "search");
    let discovery: Value = serde_json::from_str(&server.invoke(
        "query_graph",
        Map::from_iter([("question".to_owned(), json!("where is Target"))]),
    ))?;
    assert_eq!(discovery["result"]["schema"], "compass.query.discovery/1");
    let response: compass_model::query_contract::DiscoveryQueryResponse =
        serde_json::from_value(discovery["result"].clone())?;
    assert_eq!(
        discovery["semanticResultDigest"],
        format!(
            "sha256:{}",
            compass_query::discovery_response_digest(&response)?
        )
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
    let client_info = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("compass-code-query-test", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28);
    let client = client_info
        .serve_with_lifecycle(
            client_transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await?;
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
        Some("compass.code_context.v1")
    );
    assert_eq!(response.result_type, Some(ResultType::COMPLETE));
    assert_eq!(
        response
            .structured_content
            .as_ref()
            .and_then(|value| value.get("data"))
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
    let oversized = match client
        .call_tool(
            CallToolRequestParams::new("search_symbols").with_arguments(Map::from_iter([
                ("query".to_owned(), json!("Target")),
                ("max_response_bytes".to_owned(), json!(1000)),
            ])),
        )
        .await
    {
        Ok(response) => {
            return Err(
                format!("final MCP envelope ignored max_response_bytes: {response:?}").into(),
            );
        }
        Err(error) => error,
    };
    assert!(
        oversized.to_string().contains("query_response_too_large"),
        "{oversized}"
    );
    client.cancel().await?;
    server_task.await?.map_err(std::io::Error::other)?;
    Ok(())
}
