mod support;

use std::error::Error;
use std::ffi::OsString;

use compass_cli::{Frontend, run};
use compass_files::BuildGuard;
use compass_graph::GraphSnapshotBuilder;
use compass_model::code_graph::{EdgeKind, GraphDocument, NodeKind};
use compass_output::{AgentOperation, AgentQueryView};
use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore};
use serde_json::Value;

#[test]
fn typed_query_commands_share_the_versioned_json_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let cache = directory.path().join("cache");
    let graph = graph.to_string_lossy().into_owned();
    let cache = cache.to_string_lossy().into_owned();
    let root = directory.path().to_string_lossy().into_owned();
    for (command, positional, operation) in [
        ("ask", vec!["who calls Target?"], "callers"),
        ("search", vec!["Target"], "search"),
        ("callers", vec!["Target"], "callers"),
        ("callees", vec!["Caller"], "callees"),
        ("impact", vec!["Target"], "impact"),
        ("explore", vec!["Caller", "Target"], "explore"),
        ("node", vec!["Caller", "Target"], "node_trail"),
    ] {
        let mut args = vec![OsString::from(command)];
        args.extend(positional.into_iter().map(OsString::from));
        args.extend([
            OsString::from("--graph"),
            OsString::from(&graph),
            OsString::from("--cache"),
            OsString::from(&cache),
            OsString::from("--root"),
            OsString::from(&root),
            OsString::from("--format"),
            OsString::from("json"),
        ]);
        let outcome = run(Frontend::Compass, args);
        assert_eq!(outcome.code, 0, "{command}: {}", outcome.stderr);
        let response: Value = serde_json::from_str(&outcome.stdout)?;
        assert_eq!(response["schema"], "compass.query/1");
        assert_eq!(response["operation"], operation);
    }

    let agent = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            OsString::from(&graph),
            OsString::from("--format"),
            OsString::from("agent-json"),
        ],
    );
    assert_eq!(agent.code, 0, "{}", agent.stderr);
    let agent_view = AgentQueryView::from_json(agent.stdout.as_bytes())?;
    assert_eq!(agent_view.schema, "compass.query.agent-view/1");
    assert!(agent_view.answer.headline.contains("Target"));

    let reverse = run(
        Frontend::Compass,
        [
            OsString::from("node"),
            OsString::from("Target"),
            OsString::from("Caller"),
            OsString::from("--graph"),
            OsString::from(&graph),
            OsString::from("--cache"),
            OsString::from(&cache),
            OsString::from("--root"),
            OsString::from(&root),
            OsString::from("--format"),
            OsString::from("json"),
        ],
    );
    assert_eq!(reverse.code, 0, "{}", reverse.stderr);
    let response: Value = serde_json::from_str(&reverse.stdout)?;
    assert_eq!(response["paths"], serde_json::json!([]));
    assert_eq!(response["diagnostics"][0]["code"], "direction_mismatch");

    let invalid_projection = run(
        Frontend::Compass,
        [
            OsString::from("callers"),
            OsString::from("Target"),
            OsString::from("--graph"),
            OsString::from(&graph),
            OsString::from("--format"),
            OsString::from("agent-json"),
            OsString::from("--evidence"),
        ],
    );
    assert_ne!(invalid_projection.code, 0);
    assert!(invalid_projection.stderr.contains("text-only"));
    Ok(())
}

#[test]
fn affected_typed_graph_uses_shared_relationship_output_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let graph = graph.into_os_string();
    for format in ["text", "json", "agent-json"] {
        let outcome = run(
            Frontend::Compass,
            [
                OsString::from("affected"),
                OsString::from("Target"),
                OsString::from("--relation"),
                OsString::from("calls"),
                OsString::from("--graph"),
                graph.clone(),
                OsString::from("--format"),
                OsString::from(format),
            ],
        );
        assert_eq!(outcome.code, 0, "{format}: {}", outcome.stderr);
        if format == "json" {
            let value: Value = serde_json::from_str(&outcome.stdout)?;
            assert_eq!(value["operation"], "impact");
        } else if format == "agent-json" {
            let view = AgentQueryView::from_json(outcome.stdout.as_bytes())?;
            assert_eq!(view.request.operation, AgentOperation::Impact);
        } else {
            assert!(outcome.stdout.contains("Target"), "{}", outcome.stdout);
        }
    }
    Ok(())
}

#[test]
fn architecture_command_is_bounded_and_agent_readable() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let output = run(
        Frontend::Compass,
        [
            OsString::from("architecture"),
            OsString::from("--graph"),
            graph.into_os_string(),
            OsString::from("--format"),
            OsString::from("agent-json"),
        ],
    );
    assert_eq!(output.code, 0, "{}", output.stderr);
    let value: Value = serde_json::from_str(&output.stdout)?;
    assert_eq!(value["schema"], "compass.architecture.agent-view/1");
    assert!(value["answer"].as_str().is_some());
    Ok(())
}

#[test]
fn architecture_command_returns_a_sampled_summary_above_its_detail_budget()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = support::write_typed_module_graph(directory.path())?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let template = graph
        .nodes
        .iter()
        .find(|node| node.id == "n:a")
        .cloned()
        .ok_or("missing module node")?;
    for index in 0..5_001 {
        let mut node = template.clone();
        node.id = format!("n:summary-node-{index:05}");
        node.kind = NodeKind::Function;
        node.name = format!("summary_node_{index}");
        node.qualified_name = format!("fixture::summary_node_{index}");
        graph.nodes.push(node);
    }
    graph.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    std::fs::write(&graph_path, serde_json::to_vec(&graph)?)?;

    let output = run(
        Frontend::Compass,
        [
            OsString::from("architecture"),
            OsString::from("--graph"),
            graph_path.as_os_str().to_owned(),
            OsString::from("--format"),
            OsString::from("json"),
        ],
    );
    assert_eq!(output.code, 0, "{}", output.stderr);
    let value: Value = serde_json::from_str(&output.stdout)?;
    assert_eq!(value["schema"], "compass.architecture.summary/1");
    assert_eq!(value["detailsOmitted"], true);
    assert_eq!(value["limitHit"]["name"], "max_nodes");
    assert_eq!(value["limitHit"]["required"], 5_005);
    assert_eq!(value["limitHit"]["limit"], 5_000);
    assert_eq!(value["statistics"]["nodes"], 5_005);
    assert!(value["sampledCommunities"].is_array());
    assert!(value.get("nodes").is_none());
    Ok(())
}

