use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use compass_analysis::{
    AnalysisBundle, CallGraphDirection, CallResolution, UNIVERSAL_CALL_GRAPH_SCHEMA,
    UniversalCallGraphRequest, UniversalCallGraphRoot, build_universal_call_graph,
};
use compass_ir::{
    BasicBlock, FunctionIr, ModuleIr, Operation, OperationKind, ProgramBundle, SourceAnchor,
    Terminator,
};
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use serde_json::{Map, Value, json};

#[test]
fn graph_only_go_resolves_the_cursor_and_traces_callees() -> Result<(), Box<dyn std::error::Error>>
{
    let graph = go_graph();
    let response = build_universal_call_graph(
        &graph,
        None,
        &UniversalCallGraphRequest {
            root: UniversalCallGraphRoot::SourcePosition {
                file: "cmd/entire/cli/auth/control_plane.go".to_owned(),
                byte: 1_683,
                line: 42,
            },
            direction: CallGraphDirection::Callees,
            depth: 1,
            max_nodes: 20,
            max_edges: 20,
        },
    )?;

    assert_eq!(response.schema, UNIVERSAL_CALL_GRAPH_SCHEMA);
    assert_eq!(response.root_symbol, "resolve");
    assert_eq!(
        response
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["active", "resolve", "target"])
    );
    assert_eq!(response.edges.len(), 2);
    assert!(response.edges.iter().all(|edge| edge.source == "resolve"));
    assert_eq!(response.coverage.resolved, 2);
    assert_eq!(response.coverage.inferred, 0);
    assert_eq!(response.coverage.evidence_layer, "structural_graph");
    assert!(response.coverage.partial);
    Ok(())
}

#[test]
fn graph_only_direction_and_confidence_are_preserved() -> Result<(), Box<dyn std::error::Error>> {
    let graph = go_graph();
    let callers = build_universal_call_graph(
        &graph,
        None,
        &UniversalCallGraphRequest {
            root: UniversalCallGraphRoot::Symbol {
                symbol: "resolve".to_owned(),
            },
            direction: CallGraphDirection::Callers,
            depth: 1,
            max_nodes: 20,
            max_edges: 20,
        },
    )?;

    assert_eq!(
        callers
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["entry", "resolve"])
    );
    assert_eq!(callers.edges.len(), 1);
    assert_eq!(callers.edges[0].resolution, CallResolution::Inferred);
    assert_eq!(callers.coverage.inferred, 1);
    assert_eq!(callers.edges[0].call_sites[0].line, Some(12));
    Ok(())
}

#[test]
fn source_resolution_selects_the_smallest_containing_callable()
-> Result<(), Box<dyn std::error::Error>> {
    let mut graph = go_graph();
    graph.nodes.push(node(
        "nested",
        "nested()",
        "cmd/entire/cli/auth/control_plane.go",
        "function",
        44,
        48,
    ));

    let response = build_universal_call_graph(
        &graph,
        None,
        &UniversalCallGraphRequest {
            root: UniversalCallGraphRoot::SourcePosition {
                file: "cmd\\entire\\cli\\auth\\control_plane.go".to_owned(),
                byte: 1_800,
                line: 45,
            },
            direction: CallGraphDirection::Both,
            depth: 1,
            max_nodes: 20,
            max_edges: 20,
        },
    )?;

    assert_eq!(response.root_symbol, "nested");
    assert_eq!(response.nodes.len(), 1);
    Ok(())
}

#[test]
fn graph_v1_source_anchors_resolve_the_cursor() -> Result<(), Box<dyn std::error::Error>> {
    let graph = serde_json::from_value(json!({
        "directed": true,
        "multigraph": true,
        "graph": { "schema": "compass.graph/1" },
        "nodes": [
            {
                "id": "root",
                "kind": "function",
                "name": "root",
                "qualifiedName": "example::root",
                "source": {
                    "file": "src/lib.rs",
                    "startByte": 100,
                    "endByte": 220,
                    "startLine": 10,
                    "startColumn": 0,
                    "endLine": 18,
                    "endColumn": 1
                }
            },
            {
                "id": "callee",
                "kind": "function",
                "name": "callee",
                "qualifiedName": "example::callee",
                "source": {
                    "file": "src/lib.rs",
                    "startByte": 300,
                    "endByte": 360,
                    "startLine": 24,
                    "startColumn": 0,
                    "endLine": 28,
                    "endColumn": 1
                }
            }
        ],
        "links": [{
            "id": "root-calls-callee",
            "key": "root-calls-callee",
            "source": "root",
            "target": "callee",
            "kind": "calls",
            "relationshipSite": {
                "file": "src/lib.rs",
                "startByte": 160,
                "endByte": 168,
                "startLine": 14,
                "startColumn": 4,
                "endLine": 14,
                "endColumn": 12
            }
        }]
    }))?;

    let response = build_universal_call_graph(
        &graph,
        None,
        &UniversalCallGraphRequest {
            root: UniversalCallGraphRoot::SourcePosition {
                file: "src/lib.rs".to_owned(),
                byte: 165,
                line: 14,
            },
            direction: CallGraphDirection::Callees,
            depth: 1,
            max_nodes: 20,
            max_edges: 20,
        },
    )?;

    assert_eq!(response.root_symbol, "root");
    assert_eq!(response.nodes.len(), 2);
    assert_eq!(response.edges.len(), 1);
    assert_eq!(response.nodes[0].start_byte, Some(300));
    assert_eq!(response.nodes[1].start_byte, Some(100));
    assert_eq!(response.edges[0].call_sites[0].start_byte, Some(160));
    Ok(())
}

