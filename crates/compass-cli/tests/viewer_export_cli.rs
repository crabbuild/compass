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
    let output = support::compass_command()
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

#[test]
fn canonical_json_exports_one_complete_community() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {"hyperedges":[
                {"id":"inside","nodes":["run","helper"]},
                {"id":"cross","nodes":["run","other"]}
            ]},
            "nodes": [
                {"id":"run","label":"run","community":7,"source_file":"src/lib.rs","line_start":3},
                {"id":"helper","label":"helper","community":7},
                {"id":"other","label":"other","community":8}
            ],
            "links": [
                {"source":"run","target":"helper","relation":"calls","confidence":"EXTRACTED"},
                {"source":"run","target":"other","relation":"calls","confidence":"INFERRED"}
            ]
        }))?,
    )?;
    let output = support::compass_command()
        .args([
            "export",
            "json",
            "--graph",
            graph.to_string_lossy().as_ref(),
            "--community",
            "7",
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
    assert_eq!(value["stats"]["aggregated"], false);
    assert_eq!(value["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["edges"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["hyperedges"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["hyperedges"][0]["id"], "inside");

    let unsupported = support::compass_command()
        .args([
            "export",
            "html",
            "--graph",
            graph.to_string_lossy().as_ref(),
            "--community",
            "7",
        ])
        .current_dir(directory.path())
        .output()?;
    assert_ne!(unsupported.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("only valid with export json"));
    Ok(())
}

#[test]
fn callflow_json_exposes_the_shared_architecture_model() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {"project_name":"Fixture"},
            "nodes": [
                {"id":"run","label":"run","community":0,"source_file":"src/lib.rs"},
                {"id":"store","label":"store","community":1,"source_file":"src/store.rs"}
            ],
            "links": [
                {"source":"run","target":"store","relation":"calls","confidence":"INFERRED"}
            ]
        }))?,
    )?;
    let output = support::compass_command()
        .args([
            "export",
            "callflow-json",
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
    assert_eq!(value["schema"], "compass.viewer.callflow/2");
    assert_eq!(value["statistics"]["inferred"], 1);
    assert_eq!(value["coverage"]["crossSection"], 1);
    assert_eq!(value["crossSectionCalls"][0]["source"], "run");
    assert_eq!(value["crossSectionCalls"][0]["target"], "store");
    assert_eq!(value["crossSectionCalls"][0]["confidence"], "inferred");
    assert!(
        value["sections"]
            .as_array()
            .is_some_and(|sections| sections.len() >= 2)
    );
    Ok(())
}
