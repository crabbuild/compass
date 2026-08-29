use compass_mcp::CompassMcp;
use rmcp::model::{
    ClientCapabilities, ClientInfo, Implementation, ProtocolVersion, Resource, ServerCapabilities,
    Tool,
};
use rmcp::{ClientLifecycleMode, ClientServiceExt, ServerHandler, ServiceExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn normalized_contract(
    server_info: Implementation,
    capabilities: ServerCapabilities,
    tools: Vec<Tool>,
    resources: Vec<Resource>,
) -> Result<Value, serde_json::Error> {
    let tools = tools
        .into_iter()
        .map(|tool| {
            let output_schema_sha256 = tool
                .output_schema
                .as_ref()
                .map(|schema| {
                    serde_json::to_vec(schema).map(|bytes| format!("{:x}", Sha256::digest(bytes)))
                })
                .transpose()?;
            Ok(json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchemaSha256": format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&tool.input_schema)?)
                ),
                "outputSchemaSha256": output_schema_sha256,
                "meta": tool.meta,
            }))
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    Ok(json!({
        "serverInfo": server_info,
        "capabilities": capabilities,
        "tools": tools,
        "resources": resources,
    }))
}

fn expected_contract() -> Result<Value, serde_json::Error> {
    serde_json::from_str(include_str!("fixtures/rmcp-2.2-server-discover.json"))
}

#[test]
fn local_discovery_matches_rmcp_2_2_golden() -> Result<(), Box<dyn std::error::Error>> {
    let info = CompassMcp::new("unused").get_info();
    let contract = normalized_contract(
        info.server_info,
        info.capabilities,
        CompassMcp::tools(),
        CompassMcp::resources(),
    )?;
    assert_eq!(contract, expected_contract()?);
    Ok(())
}

#[tokio::test]
async fn server_discover_matches_rmcp_2_2_golden() -> Result<(), Box<dyn std::error::Error>> {
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_task = tokio::spawn(async move {
        let running = CompassMcp::new("unused")
            .serve(server_transport)
            .await
            .map_err(|error| error.to_string())?;
        running.waiting().await.map_err(|error| error.to_string())
    });
    let client_info = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("compass-test", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28);
    let client = client_info
        .serve_with_lifecycle(
            client_transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await?;
    let peer = client
        .peer_info()
        .ok_or("server/discover omitted peer data")?;
    let server_info = peer
        .server_info
        .clone()
        .ok_or("server/discover omitted server identity")?;
    let tools = client.list_tools(None).await?.tools;
    let resources = client.list_resources(None).await?.resources;
    let contract = normalized_contract(server_info, peer.capabilities.clone(), tools, resources)?;
    assert_eq!(contract, expected_contract()?);

    client.cancel().await?;
    server_task.await?.map_err(std::io::Error::other)?;
    Ok(())
}
