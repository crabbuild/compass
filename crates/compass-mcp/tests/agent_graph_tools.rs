use std::collections::BTreeSet;

use compass_agent_graph::PrincipalId;
use compass_mcp::{AgentGraphMcpConfig, CompassMcp};
use compass_model::code_graph::{BuildMetadata, GraphDocument};
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
