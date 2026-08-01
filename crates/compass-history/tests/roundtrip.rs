use std::collections::BTreeMap;

use compass_analysis::{AnalysisBundle, analyze};
use compass_history::{
    ArtifactClass, ArtifactRegistryEntry, CompletionEvidence, GraphArtifacts, canonical_json_bytes,
};
use compass_ir::{
    BasicBlock, Capability, CoverageState, EvidenceRecord, FunctionIr, ModuleIr, Operation,
    OperationKind, ProgramBundle, ProviderDescriptor, ProviderKind, SourceAnchor, Terminator,
    hex_sha256,
};
use compass_model::GraphDocument;
use compass_model::identity::file_id;
use prolly::{VersionedValue, decode_segments};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn empty_trusted_graph() -> Value {
    let digest = format!("sha256:{}", "0".repeat(64));
    json!({
        "directed": true,
        "multigraph": true,
        "graph": {
            "schema": "compass.graph/1",
            "build": {
                "builderVersion": "test",
                "schemaFingerprint": digest,
                "sourceTreeDigest": digest,
                "configurationDigest": digest,
                "generationId": digest
            }
        },
        "nodes": [],
        "links": []
    })
}

fn empty_trusted_artifacts() -> Result<GraphArtifacts, Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::write(
        directory.path().join("graph.json"),
        canonical_json_bytes(&empty_trusted_graph())?,
    )?;
    Ok(GraphArtifacts::load(directory.path())?)
}

#[test]
fn trusted_graph_partitions_store_full_typed_node_records() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let mut graph = empty_trusted_graph();
    graph["graph"]["files"] = json!([{
        "id":file_id("src/lib.rs"),
        "path":"src/lib.rs",
        "contentDigest":"sha256:fixture",
        "byteSize":10,
        "generated":false,
        "extractionStatus":"extracted"
    }]);
    graph["nodes"] = json!([{
        "id":"symbol:test",
        "kind":"function",
        "name":"test",
        "qualifiedName":"fixture.test",
        "source":{
            "file":"src/lib.rs",
            "startByte":0,
            "endByte":4,
            "startLine":1,
            "startColumn":0,
            "endLine":1,
            "endColumn":4
        },
        "evidence":[{
            "origin":"config",
            "extractor":"fixture",
            "confidence":"exact",
            "anchors":[{
                "file":"src/lib.rs",
                "startByte":0,
                "endByte":4,
                "startLine":1,
                "startColumn":0,
                "endLine":1,
                "endColumn":4
            }]
        }],
        "coverage":[{
            "capability":"node:function",
            "producer":"fixture",
            "status":"partial",
            "reason":"fixture"
        }],
        "diagnostics":[{
            "severity":"warning",
            "code":"fixture",
            "message":"fixture warning"
        }]
    }]);
    std::fs::write(
        directory.path().join("graph.json"),
        canonical_json_bytes(&graph)?,
    )?;
    let partition = GraphArtifacts::load(directory.path())?.partition(&completion())?;
    let record = VersionedValue::from_bytes(&partition.nodes[0].1)?;
    assert_eq!(record.schema, "compass.graph.node.v1");
    let payload: Value = serde_json::from_slice(&record.payload)?;
    assert_eq!(payload["evidence"][0]["origin"], "config");
    assert_eq!(payload["coverage"][0]["status"], "partial");
    assert_eq!(payload["diagnostics"][0]["code"], "fixture");
    Ok(())
}

#[test]
fn trusted_graph_partition_does_not_duplicate_full_graph_as_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph = empty_trusted_graph();
    let graph_bytes = canonical_json_bytes(&graph)?;
    std::fs::write(directory.path().join("graph.json"), &graph_bytes)?;

    let partition = GraphArtifacts::load(directory.path())?.partition(&completion())?;
    assert!(partition.metadata.iter().all(|(key, _)| {
        !matches!(
            decode_segments(key).as_deref(),
            Ok([_, _, kind, path])
                if kind == b"sidecar" && path == b".compass-history/graph.v1.json"
        )
    }));
    assert!(partition.metadata.iter().all(|(key, _)| {
        !matches!(
            decode_segments(key).as_deref(),
            Ok([_, _, kind, _]) if kind == b"node-order" || kind == b"edge-order"
        )
    }));

    let restored = GraphArtifacts::reconstruct(&partition)?;
    assert_eq!(restored.graph_json_bytes()?, graph_bytes);
    Ok(())
}

