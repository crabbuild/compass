use std::error::Error;
use std::fs;

use sha2::{Digest, Sha256};
use trail_core::{ExportInputs, LoadedGraph};

#[test]
fn export_inputs_fall_back_to_node_communities_and_tolerate_partial_sidecars()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("graphify-out");
    fs::create_dir_all(&output)?;
    let graph = output.join("graph.json");
    fs::write(
        &graph,
        r#"{"directed":false,"multigraph":false,"graph":{},"nodes":[{"id":"a","label":"A","community":0},{"id":"b","label":"B","community":"1"},{"id":"c","label":"C","community":"bad"}],"links":[]}"#,
    )?;
    fs::write(
        output.join(".graphify_analysis.json"),
        r#"{"communities":{"bad":"not-an-array"},"cohesion":{"0":0.75,"bad":1,"1":"wrong"},"gods":"wrong"}"#,
    )?;
    fs::write(
        output.join(".graphify_labels.json"),
        r#"{"0":"Core","1":7,"bad":"ignored"}"#,
    )?;
    fs::write(output.join("GRAPH_REPORT.md"), "# Fixture\n")?;

    let inputs = ExportInputs::load(&graph)?;
    assert_eq!(inputs.communities.get(&0), Some(&vec!["a".to_owned()]));
    assert_eq!(inputs.communities.get(&1), Some(&vec!["b".to_owned()]));
    assert_eq!(inputs.cohesion.get(&0), Some(&0.75));
    assert_eq!(inputs.labels.get(&0).map(String::as_str), Some("Core"));
    assert!(inputs.gods.is_empty());
    assert_eq!(inputs.report, "# Fixture\n");
    Ok(())
}

#[test]
fn loaded_graph_learning_overlay_marks_current_missing_and_unfingerprinted_sources()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("graphify-out");
    fs::create_dir_all(&output)?;
    let source = directory.path().join("source.rs");
    let contents = b"pub fn current() {}\n";
    fs::write(&source, contents)?;
    let fingerprint = format!("{:x}", Sha256::digest(contents));
    let graph = output.join("graph.json");
    fs::write(
        &graph,
        r#"{"directed":false,"multigraph":false,"graph":{},"nodes":[{"id":"a","label":"A"}],"links":[]}"#,
    )?;
    fs::write(
        output.join(".graphify_root"),
        directory.path().to_string_lossy().as_bytes(),
    )?;
    fs::write(
        output.join(".graphify_learning.json"),
        serde_json::to_vec(&serde_json::json!({
            "nodes": {
                "current": {"source_file":"source.rs","code_fingerprint":fingerprint},
                "empty": {"source_file":"","code_fingerprint":""},
                "missing": {"source_file":"missing.rs","code_fingerprint":"abc"},
                "unfingerprinted": {"source_file":"source.rs","code_fingerprint":""},
                "wrong": {"source_file":"source.rs","code_fingerprint":"wrong"},
                "ignored": "not-an-object"
            }
        }))?,
    )?;

    let loaded = LoadedGraph::load(&graph)?;
    assert_eq!(loaded.graph.node_count(), 1);
    assert_eq!(loaded.overlay["current"]["stale"], false);
    assert_eq!(loaded.overlay["empty"]["stale"], false);
    assert_eq!(loaded.overlay["missing"]["stale"], true);
    assert_eq!(loaded.overlay["unfingerprinted"]["stale"], true);
    assert_eq!(loaded.overlay["wrong"]["stale"], true);
    assert!(!loaded.overlay.contains_key("ignored"));

    let directed = LoadedGraph::load_directed(&graph)?;
    assert_eq!(directed.graph.node_count(), 1);
    fs::write(output.join(".graphify_learning.json"), "not json")?;
    assert!(LoadedGraph::load(&graph)?.overlay.is_empty());
    fs::remove_file(output.join(".graphify_learning.json"))?;
    assert!(LoadedGraph::load(&graph)?.overlay.is_empty());
    Ok(())
}
