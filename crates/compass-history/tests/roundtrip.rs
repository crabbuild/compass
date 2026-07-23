use std::collections::BTreeMap;

use compass_history::{
    ArtifactClass, ArtifactRegistryEntry, CompletionEvidence, GraphArtifacts, canonical_json_bytes,
};
use compass_model::GraphDocument;
use serde_json::json;
use sha2::{Digest, Sha256};

fn completion() -> CompletionEvidence {
    CompletionEvidence {
        extraction_succeeded: true,
        allow_partial: false,
        semantic_files_expected: 1,
        semantic_files_completed: 1,
        failed_chunks: 0,
    }
}

#[test]
fn complete_graph_and_build_state_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": false,
        "multigraph": true,
        "graph": {
            "name": "fixture",
            "hyperedges": [{"id":"flow","nodes":["a","b"]}]
        },
        "nodes": [
            {"id":"a","label":"A","community":1,"_origin":"ast"},
            {"id":"b","label":"B","community_name":"Core","_origin":"semantic"}
        ],
        "links": [
            {"source":"a","target":"b","relation":"calls","confidence":"INFERRED"},
            {"source":"a","target":"b","relation":"calls","confidence":"INFERRED"}
        ],
        "hyperedges": [
            {"nodes":["a","b"]},
            {"nodes":["a","b"]}
        ],
        "built_at_commit": "0123456789abcdef",
        "unknown": {"ordered":[3,2,1]}
    }))?;
    let artifacts = GraphArtifacts {
        document: document.clone(),
        analysis: Some(json!({"communities":{"1":["a","b"]}})),
        labels: Some(json!({"1":"Core"})),
        manifest: Some(json!({"a.py":{"ast_hash":"abc","semantic_hash":"abc","mtime":1.0}})),
        authoritative_sidecars: BTreeMap::from([(
            "semantic/custom.bin".to_owned(),
            vec![0, 1, 2, 255],
        )]),
    };
    let partitioned = artifacts.partition(&completion())?;
    let restored = GraphArtifacts::reconstruct(&partitioned)?;
    assert_eq!(restored, artifacts);
    assert_eq!(restored.document, document);
    Ok(())
}

#[test]
fn legacy_unicode_and_empty_hyperedge_placement_round_trip()
-> Result<(), Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {"hyperedges": []},
        "nodes": [{"id":"a\u{0000}雪","label":"雪"}],
        "edges": [],
        "hyperedges": [],
        "extension": true
    }))?;
    assert!(document.used_legacy_edges_key);
    let artifacts = GraphArtifacts {
        document,
        analysis: None,
        labels: None,
        manifest: None,
        authoritative_sidecars: BTreeMap::new(),
    };
    let restored = GraphArtifacts::reconstruct(&artifacts.partition(&completion())?)?;
    assert_eq!(restored, artifacts);
    let value = serde_json::to_value(&restored.document)?;
    assert!(value.get("edges").is_some());
    assert!(
        value["graph"]["hyperedges"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(value["hyperedges"].as_array().is_some_and(Vec::is_empty));
    Ok(())
}

#[test]
fn simple_duplicate_edges_and_explicit_hyperedge_ids_are_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    for document in [
        json!({
            "directed": true,
            "multigraph": false,
            "nodes": [{"id":"a"},{"id":"b"}],
            "links": [
                {"source":"a","target":"b","relation":"calls"},
                {"source":"a","target":"b","relation":"calls"}
            ]
        }),
        json!({
            "nodes": [{"id":"a"}],
            "links": [],
            "hyperedges": [{"id":"same","nodes":["a"]},{"id":"same","nodes":["a"]}]
        }),
    ] {
        let artifacts = GraphArtifacts {
            document: serde_json::from_value(document)?,
            analysis: None,
            labels: None,
            manifest: None,
            authoritative_sidecars: BTreeMap::new(),
        };
        assert!(artifacts.partition(&completion()).is_err());
    }
    Ok(())
}

