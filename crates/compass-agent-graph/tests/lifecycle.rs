mod common;

use compass_agent_graph::{
    AgentGraphErrorCode, AgentGraphOverlay, ChangeBatch, ChangeOperation, CompositionProfile,
    IdempotencyKey, OverlayRepository, ReadRequest, ReadResult, RepositoryId,
};
use compass_store::MemoryStore;

#[test]
fn node_retraction_requires_incident_edge_in_same_batch() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = common::fixture()?;
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        fixture.provider.clone(),
        RepositoryId::parse("repository:test")?,
    );
    let first = repository.apply(
        &common::grant(&fixture, "principal:owner", None)?,
        common::create_batch(&fixture, "idempotency:create-lifecycle")?,
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
        .find(|assertion| {
            matches!(
                assertion.fact,
                compass_agent_graph::ResolvedAgentFact::Node(_)
            )
        })
        .ok_or("missing node assertion")?;
    let edge = state
        .assertions
        .values()
        .find(|assertion| {
            matches!(
                assertion.fact,
                compass_agent_graph::ResolvedAgentFact::Edge(_)
            )
        })
        .ok_or("missing edge assertion")?;
    let node_only = ChangeBatch {
        schema: "compass.agent-graph.batch/1".to_owned(),
        overlay: first.overlay.clone(),
        base_generation: fixture.identity.clone(),
        expected_revision: Some(first.revision.clone()),
        idempotency_key: IdempotencyKey::parse("idempotency:node-only")?,
        operations: vec![ChangeOperation::RetractAssertion {
            assertion: node.id.clone(),
            expected_assertion_digest: node.assertion_digest.clone(),
            reason_code: "superseded".to_owned(),
            explanation: "Remove the node.".to_owned(),
        }],
    };
    let error = repository
        .apply(
            &common::grant(&fixture, "principal:owner", Some(first.revision.clone()))?,
            node_only,
        )
        .err()
        .ok_or("node-only Retraction unexpectedly succeeded")?;
    assert_eq!(error.code, AgentGraphErrorCode::ActiveDependents);

    let both = ChangeBatch {
        schema: "compass.agent-graph.batch/1".to_owned(),
        overlay: first.overlay,
        base_generation: fixture.identity.clone(),
        expected_revision: Some(first.revision.clone()),
        idempotency_key: IdempotencyKey::parse("idempotency:both")?,
        operations: vec![
            ChangeOperation::RetractAssertion {
                assertion: edge.id.clone(),
                expected_assertion_digest: edge.assertion_digest.clone(),
                reason_code: "superseded".to_owned(),
                explanation: "Remove the edge.".to_owned(),
            },
            ChangeOperation::RetractAssertion {
                assertion: node.id.clone(),
                expected_assertion_digest: node.assertion_digest.clone(),
                reason_code: "superseded".to_owned(),
                explanation: "Remove the node.".to_owned(),
            },
        ],
    };
    let receipt = repository.apply(
        &common::grant(&fixture, "principal:owner", Some(first.revision))?,
        both,
    )?;
    assert_eq!(receipt.active_assertions, 0);
    assert_eq!(receipt.retractions, 2);
    let ReadResult::EffectiveGraph(effective) = repository.read(ReadRequest::EffectiveGraph {
        overlay: receipt.overlay,
        revision: receipt.revision,
        profile: CompositionProfile::Augment,
    })?
    else {
        return Err("expected effective graph with Retraction history".into());
    };
    assert_eq!(effective.retractions.total, 2);
    assert_eq!(effective.retractions.examples.len(), 2);
    assert_eq!(effective.retractions.omitted_examples, 0);
    Ok(())
}
