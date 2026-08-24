use std::process::Command;

use compass_agent_graph::{Digest, canonical_bytes};
use compass_model::code_graph::{
    BuildMetadata, ExtractionStatus, FileRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::identity::file_id;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};

#[test]
fn status_is_versioned_and_apply_is_write_disabled_by_default()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    let graph_path = root.join("graph.json");
    let graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "test".to_owned(),
        source_tree_digest: "test".to_owned(),
        configuration_digest: "test".to_owned(),
        generation_id: "generation-cli-agent".to_owned(),
        source_commit: None,
    });
    let graph_bytes = serde_json::to_vec(&graph)?;
    std::fs::write(&graph_path, &graph_bytes)?;
    let state_root = root.join("agent-state");

    let status = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "agent-graph",
            "status",
            "--graph",
            graph_path.to_str().ok_or("non-UTF-8 graph path")?,
            "--root",
            root.to_str().ok_or("non-UTF-8 project path")?,
            "--state-root",
            state_root.to_str().ok_or("non-UTF-8 state path")?,
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_json: serde_json::Value = serde_json::from_slice(&status.stdout)?;
    assert_eq!(status_json["schema"], "compass.agent-graph.status/1");
    assert_eq!(status_json["writesEnabled"], false);
    assert_eq!(std::fs::read(&graph_path)?, graph_bytes);

    let request_path = root.join("request.json");
    let base_digest = Digest::raw_bytes(&canonical_bytes(&graph)?);
    let request = serde_json::json!({
        "schema":"compass.agent-graph.batch/1",
        "overlay":"overlay:default",
        "baseGeneration":{
            "generationId":"generation-cli-agent",
            "graphDigest":base_digest
        },
        "idempotencyKey":"idempotency:cli-write-disabled",
        "operations":[{
            "operation":"put_assertion",
            "assertion":{
                "selector":{"selector":"new","key":"key:cli-node"},
                "fact":{"factType":"node","kind":"function","name":"created","qualifiedName":"created"},
                "grounding":{"schema":"compass.agent-graph.grounding/1","policyId":"compass.agent-graph.topology-source-span","evidence":[]},
                "summary":"A bounded test assertion."
            }
        }]
    });
    std::fs::write(&request_path, serde_json::to_vec(&request)?)?;
    let denied = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "agent-graph",
            "apply",
            "--graph",
            graph_path.to_str().ok_or("non-UTF-8 graph path")?,
            "--root",
            root.to_str().ok_or("non-UTF-8 project path")?,
            "--state-root",
            state_root.to_str().ok_or("non-UTF-8 state path")?,
            "--request",
            request_path.to_str().ok_or("non-UTF-8 request path")?,
            "--format",
            "json",
        ])
        .output()?;
    assert_eq!(denied.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&denied.stderr)?;
    assert_eq!(error["code"], "writes_disabled");
    assert_eq!(std::fs::read(&graph_path)?, graph_bytes);
    Ok(())
}

#[test]
fn repeated_options_are_usage_errors() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "agent-graph",
            "status",
            "--format",
            "json",
            "--format",
            "json",
        ])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["code"], "invalid_input");
    Ok(())
}