#[test]
fn typed_ask_rejects_conflicting_current_and_revision_graph_sources() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let outcome = run(
        Frontend::Compass,
        [
            OsString::from("ask"),
            OsString::from("who calls Target?"),
            OsString::from("--graph"),
            graph.into_os_string(),
            OsString::from("--at"),
            OsString::from("HEAD"),
            OsString::from("--format"),
            OsString::from("json"),
        ],
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "error: --graph and --at are mutually exclusive"
    );
    for option in ["--engine", "--program", "--cache"] {
        let outcome = run(
            Frontend::Compass,
            [
                OsString::from("ask"),
                OsString::from("who calls Target?"),
                OsString::from("--at"),
                OsString::from("HEAD"),
                OsString::from(option),
                OsString::from("ignored"),
                OsString::from("--format"),
                OsString::from("json"),
            ],
        );
        assert_eq!(outcome.code, 1);
        assert_eq!(
            outcome.stderr,
            format!("error: {option} cannot be combined with --at")
        );
    }
    Ok(())
}

#[test]
fn natural_query_defaults_to_discovery_and_preserves_explicit_legacy_traversal()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;

    for (question, expected_node) in [
        ("who calls Target?", "Fixture.Caller"),
        ("what does Caller call?", "Fixture.Target"),
        ("what depends on Target?", "Fixture.Caller"),
        ("path from Caller to Target", "Fixture.Target"),
        ("where is Target defined?", "Fixture.Target"),
    ] {
        let outcome = run(
            Frontend::Compass,
            [
                OsString::from("query"),
                OsString::from(question),
                OsString::from("--graph"),
                graph.clone().into_os_string(),
            ],
        );
        assert_eq!(outcome.code, 0, "{question}: {}", outcome.stderr);
        assert!(outcome.stdout.starts_with("RESULT "), "{question}");
        assert!(outcome.stdout.contains("ANSWER\n"), "{question}");
        assert!(
            outcome.stdout.contains(expected_node),
            "{question}: {}",
            outcome.stdout
        );
        assert!(
            outcome.stdout.contains("CAVEATS")
                || outcome.stdout.contains("PRIMARY RESULTS")
                || outcome.stdout.contains("NODE "),
            "{question}"
        );
        assert!(outcome.stdout.contains("Pagination:"), "{question}");
    }

    for question in ["authentication flow", "where is authentication enforced?"] {
        let generic = run(
            Frontend::Compass,
            [
                OsString::from("query"),
                OsString::from(question),
                OsString::from("--graph"),
                graph.clone().into_os_string(),
            ],
        );
        assert_eq!(generic.code, 0, "{}", generic.stderr);
        assert!(generic.stdout.starts_with("RESULT "), "{question}");
        assert!(generic.stdout.contains("ANSWER\n"), "{question}");
        assert!(generic.stdout.contains("Completeness:"), "{question}");
    }

    for arguments in [
        vec!["query", "who calls Target?", "--traverse"],
        vec!["query", "who calls Target?", "--budget", "2000"],
    ] {
        let description = format!("{arguments:?}");
        let mut args = arguments
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        args.extend([OsString::from("--graph"), graph.clone().into_os_string()]);
        let outcome = run(Frontend::Compass, args);
        assert_eq!(outcome.code, 0, "{}", outcome.stderr);
        assert!(
            outcome.stdout.contains("Pagination:"),
            "arguments={description} stdout={}",
            outcome.stdout
        );
    }

    Ok(())
}

#[test]
fn discovery_cursor_survives_budget_alias_and_scope_order_but_rejects_graph_change()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let mut document = GraphDocument::load(&graph)?;
    document.links[0].context = Some("call".to_owned());
    let template = document.nodes[1].clone();
    for index in 0..40 {
        let mut alternative = template.clone();
        alternative.id = format!("n:target-alternative-{index}");
        document.nodes.push(alternative);
    }
    document.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    std::fs::write(&graph, serde_json::to_vec_pretty(&document)?)?;

    let first = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph.clone().into_os_string(),
            OsString::from("--text-budget"),
            OsString::from("500"),
            OsString::from("--context"),
            OsString::from("calls"),
            OsString::from("--context"),
            OsString::from("import"),
            OsString::from("--scope"),
            OsString::from("node:n:target"),
            OsString::from("--scope"),
            OsString::from("source:src"),
        ],
    );
    assert_eq!(first.code, 0, "{}", first.stderr);
    let cursor = first
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("Pagination: "))
        .and_then(|line| line.split(" next=").nth(1))
        .filter(|cursor| *cursor != "none")
        .ok_or("expected discovery continuation cursor")?
        .to_owned();

    let continued = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph.clone().into_os_string(),
            OsString::from("--text-budget"),
            OsString::from("1000"),
            OsString::from("--cursor"),
            OsString::from(&cursor),
            OsString::from("--context"),
            OsString::from("import"),
            OsString::from("--context"),
            OsString::from("call"),
            OsString::from("--scope"),
            OsString::from("source:src"),
            OsString::from("--scope"),
            OsString::from("node:n:target"),
        ],
    );
    assert_eq!(continued.code, 0, "{}", continued.stderr);
    assert!(continued.stdout.starts_with("RESULT "));
    assert!(
        continued.stdout.contains("Pagination:"),
        "{}",
        continued.stdout
    );

    document.nodes[0].qualified_name.push_str(".changed");
    std::fs::write(&graph, serde_json::to_vec_pretty(&document)?)?;
    let changed = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph.into_os_string(),
            OsString::from("--text-budget"),
            OsString::from("1000"),
            OsString::from("--cursor"),
            OsString::from(cursor),
            OsString::from("--context"),
            OsString::from("call"),
            OsString::from("--context"),
            OsString::from("import"),
            OsString::from("--scope"),
            OsString::from("node:n:target"),
            OsString::from("--scope"),
            OsString::from("source:src"),
        ],
    );
    assert_ne!(changed.code, 0);
    assert!(changed.stderr.contains("selected graph generation"));
    Ok(())
}

