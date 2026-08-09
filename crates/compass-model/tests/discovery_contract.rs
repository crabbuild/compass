use compass_model::query_contract::{
    DiscoveryEdge, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryQueryResponse,
};
use serde_json::json;

const LIMITS: &str = include_str!("fixtures/discovery_limits_v1.json");
const REQUEST: &str = include_str!("fixtures/discovery_request_v1.json");
const RESPONSE: &str = include_str!("fixtures/discovery_response_v1.json");

#[test]
fn stable_discovery_json_fixtures_round_trip_exactly() -> Result<(), Box<dyn std::error::Error>> {
    let limits = serde_json::from_str::<DiscoveryLimits>(LIMITS)?;
    assert_eq!(limits, DiscoveryLimits::default());
    assert_eq!(
        serde_json::to_value(&limits)?,
        serde_json::from_str::<serde_json::Value>(LIMITS)?
    );

    let request = serde_json::from_str::<DiscoveryQueryRequest>(REQUEST)?;
    assert_eq!(
        serde_json::to_value(&request)?,
        serde_json::from_str::<serde_json::Value>(REQUEST)?
    );

    let response = serde_json::from_str::<DiscoveryQueryResponse>(RESPONSE)?;
    assert_eq!(
        serde_json::to_value(&response)?,
        serde_json::from_str::<serde_json::Value>(RESPONSE)?
    );
    Ok(())
}

#[test]
fn discovery_edge_preserves_an_explicitly_missing_legacy_id()
-> Result<(), Box<dyn std::error::Error>> {
    let value = json!({
        "id": null,
        "source": "caller",
        "target": "callee",
        "kind": "calls",
        "occurrenceRule": null,
        "relationshipSite": null,
        "details": null,
        "evidence": [],
        "context": "call"
    });
    let edge = serde_json::from_value::<DiscoveryEdge>(value.clone())?;
    assert_eq!(edge.id, None);
    assert_eq!(serde_json::to_value(edge)?, value);
    Ok(())
}
