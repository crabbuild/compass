use std::error::Error;
use std::fs;

use sha2::{Digest, Sha256};
use trail_core::{
    BuildOptions, CoreError, ExportInputs, LoadedGraph, SemanticLayer, build_graph_with_layers,
};

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

#[test]
fn build_pipeline_reports_missing_and_empty_roots_and_accepts_file_only_sources()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let mut missing = BuildOptions::new(directory.path().join("missing"));
    missing.no_cluster = true;
    missing.no_viz = true;
    assert!(matches!(
        build_graph_with_layers(&missing, None, &[]),
        Err(CoreError::MissingRoot(_))
    ));

    let empty = directory.path().join("empty");
    fs::create_dir(&empty)?;
    let mut options = BuildOptions::new(empty.clone());
    options.no_cluster = false;
    options.no_viz = true;
    assert!(matches!(
        build_graph_with_layers(&options, None, &[]),
        Err(CoreError::EmptyGraph)
    ));

    fs::write(empty.join("comments.rs"), "// no declarations\n")?;
    let file_only = build_graph_with_layers(&options, None, &[])?;
    assert_eq!(file_only.nodes, 1);
    Ok(())
}

#[test]
fn semantic_build_normalizes_origins_paths_hyperedges_and_unicode_json()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("main.rs"), "pub fn main_entry() {}\n")?;
    fs::write(directory.path().join("notes.md"), "# Café 🚀\n")?;
    let mut options = BuildOptions::new(directory.path().to_path_buf());
    options.no_cluster = true;
    options.no_viz = true;
    let semantic = SemanticLayer {
        fragment: serde_json::json!({
            "directed":true,
            "multigraph":false,
            "hyperedges":[
                7,
                {"id":"h1","label":"Café 🚀","nodes":["doc","external"],"source_file":directory.path().join("notes.md")}
            ],
            "nodes":[
                {"id":"doc","label":"Café 🚀","file_type":"document","source_file":directory.path().join("notes.md")},
                {"id":"external","label":"External","file_type":"concept","source_file":""}
            ],
            "edges":[
                {"source":"doc","target":"external","relation":"mentions","source_file":directory.path().join("notes.md")}
            ]
        }),
        refreshed_files: vec![directory.path().join("notes.md")],
        partial_files: Vec::new(),
        allow_partial: false,
    };
    let result = build_graph_with_layers(&options, Some(&semantic), &[])?;
    assert!(result.nodes >= 3);
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(result.output_dir.join("graph.json"))?)?;
    let semantic_edge = document["links"]
        .as_array()
        .and_then(|edges| edges.iter().find(|edge| edge["relation"] == "mentions"))
        .ok_or("missing semantic edge")?;
    assert_eq!(semantic_edge["_origin"], "semantic");
    assert_eq!(semantic_edge["source_file"], "notes.md");
    let semantic_hyperedge = document["hyperedges"]
        .as_array()
        .and_then(|hyperedges| hyperedges.iter().find(|hyperedge| hyperedge["id"] == "h1"))
        .ok_or("missing semantic hyperedge")?;
    assert_eq!(semantic_hyperedge["_origin"], "semantic");
    assert_eq!(semantic_hyperedge["source_file"], "notes.md");
    let encoded = fs::read_to_string(result.output_dir.join("graph.json"))?;
    assert!(encoded.contains("\\u00e9"));
    assert!(encoded.contains("\\ud83d\\ude80"));
    Ok(())
}