#[test]
fn natural_discovery_exposes_the_public_json_contract_and_repeatable_or_scopes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let graph_for_agent = graph.clone();
    let mut document = GraphDocument::load(&graph)?;
    document.links[0].context = Some("call".to_owned());
    std::fs::write(&graph, serde_json::to_vec_pretty(&document)?)?;
    let outcome = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph.into_os_string(),
            OsString::from("--direction"),
            OsString::from("incoming"),
            OsString::from("--scope"),
            OsString::from("node:n:caller"),
            OsString::from("--scope=node:n:target"),
            OsString::from("--context"),
            OsString::from("call"),
            OsString::from("--dfs"),
            OsString::from("--format=json"),
        ],
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let response: Value = serde_json::from_str(&outcome.stdout)?;
    assert_eq!(response["schema"], "compass.query.discovery/1");
    assert_eq!(response["selectedDirection"], "incoming");
    assert_eq!(response["directionSource"], "explicit");
    assert_eq!(response["relationContexts"], serde_json::json!(["call"]));
    assert_eq!(response["traversal"], "dfs");
    assert_eq!(
        response["scope"],
        serde_json::json!([
            {"kind": "node", "value": "n:caller"},
            {"kind": "node", "value": "n:target"}
        ])
    );
    assert_eq!(response["seeds"][0]["nodeId"], "n:target");
    assert_eq!(response["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(response["edges"].as_array().map(Vec::len), Some(1));

    let agent = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_for_agent.into_os_string(),
            OsString::from("--format=agent-json"),
        ],
    );
    assert_eq!(agent.code, 0, "{}", agent.stderr);
    let agent_view = AgentQueryView::from_json(agent.stdout.as_bytes())?;
    assert_eq!(agent_view.schema, "compass.query.agent-view/1");
    assert!(!agent_view.primary_results.is_empty());
    Ok(())
}

#[test]
fn natural_discovery_result_envelope_is_opt_in_typed_and_digest_stable()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let base_arguments = [
        OsString::from("query"),
        OsString::from("Target"),
        OsString::from("--graph"),
        graph.into_os_string(),
        OsString::from("--format=json"),
    ];
    let direct = run(Frontend::Compass, base_arguments.clone());
    assert_eq!(direct.code, 0, "{}", direct.stderr);
    let direct_value: Value = serde_json::from_str(&direct.stdout)?;
    assert_eq!(direct_value["schema"], "compass.query.discovery/1");
    assert!(direct_value.get("semanticResultDigest").is_none());

    let mut envelope_arguments = base_arguments.to_vec();
    envelope_arguments.push(OsString::from("--result-envelope"));
    let enveloped = run(Frontend::Compass, envelope_arguments);
    assert_eq!(enveloped.code, 0, "{}", enveloped.stderr);
    let envelope: compass_model::query_contract::DiscoveryResultEnvelope =
        serde_json::from_str(&enveloped.stdout)?;
    envelope.validate().map_err(std::io::Error::other)?;
    assert_eq!(serde_json::to_value(&envelope.result)?, direct_value);
    assert_eq!(
        envelope.semantic_result_digest,
        format!(
            "sha256:{}",
            compass_query::discovery_response_digest(&envelope.result)?
        )
    );
    let mut invalid_schema = envelope.clone();
    invalid_schema.schema = "compass.query.discovery-result/2".to_owned();
    assert!(invalid_schema.validate().is_err());
    let mut invalid_digest = envelope.clone();
    invalid_digest.semantic_result_digest = "sha256:not-a-digest".to_owned();
    assert!(invalid_digest.validate().is_err());
    assert!(
        serde_json::from_value::<compass_model::query_contract::DiscoveryResultEnvelope>(
            serde_json::json!({
                "schema": "compass.query.discovery-result/2",
                "result": envelope.result,
                "semanticResultDigest": envelope.semantic_result_digest,
                "unknown": true,
            })
        )
        .is_err()
    );

    let invalid = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            directory.path().join("graph.json").into_os_string(),
            OsString::from("--result-envelope"),
        ],
    );
    assert_ne!(invalid.code, 0);
    assert!(invalid.stderr.contains("requires --format json"));
    Ok(())
}

#[test]
fn natural_discovery_rejects_invalid_duplicate_and_mixed_public_controls()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let graph = graph.to_string_lossy().into_owned();
    for (arguments, expected) in [
        (
            vec!["--direction", "sideways"],
            "--direction must be auto, incoming, outgoing, or both",
        ),
        (vec!["--scope", "Target"], "--scope must use kind:value"),
        (
            vec!["--scope", "guessed:Target"],
            "--scope kind must be community, source, package, or node",
        ),
        (vec!["--scope", "node:"], "--scope value must not be empty"),
        (
            vec!["--direction", "both", "--context", "subsystem"],
            "unsupported relationship context",
        ),
        (
            vec!["--direction", "both", "--direction", "incoming"],
            "--direction must not be repeated",
        ),
        (
            vec!["--max-nodes", "2", "--max-nodes=3"],
            "--max-nodes must not be repeated",
        ),
        (
            vec!["--include-heuristic", "--include-heuristic"],
            "--include-heuristic must not be repeated",
        ),
        (
            vec!["--direction", "both", "--traverse"],
            "legacy traversal controls cannot be combined with discovery controls",
        ),
        (
            vec!["--scope", "node:n:target", "--budget", "1000"],
            "legacy traversal controls cannot be combined with discovery controls",
        ),
        (
            vec!["--format", "json", "--page", "2"],
            "legacy traversal controls cannot be combined with discovery controls",
        ),
        (
            vec!["--format", "json", "--text-budget", "1000"],
            "text-only and cannot be used with --format json",
        ),
        (
            vec!["--format", "json", "--cursor", "not-a-cursor"],
            "text-only and cannot be used with --format json",
        ),
        (
            vec!["--format", "json", "--evidence"],
            "text-only and cannot be used with --format json",
        ),
    ] {
        let mut args = vec![OsString::from("query"), OsString::from("Target")];
        args.extend(arguments.iter().map(OsString::from));
        args.extend([OsString::from("--graph"), OsString::from(&graph)]);
        let outcome = run(Frontend::Compass, args);
        assert_ne!(outcome.code, 0, "arguments={arguments:?}");
        assert!(
            outcome.stderr.contains(expected),
            "arguments={arguments:?} stderr={}",
            outcome.stderr
        );
    }
    Ok(())
}