#[test]
fn source_inventory_uses_one_decomposed_canonical_record() -> Result<(), Box<dyn std::error::Error>>
{
    let inventory = canonical_json_bytes(&json!({
        "schema": "compass.history.source_inventory/1",
        "code_files": {"src/lib.rs": {"git_object": "fixture"}}
    }))?;
    let mut artifacts = empty_trusted_artifacts()?;
    artifacts.authoritative_sidecars.insert(
        ".compass_source_inventory.json".to_owned(),
        inventory.clone(),
    );

    let registry = artifacts.artifact_registry()?;
    let entry = registry
        .iter()
        .find(|entry| entry.relative_path == ".compass_source_inventory.json")
        .ok_or("source inventory registry entry missing")?;
    assert!(entry.storage.is_none());

    let partition = artifacts.partition(&completion())?;
    assert_eq!(GraphArtifacts::reconstruct(&partition)?, artifacts);
    assert!(partition.metadata.iter().any(|(key, _)| {
        matches!(
            decode_segments(key).as_deref(),
            Ok([_, _, name]) if name == b"source-inventory"
        )
    }));
    Ok(())
}

fn completion() -> CompletionEvidence {
    CompletionEvidence {
        extraction_succeeded: true,
        allow_partial: false,
        semantic_files_expected: 1,
        semantic_files_completed: 1,
        failed_chunks: 0,
    }
}

fn program_fixture(input: &[u8]) -> Result<AnalysisBundle, compass_analysis::AnalysisError> {
    let source = "src/lib.rs";
    let evidence_id = "e".repeat(64);
    let anchor = SourceAnchor {
        source_file: source.to_owned(),
        start_byte: 0,
        end_byte: 24,
    };
    let coverage = BTreeMap::from([(Capability::Syntax, CoverageState::Complete)]);
    analyze(ProgramBundle {
        schema: compass_ir::PROGRAM_SCHEMA.to_owned(),
        providers: vec![ProviderDescriptor {
            id: "syntax:fixture".to_owned(),
            kind: ProviderKind::Syntax,
            version: "1".to_owned(),
            scope: source.to_owned(),
            input_digest: hex_sha256(input),
            configuration_digest: hex_sha256(b"fixture-config"),
        }],
        evidence: vec![EvidenceRecord {
            id: evidence_id.clone(),
            provider_id: "syntax:fixture".to_owned(),
            source_file: Some(source.to_owned()),
            capability: Capability::CallResolution,
            detail: "fixture call".to_owned(),
        }],
        modules: vec![ModuleIr {
            source_file: source.to_owned(),
            language: "rust".to_owned(),
            source_digest: hex_sha256(input),
            graph_node_id: None,
            functions: vec![FunctionIr {
                symbol_id: "run".to_owned(),
                name: "run".to_owned(),
                graph_node_id: None,
                signature_digest: hex_sha256(b"fn run()"),
                body_digest: hex_sha256(input),
                visibility: compass_ir::Visibility::Public,
                execution_mode: compass_ir::ExecutionMode::Sync,
                is_test: false,
                anchor: anchor.clone(),
                parameters: Vec::new(),
                return_type: None,
                blocks: vec![BasicBlock {
                    id: 0,
                    operations: vec![Operation {
                        ordinal: 0,
                        anchor: anchor.clone(),
                        evidence: vec![evidence_id.clone()],
                        kind: OperationKind::Call {
                            callee: "work".to_owned(),
                            callee_anchor: anchor,
                            resolved_symbols: vec!["work".to_owned()],
                            receiver_type: None,
                        },
                    }],
                    terminator: Terminator::Return { value: None },
                    evidence: Vec::new(),
                }],
                coverage: coverage.clone(),
                evidence: vec![evidence_id.clone()],
            }],
            coverage,
            evidence: vec![evidence_id],
        }],
    })
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
        program: Some(program_fixture(b"first")?),
        analysis: Some(json!({"communities":{"1":["a","b"]}})),
        labels: Some(json!({"1":"Core"})),
        manifest: Some(json!({"a.py":{"ast_hash":"abc","semantic_hash":"abc","mtime":0}})),
        authoritative_sidecars: BTreeMap::from([(
            "semantic/custom.bin".to_owned(),
            vec![0, 1, 2, 255],
        )]),
    };
    let partitioned = artifacts.partition(&completion())?;
    assert_eq!(
        partitioned,
        artifacts.clone().into_partition(&completion())?,
        "owned publication partitioning must preserve exact record identity"
    );
    let restored = GraphArtifacts::reconstruct(&partitioned)?;
    assert_eq!(restored, artifacts);
    assert_eq!(restored.document, document);
    let mut reordered = artifacts.clone();
    reordered.document.nodes.reverse();
    reordered.document.links.reverse();
    reordered
        .document
        .graph
        .get_mut("hyperedges")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("missing graph hyperedges")?
        .reverse();
    reordered
        .document
        .extras
        .get_mut("hyperedges")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("missing top-level hyperedges")?
        .reverse();
    reordered
        .manifest
        .as_mut()
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|entries| entries.get_mut("a.py"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing manifest fixture")?
        .insert("mtime".to_owned(), json!(1234.5));
    assert_eq!(
        partitioned,
        reordered.partition(&completion())?,
        "node-link array order must not change realization identity"
    );
    assert_eq!(partitioned.program_facts.len(), 5);
    assert_eq!(partitioned.program_summaries.len(), 2);
    let module_record = partitioned
        .program_facts
        .iter()
        .find(|(key, _)| matches!(decode_segments(key).as_deref(), Ok([kind, _]) if kind == b"module"))
        .ok_or("missing indexed module record")?;
    let module = VersionedValue::from_bytes(&module_record.1)?;
    assert_eq!(module.schema, "compass.program.module-index");
    let module: Value = serde_json::from_slice(&module.payload)?;
    assert_eq!(module["module"]["functions"], json!([]));
    assert_eq!(module["function_ids"], json!(["run"]));
    Ok(())
}

