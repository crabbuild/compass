mod support;

use std::error::Error;
use std::ffi::OsString;

use compass_cli::{Frontend, run};
use compass_files::BuildGuard;
use compass_graph::GraphSnapshotBuilder;
use compass_model::code_graph::GraphDocument;
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
        assert!(outcome.stdout.starts_with("Discovery:"), "{question}");
        assert!(
            outcome.stdout.contains(expected_node),
            "{question}: {}",
            outcome.stdout
        );
        assert!(outcome.stdout.contains("Direction:"), "{question}");
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
        assert!(generic.stdout.starts_with("Discovery:"), "{question}");
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
    assert!(
        continued
            .stdout
            .contains("Relationship contexts: import,call")
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
        "--format <text|json>",
        "--result-envelope",
        "--text-budget <N>",
        "--cursor <TOKEN>",
        "Natural discovery:",
        "--include-heuristic",
        "--max-depth <N>",
        "default: 2; hard maximum: 8",
        "--max-seeds <N>",
        "--max-candidates <N>",
        "--max-nodes <N>",
        "--max-edges <N>",
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
    assert!(outcome.stdout.contains("Search:"));
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
            .contains("Node: n:target [function] Fixture.Target @ src/lib.rs:1")
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