#[test]
fn declaration_only_ranges_keep_source_driven_languages_cursor_capable()
-> Result<(), Box<dyn std::error::Error>> {
    for language in ["groovy", "zig", "r", "pascal", "dart"] {
        let file = format!("src/example.{language}");
        let graph = GraphDocument {
            directed: true,
            multigraph: true,
            graph: Map::new(),
            nodes: vec![
                node_for_language("first", "first()", &file, "function", language, 4, 4),
                node_for_language("root", "root()", &file, "function", language, 20, 20),
                node_for_language("callee", "callee()", &file, "function", language, 40, 40),
            ],
            links: vec![EdgeRecord {
                source: "root".to_owned(),
                target: "callee".to_owned(),
                attributes: object(json!({
                    "relation": "calls",
                    "source_file": file,
                    "source_location": "L25",
                    "confidence": "EXTRACTED"
                })),
            }],
            extras: BTreeMap::new(),
        };
        let response = build_universal_call_graph(
            &graph,
            None,
            &UniversalCallGraphRequest {
                root: UniversalCallGraphRoot::SourcePosition {
                    file,
                    byte: 200,
                    line: 25,
                },
                direction: CallGraphDirection::Callees,
                depth: 1,
                max_nodes: 20,
                max_edges: 20,
            },
        )?;

        assert_eq!(response.root_symbol, "root", "{language}");
        assert_eq!(response.edges.len(), 1, "{language}");
        assert!(
            response
                .coverage
                .limitations
                .iter()
                .any(|limitation| limitation == "approximate_callable_range"),
            "{language}"
        );
    }
    Ok(())
}

#[test]
fn program_ir_enriches_structural_edges_and_retains_unresolved_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let graph = go_graph();
    let analysis = program_analysis();
    let response = build_universal_call_graph(
        &graph,
        Some(&analysis),
        &UniversalCallGraphRequest {
            root: UniversalCallGraphRoot::Symbol {
                symbol: "resolve".to_owned(),
            },
            direction: CallGraphDirection::Callees,
            depth: 1,
            max_nodes: 20,
            max_edges: 20,
        },
    )?;

    assert_eq!(response.coverage.evidence_layer, "combined");
    assert_eq!(response.coverage.unresolved, 1);
    assert_eq!(
        response
            .nodes
            .iter()
            .find(|node| node.id == "resolve")
            .and_then(|node| node.symbol.as_deref()),
        Some("program:resolve")
    );
    let active = response
        .edges
        .iter()
        .find(|edge| edge.target == "active")
        .ok_or("missing active call")?;
    assert_eq!(active.evidence_layer, "combined");
    assert!(
        active
            .call_sites
            .iter()
            .any(|site| site.start_byte == Some(1_700))
    );
    assert!(
        response
            .edges
            .iter()
            .any(|edge| edge.resolution == CallResolution::Unresolved)
    );
    Ok(())
}

#[test]
fn high_fanout_traversal_stays_within_the_interactive_latency_budget()
-> Result<(), Box<dyn std::error::Error>> {
    const FANOUT: usize = 16_000;
    let file = "src/hot_path.rs";
    let mut graph = GraphDocument {
        directed: true,
        multigraph: true,
        graph: Map::new(),
        nodes: Vec::with_capacity(FANOUT + 1),
        links: Vec::with_capacity(FANOUT),
        extras: BTreeMap::new(),
    };
    graph
        .nodes
        .push(node("root", "root()", file, "function", 1, 3));
    for index in 0..FANOUT {
        let id = format!("callee-{index:05}");
        graph.nodes.push(node(
            &id,
            &format!("callee_{index}()"),
            file,
            "function",
            5,
            7,
        ));
        graph.links.push(edge("root", &id, "L2", "EXTRACTED"));
    }

    let started = Instant::now();
    let response = build_universal_call_graph(
        &graph,
        None,
        &UniversalCallGraphRequest {
            root: UniversalCallGraphRoot::Symbol {
                symbol: "root".to_owned(),
            },
            direction: CallGraphDirection::Callees,
            depth: 2,
            max_nodes: 128,
            max_edges: 256,
        },
    )?;
    let elapsed = started.elapsed();

    assert!(response.nodes.len() <= 128);
    assert!(response.edges.len() <= 256);
    assert!(response.continuations.len() <= 128);
    assert!(response.truncated);
    assert!(
        elapsed < Duration::from_millis(1_500),
        "bounded call-graph traversal took {elapsed:?}"
    );
    Ok(())
}

