use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use compass_cli::{Frontend, run, run_mcp, run_watch};
use compass_files::BuildGuard;

fn invoke(frontend: Frontend, arguments: &[&str]) -> compass_cli::Outcome {
    run(
        frontend,
        arguments.iter().map(|argument| OsString::from(*argument)),
    )
}

fn invoke_owned(frontend: Frontend, arguments: &[String]) -> compass_cli::Outcome {
    run(frontend, arguments.iter().map(OsString::from))
}

fn write_graph_fixture(path: &Path) -> Result<(), Box<dyn Error>> {
    fs::write(
        path,
        r#"{
          "directed": true,
          "multigraph": false,
          "graph": {},
          "nodes": [
            {"id":"n_transformer","label":"Transformer","source_file":"model.py","source_location":"L1","file_type":"code","community":0},
            {"id":"n_attention","label":"Attention","source_file":"attention.py","source_location":"L2","file_type":"code","community":0},
            {"id":"n_layernorm","label":"LayerNorm","source_file":"model.py","source_location":"L3","file_type":"code","community":1},
            {"id":"n_concept_attn","label":"attention mechanism","source_file":"guide.md","source_location":"L4","file_type":"document","community":1}
          ],
          "links": [
            {"source":"n_transformer","target":"n_attention","relation":"calls","confidence":"EXTRACTED"},
            {"source":"n_attention","target":"n_concept_attn","relation":"references","confidence":"INFERRED"}
          ]
        }"#,
    )?;
    Ok(())
}

fn write_diagnostic_graph(path: &Path, node_count: usize) -> Result<(), Box<dyn Error>> {
    let nodes = (0..node_count)
        .map(|index| serde_json::json!({"id":format!("node-{index}")}))
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({
            "directed":true,
            "multigraph":false,
            "nodes":nodes,
            "links":[]
        }))?,
    )?;
    Ok(())
}

fn diagnostic_node_count(graph: &Path) -> Result<usize, Box<dyn Error>> {
    let outcome = invoke_owned(
        Frontend::Compass,
        &[
            "diagnose".to_owned(),
            "multigraph".to_owned(),
            "--graph".to_owned(),
            graph.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ],
    );
    if outcome.code != 0 {
        return Err(outcome.stderr.into());
    }
    let body: serde_json::Value = serde_json::from_str(&outcome.stdout)?;
    body["summary"]["node_count"]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "missing diagnostic node count".into())
}

#[test]
fn diagnose_rereads_the_current_snapshot_for_an_explicit_public_graph() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let output = directory.path();
    let public = output.join("graph.json");
    write_diagnostic_graph(&public, 1)?;
    let snapshots = output.join("snapshots");
    let first = snapshots.join("snapshot-first");
    let second = snapshots.join("snapshot-second");
    fs::create_dir_all(&first)?;
    fs::create_dir_all(&second)?;
    write_diagnostic_graph(&first.join("graph.json"), 2)?;
    write_diagnostic_graph(&second.join("graph.json"), 3)?;

    fs::write(output.join("current-snapshot"), "snapshot-first")?;
    assert_eq!(diagnostic_node_count(&public)?, 2);
    fs::write(output.join("current-snapshot"), "snapshot-second")?;
    assert_eq!(diagnostic_node_count(&public)?, 3);
    Ok(())
}

#[test]
fn diagnose_accepts_a_standalone_graph_but_rejects_a_malformed_managed_pointer()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let public = directory.path().join("graph.json");
    write_diagnostic_graph(&public, 1)?;
    assert_eq!(diagnostic_node_count(&public)?, 1);

    fs::write(directory.path().join("current-snapshot"), "../escape")?;
    let outcome = invoke_owned(
        Frontend::Compass,
        &[
            "diagnose".to_owned(),
            "multigraph".to_owned(),
            "--graph".to_owned(),
            public.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ],
    );
    assert_ne!(outcome.code, 0);
    assert!(outcome.stderr.contains("snapshot"), "{}", outcome.stderr);
    Ok(())
}

