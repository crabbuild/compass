mod support;

use std::error::Error;
use std::path::Path;
use std::process::Command;

use serde_json::json;

fn seed(root: &Path) -> Result<(), Box<dyn Error>> {
    let output = root.join("compass-out");
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("graph.json"),
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id": "a'1", "label": "Alpha", "file_type": "python", "source_file": "src/a.py"},
                {"id": "b", "label": "Beta", "file_type": "rust", "source_file": "src/b.rs"}
            ],
            "links": [
                {"source": "a'1", "target": "b", "relation": "calls", "confidence": "EXTRACTED"}
            ]
        }))?,
    )?;
    Ok(())
}

#[test]
fn live_push_validation_is_safe_and_namespaced() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    seed(directory.path())?;
    let graph = directory.path().join("compass-out/graph.json");
    let missing = support::compass_command()
        .args(["export", "neo4j", "--push", "bolt://127.0.0.1:1", "--graph"])
        .arg(&graph)
        .current_dir(directory.path())
        .env_remove("NEO4J_PASSWORD")
        .output()?;
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&missing.stderr),
        "error: --password required for --push\n"
    );

    let failed = support::compass_command()
        .args([
            "export",
            "neo4j",
            "--push",
            "bolt://127.0.0.1:1",
            "--password",
            "never-print-this",
            "--graph",
        ])
        .arg(&graph)
        .current_dir(directory.path())
        .env("COMPASS_GRAPHDB_TIMEOUT", "1")
        .output()?;
    assert_eq!(failed.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("never-print-this"));

    let help = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["export", "--help"])
        .output()?;
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("neo4j"));
    assert!(help.contains("falkordb"));
    Ok(())
}