fn go_graph() -> GraphDocument {
    let file = "cmd/entire/cli/auth/control_plane.go";
    GraphDocument {
        directed: true,
        multigraph: true,
        graph: Map::new(),
        nodes: vec![
            node("entry", "main()", file, "function", 8, 20),
            node(
                "resolve",
                "ResolveControlPlaneTarget()",
                file,
                "function",
                42,
                55,
            ),
            node("target", "targetForContext()", file, "function", 92, 98),
            node("active", "activeContext()", file, "function", 104, 114),
        ],
        links: vec![
            edge("entry", "resolve", "L12", "INFERRED"),
            edge("resolve", "active", "L43", "EXTRACTED"),
            edge("resolve", "target", "L54", "EXTRACTED"),
        ],
        extras: BTreeMap::new(),
    }
}

fn program_analysis() -> AnalysisBundle {
    let file = "cmd/entire/cli/auth/control_plane.go";
    let call = |ordinal, start_byte, callee: &str, resolved_symbols: Vec<String>| Operation {
        ordinal,
        anchor: SourceAnchor {
            source_file: file.to_owned(),
            start_byte,
            end_byte: start_byte + 5,
        },
        evidence: vec![format!("evidence:{ordinal}")],
        kind: OperationKind::Call {
            callee: callee.to_owned(),
            callee_anchor: SourceAnchor {
                source_file: file.to_owned(),
                start_byte,
                end_byte: start_byte + 5,
            },
            resolved_symbols,
            receiver_type: None,
        },
    };
    let function =
        |symbol: &str, name: &str, graph_node_id: &str, start_byte, end_byte, operations| {
            FunctionIr {
                symbol_id: symbol.to_owned(),
                name: name.to_owned(),
                graph_node_id: Some(graph_node_id.to_owned()),
                signature_digest: format!("signature:{symbol}"),
                body_digest: format!("body:{symbol}"),
                visibility: compass_ir::Visibility::Public,
                execution_mode: compass_ir::ExecutionMode::Sync,
                is_test: false,
                anchor: SourceAnchor {
                    source_file: file.to_owned(),
                    start_byte,
                    end_byte,
                },
                parameters: Vec::new(),
                return_type: None,
                blocks: vec![BasicBlock {
                    id: 0,
                    operations,
                    terminator: Terminator::Return { value: None },
                    evidence: Vec::new(),
                }],
                coverage: BTreeMap::new(),
                evidence: Vec::new(),
            }
        };
    AnalysisBundle {
        analysis_schema_version: 1,
        analyzer_version: 1,
        program: ProgramBundle {
            schema: compass_ir::PROGRAM_SCHEMA.to_owned(),
            providers: Vec::new(),
            evidence: Vec::new(),
            modules: vec![ModuleIr {
                source_file: file.to_owned(),
                language: "go".to_owned(),
                source_digest: "source".to_owned(),
                graph_node_id: None,
                functions: vec![
                    function(
                        "program:resolve",
                        "ResolveControlPlaneTarget",
                        "resolve",
                        1_650,
                        2_100,
                        vec![
                            call(0, 1_700, "activeContext", vec!["program:active".to_owned()]),
                            call(1, 1_750, "dynamicTarget", Vec::new()),
                        ],
                    ),
                    function(
                        "program:active",
                        "activeContext",
                        "active",
                        4_000,
                        4_300,
                        Vec::new(),
                    ),
                ],
                coverage: BTreeMap::new(),
                evidence: Vec::new(),
            }],
        },
        summaries: Vec::new(),
        reverse_calls: BTreeMap::new(),
    }
}

fn node(
    id: &str,
    label: &str,
    file: &str,
    symbol_kind: &str,
    line_start: u64,
    line_end: u64,
) -> NodeRecord {
    node_for_language(id, label, file, symbol_kind, "go", line_start, line_end)
}

fn node_for_language(
    id: &str,
    label: &str,
    file: &str,
    symbol_kind: &str,
    language: &str,
    line_start: u64,
    line_end: u64,
) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        attributes: object(json!({
            "label": label,
            "source_file": file,
            "source_location": format!("L{line_start}"),
            "symbol_kind": symbol_kind,
            "language": language,
            "line_start": line_start,
            "line_end": line_end
        })),
    }
}

fn edge(source: &str, target: &str, source_location: &str, confidence: &str) -> EdgeRecord {
    EdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes: object(json!({
            "relation": "calls",
            "source_file": "cmd/entire/cli/auth/control_plane.go",
            "source_location": source_location,
            "confidence": confidence
        })),
    }
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}