#[test]
fn frontend_roots_versions_help_and_unknown_commands_are_total() {
    assert_eq!(invoke(Frontend::Compass, &[]).code, 0);
    for arguments in [vec!["--help"], vec!["help"], vec!["--version"]] {
        assert_eq!(invoke(Frontend::Compass, &arguments).code, 0);
    }
    assert_ne!(invoke(Frontend::Compass, &["query"]).code, 0);
    assert_ne!(invoke(Frontend::Compass, &["graph"]).code, 0);
    assert_eq!(invoke(Frontend::Compass, &["--version"]).code, 0);
    assert_ne!(invoke(Frontend::Compass, &["unknown"]).code, 0);
    assert_ne!(invoke(Frontend::Compass, &["watch"]).code, 0);
    assert_ne!(invoke(Frontend::Compass, &["serve"]).code, 0);

    for arguments in [
        vec![],
        vec!["--help"],
        vec!["-?"],
        vec!["version"],
        vec!["-v"],
    ] {
        assert_eq!(invoke(Frontend::Compass, &arguments).code, 0);
    }
    let unknown = invoke(Frontend::Compass, &["not-real"]);
    assert_ne!(unknown.code, 0);
    assert!(unknown.stderr.contains("unknown command"));
}

#[test]
fn graph_command_argument_failures_cover_every_local_dispatch_family() {
    let compass_cases: &[&[&str]] = &[
        &["query"],
        &["history", "unknown"],
        &["history", "status", "one", "two"],
        &["history", "status", "--unknown"],
        &["history", "list", "--format", "yaml"],
        &["history", "show"],
        &["history", "export", "HEAD"],
        &["diff"],
        &["diff", "one", "two", "three"],
        &["diff", "one", "two", "--unknown"],
        &["diff", "one", "two", "--format", "yaml"],
        &["diff", "one", "two", "--detailed", "--format", "json"],
        &["query", "x", "--depth", "bad"],
        &["query", "x", "--unknown"],
        &["path"],
        &["path", "only-one"],
        &["explain"],
        &["affected"],
        &["affected", "x", "--depth", "bad"],
        &["export"],
        &["export", "unknown-format"],
        &["benchmark", "--corpus-words", "bad"],
        &["merge-graphs"],
        &["merge-graphs", "--output"],
        &["tree", "--depth", "bad"],
        &["tree", "--unknown"],
        &["cluster-only", "--graph"],
        &["cluster-only", "--resolution", "bad"],
        &["cluster-only", "--exclude-hubs=bad"],
        &["cluster-only", "--min-community-size=bad"],
        &["diagnose"],
        &["diagnose", "multigraph", "--graph"],
        &["diagnose", "multigraph", "--max-examples", "bad"],
        &["diagnose", "multigraph", "--max-examples", "-1"],
        &["diagnose", "multigraph", "--directed", "--undirected"],
        &["diagnose", "multigraph", "--extract-path"],
        &["diagnose", "multigraph", "--wat"],
        &["update", "--mode", "invalid"],
        &["update", "--workers", "bad"],
        &["update", "--resolution", "bad"],
        &["update", "--exclude-hubs", "bad"],
        &["update", "--min-community-size", "bad"],
        &["update", "--max-nodes", "bad"],
        &["update", "--semantic-timeout", "bad"],
        &["update", "--unknown"],
        &["extract", "--code-only", "--mode", "invalid"],
        &["cache-check"],
        &["merge-chunks"],
        &["merge-semantic"],
        &["save-result"],
        &["reflect"],
        &["check-update", "--wat"],
        &["hook-check", "--wat"],
        &["hook-guard", "--wat"],
        &["merge-driver"],
        &["global"],
        &["clone"],
        &["add"],
        &["label", "--unknown"],
        &["prs", "--unknown"],
        &["hook", "--unknown"],
        &["hook-spawn", "--unknown"],
        &["hook-refresh", "--unknown"],
    ];
    for arguments in compass_cases {
        let outcome = invoke(Frontend::Compass, arguments);
        assert!(outcome.code <= 2, "invalid exit code: {arguments:?}");
    }
    assert_eq!(invoke(Frontend::Compass, &["provider"]).code, 0);
}

