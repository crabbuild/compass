mod support;

use std::error::Error;
use std::ffi::OsString;

use compass_cli::{Frontend, run};
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