#[test]
fn natural_discovery_help_documents_only_the_public_contract() {
    let outcome = run(
        Frontend::Compass,
        [OsString::from("query"), OsString::from("--help")],
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    for expected in [
        "--direction <VALUE>",
        "auto, incoming, outgoing, or both",
        "--scope <KIND:VALUE>",
        "Repeatable OR scope",
        "--context <VALUE>",
        "--format <text|agent-json|json>",
        "--result-envelope",
        "--text-budget <N>",
        "default: 8000",
        "--evidence",
        "full provenance",
        "--cursor <TOKEN>",
        "Natural discovery:",
        "--include-heuristic",
        "--max-depth <N>",
        "default: 2; hard maximum: 8",
        "--max-seeds <N>",
        "--max-candidates <N>",
        "--max-nodes <N>",
        "default: 64; hard maximum: 500",
        "--max-edges <N>",
        "default: 128; hard maximum: 1000",
        "--max-expanded-relationships <N>",
        "--max-response-bytes <N>",
        "--timeout-ms <N>",
        "Discovery deadline in milliseconds",
        "CompassQL execution timeout",
        "Legacy traversal:",
        "hard maximum",
        "clamped",
        "--at <REV>",
        "Resolve REV once to an immutable typed realization",
    ] {
        assert!(
            outcome.stdout.contains(expected),
            "missing {expected} in {}",
            outcome.stdout
        );
    }
    assert!(!outcome.stdout.contains("--relation-context"));
    assert!(!outcome.stdout.contains("--realization"));
}

#[test]
fn typed_query_defaults_to_store_and_json_remains_explicit() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = support::write_typed_graph(directory.path())?;
    let graph = GraphDocument::load(&graph_path)?;
    let store = SqliteStore::open(directory.path().join(STORE_FILE_NAME))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    std::fs::write(
        directory.path().join(STORE_REF_FILE_NAME),
        serde_json::to_vec(&store.snapshot_reference()?)?,
    )?;
    std::fs::write(&graph_path, b"not the selected JSON engine")?;

    let default = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_path.clone().into_os_string(),
        ],
    );
    assert_eq!(default.code, 0, "{}", default.stderr);
    assert!(default.stdout.contains("Fixture.Target"));

    let json = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_path.clone().into_os_string(),
            OsString::from("--engine"),
            OsString::from("json"),
        ],
    );
    assert_ne!(json.code, 0);

    let store = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_path.into_os_string(),
            OsString::from("--engine"),
            OsString::from("store"),
        ],
    );
    assert_eq!(store.code, 0, "{}", store.stderr);
    assert!(store.stdout.contains("Fixture.Target"));
    Ok(())
}

#[test]
fn typed_query_text_is_a_projection_of_the_same_response() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let outcome = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph.into_os_string(),
        ],
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(outcome.stdout.starts_with("RESULT "));
    assert!(outcome.stdout.contains("ANSWER\n"));
    assert!(outcome.stdout.contains("Fixture.Target"));
    Ok(())
}

#[test]
fn typed_query_resolves_the_current_snapshot_from_the_public_path() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    let guard = BuildGuard::begin(&output)?;
    support::write_typed_graph(guard.staging_directory())?;
    guard.commit_with_artifacts(&["graph.json"])?;

    let outcome = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            output.join("graph.json").into_os_string(),
        ],
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(outcome.stdout.contains("Fixture.Target"));
    Ok(())
}

#[test]
fn typed_query_prefers_current_snapshot_over_a_stale_root_facade() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    support::write_typed_graph(&output)?;
    let guard = BuildGuard::begin(&output)?;
    support::write_typed_graph(guard.staging_directory())?;
    guard.commit_with_artifacts(&["graph.json"])?;
    std::fs::write(output.join("graph.json"), b"{\"stale\":true}")?;

    let outcome = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            output.join("graph.json").into_os_string(),
        ],
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(outcome.stdout.contains("Fixture.Target"));
    Ok(())
}

#[test]
fn typed_query_fails_closed_on_a_malformed_snapshot_pointer() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    support::write_typed_graph(&output)?;
    std::fs::write(output.join("current-snapshot"), "../escape")?;

    let outcome = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            output.join("graph.json").into_os_string(),
        ],
    );
    assert_ne!(outcome.code, 0);
    assert!(outcome.stderr.contains("snapshot"));
    Ok(())
}

#[test]
fn natural_query_accepts_a_standalone_graph_but_rejects_a_malformed_managed_pointer()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    support::write_typed_graph(&output)?;

    let standalone = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            output.join("graph.json").into_os_string(),
        ],
    );
    assert_eq!(standalone.code, 0, "{}", standalone.stderr);

    std::fs::write(output.join("current-snapshot"), "../escape")?;
    let malformed = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            output.join("graph.json").into_os_string(),
        ],
    );
    assert_ne!(malformed.code, 0);
    assert!(
        malformed.stderr.contains("snapshot"),
        "{}",
        malformed.stderr
    );
    Ok(())
}

#[test]
fn natural_query_renders_typed_source_locations() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;

    let outcome = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph.into_os_string(),
        ],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        outcome
            .stdout
            .contains("NODE Fixture.Target [function] src/lib.rs:1")
    );
    Ok(())
}

#[test]
fn natural_query_is_concise_by_default_and_evidence_is_opt_in() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let base = [
        OsString::from("query"),
        OsString::from("Target"),
        OsString::from("--graph"),
        graph.into_os_string(),
    ];

    let concise = run(Frontend::Compass, base.clone());
    assert_eq!(concise.code, 0, "{}", concise.stderr);
    assert!(concise.stdout.starts_with("RESULT "));
    assert!(
        concise.stdout.contains("RESULT candidates") || concise.stdout.contains("RESULT answered"),
        "{}",
        concise.stdout
    );
    assert!(concise.stdout.contains("NODE Fixture.Target [function]"));
    assert!(concise.stdout.contains("provenance record(s) hidden"));
    assert!(!concise.stdout.contains("Node evidence:"));
    assert!(!concise.stdout.contains("Semantic result:"));

    let mut evidence_args = base.to_vec();
    evidence_args.push(OsString::from("--evidence"));
    let evidence = run(Frontend::Compass, evidence_args);
    assert_eq!(evidence.code, 0, "{}", evidence.stderr);
    assert!(evidence.stdout.contains("Node evidence:"));
    assert!(evidence.stdout.contains("Semantic result:"));
    Ok(())
}

#[test]
fn natural_and_typed_queries_signal_missing_exact_matches_before_fallbacks()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    for command in ["query", "ask"] {
        let outcome = run(
            Frontend::Compass,
            [
                OsString::from(command),
                OsString::from("who calls Targat?"),
                OsString::from("--graph"),
                graph.clone().into_os_string(),
            ],
        );
        assert_eq!(outcome.code, 0, "{command}: {}", outcome.stderr);
        assert!(
            outcome.stdout.starts_with("RESULT no_match"),
            "{command}: {}",
            outcome.stdout
        );
        assert!(
            outcome.stdout.contains("NO EXACT MATCH"),
            "{command}: {}",
            outcome.stdout
        );
    }
    Ok(())
}

