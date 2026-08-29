use std::collections::BTreeMap;

use compass_model::GraphDocument;
use compass_output::{
    DerivedArtifactRequest, HistoricalPublicationEvidence, HistoryBundleInput, PublicationStatus,
    SUPPORTED_HISTORY_RENDERER, publish_history_bundle,
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
    let destination = directory.path().join("compass-out");
    let document = document()?;
    let analysis = json!({"communities":{"0":["a","b"]}});
    let labels = json!({"0":"Core"});
    let manifest = json!({"src/lib.rs":{"ast_hash":"abc"}});
    let program = br#"{"schema":"compass.program","schema_version":1}"#;
    let marker = json!({
        "schema":"compass.history.completion",
        "schema_version":1,
        "extraction_succeeded":true,
        "allow_partial":false,
        "semantic_files_expected":2,
        "semantic_files_completed":2,
        "failed_chunks":0
    });
    let sidecars = BTreeMap::from([("semantic/facts.bin".to_owned(), vec![0, 1, 255])]);
    let requests = [
        "GRAPH_REPORT.md",
        "graph.html",
        "GRAPH_TREE.html",
        "labels.json.sig",
    ]
    .map(|path| DerivedArtifactRequest {
        relative_path: path.to_owned(),
        regeneration_version: SUPPORTED_HISTORY_RENDERER.to_owned(),
    });
    publish_history_bundle(
        &destination,
        &HistoryBundleInput {
            document: &document,
            graph_json: None,
            program: Some(program),
            analysis: Some(&analysis),
            labels: Some(&labels),
            manifest: Some(&manifest),
            authoritative_sidecars: &sidecars,
            semantic_marker: &marker,
            publication_evidence: None,
            derived: &requests,
        },
    )?;
    assert_eq!(
        GraphDocument::load_for_recluster(&destination.join("graph.json"))?,
        document
    );
    assert!(destination.join("GRAPH_REPORT.md").is_file());
    assert!(destination.join("graph.html").is_file());
    assert!(destination.join("GRAPH_TREE.html").is_file());
    assert!(destination.join("labels.json.sig").is_file());
    let report = std::fs::read_to_string(destination.join("GRAPH_REPORT.md"))?;
    assert!(report.starts_with("# Agent Orientation"));
    assert!(report.contains("Publication: unknown"));
    assert!(report.contains("files: unknown · words: unknown"));
    assert!(!report.contains("cohesion: 0.00"));
    let graph_html = std::fs::read_to_string(destination.join("graph.html"))?;
    assert!(graph_html.contains("id=\"compass-viewer-root\""));
    assert!(!graph_html.contains("<script src="));
    assert!(
        std::fs::read_to_string(destination.join("GRAPH_TREE.html"))?
            .contains("compass tree viewer")
    );
    let signatures: serde_json::Value =
        serde_json::from_slice(&std::fs::read(destination.join("labels.json.sig"))?)?;
    assert!(signatures.get("0").is_some());
    assert_eq!(
        std::fs::read(destination.join("semantic/facts.bin"))?,
        vec![0, 1, 255]
    );
    assert_eq!(std::fs::read(destination.join("program.json"))?, program);
    assert!(
        publish_history_bundle(
            &destination,
            &HistoryBundleInput {
                document: &document,
                graph_json: None,
                program: None,
                analysis: None,
                labels: None,
                manifest: None,
                authoritative_sidecars: &BTreeMap::new(),
                semantic_marker: &marker,
                publication_evidence: None,
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
    let destination = directory.path().join("compass-out");
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
                graph_json: None,
                program: None,
                analysis: None,
                labels: None,
                manifest: None,
                authoritative_sidecars: &BTreeMap::new(),
                semantic_marker: &marker,
                publication_evidence: None,
                derived: &request,
            },
        )
        .is_err()
    );
    assert!(!destination.exists());
    assert!(std::fs::read_dir(directory.path())?.next().is_none());
    Ok(())
}

#[test]
fn only_authoritative_graph_publication_evidence_sets_historical_completeness()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let document = document()?;
    let request = [DerivedArtifactRequest {
        relative_path: "GRAPH_REPORT.md".to_owned(),
        regeneration_version: SUPPORTED_HISTORY_RENDERER.to_owned(),
    }];
    for (name, marker) in [
        ("legacy", json!({"schema":"legacy"})),
        (
            "semantic-complete",
            json!({
                "extraction_succeeded":true,
                "semantic_files_expected":2,
                "semantic_files_completed":2,
                "failed_chunks":0
            }),
        ),
    ] {
        let destination = directory.path().join(name);
        publish_history_bundle(
            &destination,
            &HistoryBundleInput {
                document: &document,
                graph_json: None,
                program: None,
                analysis: None,
                labels: None,
                manifest: None,
                authoritative_sidecars: &BTreeMap::new(),
                semantic_marker: &marker,
                publication_evidence: None,
                derived: &request,
            },
        )?;
        let report = std::fs::read_to_string(destination.join("GRAPH_REPORT.md"))?;
        assert!(report.contains("Publication: unknown"));
        assert!(report.contains("omitted nodes: unknown"));
    }

    let evidence = HistoricalPublicationEvidence {
        publication: PublicationStatus::Partial,
        omitted_nodes: 3,
        omitted_edges: 5,
        identity_collisions: 2,
        diagnostic_examples_omitted: 7,
    };
    let destination = directory.path().join("authoritative");
    publish_history_bundle(
        &destination,
        &HistoryBundleInput {
            document: &document,
            graph_json: None,
            program: None,
            analysis: None,
            labels: None,
            manifest: None,
            authoritative_sidecars: &BTreeMap::new(),
            semantic_marker: &json!({"extraction_succeeded":true}),
            publication_evidence: Some(&evidence),
            derived: &request,
        },
    )?;
    let report = std::fs::read_to_string(destination.join("GRAPH_REPORT.md"))?;
    assert!(report.contains("Publication: partial"));
    assert!(report.contains("omitted nodes: 3"));
    assert!(report.contains("omitted edges: 5"));
    assert!(report.contains("identity collisions: 2"));
    Ok(())
}
