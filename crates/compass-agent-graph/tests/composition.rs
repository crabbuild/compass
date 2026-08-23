mod common;

use compass_agent_graph::{
    AgentGraphOverlay, BaseFactRef, ChallengeDraft, ChallengeEffect, ChallengeId,
    ChallengeSelector, ChangeBatch, ChangeOperation, GroundingEvidence, IdempotencyKey,
    OverlayRepository, ReadRequest, ReadResult, RepositoryId,
};
use compass_store::MemoryStore;

#[test]
fn curated_node_mask_reports_direct_and_cascaded_omissions()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = common::fixture()?;
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        fixture.provider.clone(),
        RepositoryId::parse("repository:test")?,
    );
    let first = repository.apply(
        &common::grant(&fixture, "principal:owner", None)?,
        common::create_batch(&fixture, "idempotency:composition-create")?,
    )?;
    let target = BaseFactRef::Node(fixture.base_node.clone());
    let mut grounding = common::grounding(fixture.anchor.clone());
    grounding.evidence.push(GroundingEvidence::BaseFact {
        fact: target.clone(),
        record_digest: fixture.base_node.record_digest.clone(),
    });
    let mask = ChangeBatch {
        schema: "compass.agent-graph.batch/1".to_owned(),
        overlay: first.overlay.clone(),
        base_generation: fixture.identity.clone(),
        expected_revision: Some(first.revision.clone()),
        idempotency_key: IdempotencyKey::parse("idempotency:composition-mask")?,
        operations: vec![ChangeOperation::PutChallenge {
            challenge: ChallengeDraft {
                selector: ChallengeSelector::New {
                    id: ChallengeId::parse("challenge:stale-target")?,
                },
                target,
                effect: ChallengeEffect::Mask,
                grounding,
                summary: "The target is obsolete in the curated view.".to_owned(),
            },
        }],
    };
    let second = repository.apply(
        &common::grant(&fixture, "principal:owner", Some(first.revision))?,
        mask,
    )?;

    let ReadResult::EffectiveGraph(augment) = repository.read(ReadRequest::EffectiveGraph {
        overlay: second.overlay.clone(),
        revision: second.revision.clone(),
        profile: compass_agent_graph::CompositionProfile::Augment,
    })?
    else {
        return Err("expected augment graph".into());
    };
    assert_eq!(augment.graph.nodes.len(), 2);
    assert_eq!(augment.graph.links.len(), 1);
    assert_eq!(augment.omissions.total, 0);
    assert!(!augment.challenges[0].masked);

    let ReadResult::EffectiveGraph(curated) = repository.read(ReadRequest::EffectiveGraph {
        overlay: second.overlay,
        revision: second.revision,
        profile: compass_agent_graph::CompositionProfile::Curated,
    })?
    else {
        return Err("expected curated graph".into());
    };
    assert_eq!(curated.graph.nodes.len(), 1);
    assert!(curated.graph.links.is_empty());
    assert_eq!(curated.omissions.direct, 1);
    assert_eq!(curated.omissions.cascaded, 1);
    assert!(curated.challenges[0].masked);
    Ok(())
}