#[test]
fn path_resolves_exact_targets_and_ranks_structural_evidence_end_to_end()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = support::write_typed_graph(directory.path())?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let template_node = graph.nodes[0].clone();
    for (id, name) in [
        ("n:strong-one", "StrongOne"),
        ("n:strong-two", "StrongTwo"),
        ("n:weak", "WeakShortcut"),
        ("n:isolated", "Isolated"),
    ] {
        let mut node = template_node.clone();
        node.id = id.to_owned();
        node.name = name.to_owned();
        node.qualified_name = format!("Fixture.{name}");
        graph.nodes.push(node);
    }
    graph.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let template_edge = graph.links[0].clone();
    graph.links.clear();
    for (id, source, target, kind) in [
        ("e:strong-1", "n:caller", "n:strong-one", EdgeKind::Calls),
        (
            "e:strong-2",
            "n:strong-one",
            "n:strong-two",
            EdgeKind::Contains,
        ),
        (
            "e:strong-3",
            "n:strong-two",
            "n:target",
            EdgeKind::DependsOn,
        ),
        ("e:weak-1", "n:caller", "n:weak", EdgeKind::References),
        ("e:weak-2", "n:weak", "n:target", EdgeKind::Documents),
    ] {
        let mut edge = template_edge.clone();
        edge.id = id.to_owned();
        edge.key = id.to_owned();
        edge.source = source.to_owned();
        edge.target = target.to_owned();
        edge.kind = kind;
        graph.links.push(edge);
    }
    std::fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let path = run(
        Frontend::Compass,
        [
            OsString::from("path"),
            OsString::from("Caller"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_path.clone().into_os_string(),
        ],
    );
    assert_eq!(path.code, 0, "{}", path.stderr);
    assert!(path.stdout.contains("Target resolved: Target"));
    assert!(
        path.stdout
            .contains("Best path (weighted, 3 hops, weight 3)")
    );
    assert!(path.stdout.contains("shorter (2-hop) but weaker path"));

    let unreachable = run(
        Frontend::Compass,
        [
            OsString::from("path"),
            OsString::from("Caller"),
            OsString::from("Isolated"),
            OsString::from("--max-depth"),
            OsString::from("4"),
            OsString::from("--graph"),
            graph_path.clone().into_os_string(),
        ],
    );
    assert_eq!(unreachable.code, 0, "{}", unreachable.stderr);
    assert!(
        unreachable
            .stdout
            .contains("NO PATH FOUND to resolved target")
    );
    assert!(unreachable.stdout.contains("depth limit 4"));

    let missing = run(
        Frontend::Compass,
        [
            OsString::from("path"),
            OsString::from("Call"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_path.into_os_string(),
        ],
    );
    assert_ne!(missing.code, 0);
    assert!(missing.stderr.contains("NO EXACT MATCH"));
    Ok(())
}

#[test]
fn brief_agent_json_is_compact_and_format_bound() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let brief = run(
        Frontend::Compass,
        [
            OsString::from("callers"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph.as_os_str().to_owned(),
            OsString::from("--format"),
            OsString::from("agent-json"),
            OsString::from("--brief"),
        ],
    );
    assert_eq!(brief.code, 0, "{}", brief.stderr);
    let view: Value = serde_json::from_str(&brief.stdout)?;
    assert_eq!(view["schema"], "compass.query.agent-view.brief/1");
    assert_eq!(view["status"]["resultState"], "answered");
    assert!(!brief.stdout.contains("viewDigest"));
    assert!(!brief.stdout.contains("\"identity\""));

    let rejected = run(
        Frontend::Compass,
        [
            OsString::from("callers"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph.as_os_str().to_owned(),
            OsString::from("--format"),
            OsString::from("json"),
            OsString::from("--brief"),
        ],
    );
    assert_ne!(rejected.code, 0);
    assert!(
        rejected
            .stderr
            .contains("--brief requires --format agent-json")
    );
    Ok(())
}

#[test]
fn typed_queries_report_an_expired_deadline_and_still_answer_within_one()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let graph_arg = graph.as_os_str().to_owned();
    let expired = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_arg.clone(),
            OsString::from("--timeout-ms"),
            OsString::from("1"),
        ],
    );
    assert_ne!(expired.code, 0);
    assert!(
        expired.stderr.contains("exceeded its timeout") && expired.stderr.contains("--timeout-ms"),
        "{}",
        expired.stderr
    );

    let answered = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_arg,
            OsString::from("--timeout-ms"),
            OsString::from("60000"),
        ],
    );
    assert_eq!(answered.code, 0, "{}", answered.stderr);
    assert!(answered.stdout.contains("Target"), "{}", answered.stdout);
    Ok(())
}

#[test]
fn path_accepts_file_shaped_input_when_modules_carry_the_file_content() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_module_graph(directory.path())?;
    let outcome = run(
        Frontend::Compass,
        [
            OsString::from("path"),
            OsString::from("src/a.ts"),
            OsString::from("src/b.ts"),
            OsString::from("--graph"),
            graph.as_os_str().to_owned(),
        ],
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        outcome.stdout.contains("Best path"),
        "file-shaped input must traverse the file's module: {}",
        outcome.stdout
    );
    assert!(
        !outcome.stdout.contains("NO PATH FOUND"),
        "{}",
        outcome.stdout
    );
    Ok(())
}

