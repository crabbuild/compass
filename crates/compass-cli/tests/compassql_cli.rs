mod support;

use std::error::Error;
use std::process::Command;

fn seed_graph(directory: &std::path::Path) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let path = directory.join("graph.json");
    std::fs::write(
        &path,
        br#"{
          "directed": true,
          "multigraph": true,
          "graph": {},
          "nodes": [
            {"id":"a","label":"a()","file_type":"function"},
            {"id":"b","label":"b()","file_type":"function"}
          ],
          "links": [
            {"source":"a","target":"b","relation":"calls","confidence":"EXTRACTED"}
          ]
        }"#,
    )?;
    Ok(path)
}

#[test]
fn compassql_cli_supports_typed_output_files_limits_and_graphify_isolation()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = seed_graph(directory.path())?;
    let output_path = directory.path().join("result.json");
    let compass = env!("CARGO_BIN_EXE_compass");
    let output = Command::new(compass)
        .args([
            "query",
            "--cql",
            "PROFILE MATCH (a)-[:CALLS]->(b) RETURN a.id AS caller, b.id AS callee",
            "--format=json",
            "--timeout-ms=2000",
            "--max-rows=10",
            "--max-path-depth=4",
            "--max-expanded-relationships=100",
            "--max-memory-bytes=1048576",
            "--graph",
        ])
        .arg(&graph)
        .args(["--output"])
        .arg(&output_path)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&output_path)?)?;
    assert_eq!(value["schema"], "compass.cql.result/1");
    assert_eq!(value["rows"][0]["caller"]["value"], "a");
    assert_eq!(value["profile"]["plan_cache_hit"], false);

    let rejected = support::compat_command()
        .args(["query", "--cql", "RETURN 1"])
        .output()?;
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("Compass-only"));
    Ok(())
}
