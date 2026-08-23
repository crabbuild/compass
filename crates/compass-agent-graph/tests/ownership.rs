mod common;

use compass_agent_graph::{
    AgentFactDraft, AgentGraphErrorCode, AgentGraphOverlay, AssertionDraft, AssertionSelector,
    ChangeBatch, ChangeOperation, IdempotencyKey, OverlayRepository, ReadRequest, ReadResult,
    RepositoryId,
};
use compass_store::MemoryStore;

#[test]
fn only_the_owner_can_replace_or_retract_an_assertion() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = common::fixture()?;
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        fixture.provider.clone(),
        RepositoryId::parse("repository:test")?,
    );
    let first = repository.apply(
        &common::grant(&fixture, "principal:owner", None)?,
        common::create_batch(&fixture, "idempotency:ownership-create")?,
    )?;
    let ReadResult::Overlay { state, .. } = repository.read(ReadRequest::Overlay {
        overlay: first.overlay.clone(),
        revision: Some(first.revision.clone()),
    })?
    else {
        return Err("expected overlay state".into());
    };
    let node = state
        .assertions
        .values()
        .find_map(|assertion| match &assertion.fact {
            compass_agent_graph::ResolvedAgentFact::Node(node) => Some((assertion, node.clone())),
            compass_agent_graph::ResolvedAgentFact::Edge(_) => None,
        })
        .ok_or("missing node assertion")?;
    let replacement = ChangeBatch {
        schema: "compass.agent-graph.batch/1".to_owned(),
        overlay: first.overlay.clone(),
        base_generation: fixture.identity.clone(),
        expected_revision: Some(first.revision.clone()),
        idempotency_key: IdempotencyKey::parse("idempotency:ownership-replace")?,
        operations: vec![ChangeOperation::PutAssertion {
            assertion: AssertionDraft {
                selector: AssertionSelector::Existing {
                    id: node.0.id.clone(),
                    expected_assertion_digest: node.0.assertion_digest.clone(),
                },
                fact: AgentFactDraft::Node(node.1),
                grounding: common::grounding(fixture.anchor.clone()),
                summary: "A replacement that preserves the exact fact.".to_owned(),
            },
        }],
    };
    let error = repository
        .apply(
            &common::grant(&fixture, "principal:other", Some(first.revision.clone()))?,
            replacement,
        )
        .err()
        .ok_or("cross-principal replacement unexpectedly succeeded")?;
    assert_eq!(error.code, AgentGraphErrorCode::OwnershipViolation);

    let retraction = ChangeBatch {
        schema: "compass.agent-graph.batch/1".to_owned(),
        overlay: first.overlay,
        base_generation: fixture.identity.clone(),
        expected_revision: Some(first.revision.clone()),
        idempotency_key: IdempotencyKey::parse("idempotency:ownership-retract")?,
        operations: vec![ChangeOperation::RetractAssertion {
            assertion: node.0.id.clone(),
            expected_assertion_digest: node.0.assertion_digest.clone(),
            reason_code: "invalid".to_owned(),
            explanation: "Cross-principal Retraction must be rejected.".to_owned(),
        }],
    };
    let error = repository
        .apply(
            &common::grant(&fixture, "principal:other", Some(first.revision))?,
            retraction,
        )
        .err()
        .ok_or("cross-principal Retraction unexpectedly succeeded")?;
    assert_eq!(error.code, AgentGraphErrorCode::OwnershipViolation);
    Ok(())
}

#[test]
fn the_closed_operation_contract_cannot_express_base_graph_deletion() {
    let raw = serde_json::json!({
        "schema":"compass.agent-graph.batch/1",
        "overlay":"overlay:review",
        "baseGeneration":{
            "generationId":"generation-1",
            "graphDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "idempotencyKey":"idempotency:no-base-delete",
        "operations":[{"operation":"delete_base_node","id":"node:target"}]
    });
    assert!(serde_json::from_value::<ChangeBatch>(raw).is_err());
}