#[test]
fn path_resolves_workspace_source_paths_like_search_and_callers() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = support::write_typed_module_graph(directory.path())?;
    let mut graph = GraphDocument::load(&graph_path)?;
    graph.nodes.retain(|node| node.kind != NodeKind::File);
    std::fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;
    let graph_arg = graph_path.as_os_str().to_owned();

    let search = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("b"),
            OsString::from("--graph"),
            graph_arg.clone(),
        ],
    );
    assert_eq!(search.code, 0, "{}", search.stderr);
    assert!(search.stdout.contains("b"), "{}", search.stdout);

    let callers = run(
        Frontend::Compass,
        [
            OsString::from("callers"),
            OsString::from("b"),
            OsString::from("--graph"),
            graph_arg.clone(),
        ],
    );
    assert_eq!(callers.code, 0, "{}", callers.stderr);
    assert!(callers.stdout.contains("a"), "{}", callers.stdout);

    let path = run(
        Frontend::Compass,
        [
            OsString::from("path"),
            OsString::from("src/a.ts"),
            OsString::from("src/b.ts"),
            OsString::from("--graph"),
            graph_arg,
        ],
    );
    assert_eq!(path.code, 0, "{}", path.stderr);
    assert!(path.stdout.contains("Best path"), "{}", path.stdout);

    let mut ambiguous_graph = GraphDocument::load(&graph_path)?;
    let mut second_module = ambiguous_graph
        .nodes
        .iter()
        .find(|node| node.id == "n:a")
        .cloned()
        .ok_or("missing source module")?;
    second_module.id = "n:a-duplicate".to_owned();
    second_module.name = "a-alias".to_owned();
    second_module.qualified_name = "a-alias".to_owned();
    ambiguous_graph.nodes.push(second_module);
    std::fs::write(&graph_path, serde_json::to_vec_pretty(&ambiguous_graph)?)?;
    let ambiguous = run(
        Frontend::Compass,
        [
            OsString::from("path"),
            OsString::from("src/a.ts"),
            OsString::from("src/b.ts"),
            OsString::from("--graph"),
            graph_path.as_os_str().to_owned(),
        ],
    );
    assert_ne!(ambiguous.code, 0);
    assert!(
        ambiguous.stderr.contains("AMBIGUOUS EXACT MATCH"),
        "{}",
        ambiguous.stderr
    );

    let oversized_path = run(
        Frontend::Compass,
        [
            OsString::from("path"),
            OsString::from(format!("src/{}.ts", "x".repeat(4_100))),
            OsString::from("src/b.ts"),
            OsString::from("--graph"),
            graph_path.as_os_str().to_owned(),
        ],
    );
    assert_ne!(oversized_path.code, 0);
    assert!(
        oversized_path.stderr.contains("source-path limit"),
        "{}",
        oversized_path.stderr
    );
    Ok(())
}

#[test]
fn configured_typescript_path_alias_agrees_across_search_callers_and_path()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("project");
    let app = root.join("apps/web");
    std::fs::create_dir_all(app.join("src"))?;
    std::fs::write(
        app.join("tsconfig.json"),
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["./src/*"]}},"include":["src/**/*.ts"]}"#,
    )?;
    std::fs::write(
        app.join("src/api.ts"),
        "export function Widget() { return 1; }\n",
    )?;
    std::fs::write(
        app.join("src/consumer.ts"),
        "import { Widget } from \"@/api\";\nexport function useWidget() { return Widget(); }\n",
    )?;
    let output_root = directory.path().join("generated");
    let ensured = run(
        Frontend::Compass,
        [
            OsString::from("ensure"),
            root.as_os_str().to_owned(),
            OsString::from("--out"),
            output_root.as_os_str().to_owned(),
            OsString::from("--store"),
            OsString::from("json"),
            OsString::from("--no-cluster"),
            OsString::from("--no-viz"),
        ],
    );
    assert_eq!(ensured.code, 0, "{}", ensured.stderr);

    let graph_path = output_root.join("compass-out/graph.json");
    let mut graph = GraphDocument::load(&graph_path)?;
    graph.nodes.retain(|node| node.kind != NodeKind::File);
    std::fs::write(&graph_path, serde_json::to_vec(&graph)?)?;

    let search = run(
        Frontend::Compass,
        [
            OsString::from("search"),
            OsString::from("Widget"),
            OsString::from("--graph"),
            graph_path.as_os_str().to_owned(),
            OsString::from("--format"),
            OsString::from("json"),
        ],
    );
    assert_eq!(search.code, 0, "{}", search.stderr);
    let search_value: Value = serde_json::from_str(&search.stdout)?;
    let target = search_value["nodes"]
        .as_array()
        .and_then(|nodes| {
            nodes.iter().find(|node| {
                node["kind"] == "function"
                    && node["source"]["file"] == "apps/web/src/api.ts"
                    && node["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with("Widget"))
            })
        })
        .and_then(|node| node["id"].as_str())
        .ok_or("search did not return the path-mapped Widget declaration")?
        .to_owned();

    let callers = run(
        Frontend::Compass,
        [
            OsString::from("callers"),
            OsString::from(target),
            OsString::from("--graph"),
            graph_path.as_os_str().to_owned(),
        ],
    );
    assert_eq!(callers.code, 0, "{}", callers.stderr);
    assert!(callers.stdout.contains("useWidget"), "{}", callers.stdout);
    assert!(callers.stdout.contains("consumer.ts"), "{}", callers.stdout);

    let path = run(
        Frontend::Compass,
        [
            OsString::from("path"),
            OsString::from("apps/web/src/consumer.ts"),
            OsString::from("apps/web/src/api.ts"),
            OsString::from("--graph"),
            graph_path.as_os_str().to_owned(),
        ],
    );
    assert_eq!(path.code, 0, "{}", path.stderr);
    assert!(path.stdout.contains("Best path"), "{}", path.stdout);
    assert!(path.stdout.contains("imports"), "{}", path.stdout);
    Ok(())
}

