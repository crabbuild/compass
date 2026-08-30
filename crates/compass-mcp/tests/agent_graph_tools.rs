use std::collections::BTreeSet;

use compass_agent_graph::{Digest, PrincipalId};
use compass_mcp::{AgentGraphMcpConfig, CompassMcp};
use compass_model::code_graph::{
    BuildMetadata, ExtractionStatus, FileRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::identity::file_id;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
use serde_json::{Map, json};

#[test]
fn agent_graph_tools_are_configured_deny_by_default_and_scoped()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let project = directory.path().canonicalize()?;
    let output = project.join("compass-out");
    std::fs::create_dir_all(&output)?;
    let graph_path = output.join("graph.json");
    let graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "test".to_owned(),
        source_tree_digest: "test".to_owned(),
        configuration_digest: "test".to_owned(),
        generation_id: "generation-mcp-agent".to_owned(),
        source_commit: None,
    });
    std::fs::write(&graph_path, serde_json::to_vec(&graph)?)?;

    let unconfigured = CompassMcp::new(&graph_path);
    assert!(
        !unconfigured
            .configured_tools()
            .iter()
            .any(|tool| tool.name == "inspect_agent_graph")
    );
    let read_only = CompassMcp::new(&graph_path).with_agent_graph(AgentGraphMcpConfig {
        writes_enabled: false,
        masks_enabled: false,
        principal: PrincipalId::parse("principal:mcp-test")?,
        allowed_projects: BTreeSet::from([project.clone()]),
        non_git_state_root: Some(project.join("agent-state")),
    })?;
    let tools = read_only.configured_tools();
    assert!(tools.iter().any(|tool| tool.name == "inspect_agent_graph"));
    assert!(!tools.iter().any(|tool| tool.name == "apply_agent_graph"));

    let response = read_only.invoke(
        "inspect_agent_graph",
        Map::from_iter([
            ("project_path".to_owned(), json!(project.to_string_lossy())),
            ("operation".to_owned(), json!("status")),
            ("overlay".to_owned(), json!("overlay:review")),
        ]),
    );
    assert!(response.contains("compass.agent-graph.status/1"));
    assert!(response.contains("generation-mcp-agent"));
    assert!(response.contains("\"writesEnabled\":false"));
    Ok(())
}

#[test]
fn non_git_state_root_cannot_be_shared_across_project_scopes()
-> Result<(), Box<dyn std::error::Error>> {
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    let config = AgentGraphMcpConfig {
        writes_enabled: true,
        masks_enabled: false,
        principal: PrincipalId::parse("principal:mcp-test")?,
        allowed_projects: BTreeSet::from([
            first.path().canonicalize()?,
            second.path().canonicalize()?,
        ]),
        non_git_state_root: Some(first.path().join("state")),
    };
    assert!(
        CompassMcp::new("unused.json")
            .with_agent_graph(config)
            .is_err()
    );
    Ok(())
}

#[test]
fn read_only_agent_tool_prepares_exact_ingestion_material() -> Result<(), Box<dyn std::error::Error>>
{
    const SOURCE_PATH: &str = "src/lib.rs";
    const SOURCE: &[u8] = b"pub fn target() {}\n";
    const NODE_ID: &str = "node:target";

    let directory = tempfile::tempdir()?;
    let project = directory.path().canonicalize()?;
    let output = project.join("compass-out");
    std::fs::create_dir_all(&output)?;
    std::fs::create_dir_all(project.join("src"))?;
    std::fs::write(project.join(SOURCE_PATH), SOURCE)?;
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
        generation_id: "generation-mcp-prepare".to_owned(),
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
        extractor_versions: vec!["mcp-prepare-test".to_owned()],
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
    let graph_path = output.join("graph.json");
    std::fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
    let server = CompassMcp::new(&graph_path).with_agent_graph(AgentGraphMcpConfig {
        writes_enabled: false,
        masks_enabled: false,
        principal: PrincipalId::parse("principal:mcp-test")?,
        allowed_projects: BTreeSet::from([project.clone()]),
        non_git_state_root: Some(project.join("agent-state")),
    })?;
    let response = server.invoke(
        "inspect_agent_graph",
        Map::from_iter([
            ("project_path".to_owned(), json!(project.to_string_lossy())),
            ("operation".to_owned(), json!("prepare")),
            ("overlay".to_owned(), json!("overlay:review")),
            ("base_nodes".to_owned(), json!([NODE_ID])),
            (
                "source_spans".to_owned(),
                json!([{"file":SOURCE_PATH,"startByte":0,"endByte":18}]),
            ),
        ]),
    );
    let envelope: serde_json::Value = serde_json::from_str(&response)?;
    assert_eq!(envelope["schema"], "compass.mcp.tool-result/1");
    assert_eq!(
        envelope["result"]["schema"],
        "compass.agent-graph.ingestion-preparation/1"
    );
    assert_eq!(envelope["result"]["baseNodes"][0]["id"], NODE_ID);
    assert_eq!(
        envelope["result"]["grounding"]["evidence"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert!(!response.contains("GROUNDED"));

    let rejected = server.invoke(
        "inspect_agent_graph",
        Map::from_iter([
            ("project_path".to_owned(), json!(project.to_string_lossy())),
            ("operation".to_owned(), json!("prepare")),
            ("overlay".to_owned(), json!("overlay:review")),
            ("revision".to_owned(), json!("0".repeat(64))),
            (
                "source_spans".to_owned(),
                json!([{"file":SOURCE_PATH,"startByte":0,"endByte":18}]),
            ),
        ]),
    );
    assert!(rejected.contains("do not pass revision"));
    Ok(())
}
