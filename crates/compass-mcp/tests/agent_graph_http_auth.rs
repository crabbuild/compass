use std::collections::BTreeSet;

use compass_agent_graph::PrincipalId;
use compass_mcp::{AgentGraphMcpConfig, HttpOptions, serve_http};

fn write_config(
    project: &std::path::Path,
) -> Result<AgentGraphMcpConfig, Box<dyn std::error::Error>> {
    Ok(AgentGraphMcpConfig {
        writes_enabled: true,
        masks_enabled: false,
        principal: PrincipalId::parse("principal:http-test")?,
        allowed_projects: BTreeSet::from([project.to_path_buf()]),
        non_git_state_root: Some(project.join("agent-state")),
    }
    .validate()?)
}

#[tokio::test]
async fn http_write_server_rejects_missing_independent_capability_key()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let project = directory.path().canonicalize()?;
    let mut options = HttpOptions::new(project.join("graph.json"));
    options.api_key = Some("read-secret".to_owned());
    options.agent_graph = Some(write_config(&project)?);

    let error = serve_http(options)
        .await
        .err()
        .ok_or("write server unexpectedly accepted a missing capability key")?;
    assert!(error.contains("both a read API key and a separate write capability key"));
    Ok(())
}

#[tokio::test]
async fn http_write_server_rejects_a_shared_read_and_write_key()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let project = directory.path().canonicalize()?;
    let mut options = HttpOptions::new(project.join("graph.json"));
    options.api_key = Some("shared-secret".to_owned());
    options.write_api_key = Some("shared-secret".to_owned());
    options.agent_graph = Some(write_config(&project)?);

    let error = serve_http(options)
        .await
        .err()
        .ok_or("write server unexpectedly accepted one shared credential")?;
    assert!(error.contains("must be distinct"));
    Ok(())
}