#[test]
fn typed_text_paging_continues_the_same_result_with_a_cursor() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_graph(directory.path())?;
    let graph_arg = graph.as_os_str().to_owned();
    let first = run(
        Frontend::Compass,
        [
            OsString::from("callers"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_arg.clone(),
            OsString::from("--format"),
            OsString::from("text"),
            OsString::from("--text-budget"),
            OsString::from("120"),
        ],
    );
    assert_eq!(first.code, 0, "{}", first.stderr);
    let cursor = first
        .stdout
        .lines()
        .find_map(|line| line.split("next=").nth(1))
        .ok_or("expected a continuation cursor")?
        .to_owned();
    assert!(first.stdout.contains("range=1-"), "{}", first.stdout);

    let second = run(
        Frontend::Compass,
        [
            OsString::from("callers"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph_arg,
            OsString::from("--format"),
            OsString::from("text"),
            OsString::from("--text-budget"),
            OsString::from("120"),
            OsString::from("--cursor"),
            OsString::from(cursor),
        ],
    );
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert!(second.stdout.contains("range=2-"), "{}", second.stdout);

    let rejected = run(
        Frontend::Compass,
        [
            OsString::from("callers"),
            OsString::from("Target"),
            OsString::from("--graph"),
            graph.as_os_str().to_owned(),
            OsString::from("--format"),
            OsString::from("json"),
            OsString::from("--text-budget"),
            OsString::from("120"),
        ],
    );
    assert_ne!(rejected.code, 0);
    assert!(rejected.stderr.contains("text-only"), "{}", rejected.stderr);
    Ok(())
}

#[test]
fn ambiguous_typed_lookup_returns_a_pick_list_instead_of_an_empty_result()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = support::write_typed_ambiguous_graph(directory.path())?;

    let outcome = run(
        Frontend::Compass,
        [
            OsString::from("callers"),
            OsString::from("run"),
            OsString::from("--graph"),
            graph.as_os_str().to_owned(),
            OsString::from("--format"),
            OsString::from("agent-json"),
        ],
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let view: Value = serde_json::from_str(&outcome.stdout)?;
    assert_eq!(view["status"]["resultState"], "needs_resolution");
    assert_eq!(view["status"]["matchState"], "ambiguous");
    let results = view["primaryResults"]
        .as_array()
        .ok_or("primaryResults must be an array")?;
    assert_eq!(results.len(), 2, "{}", outcome.stdout);
    assert_eq!(results[0]["source"]["file"], "src/a.rs");
    assert_eq!(results[1]["source"]["file"], "src/b.rs");
    assert!(view["relationships"].as_array().is_some_and(Vec::is_empty));
    assert!(
        view["nextActions"]
            .as_array()
            .is_some_and(|actions| actions.iter().any(|action| {
                action["kind"] == "retry_with_exact_id" && action["cli"]["argv"][2] == "n:alpha-run"
            })),
        "{}",
        outcome.stdout
    );
    Ok(())
}

#[test]
fn explain_source_bounds_the_neighbourhood_and_an_explicit_budget_overrides_it()
-> Result<(), Box<dyn Error>> {
    use sha2::{Digest, Sha256};

    let directory = tempfile::tempdir()?;
    let source_dir = directory.path().join("src");
    std::fs::create_dir_all(&source_dir)?;
    let source = "fn run() {\n    body();\n}\n";
    std::fs::write(source_dir.join("lib.rs"), source)?;
    let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
    let mut links = Vec::new();
    let mut nodes = vec![serde_json::json!({
        "id": "n:run",
        "kind": "function",
        "name": "run",
        "qualifiedName": "sample::run",
        "source": {
            "file": "src/lib.rs",
            "startByte": 0,
            "endByte": source.len(),
            "startLine": 1,
            "startColumn": 0,
            "endLine": 3,
            "endColumn": 1
        },
        "details": {"type": "symbol", "data": {"sourceDigest": digest}}
    })];
    for index in 0..12 {
        let id = format!("n:caller-{index:02}");
        nodes.push(serde_json::json!({
            "id": id,
            "kind": "function",
            "name": format!("caller{index:02}"),
            "source": {
                "file": format!("src/caller_{index:02}.rs"),
                "startLine": index + 1,
                "startColumn": 0,
                "endLine": index + 1,
                "endColumn": 4
            }
        }));
        links.push(serde_json::json!({
            "source": id,
            "target": "n:run",
            "relation": "calls",
            "confidence": "EXTRACTED",
            "source_file": format!("src/caller_{index:02}.rs"),
            "source_location": format!("L{}:0-L{}:4", index + 1, index + 1)
        }));
    }
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::json!({
            "directed": true,
            "multigraph": true,
            "graph": {},
            "nodes": nodes,
            "links": links
        })
        .to_string(),
    )?;
    let run_command = |extra: Vec<OsString>| {
        let mut arguments = vec![
            OsString::from("explain"),
            OsString::from("sample::run"),
            OsString::from("--source"),
            OsString::from("--root"),
            directory.path().as_os_str().to_owned(),
        ];
        arguments.extend(extra);
        arguments.push(OsString::from("--graph"));
        arguments.push(graph.as_os_str().to_owned());
        run(Frontend::Compass, arguments)
    };

    let bounded = run_command(Vec::new());
    assert_eq!(bounded.code, 0, "{}", bounded.stderr);
    let rows = bounded
        .stdout
        .lines()
        .filter(|line| line.starts_with("  <-- ") || line.starts_with("  --> "))
        .count();
    assert!(
        rows > 0 && rows < 12,
        "a source request leads with the declaration, not the whole neighbourhood: {}",
        bounded.stdout
    );
    assert!(
        bounded.stdout.contains("Pagination: page=1/2 connections="),
        "the slice names the list's true total and its continuation: {}",
        bounded.stdout
    );
    assert!(
        bounded
            .stdout
            .contains("SOURCE src/lib.rs L1-L3 (digest-verified)"),
        "{}",
        bounded.stdout
    );
    assert!(
        bounded.stdout.contains("next=2"),
        "the remainder is reachable: {}",
        bounded.stdout
    );

    let explicit = run_command(vec![OsString::from("--budget"), OsString::from("2000")]);
    assert_eq!(explicit.code, 0, "{}", explicit.stderr);
    assert_eq!(
        explicit
            .stdout
            .lines()
            .filter(|line| line.starts_with("  <-- ") || line.starts_with("  --> "))
            .count(),
        12,
        "an explicit budget lists every connection: {}",
        explicit.stdout
    );
    assert!(
        explicit
            .stdout
            .contains("Pagination: page=1/1 connections=1-12/12 next=none"),
        "{}",
        explicit.stdout
    );
    Ok(())
}

#[test]
fn explain_source_returns_digest_verified_declaration_text() -> Result<(), Box<dyn Error>> {
    use sha2::{Digest, Sha256};

    let directory = tempfile::tempdir()?;
    let source_dir = directory.path().join("src");
    std::fs::create_dir_all(&source_dir)?;
    let source = "fn run() {\n    body();\n}\n";
    let source_path = source_dir.join("lib.rs");
    std::fs::write(&source_path, source)?;
    let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
    let node_id = format!("sha256:{}", "a".repeat(64));
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::json!({
            "directed": true,
            "multigraph": true,
            "nodes": [{
                "id": node_id,
                "kind": "function",
                "name": "run",
                "qualifiedName": "sample::run",
                "source": {
                    "file": "src/lib.rs",
                    "startByte": 0,
                    "endByte": source.len(),
                    "startLine": 1,
                    "startColumn": 0,
                    "endLine": 3,
                    "endColumn": 1
                },
                "details": {"type": "symbol", "data": {"sourceDigest": digest}}
            }],
            "links": []
        })
        .to_string(),
    )?;

    let explained = run(
        Frontend::Compass,
        [
            OsString::from("explain"),
            OsString::from("run"),
            OsString::from("--root"),
            directory.path().as_os_str().to_owned(),
            OsString::from("--graph"),
            graph.as_os_str().to_owned(),
        ],
    );
    assert_eq!(explained.code, 0, "{}", explained.stderr);
    assert!(
        explained
            .stdout
            .contains("SOURCE src/lib.rs L1-L3 (digest-verified)"),
        "{}",
        explained.stdout
    );
    assert!(
        explained.stdout.contains("1: fn run() {"),
        "{}",
        explained.stdout
    );
    assert!(explained.stdout.contains("3: }"), "{}", explained.stdout);

    let omitted = run(
        Frontend::Compass,
        [
            OsString::from("explain"),
            OsString::from("run"),
            OsString::from("--no-source"),
            OsString::from("--root"),
            directory.path().as_os_str().to_owned(),
            OsString::from("--graph"),
            graph.as_os_str().to_owned(),
        ],
    );
    assert_eq!(omitted.code, 0, "{}", omitted.stderr);
    assert!(
        !omitted.stdout.contains("SOURCE src/lib.rs"),
        "{}",
        omitted.stdout
    );

    std::fs::write(&source_path, "fn run() {\n    other();\n}\n")?;
    let stale = run(
        Frontend::Compass,
        [
            OsString::from("explain"),
            OsString::from("run"),
            OsString::from("--source"),
            OsString::from("--root"),
            directory.path().as_os_str().to_owned(),
            OsString::from("--graph"),
            graph.as_os_str().to_owned(),
        ],
    );
    assert_eq!(stale.code, 0, "{}", stale.stderr);
    assert!(
        stale.stdout.contains("SOURCE unavailable") && stale.stdout.contains("does not match"),
        "{}",
        stale.stdout
    );
    Ok(())
}