#[test]
fn completed_command_help_routes_and_parser_boundaries_are_total() {
    for command in [
        "history",
        "update",
        "extract",
        "watch",
        "serve",
        "cluster-only",
        "label",
        "prs",
        "query",
        "path",
        "explain",
        "affected",
        "tree",
        "export",
        "benchmark",
        "diagnose",
        "merge-graphs",
        "cache-check",
        "merge-chunks",
        "merge-semantic",
        "provider",
        "save-result",
        "reflect",
        "check-update",
        "merge-driver",
        "global",
        "clone",
        "add",
        "hook",
        "install",
        "uninstall",
    ] {
        let outcome = invoke(Frontend::Compass, &[command, "--help"]);
        assert_eq!(outcome.code, 0, "{command}: {}", outcome.stderr);
        assert!(!outcome.stdout.is_empty(), "{command}");
    }
    assert_eq!(invoke(Frontend::Compass, &["not-real", "--help"]).code, 2);

    for arguments in [
        vec!["cluster-only", "--resolution"],
        vec!["cluster-only", "--exclude-hubs"],
        vec!["cluster-only", "--backend", "fixture"],
        vec!["cluster-only", "--model", "fixture"],
        vec!["cluster-only", "--max-concurrency", "2"],
        vec!["cluster-only", "--batch-size", "2"],
        vec!["cluster-only", "--backend=fixture"],
        vec!["cluster-only", "--model=fixture"],
        vec!["cluster-only", "--max-concurrency=2"],
        vec!["cluster-only", "--batch-size=2"],
        vec!["cluster-only", "--missing-only", "--legacy-option"],
    ] {
        let outcome = invoke(Frontend::Compass, &arguments);
        assert_ne!(outcome.code, 0, "{arguments:?}");
    }

    for arguments in [
        vec!["cluster-only", "missing", "second"],
        vec!["cluster-only", "--exclude-hubs", "not-a-number"],
        vec!["cluster-only", "--resolution=not-a-number"],
        vec!["cluster-only", "--exclude-hubs=2"],
        vec!["cluster-only", "--min-community-size=2"],
        vec!["cluster-only", "--unsupported"],
    ] {
        assert_ne!(
            invoke(Frontend::Compass, &arguments).code,
            0,
            "{arguments:?}"
        );
    }
}

#[test]
fn read_command_missing_values_and_load_errors_are_diagnostic() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let malformed = directory.path().join("malformed.json");
    let wrong_extension = directory.path().join("graph.txt");
    fs::write(&malformed, "not json")?;
    fs::write(&wrong_extension, "{}")?;
    let malformed = malformed.to_string_lossy().into_owned();
    let wrong_extension = wrong_extension.to_string_lossy().into_owned();

    let cases = [
        vec!["query".to_owned(), "q".to_owned(), "--budget".to_owned()],
        vec!["query".to_owned(), "q".to_owned(), "--context".to_owned()],
        vec!["query".to_owned(), "q".to_owned(), "--graph".to_owned()],
        vec![
            "query".to_owned(),
            "q".to_owned(),
            "--budget=bad".to_owned(),
        ],
        vec!["affected".to_owned(), "q".to_owned(), "--graph".to_owned()],
        vec!["affected".to_owned(), "q".to_owned(), "--depth".to_owned()],
        vec![
            "affected".to_owned(),
            "q".to_owned(),
            "--relation".to_owned(),
        ],
        vec![
            "affected".to_owned(),
            "q".to_owned(),
            "--depth=bad".to_owned(),
        ],
        vec![
            "explain".to_owned(),
            "q".to_owned(),
            format!("--graph={wrong_extension}"),
        ],
        vec![
            "explain".to_owned(),
            "q".to_owned(),
            format!("--graph={malformed}"),
        ],
    ];
    for arguments in cases {
        let outcome = invoke_owned(Frontend::Compass, &arguments);
        assert_ne!(outcome.code, 0, "{arguments:?}");
        assert!(!outcome.stderr.is_empty(), "{arguments:?}");
    }
    for frontend in [Frontend::Compass, Frontend::Compass] {
        for arguments in [
            &["query", "q", "--at"][..],
            &["path", "a", "b", "--at"][..],
            &["explain", "a", "--at="][..],
            &["query", "q", "--graph", "graph.json", "--at", "HEAD"][..],
            &["path", "a", "b", "--at", "HEAD", "--at", "HEAD~1"][..],
            &["explain", "a", "extra"][..],
        ] {
            let outcome = invoke(frontend, arguments);
            assert_ne!(outcome.code, 0, "{frontend:?} {arguments:?}");
            assert!(!outcome.stderr.is_empty(), "{frontend:?} {arguments:?}");
        }
    }
    Ok(())
}

