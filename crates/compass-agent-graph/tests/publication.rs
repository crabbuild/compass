mod common;

use compass_agent_graph::{
    AgentFactDraft, AgentGraphErrorCode, AgentGraphOverlay, AgentNodeDraft, AssertionDraft,
    AssertionKey, AssertionSelector, ChangeBatch, ChangeOperation, Digest, GcAuthority,
    GroundingEvidence, IdempotencyKey, OverlayRepository, PinId, ReadRequest, ReadResult,
    RepositoryId,
};
use compass_model::code_graph::NodeKind;
use compass_store::{
    Key, KeyRange, MemoryStore, NamespaceId, PartitionKey, ScanLimits, Store, WriteCondition,
};

#[test]
fn immutable_revision_reopens_and_idempotent_retry_returns_original_receipt()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = common::fixture()?;
    let batch = common::create_batch(&fixture, "idempotency:create")?;
    let grant = common::grant(&fixture, "principal:owner", None)?;
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        fixture.provider,
        RepositoryId::parse("repository:test")?,
    );

    let first = repository.apply(&grant, batch.clone())?;
    assert_eq!(first.active_assertions, 2);
    let replay = repository.apply(&grant, batch)?;
    assert_eq!(replay.revision, first.revision);
    assert!(replay.idempotent_replay);

    let result = repository.read(ReadRequest::Overlay {
        overlay: first.overlay.clone(),
        revision: Some(first.revision.clone()),
    })?;
    let ReadResult::Overlay { state, .. } = result else {
        return Err("expected overlay result".into());
    };
    assert_eq!(state.assertions.len(), 2);
    let ReadResult::Audit(audit) = repository.read(ReadRequest::Audit {
        revision: first.revision,
    })?
    else {
        return Err("expected audit record".into());
    };
    assert_eq!(audit.records.len(), 1);
    assert_eq!(audit.records[0].adapter, "local");
    let encoded = serde_json::to_string(&audit)?;
    assert!(!encoded.contains("pub fn caller"));
    assert!(!encoded.contains("credential"));
    Ok(())
}

#[test]
fn corrupted_audit_storage_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = common::fixture()?;
    let store = std::sync::Arc::new(MemoryStore::default());
    let repository = OverlayRepository::new(
        store.clone(),
        fixture.provider.clone(),
        RepositoryId::parse("repository:test")?,
    );
    let receipt = repository.apply(
        &common::grant(&fixture, "principal:owner", None)?,
        common::create_batch(&fixture, "idempotency:corrupt-audit")?,
    )?;
    let namespace = NamespaceId::new(b"compass.agent-graph.v1")?;
    let partition = PartitionKey::new(b"audit")?;
    let page = store.scan(
        &namespace,
        &partition,
        &KeyRange::default(),
        ScanLimits {
            max_items: 64,
            max_bytes: 1024 * 1024,
        },
        None,
    )?;
    let audit = page.entries.first().ok_or("missing stored audit record")?;
    store.put(
        &namespace,
        &partition,
        &Key::new(&audit.key)?,
        br#"{"schema":"compass.agent-graph.audit/1","adapter":"oversized-or-incomplete"}"#,
        WriteCondition::Version(audit.version),
    )?;

    let error = repository
        .read(ReadRequest::Audit {
            revision: receipt.revision,
        })
        .err()
        .ok_or("corrupted audit unexpectedly passed")?;
    assert_eq!(error.code, AgentGraphErrorCode::CorruptOverlay);
    Ok(())
}

#[test]
fn sqlite_revision_survives_close_and_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("agent-graph.sqlite3");
    let fixture = common::fixture()?;
    let provider = fixture.provider.clone();
    let receipt = {
        let repository = OverlayRepository::new(
            compass_store::SqliteStore::open(&path)?,
            provider.clone(),
            RepositoryId::parse("repository:test")?,
        );
        repository.apply(
            &common::grant(&fixture, "principal:owner", None)?,
            common::create_batch(&fixture, "idempotency:sqlite-reopen")?,
        )?
    };
    let reopened = OverlayRepository::new(
        compass_store::SqliteStore::open(&path)?,
        provider,
        RepositoryId::parse("repository:test")?,
    );
    let ReadResult::Overlay { state, .. } = reopened.read(ReadRequest::Overlay {
        overlay: receipt.overlay,
        revision: Some(receipt.revision),
    })?
    else {
        return Err("expected reopened overlay".into());
    };
    assert_eq!(state.assertions.len(), 2);
    Ok(())
}