#[test]
fn explain_requires_an_exact_id_for_ambiguous_typed_nodes() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    let first = format!("sha256:{}", "a".repeat(64));
    let second = format!("sha256:{}", "b".repeat(64));
    std::fs::write(
        &graph,
        format!(
            r#"{{
                "directed": true, "multigraph": true, "nodes": [
                    {{"id":"{first}","kind":"method","name":".run()","source":{{"file":"src/a.rs","startLine":3,"startColumn":1,"endLine":3,"endColumn":6}}}},
                    {{"id":"{second}","kind":"method","name":".run()","source":{{"file":"src/b.rs","startLine":7,"startColumn":1,"endLine":7,"endColumn":6}}}}
                ], "links": []
            }}"#
        ),
    )?;

    let ambiguous = run(
        Frontend::Compass,
        [
            OsString::from("explain"),
            OsString::from("run"),
            OsString::from("--graph"),
            graph.clone().into_os_string(),
        ],
    );
    assert_eq!(ambiguous.code, 0, "{}", ambiguous.stderr);
    assert!(
        ambiguous
            .stdout
            .contains("Ambiguous: 'run' matches 2 source-backed nodes.")
    );
    assert!(ambiguous.stdout.contains("Retry with the full node ID."));

    let exact = run(
        Frontend::Compass,
        [
            OsString::from("explain"),
            OsString::from(&second),
            OsString::from("--graph"),
            graph.into_os_string(),
        ],
    );
    assert_eq!(exact.code, 0, "{}", exact.stderr);
    assert!(exact.stdout.contains("Source:    src/b.rs L7:1-L7:6"));
    assert!(exact.stdout.contains("Type:      code"));
    Ok(())
}

#[test]
fn explain_resolves_a_unique_qualified_name_from_the_traversal_projection()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    let node_id = format!("sha256:{}", "c".repeat(64));
    std::fs::write(
        &graph,
        format!(
            r#"{{
                "directed": true, "multigraph": true, "nodes": [
                    {{"id":"{node_id}","kind":"function","name":"start()","qualifiedName":"cmd/daemon.start","source":{{"file":"cmd/daemon/start.go","startLine":12,"startColumn":1,"endLine":18,"endColumn":2}}}}
                ], "links": []
            }}"#
        ),
    )?;

    let explained = run(
        Frontend::Compass,
        [
            OsString::from("explain"),
            OsString::from("cmd/daemon.start"),
            OsString::from("--graph"),
            graph.into_os_string(),
        ],
    );

    assert_eq!(explained.code, 0, "{}", explained.stderr);
    assert!(explained.stdout.contains(&node_id));
    assert!(
        explained
            .stdout
            .contains("Source:    cmd/daemon/start.go L12:1-L18:2")
    );
    Ok(())
}

#[test]
fn natural_query_and_explain_accept_agent_controlled_budgets_and_pages()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    let nodes = std::iter::once(serde_json::json!({
        "id": "seed", "label": "Seed", "source_file": "src/seed.rs", "source_location": "L1"
    }))
    .chain((0..8).map(|index| {
        serde_json::json!({
            "id": format!("neighbor-{index}"),
            "label": format!("Neighbor{index}"),
            "source_file": format!("src/neighbor_{index}.rs"),
            "source_location": "L1"
        })
    }))
    .collect::<Vec<_>>();
    let links = (0..8)
        .map(|index| {
            serde_json::json!({
                "source": "seed",
                "target": format!("neighbor-{index}"),
                "relation": "calls",
                "confidence": "EXTRACTED"
            })
        })
        .collect::<Vec<_>>();
    std::fs::write(
        &graph,
        serde_json::to_vec(&serde_json::json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": nodes,
            "links": links
        }))?,
    )?;

    for command in ["query", "explain"] {
        let first = run(
            Frontend::Compass,
            [
                OsString::from(command),
                OsString::from("Seed"),
                OsString::from("--budget=60"),
                OsString::from("--page=1"),
                OsString::from("--graph"),
                graph.clone().into_os_string(),
            ],
        );
        assert_eq!(first.code, 0, "{command}: {}", first.stderr);
        assert!(first.stdout.contains("Pagination: page=1/"));
        assert!(first.stdout.contains("next=2"));

        let second = run(
            Frontend::Compass,
            [
                OsString::from(command),
                OsString::from("Seed"),
                OsString::from("--budget"),
                OsString::from("60"),
                OsString::from("--page"),
                OsString::from("2"),
                OsString::from("--graph"),
                graph.clone().into_os_string(),
            ],
        );
        assert_eq!(second.code, 0, "{command}: {}", second.stderr);
        assert!(second.stdout.contains("Pagination: page=2/"));
        assert_ne!(first.stdout, second.stdout);
    }

    for arguments in [
        vec!["query", "Seed", "--page=0"],
        vec!["explain", "Seed", "--budget=0"],
    ] {
        let outcome = run(Frontend::Compass, arguments.into_iter().map(OsString::from));
        assert_ne!(outcome.code, 0);
        assert!(outcome.stderr.contains("error:"));
    }

    let out_of_range = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Seed"),
            OsString::from("--page=999"),
            OsString::from("--graph"),
            graph.into_os_string(),
        ],
    );
    assert_ne!(out_of_range.code, 0);
    assert!(out_of_range.stderr.contains("last available page"));
    Ok(())
}