#[test]
fn prepare_returns_apply_ready_base_ref_and_source_grounding()
-> Result<(), Box<dyn std::error::Error>> {
    const SOURCE_PATH: &str = "src/lib.rs";
    const SOURCE: &[u8] = b"pub fn target() {}\n";
    const NODE_ID: &str = "node:target";

    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    std::fs::create_dir(root.join("src"))?;
    std::fs::write(root.join(SOURCE_PATH), SOURCE)?;
    let anchor = SourceAnchor {
        file: SOURCE_PATH.to_owned(),
        start_byte: 0,
        end_byte: 18,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 18,
    };
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "test".to_owned(),
        source_tree_digest: "test".to_owned(),
        configuration_digest: "test".to_owned(),
        generation_id: "generation-cli-prepare".to_owned(),
        source_commit: None,
    });
    graph.graph.files.push(FileRecord {
        id: file_id(SOURCE_PATH),
        path: SOURCE_PATH.to_owned(),
        language: Some("rust".to_owned()),
        content_digest: Digest::raw_bytes(SOURCE).as_str().to_owned(),
        byte_size: SOURCE.len() as u64,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["cli-prepare-test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    graph.nodes.push(NodeRecord {
        id: NODE_ID.to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: "target".to_owned(),
        qualified_name: "crate::target".to_owned(),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(anchor.clone()),
        details: None,
        evidence: vec![Provenance::direct(
            EvidenceOrigin::Ast,
            "test.extractor",
            EvidenceConfidence::Exact,
            anchor,
        )?],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    });
    let graph_path = root.join("graph.json");
    std::fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
    let state_root = root.join("agent-state");
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "agent-graph",
            "prepare",
            "--graph",
            graph_path.to_str().ok_or("non-UTF-8 graph path")?,
            "--root",
            root.to_str().ok_or("non-UTF-8 project path")?,
            "--state-root",
            state_root.to_str().ok_or("non-UTF-8 state path")?,
            "--overlay",
            "overlay:review",
            "--base-node",
            NODE_ID,
            "--source-span",
            "src/lib.rs:0:18",
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prepared: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        prepared["schema"],
        "compass.agent-graph.ingestion-preparation/1"
    );
    assert_eq!(prepared["overlay"], "overlay:review");
    assert_eq!(prepared["baseNodes"][0]["id"], NODE_ID);
    assert_eq!(
        prepared["baseNodes"][0]["recordDigest"]
            .as_str()
            .map(str::len),
        Some(64)
    );
    assert_eq!(
        prepared["grounding"]["evidence"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(prepared["grounding"]["evidence"][0]["anchor"]["endLine"], 1);
    assert!(prepared.get("expectedRevision").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("GROUNDED"));

    let request_path = root.join("prepared-batch.json");
    let request = serde_json::json!({
        "schema": "compass.agent-graph.batch/1",
        "overlay": prepared["overlay"].clone(),
        "baseGeneration": prepared["baseGeneration"].clone(),
        "idempotencyKey": "idempotency:cli-prepared-ingestion",
        "operations": [
            {
                "operation": "put_assertion",
                "assertion": {
                    "selector": {"selector": "new", "key": "key:prepared-caller"},
                    "fact": {
                        "factType": "node",
                        "kind": "function",
                        "name": "prepared_caller",
                        "qualifiedName": "crate::prepared_caller",
                        "language": "rust"
                    },
                    "grounding": prepared["grounding"].clone(),
                    "summary": "Prepared source-backed caller."
                }
            },
            {
                "operation": "put_assertion",
                "assertion": {
                    "selector": {"selector": "new", "key": "key:prepared-edge"},
                    "fact": {
                        "factType": "edge",
                        "source": {
                            "referenceType": "created_in_this_batch",
                            "key": "key:prepared-caller"
                        },
                        "target": {
                            "referenceType": "base",
                            "node": prepared["baseNodes"][0].clone()
                        },
                        "kind": "calls",
                        "context": "Prepared exact endpoint"
                    },
                    "grounding": prepared["grounding"].clone(),
                    "summary": "Prepared caller reaches the exact Base node."
                }
            }
        ]
    });
    std::fs::write(&request_path, serde_json::to_vec(&request)?)?;
    let applied = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "agent-graph",
            "apply",
            "--graph",
            graph_path.to_str().ok_or("non-UTF-8 graph path")?,
            "--root",
            root.to_str().ok_or("non-UTF-8 project path")?,
            "--state-root",
            state_root.to_str().ok_or("non-UTF-8 state path")?,
            "--overlay",
            "overlay:review",
            "--request",
            request_path.to_str().ok_or("non-UTF-8 request path")?,
            "--principal",
            "principal:local",
            "--enable-writes",
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&applied.stdout)?;
    assert_eq!(receipt["schema"], "compass.agent-graph.receipt/1");
    assert_eq!(receipt["activeAssertions"], 2);

    std::fs::write(root.join(SOURCE_PATH), b"pub fn changed() {}\n")?;
    let stale = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "agent-graph",
            "prepare",
            "--graph",
            graph_path.to_str().ok_or("non-UTF-8 graph path")?,
            "--root",
            root.to_str().ok_or("non-UTF-8 project path")?,
            "--state-root",
            state_root.to_str().ok_or("non-UTF-8 state path")?,
            "--overlay",
            "overlay:review",
            "--source-span",
            "src/lib.rs:0:18",
            "--format",
            "json",
        ])
        .output()?;
    assert_eq!(stale.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&stale.stderr)?;
    assert_eq!(error["code"], "invalid_citation");
    Ok(())
}

