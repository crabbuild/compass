use std::ffi::OsString;

use trail_cli::{Frontend, McpFrontend, run, run_graphify_watch, run_mcp, run_watch};

fn invoke(frontend: Frontend, arguments: &[&str]) -> trail_cli::Outcome {
    run(
        frontend,
        arguments.iter().map(|argument| OsString::from(*argument)),
    )
}

#[test]
fn frontend_roots_versions_help_and_unknown_commands_are_total() {
    assert_ne!(invoke(Frontend::Trail, &[]).code, 0);
    for arguments in [vec!["--help"], vec!["help"], vec!["--version"]] {
        assert_eq!(invoke(Frontend::Trail, &arguments).code, 0);
    }
    assert_ne!(invoke(Frontend::Trail, &["query"]).code, 0);
    assert_eq!(invoke(Frontend::Trail, &["graph"]).code, 0);
    assert_eq!(invoke(Frontend::Trail, &["graph", "--version"]).code, 0);
    assert_ne!(invoke(Frontend::Trail, &["graph", "unknown"]).code, 0);
    assert_ne!(invoke(Frontend::Trail, &["graph", "watch"]).code, 0);
    assert_ne!(invoke(Frontend::Trail, &["graph", "serve"]).code, 0);

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
    let trail_cases: &[&[&str]] = &[
        &["graph", "query"],
        &["graph", "query", "x", "--depth", "bad"],
        &["graph", "query", "x", "--unknown"],
        &["graph", "path"],
        &["graph", "path", "only-one"],
        &["graph", "explain"],
        &["graph", "affected"],
        &["graph", "affected", "x", "--depth", "bad"],
        &["graph", "export"],
        &["graph", "export", "unknown-format"],
        &["graph", "benchmark", "--corpus-words", "bad"],
        &["graph", "merge-graphs"],
        &["graph", "merge-graphs", "--output"],
        &["graph", "tree", "--depth", "bad"],
        &["graph", "tree", "--unknown"],
        &["graph", "cluster-only", "--graph"],
        &["graph", "cluster-only", "--resolution", "bad"],
        &["graph", "cluster-only", "--exclude-hubs=bad"],
        &["graph", "cluster-only", "--min-community-size=bad"],
        &["graph", "diagnose"],
        &["graph", "diagnose", "multigraph", "--graph"],
        &["graph", "diagnose", "multigraph", "--max-examples", "bad"],
        &["graph", "diagnose", "multigraph", "--max-examples", "-1"],
        &[
            "graph",
            "diagnose",
            "multigraph",
            "--directed",
            "--undirected",
        ],
        &["graph", "diagnose", "multigraph", "--extract-path"],
        &["graph", "diagnose", "multigraph", "--wat"],
        &["graph", "update", "--mode", "invalid"],
        &["graph", "update", "--workers", "bad"],
        &["graph", "update", "--resolution", "bad"],
        &["graph", "update", "--exclude-hubs", "bad"],
        &["graph", "update", "--min-community-size", "bad"],
        &["graph", "update", "--max-nodes", "bad"],
        &["graph", "update", "--semantic-timeout", "bad"],
        &["graph", "update", "--unknown"],
        &["graph", "extract", "--code-only", "--mode", "invalid"],
        &["graph", "cache-check"],
        &["graph", "merge-chunks"],
        &["graph", "merge-semantic"],
        &["graph", "save-result"],
        &["graph", "reflect"],
        &["graph", "check-update", "--wat"],
        &["graph", "hook-check", "--wat"],
        &["graph", "hook-guard", "--wat"],
        &["graph", "merge-driver"],
        &["graph", "global"],
        &["graph", "clone"],
        &["graph", "add"],
        &["graph", "label", "--unknown"],
        &["graph", "prs", "--unknown"],
        &["graph", "hook", "--unknown"],
        &["graph", "hook-spawn", "--unknown"],
        &["graph", "hook-refresh", "--unknown"],
    ];
    for arguments in trail_cases {
        let outcome = invoke(Frontend::Trail, arguments);
        assert!(outcome.code <= 2, "invalid exit code: {arguments:?}");
    }
    assert_eq!(invoke(Frontend::Trail, &["graph", "provider"]).code, 0);
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
fn mcp_option_parser_covers_help_equals_missing_and_invalid_values() {
    for frontend in [McpFrontend::Trail, McpFrontend::Graphify] {
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
            run_mcp(McpFrontend::Trail, &args, &mut stdout, &mut stderr),
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
            run_mcp(McpFrontend::Trail, &args, &mut stdout, &mut stderr),
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