#[test]
fn export_parser_reports_all_missing_and_invalid_option_values() {
    for option in [
        "--graph",
        "--labels",
        "--report",
        "--sections",
        "--output",
        "--dir",
        "--push",
        "--user",
        "--password",
        "--lang",
        "--max-sections",
        "--max-diagram-nodes",
        "--max-diagram-edges",
        "--node-limit",
        "--diagram-scale",
    ] {
        let outcome = invoke(Frontend::Compass, &["export", "callflow-html", option]);
        assert_ne!(outcome.code, 0, "{option}");
        assert!(!outcome.stderr.is_empty(), "{option}");
    }
    for (option, value) in [
        ("--max-sections", "bad"),
        ("--max-diagram-nodes", "bad"),
        ("--max-diagram-edges", "bad"),
        ("--node-limit", "bad"),
        ("--diagram-scale", "bad"),
    ] {
        let outcome = invoke(
            Frontend::Compass,
            &["export", "callflow-html", option, value],
        );
        assert_ne!(outcome.code, 0, "{option}");
    }
    assert_eq!(
        invoke(Frontend::Compass, &["export", "callflow-html", "--help"]).code,
        0
    );
}

#[test]
fn compass_legacy_parsers_tolerate_or_report_frozen_edge_cases() {
    let cases: &[&[&str]] = &[
        &["query"],
        &["path"],
        &["explain"],
        &["affected"],
        &["export"],
        &["benchmark", "--corpus-words", "bad"],
        &["merge-graphs"],
        &["diagnose"],
        &["diagnose", "multigraph", "--max-examples", "bad"],
        &[
            "cluster-only",
            "--graph",
            "missing.json",
            "--unknown-legacy",
        ],
        &["update", "--mode", "bad"],
        &["extract", "--mode", "bad"],
        &["cache-check"],
        &["merge-chunks"],
        &["merge-semantic"],
        &["provider"],
        &["save-result"],
        &["reflect"],
        &["global"],
        &["clone"],
        &["add"],
        &["label"],
        &["prs", "--unknown"],
    ];
    for arguments in cases {
        let outcome = invoke(Frontend::Compass, arguments);
        if outcome.code != 0 {
            assert!(!outcome.stderr.is_empty(), "missing error: {arguments:?}");
        }
    }
}

#[test]
fn dense_extract_value_forms_and_compass_formatting_run_end_to_end() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir_all(directory.path().join("src"))?;
    fs::write(directory.path().join("src/lib.rs"), "pub fn run() {}\n")?;
    fs::write(directory.path().join("notes.md"), "# Notes\n")?;
    fs::write(directory.path().join("paper.pdf"), b"%PDF-1.4\n")?;
    fs::write(directory.path().join("image.png"), b"not an image")?;
    for index in 0..8 {
        fs::write(directory.path().join(format!("raw-{index}.blob")), b"raw")?;
    }
    let root = directory.path().to_string_lossy().into_owned();
    let output_path = directory.path().join("artifacts");
    let output = output_path.to_string_lossy().into_owned();
    let arguments = vec![
        "extract".to_owned(),
        root,
        "--code-only".to_owned(),
        "--no-cluster".to_owned(),
        "--force".to_owned(),
        "--timing".to_owned(),
        "--mode".to_owned(),
        "deep".to_owned(),
        "--token-budget".to_owned(),
        "100".to_owned(),
        "--max-concurrency".to_owned(),
        "2".to_owned(),
        "--max-workers".to_owned(),
        "2".to_owned(),
        "--api-timeout".to_owned(),
        "0.25".to_owned(),
        "--exclude".to_owned(),
        "ignored".to_owned(),
        "--resolution".to_owned(),
        "1.0".to_owned(),
        "--exclude-hubs".to_owned(),
        "99".to_owned(),
        "--out".to_owned(),
        output,
    ];
    let outcome = invoke_owned(Frontend::Compass, &arguments);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(outcome.stderr.contains("[compass timing] publish"));
    Ok(())
}

