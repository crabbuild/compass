use compass_agent_graph::{
    AGENT_GRAPH_BATCH_SCHEMA_V1, AgentGraphErrorCode, AgentGraphLimits, ChangeBatch,
};
use serde_json::json;

fn digest() -> String {
    "0".repeat(64)
}

fn valid_batch() -> serde_json::Value {
    json!({
        "schema": AGENT_GRAPH_BATCH_SCHEMA_V1,
        "overlay": "overlay:review",
        "baseGeneration": {
            "generationId": "generation-1",
            "graphDigest": digest()
        },
        "idempotencyKey": "idempotency:request-1",
        "operations": [{
            "operation": "put_assertion",
            "assertion": {
                "selector": {"selector": "new", "key": "key:concept"},
                "fact": {
                    "factType": "node",
                    "kind": "resource",
                    "name": "Grounded concept",
                    "qualifiedName": "concept::grounded"
                },
                "grounding": {
                    "schema": "compass.agent-graph.grounding/1",
                    "policyId": "compass.agent-graph.topology-source-span",
                    "evidence": [{
                        "evidenceType": "source_span",
                        "file": "src/lib.rs",
                        "anchor": {
                            "file": "src/lib.rs",
                            "startByte": 0,
                            "endByte": 1,
                            "startLine": 1,
                            "startColumn": 0,
                            "endLine": 1,
                            "endColumn": 1
                        },
                        "fileDigest": digest(),
                        "excerptDigest": digest()
                    }]
                },
                "summary": "A concise claim"
            }
        }]
    })
}

#[test]
fn batch_is_strict_versioned_and_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let batch: ChangeBatch = serde_json::from_value(valid_batch())?;
    batch.validate(AgentGraphLimits::default())?;

    let mut wrong_schema = valid_batch();
    wrong_schema["schema"] = json!("compass.agent-graph.batch/2");
    let error = serde_json::from_value::<ChangeBatch>(wrong_schema)?
        .validate(AgentGraphLimits::default())
        .err()
        .ok_or("unknown major unexpectedly passed validation")?;
    assert_eq!(error.code, AgentGraphErrorCode::UnsupportedSchema);

    let mut unknown = valid_batch();
    unknown["callerGrounded"] = json!(true);
    assert!(serde_json::from_value::<ChangeBatch>(unknown).is_err());
    Ok(())
}

#[test]
fn identifiers_reject_missing_domain_prefix() {
    let mut invalid = valid_batch();
    invalid["overlay"] = json!("review");
    assert!(serde_json::from_value::<ChangeBatch>(invalid).is_err());
}

#[test]
fn checked_in_v1_fixtures_match_the_rust_contracts() -> Result<(), Box<dyn std::error::Error>> {
    let batch = serde_json::from_str::<ChangeBatch>(include_str!(
        "../../../fixtures/contracts/agent-graph/batch-v1.json"
    ))?;
    batch.validate(AgentGraphLimits::default())?;
    let receipt = serde_json::from_str::<compass_agent_graph::CommitReceipt>(include_str!(
        "../../../fixtures/contracts/agent-graph/receipt-v1.json"
    ))?;
    assert_eq!(receipt.schema, "compass.agent-graph.receipt/1");
    let preparation = serde_json::from_str::<compass_agent_graph::IngestionPreparation>(
        include_str!("../../../fixtures/contracts/agent-graph/ingestion-preparation-v1.json"),
    )?;
    assert_eq!(
        preparation.schema,
        "compass.agent-graph.ingestion-preparation/1"
    );
    let overlay = serde_json::from_str::<compass_agent_graph::OverlayState>(include_str!(
        "../../../fixtures/contracts/agent-graph/overlay-v1.json"
    ))?;
    assert_eq!(overlay.schema, "compass.agent-graph.overlay/1");
    let effective = serde_json::from_str::<compass_agent_graph::EffectiveGraph>(include_str!(
        "../../../fixtures/contracts/agent-graph/effective-v1.json"
    ))?;
    assert_eq!(effective.schema, "compass.agent-graph.effective/1");
    let plan = serde_json::from_str::<compass_agent_graph::RebasePlan>(include_str!(
        "../../../fixtures/contracts/agent-graph/rebase-plan-v1.json"
    ))?;
    plan.validate()?;
    let audit = serde_json::from_str::<compass_agent_graph::AuditRecord>(include_str!(
        "../../../fixtures/contracts/agent-graph/audit-v1.json"
    ))?;
    audit.validate()?;
    Ok(())
}