#[test]
fn operational_provenance_does_not_change_partition_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "nodes": [{"id":"a"}],
        "links": []
    }))?;
    let first = GraphArtifacts {
        document: document.clone(),
        analysis: None,
        labels: None,
        manifest: None,
        authoritative_sidecars: BTreeMap::new(),
    };
    let second = GraphArtifacts {
        document,
        analysis: None,
        labels: None,
        manifest: None,
        authoritative_sidecars: BTreeMap::new(),
    };
    let first_partition = first.partition(&completion())?;
    let second_partition = second.partition(&completion())?;
    assert_eq!(first_partition, second_partition);
    let metadata_bytes = serde_json::to_vec(&first_partition.metadata)?;
    let metadata_text = String::from_utf8_lossy(&metadata_bytes);
    assert!(!metadata_text.contains("duration_ms"));
    assert!(!metadata_text.contains("cost"));
    assert!(!metadata_text.contains("tokens"));
    Ok(())
}

#[test]
fn completed_seed_writes_normalized_marker_and_opaque_sidecars()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let completed = compass_history::CompletedGraphArtifacts {
        artifacts: GraphArtifacts {
            document: serde_json::from_value(json!({"nodes": [], "links": []}))?,
            analysis: None,
            labels: None,
            manifest: None,
            authoritative_sidecars: BTreeMap::from([(
                "semantic/custom.bin".to_owned(),
                vec![0, 1, 255],
            )]),
        },
        completion: completion(),
    };
    completed.write_seed(directory.path())?;
    assert_eq!(
        std::fs::read(directory.path().join("semantic/custom.bin"))?,
        vec![0, 1, 255]
    );
    let marker: serde_json::Value = serde_json::from_slice(&std::fs::read(
        directory.path().join(".compass_semantic_marker"),
    )?)?;
    assert_eq!(marker["schema"], "compass.history.completion");
    assert_eq!(marker["semantic_files_expected"], 1);
    assert!(marker.get("output_tokens").is_none());
    assert!(!directory.path().join("GRAPH_REPORT.md").exists());
    assert!(!directory.path().join("graph.html").exists());
    Ok(())
}

#[test]
fn seed_round_trip_includes_every_optional_authoritative_json_file()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let artifacts = GraphArtifacts {
        document: serde_json::from_value(json!({"nodes": [], "links": []}))?,
        analysis: Some(json!({"score": 1})),
        labels: Some(json!({"0": "Core"})),
        manifest: Some(json!({"fixture.rs": {"ast_hash": "abc"}})),
        authoritative_sidecars: BTreeMap::new(),
    };
    artifacts.write_seed(directory.path(), &completion())?;
    let loaded = compass_history::CompletedGraphArtifacts::load(directory.path(), completion())?;
    assert_eq!(loaded.artifacts, artifacts);
    assert_eq!(loaded.partition()?, artifacts.partition(&completion())?);
    Ok(())
}

#[test]
fn incomplete_completion_and_unsafe_sidecar_paths_are_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({"nodes": [], "links": []}))?;
    let artifacts = GraphArtifacts {
        document,
        analysis: None,
        labels: None,
        manifest: None,
        authoritative_sidecars: BTreeMap::from([("../escape".to_owned(), vec![1])]),
    };
    assert!(artifacts.partition(&completion()).is_err());
    let incomplete = CompletionEvidence {
        extraction_succeeded: true,
        allow_partial: false,
        semantic_files_expected: 2,
        semantic_files_completed: 1,
        failed_chunks: 0,
    };
    assert!(artifacts.partition(&incomplete).is_err());
    Ok(())
}