#[test]
fn mcp_option_parser_covers_help_equals_missing_and_invalid_values() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        run_mcp(&[OsString::from("--help")], &mut stdout, &mut stderr),
        0
    );
    assert!(!stdout.is_empty());

    let invalid: &[&[&str]] = &[
        &["--graph"],
        &["--transport", "invalid"],
        &["--transport=invalid"],
        &["--port", "bad"],
        &["--port=bad"],
        &["--session-timeout", "bad"],
        &["--session-timeout=NaN"],
        &["--session-timeout=inf"],
        &["--session-timeout=1e999"],
        &["--wat"],
        &["one.json", "two.json"],
    ];
    for arguments in invalid {
        let args = arguments.iter().map(OsString::from).collect::<Vec<_>>();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(run_mcp(&args, &mut stdout, &mut stderr), 2, "{arguments:?}");
        assert!(!stderr.is_empty());
    }
}

#[test]
fn mcp_valid_option_forms_reach_native_load_failures_without_starting_a_server() {
    let missing = "definitely-missing-coverage-graph.json";
    for arguments in [
        vec![
            "--graph",
            missing,
            "--transport",
            "http",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--api-key",
            "fixture-key",
            "--path",
            "fixture?invalid",
            "--json-response",
            "--stateless",
            "--session-timeout",
            "0",
        ],
        vec![
            "--graph=definitely-missing-coverage-graph.json",
            "--transport=http",
            "--host=127.0.0.1",
            "--port=0",
            "--api-key=fixture-key",
            "--path=fixture#invalid",
            "--session-timeout=-1",
        ],
    ] {
        let args = arguments.iter().map(OsString::from).collect::<Vec<_>>();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(run_mcp(&args, &mut stdout, &mut stderr), 1, "{arguments:?}");
        assert!(!stderr.is_empty());
    }
}

#[test]
fn watch_option_parser_covers_help_validation_and_missing_path() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        run_watch(&[OsString::from("--help")], &mut stdout, &mut stderr),
        0
    );
    assert!(!stdout.is_empty());

    for arguments in [
        vec!["--unknown"],
        vec!["--debounce", "bad"],
        vec!["--debounce=0"],
        vec!["one", "two"],
    ] {
        let args = arguments.iter().map(OsString::from).collect::<Vec<_>>();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(run_watch(&args, &mut stdout, &mut stderr), 1);
        assert!(!stderr.is_empty());
    }

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        run_watch(
            &[OsString::from("definitely-missing-watch-root")],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert!(!stderr.is_empty());
}