#[test]
fn competing_writers_do_not_lose_an_update() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, Barrier};

    let fixture = common::fixture()?;
    let repository = Arc::new(OverlayRepository::new(
        MemoryStore::default(),
        fixture.provider.clone(),
        RepositoryId::parse("repository:test")?,
    ));
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for suffix in ["one", "two"] {
        let repository = Arc::clone(&repository);
        let barrier = Arc::clone(&barrier);
        let grant = common::grant(&fixture, "principal:owner", None)?;
        let batch = common::create_batch(&fixture, &format!("idempotency:{suffix}"))?;
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            repository.apply(&grant, batch)
        }));
    }
    let mut successes = 0;
    let mut conflicts = 0;
    for handle in handles {
        match handle.join().map_err(|_| "writer thread panicked")? {
            Ok(_) => successes += 1,
            Err(error)
                if error.code == compass_agent_graph::AgentGraphErrorCode::RevisionConflict =>
            {
                conflicts += 1;
            }
            Err(error) => return Err(error.into()),
        }
    }
    assert_eq!(successes, 1);
    assert_eq!(conflicts, 1);
    Ok(())
}

#[test]
fn effective_read_contains_grounded_node_and_directed_edge()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = common::fixture()?;
    let batch = common::create_batch(&fixture, "idempotency:effective")?;
    let grant = common::grant(&fixture, "principal:owner", None)?;
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        fixture.provider,
        RepositoryId::parse("repository:test")?,
    );
    let receipt = repository.apply(&grant, batch)?;
    let result = repository.read(ReadRequest::EffectiveGraph {
        overlay: receipt.overlay,
        revision: receipt.revision,
        profile: compass_agent_graph::CompositionProfile::Augment,
    })?;
    let ReadResult::EffectiveGraph(effective) = result else {
        return Err("expected Effective Graph".into());
    };
    assert_eq!(effective.graph.nodes.len(), 2);
    assert_eq!(effective.graph.links.len(), 1);
    assert_eq!(effective.graph.links[0].target, common::BASE_NODE_ID);
    assert_eq!(effective.agent_facts.len(), 2);
    Ok(())
}

#[test]
fn repository_resolves_prior_assertions_from_the_exact_immutable_revision()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = common::fixture()?;
    let repository = OverlayRepository::new(
        MemoryStore::default(),
        fixture.provider.clone(),
        RepositoryId::parse("repository:test")?,
    );
    let first = repository.apply(
        &common::grant(&fixture, "principal:owner", None)?,
        common::create_batch(&fixture, "idempotency:prior-source")?,
    )?;
    let ReadResult::Overlay { state, .. } = repository.read(ReadRequest::Overlay {
        overlay: first.overlay.clone(),
        revision: Some(first.revision.clone()),
    })?
    else {
        return Err("expected overlay state".into());
    };
    let prior = state
        .assertions
        .values()
        .next()
        .ok_or("missing assertion")?;
    let mut grounding = common::grounding(fixture.anchor.clone());
    grounding.evidence.push(GroundingEvidence::PriorAssertion {
        assertion: prior.id.clone(),
        revision: first.revision.clone(),
        assertion_digest: prior.assertion_digest.clone(),
    });
    let batch = ChangeBatch {
        schema: "compass.agent-graph.batch/1".to_owned(),
        overlay: first.overlay.clone(),
        base_generation: fixture.identity.clone(),
        expected_revision: Some(first.revision.clone()),
        idempotency_key: IdempotencyKey::parse("idempotency:prior-dependent")?,
        operations: vec![ChangeOperation::PutAssertion {
            assertion: AssertionDraft {
                selector: AssertionSelector::New {
                    key: AssertionKey::parse("key:prior-dependent")?,
                },
                fact: AgentFactDraft::Node(AgentNodeDraft {
                    kind: NodeKind::Function,
                    roles: Vec::new(),
                    name: "dependent".to_owned(),
                    qualified_name: "crate::dependent".to_owned(),
                    language: Some("rust".to_owned()),
                    framework: None,
                    details: None,
                }),
                grounding,
                summary: "Depends on an exact prior Agent Assertion.".to_owned(),
            },
        }],
    };
    let second = repository.apply(
        &common::grant(&fixture, "principal:owner", Some(first.revision))?,
        batch,
    )?;
    let reopened = repository.read(ReadRequest::Overlay {
        overlay: second.overlay,
        revision: Some(second.revision),
    })?;
    let ReadResult::Overlay { state, .. } = reopened else {
        return Err("expected reopened overlay state".into());
    };
    assert_eq!(state.assertions.len(), 3);

    let unknown = repository.read(ReadRequest::Overlay {
        overlay: compass_agent_graph::OverlayId::parse("overlay:review")?,
        revision: Some(compass_agent_graph::OverlayRevisionId(Digest::raw_bytes(
            b"unknown-prior",
        ))),
    });
    assert_eq!(
        unknown.err().map(|error| error.code),
        Some(AgentGraphErrorCode::UnknownOverlay)
    );
    Ok(())
}