#[test]
fn unicode_and_empty_hyperedge_placement_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {"hyperedges": []},
        "nodes": [{"id":"a\u{0000}雪","label":"雪"}],
        "links": [],
        "hyperedges": [],
        "extension": true
    }))?;
    let artifacts = GraphArtifacts {
        document,
        program: None,
        analysis: None,
        labels: None,
        manifest: None,
        authoritative_sidecars: BTreeMap::new(),
    };
    let restored = GraphArtifacts::reconstruct(&artifacts.partition(&completion())?)?;
    assert_eq!(restored, artifacts);
    let value = serde_json::to_value(&restored.document)?;
    assert!(value.get("links").is_some());
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
            program: None,
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
fn undirected_reciprocal_edges_use_persisted_true_directions()
-> Result<(), Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": false,
        "multigraph": false,
        "nodes": [{"id":"a"},{"id":"b"}],
        "links": [
            {
                "source":"a",
                "target":"b",
                "relation":"calls"
            },
            {
                "source":"b",
                "target":"a",
                "relation":"calls"
            }
        ]
    }))?;
    let artifacts = GraphArtifacts {
        document,
        program: None,
        analysis: None,
        labels: None,
        manifest: None,
        authoritative_sidecars: BTreeMap::new(),
    };
    let partitioned = artifacts.partition(&completion())?;
    assert_eq!(partitioned.edges.len(), 2);
    assert_eq!(GraphArtifacts::reconstruct(&partitioned)?, artifacts);
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
        program: None,
        analysis: None,
        labels: None,
        manifest: None,
        authoritative_sidecars: BTreeMap::new(),
    };
    let second = GraphArtifacts {
        document,
        program: None,
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
    let mut artifacts = empty_trusted_artifacts()?;
    artifacts
        .authoritative_sidecars
        .insert("semantic/custom.bin".to_owned(), vec![0, 1, 255]);
    let completed = compass_history::CompletedGraphArtifacts {
        artifacts,
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
    let mut artifacts = empty_trusted_artifacts()?;
    artifacts.program = Some(program_fixture(b"seed")?);
    artifacts.analysis = Some(json!({"score": 1}));
    artifacts.labels = Some(json!({"0": "Core"}));
    artifacts.manifest = Some(json!({"fixture.rs": {"ast_hash": "abc"}}));
    artifacts.write_seed(directory.path(), &completion())?;
    assert_eq!(
        std::fs::read(directory.path().join("program.json"))?,
        artifacts
            .program
            .as_ref()
            .ok_or("missing fixture program")?
            .canonical_bytes()?
    );
    let loaded = compass_history::CompletedGraphArtifacts::load(directory.path(), completion())?;
    assert_eq!(loaded.artifacts, artifacts);
    assert_eq!(loaded.partition()?, artifacts.partition(&completion())?);
    let publication = compass_history::CompletedGraphArtifacts::load_for_publication(
        directory.path(),
        completion(),
    )?;
    assert_eq!(
        publication.partition()?,
        artifacts.partition(&completion())?
    );
    assert_eq!(publication.artifacts.export_sidecars(), BTreeMap::new());

    let mut noncanonical = std::fs::read(directory.path().join("program.json"))?;
    noncanonical.extend_from_slice(b" \n");
    std::fs::write(directory.path().join("program.json"), noncanonical)?;
    let publication = compass_history::CompletedGraphArtifacts::load_for_publication(
        directory.path(),
        completion(),
    )?;
    assert!(publication.partition().is_err());
    Ok(())
}

#[test]
fn incomplete_completion_and_unsafe_sidecar_paths_are_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({"nodes": [], "links": []}))?;
    let artifacts = GraphArtifacts {
        document,
        program: None,
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
    let graph = canonical_json_bytes(&empty_trusted_graph())?;
    let analysis = canonical_json_bytes(&json!({"score": 1}))?;
    let labels = canonical_json_bytes(&json!({"0": "Core"}))?;
    let manifest = canonical_json_bytes(&json!({"a.rs": {"ast_hash": "a", "mtime": 0}}))?;
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
    assert!(loaded.document.nodes.is_empty());
    assert_eq!(
        loaded.document.graph["schema"],
        serde_json::Value::String("compass.graph/1".to_owned())
    );
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
        program: None,
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
