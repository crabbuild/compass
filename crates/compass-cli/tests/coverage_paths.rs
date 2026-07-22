use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use compass_cli::{Frontend, McpFrontend, run, run_graphify_watch, run_mcp, run_watch};

fn invoke(frontend: Frontend, arguments: &[&str]) -> compass_cli::Outcome {
    run(
        frontend,
        arguments.iter().map(|argument| OsString::from(*argument)),
    )
}

fn invoke_owned(frontend: Frontend, arguments: &[String]) -> compass_cli::Outcome {
    run(frontend, arguments.iter().map(OsString::from))
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
        assert_eq!(invoke(Frontend::Graphify, &arguments).code, 0);
    }
    let unknown = invoke(Frontend::Graphify, &["not-real"]);
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
        "not-real",
    ] {
        let outcome = invoke(Frontend::Compass, &[command, "--help"]);
        assert_eq!(outcome.code, 0, "{command}: {}", outcome.stderr);
        assert!(!outcome.stdout.is_empty(), "{command}");
    }

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
        let outcome = invoke(Frontend::Graphify, &arguments);
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
        let outcome = invoke(Frontend::Graphify, &["export", "callflow-html", option]);
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
            Frontend::Graphify,
            &["export", "callflow-html", option, value],
        );
        assert_ne!(outcome.code, 0, "{option}");
    }
    assert_eq!(
        invoke(Frontend::Graphify, &["export", "callflow-html", "--help"]).code,
        0
    );
}

#[test]
fn graphify_legacy_parsers_tolerate_or_report_frozen_edge_cases() {
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
        let outcome = invoke(Frontend::Graphify, arguments);
        if outcome.code != 0 {
            assert!(!outcome.stderr.is_empty(), "missing error: {arguments:?}");
        }
    }
}

#[test]
fn dense_extract_value_forms_and_graphify_formatting_run_end_to_end() -> Result<(), Box<dyn Error>>
{
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
    let output = directory.path().join("artifacts");
    let output = output.to_string_lossy().into_owned();
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
    let outcome = invoke_owned(Frontend::Graphify, &arguments);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(outcome.stdout.contains("--force"));
    assert!(outcome.stdout.contains("--code-only"));
    assert!(outcome.stdout.contains("(+2 more)"));
    assert!(outcome.stdout.contains("no clustering"));
    assert!(outcome.stderr.contains("[graphify timing] write"));
    Ok(())
}

#[test]
fn mcp_option_parser_covers_help_equals_missing_and_invalid_values() {
    for frontend in [McpFrontend::Compass, McpFrontend::Graphify] {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_mcp(
                frontend,
                &[OsString::from("--help")],
                &mut stdout,
                &mut stderr
            ),
            0
        );
        assert!(!stdout.is_empty());
    }

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
        assert_eq!(
            run_mcp(McpFrontend::Compass, &args, &mut stdout, &mut stderr),
            2,
            "{arguments:?}"
        );
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
        assert_eq!(
            run_mcp(McpFrontend::Compass, &args, &mut stdout, &mut stderr),
            1,
            "{arguments:?}"
        );
        assert!(!stderr.is_empty());
    }
}

#[test]
fn watch_option_parser_covers_help_validation_and_legacy_missing_path() {
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
        run_graphify_watch(
            &[OsString::from("definitely-missing-watch-root")],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert!(String::from_utf8_lossy(&stderr).contains("path not found"));
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
    let output = directory.path().join("graphify-out");
    fs::create_dir_all(&output)?;
    let graph = output.join("graph.json");
    let repository = std::env::var_os("GRAPHIFY_REPO_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .map(Path::to_path_buf)
        })
        .ok_or("repository root")?;
    fs::copy(repository.join("tests/fixtures/extraction.json"), &graph)?;
    fs::write(output.join(".graphify_labels.json"), r#"{"0":"Core"}"#)?;
    fs::write(
        output.join(".graphify_analysis.json"),
        r#"{"communities":{"0":["n_transformer","n_attention","n_layernorm","n_concept_attn"]},"cohesion":{"0":0.75}}"#,
    )?;
    fs::write(output.join("GRAPH_REPORT.md"), "# Fixture\n")?;
    let graph = graph.to_string_lossy().into_owned();

    let cases = [
        vec![
            "query".to_owned(),
            "attention".to_owned(),
            "--dfs".to_owned(),
            "--budget=100".to_owned(),
            "--context=model.py".to_owned(),
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
        Frontend::Graphify,
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
    let output = directory.path().join("graphify-out");
    fs::create_dir_all(&output)?;
    let graph = output.join("graph.json");
    let repository = std::env::var_os("GRAPHIFY_REPO_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .map(Path::to_path_buf)
        })
        .ok_or("repository root")?;
    fs::copy(repository.join("tests/fixtures/extraction.json"), &graph)?;
    fs::write(output.join(".graphify_labels.json"), r#"{"0":"Core"}"#)?;
    fs::write(
        output.join(".graphify_analysis.json"),
        r#"{"communities":{"0":["n_transformer","n_attention"]},"cohesion":{"0":0.5}}"#,
    )?;
    fs::write(output.join("GRAPH_REPORT.md"), "# Fixture\n")?;
    let graph_text = graph.to_string_lossy().into_owned();

    for arguments in [
        vec![
            "query".to_owned(),
            "attention".to_owned(),
            "--budget".to_owned(),
            "80".to_owned(),
            "--context".to_owned(),
            "call".to_owned(),
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
        Frontend::Graphify,
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
        Frontend::Graphify,
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
    assert!(clustered.stderr.contains("[graphify timing] total"));
    Ok(())
}

#[test]
fn install_and_extract_equals_forms_cover_namespaced_parser_boundaries()
-> Result<(), Box<dyn Error>> {
    for (frontend, arguments) in [
        (Frontend::Compass, vec!["install", "--platform"]),
        (Frontend::Compass, vec!["install", "--platform=unknown"]),
        (Frontend::Compass, vec!["install", "--unknown"]),
        (Frontend::Compass, vec!["install", "cursor", "claude"]),
        (Frontend::Compass, vec!["uninstall", "--platform"]),
        (Frontend::Compass, vec!["uninstall", "--unknown"]),
        (Frontend::Graphify, vec!["install", "--platform"]),
        (Frontend::Graphify, vec!["uninstall", "--platform"]),
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
fn semantic_provider_failures_are_formatted_for_both_frontends_after_ast_detection()
-> Result<(), Box<dyn Error>> {
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

    let graphify = invoke_owned(
        Frontend::Graphify,
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
    assert_ne!(graphify.code, 0);
    assert!(
        graphify.stderr.contains("unknown backend") || graphify.stdout.contains("unknown backend"),
        "stdout={} stderr={}",
        graphify.stdout,
        graphify.stderr
    );
    Ok(())
}