#[test]
fn valid_watch_options_reach_missing_root_failure_after_full_parse() {
    let args = [
        "definitely-missing-watch-coverage-root",
        "--no-cluster",
        "--no-viz",
        "--no-gitignore",
        "--poll",
        "--debounce=0.01",
        "--out=coverage-out",
        "--exclude=vendor/**",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(run_watch(&args, &mut stdout, &mut stderr), 1);
    assert!(!stderr.is_empty());
}

#[test]
fn completed_read_query_diagnostic_merge_tree_and_export_commands_run_end_to_end()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    fs::create_dir_all(&output)?;
    let graph = output.join("graph.json");
    write_graph_fixture(&graph)?;
    fs::write(output.join("labels.json"), r#"{"0":"Core"}"#)?;
    fs::write(
        output.join("analysis.json"),
        r#"{"communities":{"0":["n_transformer","n_attention","n_layernorm","n_concept_attn"]},"cohesion":{"0":0.75}}"#,
    )?;
    fs::write(output.join("GRAPH_REPORT.md"), "# Fixture\n")?;
    let graph = graph.to_string_lossy().into_owned();

    let cases = [
        vec![
            "query".to_owned(),
            "attention".to_owned(),
            "--budget=100".to_owned(),
            format!("--graph={graph}"),
        ],
        vec![
            "path".to_owned(),
            "Transformer".to_owned(),
            "attention mechanism".to_owned(),
            "--graph".to_owned(),
            graph.clone(),
        ],
        vec![
            "explain".to_owned(),
            "MultiHeadAttention".to_owned(),
            format!("--graph={graph}"),
        ],
        vec![
            "affected".to_owned(),
            "Transformer".to_owned(),
            "--depth=3".to_owned(),
            "--relation=contains".to_owned(),
            format!("--graph={graph}"),
        ],
        vec!["benchmark".to_owned(), graph.clone()],
        vec![
            "diagnose".to_owned(),
            "multigraph".to_owned(),
            "--graph".to_owned(),
            graph.clone(),
            "--json".to_owned(),
            "--max-examples".to_owned(),
            "0".to_owned(),
            "--directed".to_owned(),
        ],
    ];
    for arguments in cases {
        let result = invoke_owned(Frontend::Compass, &arguments);
        assert_eq!(result.code, 0, "{arguments:?}: {}", result.stderr);
        assert!(!result.stdout.is_empty());
    }

    let tree = directory.path().join("tree.html");
    let result = invoke_owned(
        Frontend::Compass,
        &[
            "tree".to_owned(),
            "--graph".to_owned(),
            graph.clone(),
            "--output".to_owned(),
            tree.to_string_lossy().into_owned(),
            "--root".to_owned(),
            "src".to_owned(),
            "--max-children".to_owned(),
            "2".to_owned(),
            "--top-k-edges".to_owned(),
            "4".to_owned(),
            "--label".to_owned(),
            "Fixture".to_owned(),
        ],
    );
    assert_eq!(result.code, 0, "{}", result.stderr);
    assert!(tree.is_file());

    let second = directory.path().join("second.json");
    fs::copy(&graph, &second)?;
    let merged = directory.path().join("merged.json");
    let merge = invoke_owned(
        Frontend::Compass,
        &[
            "merge-graphs".to_owned(),
            graph.clone(),
            second.to_string_lossy().into_owned(),
            "--out".to_owned(),
            merged.to_string_lossy().into_owned(),
        ],
    );
    assert_eq!(merge.code, 0, "{}", merge.stderr);
    assert!(merged.is_file());

    for format in [
        "html", "svg", "graphml", "neo4j", "falkordb", "obsidian", "wiki",
    ] {
        let mut arguments = vec![
            "export".to_owned(),
            format.to_owned(),
            "--graph".to_owned(),
            graph.clone(),
        ];
        if format == "html" {
            arguments.push("--no-viz".to_owned());
        }
        if format == "obsidian" {
            arguments.extend([
                "--dir".to_owned(),
                directory
                    .path()
                    .join("vault")
                    .to_string_lossy()
                    .into_owned(),
            ]);
        }
        let result = invoke_owned(Frontend::Compass, &arguments);
        assert_eq!(result.code, 0, "{format}: {}", result.stderr);
    }

    let labels = directory.path().join("labels.json");
    let report = directory.path().join("report.md");
    let sections = directory.path().join("sections.json");
    let callflow = directory.path().join("callflow.html");
    fs::write(&labels, r#"{"labels":{"0":{"name":"Runtime"}}}"#)?;
    fs::write(&report, "# Runtime report\n")?;
    fs::write(
        &sections,
        r#"{"sections":[{"id":"runtime","name":"Runtime","communities":["0"]}]}"#,
    )?;
    let callflow_result = invoke_owned(
        Frontend::Compass,
        &[
            "export".to_owned(),
            "callflow-html".to_owned(),
            graph,
            "--labels".to_owned(),
            labels.to_string_lossy().into_owned(),
            "--report".to_owned(),
            report.to_string_lossy().into_owned(),
            "--sections".to_owned(),
            sections.to_string_lossy().into_owned(),
            "--output".to_owned(),
            callflow.to_string_lossy().into_owned(),
            "--lang".to_owned(),
            "en".to_owned(),
            "--max-sections".to_owned(),
            "1".to_owned(),
            "--max-diagram-nodes".to_owned(),
            "2".to_owned(),
            "--max-diagram-edges".to_owned(),
            "2".to_owned(),
            "--diagram-scale".to_owned(),
            "1.25".to_owned(),
        ],
    );
    assert_eq!(callflow_result.code, 0, "{}", callflow_result.stderr);
    assert!(callflow.is_file());
    Ok(())
}

#[test]
fn split_value_read_export_and_cluster_forms_complete_against_a_real_graph()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    let guard = BuildGuard::begin(&output)?;
    let snapshot = guard.staging_directory().to_path_buf();
    write_graph_fixture(&snapshot.join("graph.json"))?;
    fs::write(snapshot.join("labels.json"), r#"{"0":"Core"}"#)?;
    fs::write(
        snapshot.join("analysis.json"),
        r#"{"communities":{"0":["n_transformer","n_attention"]},"cohesion":{"0":0.5}}"#,
    )?;
    fs::write(snapshot.join("GRAPH_REPORT.md"), "# Fixture\n")?;
    let artifacts = [
        "graph.json",
        "labels.json",
        "analysis.json",
        "GRAPH_REPORT.md",
    ];
    guard.commit_with_artifacts(&artifacts)?;
    BuildGuard::publish_root_artifacts(&output, &artifacts, true)?;
    let graph = output.join("graph.json");
    let graph_text = graph.to_string_lossy().into_owned();

    for arguments in [
        vec![
            "query".to_owned(),
            "attention".to_owned(),
            "--budget".to_owned(),
            "80".to_owned(),
            "--graph".to_owned(),
            graph_text.clone(),
        ],
        vec![
            "affected".to_owned(),
            "Transformer".to_owned(),
            "--depth".to_owned(),
            "3".to_owned(),
            "--relation".to_owned(),
            "contains".to_owned(),
            "--graph".to_owned(),
            graph_text.clone(),
        ],
    ] {
        let result = invoke_owned(Frontend::Compass, &arguments);
        assert_eq!(result.code, 0, "{arguments:?}: {}", result.stderr);
    }

    let html = invoke_owned(
        Frontend::Compass,
        &[
            "export".to_owned(),
            "html".to_owned(),
            "--graph".to_owned(),
            graph_text.clone(),
            "--node-limit".to_owned(),
            "0".to_owned(),
            "--no-viz".to_owned(),
        ],
    );
    assert_eq!(html.code, 0, "{}", html.stderr);

    let callflow = directory.path().join("directory-callflow.html");
    let callflow_result = invoke_owned(
        Frontend::Compass,
        &[
            "export".to_owned(),
            "callflow-html".to_owned(),
            output.to_string_lossy().into_owned(),
            "--output".to_owned(),
            callflow.to_string_lossy().into_owned(),
        ],
    );
    assert_eq!(callflow_result.code, 0, "{}", callflow_result.stderr);
    assert!(callflow.is_file());

    let clustered = invoke_owned(
        Frontend::Compass,
        &[
            "cluster-only".to_owned(),
            directory.path().to_string_lossy().into_owned(),
            "--graph".to_owned(),
            graph_text,
            "--no-label".to_owned(),
            "--no-viz".to_owned(),
            "--timing".to_owned(),
            "--resolution".to_owned(),
            "1".to_owned(),
            "--exclude-hubs".to_owned(),
            "100".to_owned(),
            "--min-community-size=1".to_owned(),
        ],
    );
    assert_eq!(clustered.code, 0, "{}", clustered.stderr);
    assert!(clustered.stdout.contains("communities"));
    assert!(clustered.stderr.contains("[compass timing] total"));
    Ok(())
}

#[test]
fn install_and_extract_equals_forms_cover_namespaced_parser_boundaries()
-> Result<(), Box<dyn Error>> {
    for (frontend, arguments) in [
        (Frontend::Compass, vec!["install", "--platform"]),
        (Frontend::Compass, vec!["install", "--platform=unknown"]),
        (Frontend::Compass, vec!["install", "--unknown"]),
        (Frontend::Compass, vec!["install", "--all", "claude"]),
        (Frontend::Compass, vec!["uninstall", "--platform"]),
        (Frontend::Compass, vec!["uninstall", "--unknown"]),
        (Frontend::Compass, vec!["install", "--platform"]),
        (Frontend::Compass, vec!["uninstall", "--platform"]),
    ] {
        let outcome = invoke(frontend, &arguments);
        assert_ne!(outcome.code, 0, "{arguments:?}");
        assert!(!outcome.stderr.is_empty(), "{arguments:?}");
    }

    let directory = tempfile::tempdir()?;
    let missing = directory.path().join("missing-root");
    let output = directory.path().join("out");
    let arguments = vec![
        "extract".to_owned(),
        missing.to_string_lossy().into_owned(),
        "--as=fixture".to_owned(),
        "--backend=fixture".to_owned(),
        "--model=fixture-model".to_owned(),
        "--mode=deep".to_owned(),
        "--token-budget=1".to_owned(),
        "--max-concurrency=1".to_owned(),
        "--api-timeout=0.01".to_owned(),
        format!("--out={}", output.display()),
        "--exclude=vendor/**".to_owned(),
        "--resolution=1".to_owned(),
        "--exclude-hubs=2".to_owned(),
        "--max-workers=1".to_owned(),
        "--allow-partial".to_owned(),
        "--timing".to_owned(),
    ];
    let outcome = invoke_owned(Frontend::Compass, &arguments);
    assert_ne!(outcome.code, 0);
    assert!(
        outcome.stderr.contains("missing-root"),
        "{}",
        outcome.stderr
    );

    let postgres = invoke(
        Frontend::Compass,
        &["extract", "missing", "--postgres=not-a-dsn"],
    );
    assert_ne!(postgres.code, 0);
    assert!(!postgres.stderr.is_empty());

    for option in [
        "--mode=shallow",
        "--token-budget=0",
        "--max-concurrency=0",
        "--api-timeout=inf",
        "--resolution=0",
        "--exclude-hubs=NaN",
        "--max-workers=0",
    ] {
        let outcome = invoke(Frontend::Compass, &["extract", "missing", option]);
        assert_ne!(outcome.code, 0, "{option}");
    }
    Ok(())
}

#[test]
fn semantic_provider_failures_are_formatted_after_ast_detection() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("main.rs"), "pub fn local() {}\n")?;
    fs::write(
        directory.path().join("guide.md"),
        "# Guide\n\nA semantic concept connects the local service to an external system.\n",
    )?;
    let root = directory.path().to_string_lossy().into_owned();

    let compass = invoke_owned(
        Frontend::Compass,
        &[
            "extract".to_owned(),
            root.clone(),
            "--backend".to_owned(),
            "definitely-missing".to_owned(),
            "--no-cluster".to_owned(),
            "--no-viz".to_owned(),
        ],
    );
    assert_ne!(compass.code, 0);
    assert!(
        compass.stderr.contains("unknown backend"),
        "{}",
        compass.stderr
    );

    let compass = invoke_owned(
        Frontend::Compass,
        &[
            "extract".to_owned(),
            root,
            "--backend".to_owned(),
            "definitely-missing".to_owned(),
            "--no-cluster".to_owned(),
            "--no-viz".to_owned(),
            "--force".to_owned(),
        ],
    );
    assert_ne!(compass.code, 0);
    assert!(
        compass.stderr.contains("unknown backend") || compass.stdout.contains("unknown backend"),
        "stdout={} stderr={}",
        compass.stdout,
        compass.stderr
    );
    Ok(())
}
