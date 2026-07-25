mod support;

use std::error::Error;

use serde_json::{Value, json};

#[test]
fn viewer_json_exposes_the_same_versioned_graph_model() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id":"run","label":"run","source_file":"src/lib.rs","line_start":3},
                {"id":"helper","label":"helper"}
            ],
            "links": [
                {"source":"run","target":"helper","relation":"calls","confidence":"EXTRACTED"}
            ]
        }))?,
    )?;
    let output = support::compat_command()
        .args([
            "export",
            "viewer-json",
            "--graph",
            graph.to_string_lossy().as_ref(),
        ])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["schema"], "compass.viewer.graph/1");
    assert_eq!(value["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["edges"][0]["relation"], "calls");
    assert_eq!(value["edges"][0]["source"], "run");
    assert_eq!(value["nodes"][0]["source"]["file"], "src/lib.rs");
    Ok(())
}