#[test]
fn historical_realization_is_an_exact_mutually_exclusive_base_selector()
-> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "agent-graph",
            "status",
            "--realization",
            &"0".repeat(64),
            "--graph",
            "graph.json",
            "--format",
            "json",
        ])
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["code"], "invalid_input");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("cannot be combined"))
    );
    Ok(())
}

#[test]
fn export_is_atomic_and_refuses_to_replace_an_existing_destination()
-> Result<(), Box<dyn std::error::Error>> {
    const SOURCE_PATH: &str = "src/lib.rs";
    const SOURCE: &[u8] = b"x";

    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    std::fs::create_dir(root.join("src"))?;
    std::fs::write(root.join(SOURCE_PATH), SOURCE)?;
    let graph_path = root.join("graph.json");
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "test".to_owned(),
        source_tree_digest: "test".to_owned(),
        configuration_digest: "test".to_owned(),
        generation_id: "generation-cli-export".to_owned(),
        source_commit: None,
    });
    graph.graph.files.push(FileRecord {
        id: file_id(SOURCE_PATH),
        path: SOURCE_PATH.to_owned(),
        language: Some("rust".to_owned()),
        content_digest: Digest::raw_bytes(SOURCE).as_str().to_owned(),
        byte_size: SOURCE.len() as u64,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["cli-export-test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    std::fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
    let state_root = root.join("agent-state");
    let request_path = root.join("request.json");
    let source_digest = Digest::raw_bytes(SOURCE);
    let base_digest = Digest::raw_bytes(&canonical_bytes(&graph)?);
    let request = serde_json::json!({
        "schema":"compass.agent-graph.batch/1",
        "overlay":"overlay:default",
        "baseGeneration":{
            "generationId":"generation-cli-export",
            "graphDigest":base_digest
        },
        "idempotencyKey":"idempotency:cli-export-create",
        "operations":[{
            "operation":"put_assertion",
            "assertion":{
                "selector":{"selector":"new","key":"key:cli-export-node"},
                "fact":{
                    "factType":"node",
                    "kind":"function",
                    "name":"created",
                    "qualifiedName":"crate::created",
                    "language":"rust"
                },
                "grounding":{
                    "schema":"compass.agent-graph.grounding/1",
                    "policyId":"compass.agent-graph.topology-source-span",
                    "evidence":[{
                        "evidenceType":"source_span",
                        "file":SOURCE_PATH,
                        "anchor":{
                            "file":SOURCE_PATH,
                            "startByte":0,
                            "endByte":1,
                            "startLine":1,
                            "startColumn":0,
                            "endLine":1,
                            "endColumn":1
                        },
                        "fileDigest":source_digest,
                        "excerptDigest":source_digest
                    }]
                },
                "summary":"The source contains one grounded node."
            }
        }]
    });
    std::fs::write(&request_path, serde_json::to_vec(&request)?)?;
    let apply = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "agent-graph",
            "apply",
            "--graph",
            graph_path.to_str().ok_or("non-UTF-8 graph path")?,
            "--root",
            root.to_str().ok_or("non-UTF-8 project path")?,
            "--state-root",
            state_root.to_str().ok_or("non-UTF-8 state path")?,
            "--request",
            request_path.to_str().ok_or("non-UTF-8 request path")?,
            "--enable-writes",
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&apply.stdout)?;
    let revision = receipt["revision"]
        .as_str()
        .ok_or("apply receipt omitted revision")?;
    let output_path = root.join("effective.json");
    let export_arguments = [
        "agent-graph",
        "export",
        "--graph",
        graph_path.to_str().ok_or("non-UTF-8 graph path")?,
        "--root",
        root.to_str().ok_or("non-UTF-8 project path")?,
        "--state-root",
        state_root.to_str().ok_or("non-UTF-8 state path")?,
        "--revision",
        revision,
        "--output",
        output_path.to_str().ok_or("non-UTF-8 output path")?,
        "--format",
        "json",
    ];
    let first = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(export_arguments)
        .output()?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let original = std::fs::read(&output_path)?;
    let second = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(export_arguments)
        .output()?;
    assert_eq!(second.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&second.stderr)?;
    assert_eq!(error["code"], "storage_failure");
    assert_eq!(std::fs::read(&output_path)?, original);
    Ok(())
}