#[test]
fn gc_retains_active_and_pinned_history_and_sweeps_only_exact_unreachable_objects()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = common::fixture()?;
    let store = std::sync::Arc::new(MemoryStore::default());
    let repository_id = RepositoryId::parse("repository:test")?;
    let grant = common::grant(&fixture, "principal:owner", None)?;
    let batch = common::create_batch(&fixture, "idempotency:gc")?;
    let repository = OverlayRepository::new(store.clone(), fixture.provider, repository_id.clone());
    let receipt = repository.apply(&grant, batch)?;
    repository.pin_revision(
        PinId::parse("pin:review")?,
        receipt.overlay.clone(),
        receipt.revision.clone(),
    )?;

    let orphan_value = [vec![1_u8], b"unreachable".to_vec()].concat();
    let orphan_digest = Digest::raw_bytes(&orphan_value);
    store.put(
        &NamespaceId::new(b"compass.agent-graph.v1")?,
        &PartitionKey::new(b"objects")?,
        &Key::new(orphan_digest.as_str().as_bytes())?,
        &orphan_value,
        WriteCondition::Missing,
    )?;

    let plan = repository.plan_gc(10_000, 4 * 1024 * 1024)?;
    assert!(!plan.truncated);
    assert!(plan.unreachable_revisions.is_empty());
    assert_eq!(plan.unreachable_objects, vec![orphan_digest.clone()]);
    let disabled = GcAuthority::disabled(repository_id.clone()).mint();
    assert!(disabled.is_err());
    let receipt = repository.sweep_gc(
        &GcAuthority::explicitly_quiescent(repository_id).mint()?,
        &plan,
    )?;
    assert_eq!(receipt.deleted_objects, 1);
    assert_eq!(receipt.deleted_audits, 0);
    assert!(
        store
            .get(
                &NamespaceId::new(b"compass.agent-graph.v1")?,
                &PartitionKey::new(b"objects")?,
                &Key::new(orphan_digest.as_str().as_bytes())?,
            )?
            .is_none()
    );
    assert!(
        repository
            .read(ReadRequest::Overlay {
                overlay: receipt_for_overlay(&repository)?,
                revision: None,
            })
            .is_ok()
    );
    Ok(())
}

fn receipt_for_overlay<S, P>(
    repository: &OverlayRepository<S, P>,
) -> Result<compass_agent_graph::OverlayId, Box<dyn std::error::Error>>
where
    S: Store + Send + Sync,
    P: compass_agent_graph::BaseGenerationProvider,
{
    let overlay = compass_agent_graph::OverlayId::parse("overlay:review")?;
    if repository.active_revision(&overlay)?.is_none() {
        return Err("active revision disappeared after GC".into());
    }
    Ok(overlay)
}
