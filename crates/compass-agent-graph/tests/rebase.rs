mod common;

use compass_agent_graph::{
    AgentGraphErrorCode, AgentGraphOverlay, ChangeOperation, IdempotencyKey,
    InMemoryBaseGenerationProvider, OverlayRepository, ReadRequest, ReadResult,
    RebaseCommitRequest, RebaseDisposition, RepositoryId, ResolvedAgentFact,
};
use compass_store::MemoryStore;

#[test]
fn exact_rebase_regrounds_retained_facts_and_preserves_old_revision()
-> Result<(), Box<dyn std::error::Error>> {
    let old = common::fixture_for_generation("generation-old")?;
    let target = common::fixture_for_generation("generation-target")?;
    let provider = InMemoryBaseGenerationProvider::default()
        .with_generation(old.generation.clone())
        .with_generation(target.generation.clone());
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        provider,
        RepositoryId::parse("repository:test")?,
    );
    let first = repository.apply(
        &common::grant(&old, "principal:owner", None)?,
        common::create_batch(&old, "idempotency:rebase-source")?,
    )?;
    let ReadResult::RebasePlan(plan) = repository.read(ReadRequest::PrepareRebase {
        overlay: first.overlay.clone(),
        source_revision: first.revision.clone(),
        target_base_generation: target.identity.clone(),
    })?
    else {
        return Err("expected rebase plan".into());
    };
    assert_eq!(plan.unresolved_count, 0);
    assert!(
        plan.items
            .iter()
            .all(|item| item.disposition == RebaseDisposition::RetainedExact)
    );
    let receipt = repository.commit_rebase(
        &common::grant(&target, "principal:owner", Some(first.revision.clone()))?,
        RebaseCommitRequest {
            schema: "compass.agent-graph.rebase-commit/1".to_owned(),
            plan,
            idempotency_key: IdempotencyKey::parse("idempotency:rebase-commit")?,
            resolution_operations: Vec::new(),
        },
    )?;
    assert_eq!(receipt.base_generation, target.identity);
    assert_eq!(receipt.parent_revision, Some(first.revision.clone()));

    let ReadResult::Overlay {
        state: old_state, ..
    } = repository.read(ReadRequest::Overlay {
        overlay: first.overlay.clone(),
        revision: Some(first.revision),
    })?
    else {
        return Err("expected old overlay revision".into());
    };
    assert_eq!(old_state.base_generation, old.identity);
    let ReadResult::Overlay {
        state: new_state, ..
    } = repository.read(ReadRequest::Overlay {
        overlay: first.overlay,
        revision: Some(receipt.revision),
    })?
    else {
        return Err("expected rebased overlay revision".into());
    };
    assert_eq!(new_state.base_generation, target.identity);
    assert!(new_state.assertions.values().all(|assertion| {
        assertion.certificate.base_generation() == &new_state.base_generation
    }));
    Ok(())
}

#[test]
fn changed_source_is_unresolved_and_cannot_be_committed_without_resolution()
-> Result<(), Box<dyn std::error::Error>> {
    let old = common::fixture_for_generation("generation-old")?;
    let target = common::fixture_for_generation_and_source(
        "generation-target",
        b"pub fn caller() { changed(); }\npub fn target() {}\n",
    )?;
    let provider = InMemoryBaseGenerationProvider::default()
        .with_generation(old.generation.clone())
        .with_generation(target.generation.clone());
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        provider,
        RepositoryId::parse("repository:test")?,
    );
    let first = repository.apply(
        &common::grant(&old, "principal:owner", None)?,
        common::create_batch(&old, "idempotency:changed-source")?,
    )?;
    let ReadResult::RebasePlan(plan) = repository.read(ReadRequest::PrepareRebase {
        overlay: first.overlay,
        source_revision: first.revision.clone(),
        target_base_generation: target.identity.clone(),
    })?
    else {
        return Err("expected rebase plan".into());
    };
    assert_eq!(plan.unresolved_count, 2);
    assert!(
        plan.items
            .iter()
            .all(|item| item.exact_candidates.is_empty())
    );
    let error = repository
        .commit_rebase(
            &common::grant(&target, "principal:owner", Some(first.revision))?,
            RebaseCommitRequest {
                schema: "compass.agent-graph.rebase-commit/1".to_owned(),
                plan: plan.clone(),
                idempotency_key: IdempotencyKey::parse("idempotency:unresolved")?,
                resolution_operations: Vec::new(),
            },
        )
        .err()
        .ok_or("unresolved rebase unexpectedly committed")?;
    assert_eq!(error.code, AgentGraphErrorCode::RebaseUnresolved);

    let ReadResult::Overlay { state, .. } = repository.read(ReadRequest::Overlay {
        overlay: plan.overlay.clone(),
        revision: Some(plan.source_revision.clone()),
    })?
    else {
        return Err("expected source Overlay Revision".into());
    };
    let mut active = state.assertions.values().collect::<Vec<_>>();
    active.sort_by_key(|assertion| match &assertion.fact {
        ResolvedAgentFact::Edge(_) => 0,
        ResolvedAgentFact::Node(_) => 1,
    });
    let resolution_operations = active
        .into_iter()
        .map(|assertion| ChangeOperation::RetractAssertion {
            assertion: assertion.id.clone(),
            expected_assertion_digest: assertion.assertion_digest.clone(),
            reason_code: "historical_evidence_changed".to_owned(),
            explanation: "Explicitly retract stale assertion during rebase.".to_owned(),
        })
        .collect();
    let resolved = repository.commit_rebase(
        &common::grant(
            &target,
            "principal:owner",
            Some(plan.source_revision.clone()),
        )?,
        RebaseCommitRequest {
            schema: "compass.agent-graph.rebase-commit/1".to_owned(),
            plan,
            idempotency_key: IdempotencyKey::parse("idempotency:explicit-retractions")?,
            resolution_operations,
        },
    )?;
    assert_eq!(resolved.active_assertions, 0);
    assert_eq!(resolved.retractions, 2);
    Ok(())
}
