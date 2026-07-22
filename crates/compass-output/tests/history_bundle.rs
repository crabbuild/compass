use std::collections::BTreeMap;

use compass_model::GraphDocument;
use compass_output::{
    DerivedArtifactRequest, HistoryBundleInput, SUPPORTED_HISTORY_RENDERER, publish_history_bundle,
};
use serde_json::json;

fn document() -> Result<GraphDocument, serde_json::Error> {
    serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {"name":"fixture"},
        "nodes": [
            {"id":"a","label":"A","community":0},
            {"id":"b","label":"B","community":0}
        ],
        "links": [{"source":"a","target":"b","relation":"calls"}],
        "built_at_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }))
}

#[test]
fn v1_renderer_publishes_a_valid_complete_bundle_atomically()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("graphify-out");
    let document = document()?;
    let analysis = json!({"communities":{"0":["a","b"]}});
    let labels = json!({"0":"Core"});
    let manifest = json!({"src/lib.rs":{"ast_hash":"abc"}});
    let marker = json!({"schema":"compass.history.completion","schema_version":1});
    let sidecars = BTreeMap::from([("semantic/facts.bin".to_owned(), vec![0, 1, 255])]);
    let requests = [
        "GRAPH_REPORT.md",
        "graph.html",
        "GRAPH_TREE.html",
        ".graphify_labels.json.sig",
    ]
    .map(|path| DerivedArtifactRequest {
        relative_path: path.to_owned(),
        regeneration_version: SUPPORTED_HISTORY_RENDERER.to_owned(),
    });
    publish_history_bundle(
        &destination,
        &HistoryBundleInput {
            document: &document,
            analysis: Some(&analysis),
            labels: Some(&labels),
            manifest: Some(&manifest),
            authoritative_sidecars: &sidecars,
            semantic_marker: &marker,
            derived: &requests,
        },
    )?;
    assert_eq!(
        GraphDocument::load_for_recluster_compatibility(&destination.join("graph.json"))?,
        document
    );
    assert!(destination.join("GRAPH_REPORT.md").is_file());
    assert!(destination.join("graph.html").is_file());
    assert!(destination.join("GRAPH_TREE.html").is_file());
    assert!(destination.join(".graphify_labels.json.sig").is_file());
    assert!(
        std::fs::read_to_string(destination.join("GRAPH_REPORT.md"))?
            .contains("# Graph Report - fixture")
    );
    assert!(std::fs::read_to_string(destination.join("graph.html"))?.contains("data-nid"));
    assert!(
        std::fs::read_to_string(destination.join("GRAPH_TREE.html"))?
            .contains("graphify tree viewer")
    );
    let signatures: serde_json::Value = serde_json::from_slice(&std::fs::read(
        destination.join(".graphify_labels.json.sig"),
    )?)?;
    assert!(signatures.get("0").is_some());
    assert_eq!(
        std::fs::read(destination.join("semantic/facts.bin"))?,
        vec![0, 1, 255]
    );
    assert!(
        publish_history_bundle(
            &destination,
            &HistoryBundleInput {
                document: &document,
                analysis: None,
                labels: None,
                manifest: None,
                authoritative_sidecars: &BTreeMap::new(),
                semantic_marker: &marker,
                derived: &[],
            },
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn unknown_renderer_fails_without_creating_destination() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let destination = directory.path().join("graphify-out");
    let document = document()?;
    let marker = json!({});
    let request = [DerivedArtifactRequest {
        relative_path: "GRAPH_REPORT.md".to_owned(),
        regeneration_version: "compass-output/future".to_owned(),
    }];
    assert!(
        publish_history_bundle(
            &destination,
            &HistoryBundleInput {
                document: &document,
                analysis: None,
                labels: None,
                manifest: None,
                authoritative_sidecars: &BTreeMap::new(),
                semantic_marker: &marker,
                derived: &request,
            },
        )
        .is_err()
    );
    assert!(!destination.exists());
    assert!(std::fs::read_dir(directory.path())?.next().is_none());
    Ok(())
}