#[test]
fn registry_loading_verifies_builtin_opaque_derived_and_operational_artifacts()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "nodes": [{"id":"a"}],
        "links": []
    }))?;
    let graph = canonical_json_bytes(&serde_json::to_value(&document)?)?;
    let analysis = canonical_json_bytes(&json!({"score": 1}))?;
    let labels = canonical_json_bytes(&json!({"0": "Core"}))?;
    let manifest = canonical_json_bytes(&json!({"a.rs": {"ast_hash": "a"}}))?;
    let opaque = vec![0, 1, 255];
    for (path, bytes) in [
        ("graph.json", graph.as_slice()),
        (".compass_analysis.json", analysis.as_slice()),
        (".compass_labels.json", labels.as_slice()),
        ("manifest.json", manifest.as_slice()),
        ("semantic/facts.bin", opaque.as_slice()),
    ] {
        let destination = directory.path().join(path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(destination, bytes)?;
    }
    let authoritative = |path: &str, bytes: &[u8]| ArtifactRegistryEntry {
        registry_version: 1,
        relative_path: path.to_owned(),
        class: ArtifactClass::Authoritative,
        media_type: "application/json".to_owned(),
        schema_version: Some(1),
        content_digest: Some(Sha256::digest(bytes).into()),
        storage: None,
        regeneration_version: None,
    };
    let mut registry = vec![
        authoritative("graph.json", &graph),
        authoritative(".compass_analysis.json", &analysis),
        authoritative(".compass_labels.json", &labels),
        authoritative("manifest.json", &manifest),
        authoritative("semantic/facts.bin", &opaque),
    ];
    registry.push(ArtifactRegistryEntry {
        registry_version: 1,
        relative_path: "GRAPH_REPORT.md".to_owned(),
        class: ArtifactClass::Derived,
        media_type: "text/markdown".to_owned(),
        schema_version: None,
        content_digest: None,
        storage: None,
        regeneration_version: Some("report-v1".to_owned()),
    });
    registry.push(ArtifactRegistryEntry {
        registry_version: 1,
        relative_path: "attempt.log".to_owned(),
        class: ArtifactClass::Operational,
        media_type: "text/plain".to_owned(),
        schema_version: None,
        content_digest: None,
        storage: None,
        regeneration_version: None,
    });
    let loaded = GraphArtifacts::load_with_registry(directory.path(), &registry)?;
    assert_eq!(loaded.document, document);
    assert_eq!(loaded.analysis, Some(json!({"score": 1})));
    assert_eq!(loaded.labels, Some(json!({"0": "Core"})));
    assert_eq!(loaded.authoritative_sidecars["semantic/facts.bin"], opaque);

    std::fs::write(directory.path().join("graph.json"), b"{\"nodes\":[]}")?;
    assert!(GraphArtifacts::load_with_registry(directory.path(), &registry).is_err());
    Ok(())
}

#[test]
fn reconstruction_rejects_missing_and_malformed_typed_records()
-> Result<(), Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "nodes": [{"id":"a","community":1}],
        "links": []
    }))?;
    let artifacts = GraphArtifacts {
        document,
        analysis: Some(json!({"score": 1})),
        labels: None,
        manifest: None,
        authoritative_sidecars: BTreeMap::from([("semantic/facts.bin".to_owned(), vec![1])]),
    };
    let base = artifacts.partition(&completion())?;

    let mut invalid = base.clone();
    invalid.analysis.push((vec![0xff], vec![]));
    assert!(GraphArtifacts::reconstruct(&invalid).is_err());
    let mut invalid = base.clone();
    invalid.metadata.push((vec![0xff], vec![]));
    assert!(GraphArtifacts::reconstruct(&invalid).is_err());

    for metadata_name in ["document", "completion", "artifact-registry"] {
        let mut invalid = base.clone();
        invalid.metadata.retain(|(key, _)| {
            prolly::decode_segments(key)
                .map(|segments| {
                    !segments
                        .iter()
                        .any(|segment| segment.as_slice() == metadata_name.as_bytes())
                })
                .unwrap_or(true)
        });
        assert!(GraphArtifacts::reconstruct(&invalid).is_err());
    }

    let mut missing_node = base.clone();
    missing_node.nodes.clear();
    assert!(GraphArtifacts::reconstruct(&missing_node).is_err());

    let mut registry_mismatch = base.clone();
    registry_mismatch.metadata.retain(|(key, _)| {
        prolly::decode_segments(key)
            .map(|segments| {
                !segments
                    .iter()
                    .any(|segment| segment.as_slice() == b"sidecar")
            })
            .unwrap_or(true)
    });
    assert!(GraphArtifacts::reconstruct(&registry_mismatch).is_err());
    Ok(())
}
