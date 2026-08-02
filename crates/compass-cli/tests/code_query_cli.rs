mod support;

use std::error::Error;
use std::ffi::OsString;

use compass_cli::{Frontend, run};
use compass_files::BuildGuard;
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
fn typed_query_resolves_the_active_generation_from_the_public_path() -> Result<(), Box<dyn Error>> {
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
fn typed_query_prefers_active_generation_over_stale_legacy_graph() -> Result<(), Box<dyn Error>> {
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
fn typed_query_fails_closed_on_a_malformed_generation_pointer() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    support::write_typed_graph(&output)?;
    std::fs::write(output.join(".compass-active-generation"), "../escape")?;

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
    assert!(outcome.stderr.contains("generation"));
    Ok(())
}

#[test]
fn natural_query_reads_legacy_only_when_the_generation_pointer_is_absent()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("compass-out");
    support::write_typed_graph(&output)?;

    let legacy = run(
        Frontend::Compass,
        [
            OsString::from("query"),
            OsString::from("Target"),
            OsString::from("--graph"),
            output.join("graph.json").into_os_string(),
        ],
    );
    assert_eq!(legacy.code, 0, "{}", legacy.stderr);

    std::fs::write(output.join(".compass-active-generation"), "../escape")?;
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
        malformed.stderr.contains("generation"),
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
