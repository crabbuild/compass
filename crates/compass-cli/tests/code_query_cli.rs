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
    Ok(())
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
            .contains("NODE Target [src=src/lib.rs loc=L1:0-L1:4")
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
