use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use compass_store::{
    ImmutableWrite, Key, KeyRange, MAX_IMMUTABLE_BATCH_BYTES, MAX_IMMUTABLE_BATCH_ITEMS,
    NamespaceId, PartitionKey, ScanCursor, ScanLimits, Store, StoreError, VersionToken,
    WriteCondition,
};
use serde::{Deserialize, Serialize};

use crate::compose::COMPOSITION_VERSION_V1;
use crate::maintenance::{
    AGENT_GRAPH_GC_PLAN_SCHEMA_V1, AGENT_GRAPH_GC_RECEIPT_SCHEMA_V1, AGENT_GRAPH_PIN_SCHEMA_V1,
    GcPlan, GcReceipt,
};
use crate::{
    AGENT_GRAPH_RECEIPT_SCHEMA_V1, ActiveAssertion, ActiveChallenge, AgentFactDraft,
    AgentGraphError, AgentGraphErrorCode, AssertionDraft, AssertionId, AssertionKey,
    AssertionSelector, BaseGenerationId, BaseGenerationView, ChallengeEffect, ChallengeId,
    ChallengeSelector, ChangeBatch, ChangeOperation, CommitReceipt, CompositionProfile, Digest,
    EffectiveGraph, GroundedEffect, GroundingPolicy, IdempotencyKey, IdempotencyRecord,
    InMemoryBaseGeneration, IngestionPreparation, IngestionPreparationRequest, NodeRef,
    OperationPermission, OverlayId, OverlayRevision, OverlayRevisionId, OverlayState, PinId,
    PrincipalId, QuiescentGcGrant, RebaseCommitRequest, RebasePlan, RepositoryId,
    ResolvedAgentEdge, ResolvedAgentFact, ResolvedNodeRef, Retraction, RevisionPin, WriteGrant,
    canonical_bytes, canonical_digest, compose_effective, ground_assertion, ground_challenge,
    prepare_ingestion,
};

const NAMESPACE: &[u8] = b"compass.agent-graph.v1";
const OBJECT_PARTITION: &[u8] = b"objects";
const REVISION_PARTITION: &[u8] = b"revisions";
const HEAD_PARTITION: &[u8] = b"heads";
const PIN_PARTITION: &[u8] = b"pins";
const AUDIT_PARTITION: &[u8] = b"audit";
const LEAF_PREFIX: u8 = 1;
const BRANCH_PREFIX: u8 = 2;
const LEAF_BYTES: usize = 128 * 1024;
const BRANCH_FANOUT: usize = 1_000;
const MAX_TREE_DEPTH: usize = 8;
const MAX_TREE_OBJECTS: usize = 100_000;
const MAX_STATE_BYTES: usize = 2 * 1024 * 1024 * 1024;
const OVERLAY_REVISION_SCHEMA_V1: &str = "compass.agent-graph.revision/1";
const HEAD_SCHEMA_V1: &str = "compass.agent-graph.head/1";

type ValueScan = (Vec<(Vec<u8>, Vec<u8>)>, usize, usize, bool);
type KeyScan = (Vec<Vec<u8>>, usize, usize, bool);

pub trait BaseGenerationProvider: Send + Sync {
    fn open(
        &self,
        identity: &BaseGenerationId,
    ) -> Result<Arc<dyn BaseGenerationView>, AgentGraphError>;
}

#[derive(Clone, Default)]
pub struct InMemoryBaseGenerationProvider {
    generations: BTreeMap<BaseGenerationId, Arc<InMemoryBaseGeneration>>,
}

impl InMemoryBaseGenerationProvider {
    #[must_use]
    pub fn with_generation(mut self, generation: InMemoryBaseGeneration) -> Self {
        self.generations
            .insert(generation.identity().clone(), Arc::new(generation));
        self
    }
}

impl BaseGenerationProvider for InMemoryBaseGenerationProvider {
    fn open(
        &self,
        identity: &BaseGenerationId,
    ) -> Result<Arc<dyn BaseGenerationView>, AgentGraphError> {
        self.generations
            .get(identity)
            .cloned()
            .map(|generation| generation as Arc<dyn BaseGenerationView>)
            .ok_or_else(|| {
                AgentGraphError::new(
                    AgentGraphErrorCode::UnknownBaseGeneration,
                    "the exact Base Generation is unavailable",
                )
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadRequest {
    Overlay {
        overlay: OverlayId,
        revision: Option<OverlayRevisionId>,
    },
    EffectiveGraph {
        overlay: OverlayId,
        revision: OverlayRevisionId,
        profile: CompositionProfile,
    },
    PrepareIngestion {
        overlay: OverlayId,
        base_generation: BaseGenerationId,
        request: IngestionPreparationRequest,
    },
    History {
        overlay: OverlayId,
        limit: usize,
    },
    Diff {
        overlay: OverlayId,
        old: OverlayRevisionId,
        new: OverlayRevisionId,
    },
    PrepareRebase {
        overlay: OverlayId,
        source_revision: OverlayRevisionId,
        target_base_generation: BaseGenerationId,
    },
    Audit {
        revision: OverlayRevisionId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReadResult {
    Overlay {
        revision: OverlayRevisionId,
        manifest: OverlayRevision,
        state: OverlayState,
    },
    EffectiveGraph(EffectiveGraph),
    IngestionPreparation(IngestionPreparation),
    History(HistoryResult),
    Diff(DiffResult),
    RebasePlan(RebasePlan),
    Audit(crate::AuditResult),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryResult {
    pub schema: String,
    pub overlay: OverlayId,
    pub revisions: Vec<OverlayRevisionId>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiffResult {
    pub schema: String,
    pub overlay: OverlayId,
    pub old: OverlayRevisionId,
    pub new: OverlayRevisionId,
    pub added_assertions: Vec<AssertionId>,
    pub removed_assertions: Vec<AssertionId>,
    pub changed_assertions: Vec<AssertionId>,
    pub added_challenges: Vec<ChallengeId>,
    pub removed_challenges: Vec<ChallengeId>,
    pub changed_challenges: Vec<ChallengeId>,
}

pub trait AgentGraphOverlay: Send + Sync {
    fn read(&self, request: ReadRequest) -> Result<ReadResult, AgentGraphError>;
    fn apply(
        &self,
        grant: &WriteGrant,
        batch: ChangeBatch,
    ) -> Result<CommitReceipt, AgentGraphError>;
    fn commit_rebase(
        &self,
        grant: &WriteGrant,
        request: RebaseCommitRequest,
    ) -> Result<CommitReceipt, AgentGraphError>;
}

pub struct OverlayRepository<S, P> {
    store: S,
    provider: P,
    repository: RepositoryId,
}

/// Grounds source/Base evidence through the selected generation while resolving a
/// Prior Assertion only from this repository's exact immutable revision. Providers
/// must not guess overlay state or fabricate prior claims.
struct OverlayAwareBase<'a, S, P> {
    repository: &'a OverlayRepository<S, P>,
    base: &'a dyn BaseGenerationView,
}

impl<S, P> BaseGenerationView for OverlayAwareBase<'_, S, P>
where
    S: Store + Send + Sync,
    P: BaseGenerationProvider,
{
    fn identity(&self) -> &BaseGenerationId {
        self.base.identity()
    }

    fn graph(&self) -> &compass_model::code_graph::GraphDocument {
        self.base.graph()
    }

    fn source_bytes(&self, repository_path: &str) -> Result<Option<Vec<u8>>, AgentGraphError> {
        self.base.source_bytes(repository_path)
    }

    fn artifact(&self, artifact: &str) -> Result<Option<crate::ArtifactRecord>, AgentGraphError> {
        self.base.artifact(artifact)
    }

    fn prior_assertion(
        &self,
        revision: &OverlayRevisionId,
        assertion: &AssertionId,
    ) -> Result<Option<crate::PriorAssertionRecord>, AgentGraphError> {
        let (manifest, state) = self.repository.read_revision(revision)?;
        if manifest.base_generation != *self.identity() {
            return Ok(None);
        }
        Ok(state
            .assertions
            .get(assertion)
            .map(|record| crate::PriorAssertionRecord {
                digest: record.assertion_digest.clone(),
            }))
    }
}

impl<S, P> OverlayRepository<S, P>
where
    S: Store + Send + Sync,
    P: BaseGenerationProvider,
{
    #[must_use]
    pub const fn new(store: S, provider: P, repository: RepositoryId) -> Self {
        Self {
            store,
            provider,
            repository,
        }
    }

    pub fn active_revision(
        &self,
        overlay: &OverlayId,
    ) -> Result<Option<OverlayRevisionId>, AgentGraphError> {
        Ok(self.read_head(overlay)?.map(|(head, _)| head.revision))
    }

    /// Pin an exact immutable revision so maintenance reachability retains it and its ancestry.
    pub fn pin_revision(
        &self,
        pin: PinId,
        overlay: OverlayId,
        revision: OverlayRevisionId,
    ) -> Result<RevisionPin, AgentGraphError> {
        let (manifest, _) = self.read_revision(&revision)?;
        if manifest.overlay != overlay {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnknownOverlay,
                "pinned revision does not belong to the requested overlay",
            ));
        }
        let record = RevisionPin {
            schema: AGENT_GRAPH_PIN_SCHEMA_V1.to_owned(),
            pin: pin.clone(),
            overlay,
            revision,
        };
        let bytes = canonical_bytes(&record)?;
        match self.store.put(
            &namespace()?,
            &partition(PIN_PARTITION)?,
            &key(pin.as_str().as_bytes())?,
            &bytes,
            WriteCondition::Missing,
        ) {
            Ok(_) => Ok(record),
            Err(StoreError::Conflict) => {
                let existing = self
                    .store
                    .get(
                        &namespace()?,
                        &partition(PIN_PARTITION)?,
                        &key(pin.as_str().as_bytes())?,
                    )
                    .map_err(storage)?
                    .ok_or_else(|| {
                        AgentGraphError::new(
                            AgentGraphErrorCode::PublicationConflict,
                            "pin conflicted and then disappeared",
                        )
                    })?;
                let existing = serde_json::from_slice::<RevisionPin>(&existing.value)
                    .map_err(|error| corrupt_error(format!("revision pin is corrupt: {error}")))?;
                if existing == record {
                    Ok(existing)
                } else {
                    Err(AgentGraphError::new(
                        AgentGraphErrorCode::RevisionConflict,
                        "pin ID is already bound to another revision",
                    ))
                }
            }
            Err(error) => Err(storage(error)),
        }
    }

    pub fn unpin_revision(&self, pin: &PinId) -> Result<bool, AgentGraphError> {
        self.store
            .delete(
                &namespace()?,
                &partition(PIN_PARTITION)?,
                &key(pin.as_str().as_bytes())?,
                WriteCondition::Any,
            )
            .map_err(storage)
    }

    /// Build a deterministic, bounded reachability plan without deleting data.
    pub fn plan_gc(
        &self,
        max_keys: usize,
        max_key_bytes: usize,
    ) -> Result<GcPlan, AgentGraphError> {
        if max_keys == 0
            || max_keys > MAX_TREE_OBJECTS
            || max_key_bytes == 0
            || max_key_bytes > 64 * 1024 * 1024
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                "GC limits must be within 1..=100000 keys and 1..=67108864 key bytes",
            ));
        }
        let (heads, head_keys, head_key_bytes, heads_truncated) =
            self.scan_values(HEAD_PARTITION, max_keys, max_key_bytes)?;
        let remaining_keys = max_keys.saturating_sub(head_keys);
        let remaining_bytes = max_key_bytes.saturating_sub(head_key_bytes);
        let (pins, pin_keys, pin_key_bytes, pins_truncated) =
            if remaining_keys == 0 || remaining_bytes == 0 {
                (Vec::new(), 0, 0, true)
            } else {
                self.scan_values(PIN_PARTITION, remaining_keys, remaining_bytes)?
            };
        let mut head_records = Vec::new();
        let mut revision_roots = Vec::new();
        let mut tree_roots = Vec::new();
        for (_, value) in heads {
            let head = serde_json::from_slice::<ActiveHead>(&value)
                .map_err(|error| corrupt_error(format!("active head is corrupt: {error}")))?;
            if head.schema != HEAD_SCHEMA_V1 {
                return corrupt("active head has an unsupported schema");
            }
            revision_roots.push(head.revision.clone());
            tree_roots.push(head.idempotency_root.clone());
            head_records.push(head);
        }
        let mut pin_records = Vec::new();
        for (_, value) in pins {
            let pin = serde_json::from_slice::<RevisionPin>(&value)
                .map_err(|error| corrupt_error(format!("revision pin is corrupt: {error}")))?;
            if pin.schema != AGENT_GRAPH_PIN_SCHEMA_V1 {
                return corrupt("revision pin has an unsupported schema");
            }
            revision_roots.push(pin.revision.clone());
            pin_records.push(pin);
        }
        head_records.sort_by(|left, right| left.overlay.cmp(&right.overlay));
        pin_records.sort_by(|left, right| left.pin.cmp(&right.pin));
        revision_roots.sort();
        revision_roots.dedup();
        tree_roots.sort();
        tree_roots.dedup();
        let reachability_digest = canonical_digest(
            "compass.agent-graph.gc-reachability/1",
            &(&head_records, &pin_records),
        )?;
        let mut reachable_revisions = BTreeSet::new();
        let mut cursor = revision_roots;
        while let Some(revision) = cursor.pop() {
            if !reachable_revisions.insert(revision.clone()) {
                continue;
            }
            if reachable_revisions.len() > max_keys {
                return Ok(truncated_gc_plan(
                    self.repository.clone(),
                    reachability_digest,
                    head_keys.saturating_add(pin_keys),
                    head_key_bytes.saturating_add(pin_key_bytes),
                ));
            }
            let (manifest, _) = self.read_revision(&revision)?;
            tree_roots.push(manifest.state_root);
            if let Some(parent) = manifest.parent_revision {
                cursor.push(parent);
            }
        }
        let mut reachable_objects = BTreeSet::new();
        for root in tree_roots {
            self.collect_tree_addresses(&root, 0, max_keys, &mut reachable_objects)?;
        }
        let used = head_keys.saturating_add(pin_keys);
        let used_bytes = head_key_bytes.saturating_add(pin_key_bytes);
        let remaining_keys = max_keys.saturating_sub(used);
        let remaining_bytes = max_key_bytes.saturating_sub(used_bytes);
        if remaining_keys == 0 || remaining_bytes == 0 || heads_truncated || pins_truncated {
            return Ok(truncated_gc_plan(
                self.repository.clone(),
                reachability_digest,
                used,
                used_bytes,
            ));
        }
        let (revision_keys, revision_count, revision_bytes, revision_truncated) =
            self.scan_keys(REVISION_PARTITION, remaining_keys, remaining_bytes)?;
        let used = used.saturating_add(revision_count);
        let used_bytes = used_bytes.saturating_add(revision_bytes);
        let remaining_keys = max_keys.saturating_sub(used);
        let remaining_bytes = max_key_bytes.saturating_sub(used_bytes);
        if remaining_keys == 0 || remaining_bytes == 0 || revision_truncated {
            return Ok(truncated_gc_plan(
                self.repository.clone(),
                reachability_digest,
                used,
                used_bytes,
            ));
        }
        let (object_keys, object_count, object_bytes, object_truncated) =
            self.scan_keys(OBJECT_PARTITION, remaining_keys, remaining_bytes)?;
        let scanned_keys = used.saturating_add(object_count);
        let scanned_key_bytes = used_bytes.saturating_add(object_bytes);
        if object_truncated {
            return Ok(truncated_gc_plan(
                self.repository.clone(),
                reachability_digest,
                scanned_keys,
                scanned_key_bytes,
            ));
        }
        let mut unreachable_revisions = Vec::new();
        for bytes in revision_keys {
            let value = std::str::from_utf8(&bytes)
                .map_err(|_| corrupt_error("revision key is not UTF-8"))?;
            let revision = OverlayRevisionId(Digest::parse(value.to_owned())?);
            if !reachable_revisions.contains(&revision) {
                unreachable_revisions.push(revision);
            }
        }
        let mut unreachable_objects = Vec::new();
        for bytes in object_keys {
            let value = std::str::from_utf8(&bytes)
                .map_err(|_| corrupt_error("object key is not UTF-8"))?;
            let digest = Digest::parse(value.to_owned())?;
            if !reachable_objects.contains(&digest) {
                unreachable_objects.push(digest);
            }
        }
        unreachable_revisions.sort();
        unreachable_objects.sort();
        Ok(GcPlan {
            schema: AGENT_GRAPH_GC_PLAN_SCHEMA_V1.to_owned(),
            repository: self.repository.clone(),
            reachability_digest,
            unreachable_revisions,
            unreachable_objects,
            scanned_keys: scanned_keys as u64,
            scanned_key_bytes: scanned_key_bytes as u64,
            truncated: false,
        })
    }

    /// Sweep exactly one previously computed plan while the repository is quiescent.
    pub fn sweep_gc(
        &self,
        grant: &QuiescentGcGrant,
        plan: &GcPlan,
    ) -> Result<GcReceipt, AgentGraphError> {
        grant.authorize(&self.repository)?;
        if plan.schema != AGENT_GRAPH_GC_PLAN_SCHEMA_V1
            || plan.repository != self.repository
            || plan.truncated
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::InvalidInput,
                "GC sweep requires a complete plan for this repository",
            ));
        }
        let fresh = self.plan_gc(
            usize::try_from(plan.scanned_keys)
                .unwrap_or(MAX_TREE_OBJECTS)
                .clamp(1, MAX_TREE_OBJECTS),
            usize::try_from(plan.scanned_key_bytes)
                .unwrap_or(64 * 1024 * 1024)
                .clamp(1, 64 * 1024 * 1024),
        )?;
        if fresh.truncated
            || fresh.reachability_digest != plan.reachability_digest
            || fresh.unreachable_revisions != plan.unreachable_revisions
            || fresh.unreachable_objects != plan.unreachable_objects
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::RevisionConflict,
                "repository reachability changed after the GC plan was created",
            ));
        }
        let revision_keys = plan
            .unreachable_revisions
            .iter()
            .map(|revision| key(revision.as_digest().as_str().as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        let object_keys = plan
            .unreachable_objects
            .iter()
            .map(|digest| key(digest.as_str().as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        let deleted_revisions = self.delete_key_batches(REVISION_PARTITION, &revision_keys)?;
        let mut deleted_audits = 0_u64;
        for revision in &plan.unreachable_revisions {
            deleted_audits =
                deleted_audits.saturating_add(self.delete_audits_for_revision(revision)?);
        }
        let deleted_objects = self.delete_key_batches(OBJECT_PARTITION, &object_keys)?;
        Ok(GcReceipt {
            schema: AGENT_GRAPH_GC_RECEIPT_SCHEMA_V1.to_owned(),
            plan_digest: plan.digest()?,
            deleted_revisions,
            deleted_objects,
            deleted_audits,
        })
    }

    fn delete_key_batches(
        &self,
        partition_name: &[u8],
        keys: &[Key],
    ) -> Result<u64, AgentGraphError> {
        let mut deleted = 0_u64;
        for batch in keys.chunks(1_000) {
            deleted = deleted.saturating_add(
                self.store
                    .delete_batch(&namespace()?, &partition(partition_name)?, batch)
                    .map_err(storage)?,
            );
        }
        Ok(deleted)
    }

    fn scan_values(
        &self,
        partition_name: &[u8],
        max_items: usize,
        max_bytes: usize,
    ) -> Result<ValueScan, AgentGraphError> {
        let mut values = Vec::new();
        let mut cursor: Option<ScanCursor> = None;
        let mut key_bytes = 0_usize;
        loop {
            let page = self
                .store
                .scan(
                    &namespace()?,
                    &partition(partition_name)?,
                    &KeyRange::default(),
                    ScanLimits {
                        max_items: 1_000.min(max_items.saturating_sub(values.len()).max(1)),
                        max_bytes: 1024 * 1024,
                    },
                    cursor.as_ref(),
                )
                .map_err(storage)?;
            for entry in page.entries {
                if values.len() == max_items
                    || key_bytes.saturating_add(entry.key.len()) > max_bytes
                {
                    return Ok((values, max_items, key_bytes, true));
                }
                key_bytes = key_bytes.saturating_add(entry.key.len());
                values.push((entry.key, entry.value));
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => {
                    let count = values.len();
                    return Ok((values, count, key_bytes, false));
                }
            }
        }
    }

    fn scan_keys(
        &self,
        partition_name: &[u8],
        max_items: usize,
        max_bytes: usize,
    ) -> Result<KeyScan, AgentGraphError> {
        let mut keys = Vec::new();
        let mut cursor: Option<ScanCursor> = None;
        let mut key_bytes = 0_usize;
        loop {
            let page = self
                .store
                .scan_keys(
                    &namespace()?,
                    &partition(partition_name)?,
                    &KeyRange::default(),
                    ScanLimits {
                        max_items: 1_000.min(max_items.saturating_sub(keys.len()).max(1)),
                        max_bytes: (1024 * 1024).min(max_bytes.saturating_sub(key_bytes).max(1)),
                    },
                    cursor.as_ref(),
                )
                .map_err(storage)?;
            for address in page.keys {
                if keys.len() == max_items || key_bytes.saturating_add(address.len()) > max_bytes {
                    return Ok((keys, max_items, key_bytes, true));
                }
                key_bytes = key_bytes.saturating_add(address.len());
                keys.push(address);
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => {
                    let count = keys.len();
                    return Ok((keys, count, key_bytes, false));
                }
            }
        }
    }

    fn collect_tree_addresses(
        &self,
        digest: &Digest,
        depth: usize,
        maximum: usize,
        output: &mut BTreeSet<Digest>,
    ) -> Result<(), AgentGraphError> {
        if depth > MAX_TREE_DEPTH || output.len() >= maximum {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                "reachable object tree exceeds GC bounds",
            ));
        }
        if !output.insert(digest.clone()) {
            return Ok(());
        }
        let entry = self
            .store
            .get(
                &namespace()?,
                &partition(OBJECT_PARTITION)?,
                &key(digest.as_str().as_bytes())?,
            )
            .map_err(storage)?
            .ok_or_else(|| corrupt_error("reachable object tree references a missing object"))?;
        if Digest::raw_bytes(&entry.value) != *digest {
            return corrupt("reachable object content digest does not match its address");
        }
        let Some((prefix, body)) = entry.value.split_first() else {
            return corrupt("reachable object is empty");
        };
        match *prefix {
            LEAF_PREFIX => Ok(()),
            BRANCH_PREFIX => {
                let branch = serde_json::from_slice::<TreeBranch>(body).map_err(|error| {
                    corrupt_error(format!("reachable object branch is corrupt: {error}"))
                })?;
                if branch.schema != "compass.agent-graph.object-tree/1"
                    || branch.children.is_empty()
                    || branch.children.len() > BRANCH_FANOUT
                {
                    return corrupt("reachable object branch has invalid schema or fanout");
                }
                for child in branch.children {
                    self.collect_tree_addresses(
                        &child.digest,
                        depth.saturating_add(1),
                        maximum,
                        output,
                    )?;
                }
                Ok(())
            }
            _ => corrupt("reachable object has an unknown kind"),
        }
    }

    fn apply_inner(
        &self,
        grant: &WriteGrant,
        batch: ChangeBatch,
    ) -> Result<CommitReceipt, AgentGraphError> {
        batch.validate(grant.limits())?;
        grant.authorize_scope(
            &self.repository,
            &batch.overlay,
            &batch.base_generation,
            &batch.expected_revision,
        )?;
        authorize_operations(grant, &batch.operations)?;
        let base = self.provider.open(&batch.base_generation)?;
        let grounding_base = OverlayAwareBase {
            repository: self,
            base: base.as_ref(),
        };
        let batch_digest = canonical_digest("compass.agent-graph.batch/1", &batch)?;
        let observed_head = self.read_head(&batch.overlay)?;
        let (parent_state, parent_revision, observed_token, idempotency) =
            match observed_head.as_ref() {
                Some((head, token)) => {
                    let (manifest, state) = self.read_revision(&head.revision)?;
                    if manifest.overlay != batch.overlay {
                        return corrupt("active head names a revision from another overlay");
                    }
                    let records = self.read_idempotency(&head.idempotency_root)?;
                    (state, Some(head.revision.clone()), Some(*token), records)
                }
                None => (
                    OverlayState::empty(batch.overlay.clone(), batch.base_generation.clone()),
                    None,
                    None,
                    BTreeMap::new(),
                ),
            };
        let idempotency_address = idempotency_address(grant.principal(), &batch.idempotency_key);
        if let Some(record) = idempotency.get(&idempotency_address) {
            if record.batch_digest != batch_digest {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::IdempotencyConflict,
                    "idempotency key was already used for different content",
                ));
            }
            let (manifest, _) = self.read_revision(&record.revision)?;
            return Ok(receipt_for(
                record.revision.clone(),
                &manifest,
                record.batch_digest.clone(),
                true,
            ));
        }
        if parent_revision != batch.expected_revision {
            return Err(AgentGraphError {
                observed_revision: parent_revision,
                ..AgentGraphError::new(
                    AgentGraphErrorCode::RevisionConflict,
                    "expected overlay revision does not match the active head",
                )
            });
        }
        if parent_state.base_generation != batch.base_generation {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::RebaseRequired,
                "active overlay belongs to a different Base Generation",
            ));
        }

        let mut next_state = apply_operations(
            parent_state,
            &batch,
            grant,
            &grounding_base,
            &self.repository,
        )?;
        next_state.sequence = next_state.sequence.saturating_add(1);
        next_state.parent_revision = parent_revision.clone();

        self.publish_state(
            observed_token,
            idempotency,
            idempotency_address,
            batch_digest,
            next_state,
            parent_revision,
            &grounding_base,
            grant,
        )
    }

    fn commit_rebase_inner(
        &self,
        grant: &WriteGrant,
        request: RebaseCommitRequest,
    ) -> Result<CommitReceipt, AgentGraphError> {
        request.validate(grant.limits())?;
        grant.authorize_scope(
            &self.repository,
            &request.plan.overlay,
            &request.plan.target_base_generation,
            &Some(request.plan.source_revision.clone()),
        )?;
        grant.authorize_operation(OperationPermission::CommitRebase)?;
        authorize_operations(grant, &request.resolution_operations)?;
        let target = self.provider.open(&request.plan.target_base_generation)?;
        let grounding_target = OverlayAwareBase {
            repository: self,
            base: target.as_ref(),
        };
        let request_digest = canonical_digest("compass.agent-graph.rebase-commit/1", &request)?;
        let Some((head, token)) = self.read_head(&request.plan.overlay)? else {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnknownOverlay,
                "rebase source overlay has no active revision",
            ));
        };
        let idempotency = self.read_idempotency(&head.idempotency_root)?;
        let idempotency_address = idempotency_address(grant.principal(), &request.idempotency_key);
        if let Some(record) = idempotency.get(&idempotency_address) {
            if record.batch_digest != request_digest {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::IdempotencyConflict,
                    "idempotency key was already used for different rebase content",
                ));
            }
            let (manifest, _) = self.read_revision(&record.revision)?;
            return Ok(receipt_for(
                record.revision.clone(),
                &manifest,
                record.batch_digest.clone(),
                true,
            ));
        }
        if head.revision != request.plan.source_revision {
            return Err(AgentGraphError {
                observed_revision: Some(head.revision),
                ..AgentGraphError::new(
                    AgentGraphErrorCode::RebasePlanStale,
                    "active overlay head changed after rebase planning",
                )
            });
        }
        let (source_manifest, source_state) = self.read_revision(&head.revision)?;
        if source_manifest.overlay != request.plan.overlay
            || source_manifest.base_generation != request.plan.source_base_generation
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::RebasePlanStale,
                "rebase source identity no longer matches its plan",
            ));
        }
        let fresh_plan = crate::rebase::prepare_rebase(
            &source_state,
            &head.revision,
            &grounding_target,
            grant.limits(),
        )?;
        if fresh_plan != request.plan {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::RebasePlanStale,
                "rebase dependencies changed after the plan was created",
            ));
        }
        let (rebased_state, unresolved) = crate::rebase::materialize_rebase(
            &source_state,
            &fresh_plan,
            &grounding_target,
            grant.limits(),
        )?;
        let resolution_batch = ChangeBatch {
            schema: crate::AGENT_GRAPH_BATCH_SCHEMA_V1.to_owned(),
            overlay: request.plan.overlay.clone(),
            base_generation: request.plan.target_base_generation.clone(),
            expected_revision: Some(request.plan.source_revision.clone()),
            idempotency_key: request.idempotency_key,
            operations: request.resolution_operations,
        };
        if !resolution_batch.operations.is_empty() {
            resolution_batch.validate(grant.limits())?;
        }
        let mut next_state = apply_operations(
            rebased_state,
            &resolution_batch,
            grant,
            &grounding_target,
            &self.repository,
        )?;
        ensure_rebase_resolved(&source_state, &next_state, &unresolved)?;
        next_state.sequence = source_state.sequence.saturating_add(1);
        next_state.parent_revision = Some(request.plan.source_revision.clone());
        self.publish_state(
            Some(token),
            idempotency,
            idempotency_address,
            request_digest,
            next_state,
            Some(request.plan.source_revision),
            &grounding_target,
            grant,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_state(
        &self,
        observed_token: Option<VersionToken>,
        mut idempotency: BTreeMap<String, IdempotencyRecord>,
        idempotency_address: String,
        batch_digest: Digest,
        next_state: OverlayState,
        parent_revision: Option<OverlayRevisionId>,
        base: &dyn BaseGenerationView,
        grant: &WriteGrant,
    ) -> Result<CommitReceipt, AgentGraphError> {
        let state_bytes = canonical_bytes(&next_state)?;
        if state_bytes.len() > MAX_STATE_BYTES {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                format!(
                    "overlay state is {} bytes; maximum is {MAX_STATE_BYTES}",
                    state_bytes.len()
                ),
            ));
        }
        let state_digest = Digest::raw_bytes(&state_bytes);
        let state_root = self.write_tree(&state_bytes)?;
        let mutation_digest = canonical_digest(
            "compass.agent-graph.mutation/1",
            &(&parent_revision, &state_digest),
        )?;
        let manifest = OverlayRevision {
            schema: OVERLAY_REVISION_SCHEMA_V1.to_owned(),
            overlay: next_state.overlay.clone(),
            base_generation: next_state.base_generation.clone(),
            parent_revision: parent_revision.clone(),
            sequence: next_state.sequence,
            state_root,
            state_digest,
            state_bytes: state_bytes.len() as u64,
            active_assertions: next_state.assertions.len() as u64,
            active_challenges: next_state.challenges.len() as u64,
            retractions: next_state.retractions.len() as u64,
            challenge_retractions: next_state.challenge_retractions.len() as u64,
            mutation_digest,
            composition_version: COMPOSITION_VERSION_V1.to_owned(),
        };
        let revision = OverlayRevisionId(canonical_digest(
            "compass.agent-graph.overlay-revision/1",
            &manifest,
        )?);
        self.publish_manifest(&revision, &manifest)?;
        self.publish_audit(&revision, &next_state.overlay, grant, batch_digest.clone())?;
        let receipt = receipt_for(revision.clone(), &manifest, batch_digest.clone(), false);
        idempotency.insert(
            idempotency_address.clone(),
            IdempotencyRecord {
                batch_digest: batch_digest.clone(),
                revision: revision.clone(),
            },
        );
        let idempotency_root = self.write_tree(&canonical_bytes(&idempotency)?)?;
        let head = ActiveHead {
            schema: HEAD_SCHEMA_V1.to_owned(),
            overlay: next_state.overlay.clone(),
            revision: revision.clone(),
            idempotency_root,
        };
        let head_bytes = canonical_bytes(&head)?;
        let condition = observed_token.map_or(WriteCondition::Missing, WriteCondition::Version);
        match self.store.put(
            &namespace()?,
            &partition(HEAD_PARTITION)?,
            &key(next_state.overlay.as_str().as_bytes())?,
            &head_bytes,
            condition,
        ) {
            Ok(_) => {}
            Err(StoreError::Conflict) => {
                let observed_revision = self.active_revision(&next_state.overlay)?;
                return Err(AgentGraphError {
                    observed_revision,
                    ..AgentGraphError::new(
                        AgentGraphErrorCode::RevisionConflict,
                        "another writer activated a revision first",
                    )
                });
            }
            Err(error) => return Err(storage(error)),
        }
        let (reopened_manifest, reopened_state) = self.read_revision(&revision)?;
        self.validate_reopened(&reopened_manifest, &reopened_state, base)?;
        let Some((reopened_head, _)) = self.read_head(&next_state.overlay)? else {
            return corrupt("active head disappeared after publication");
        };
        if reopened_head.revision != revision {
            let reopened_idempotency = self.read_idempotency(&reopened_head.idempotency_root)?;
            if reopened_idempotency.get(&idempotency_address)
                != Some(&IdempotencyRecord {
                    batch_digest,
                    revision,
                })
            {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::PublicationConflict,
                    "active head changed without retaining the committed idempotency record",
                ));
            }
        }
        Ok(receipt)
    }

    fn read_inner(&self, request: ReadRequest) -> Result<ReadResult, AgentGraphError> {
        match request {
            ReadRequest::Overlay { overlay, revision } => {
                let revision = match revision {
                    Some(revision) => revision,
                    None => self.active_revision(&overlay)?.ok_or_else(|| {
                        AgentGraphError::new(
                            AgentGraphErrorCode::UnknownOverlay,
                            "overlay has no active revision",
                        )
                    })?,
                };
                let (manifest, state) = self.read_revision(&revision)?;
                if manifest.overlay != overlay {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::UnknownOverlay,
                        "revision does not belong to the requested overlay",
                    ));
                }
                let base = self.provider.open(&manifest.base_generation)?;
                let grounding_base = OverlayAwareBase {
                    repository: self,
                    base: base.as_ref(),
                };
                self.validate_reopened(&manifest, &state, &grounding_base)?;
                Ok(ReadResult::Overlay {
                    revision,
                    manifest,
                    state,
                })
            }
            ReadRequest::EffectiveGraph {
                overlay,
                revision,
                profile,
            } => {
                let (manifest, state) = self.read_revision(&revision)?;
                if manifest.overlay != overlay {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::UnknownOverlay,
                        "revision does not belong to the requested overlay",
                    ));
                }
                let base = self.provider.open(&manifest.base_generation)?;
                let grounding_base = OverlayAwareBase {
                    repository: self,
                    base: base.as_ref(),
                };
                self.validate_reopened(&manifest, &state, &grounding_base)?;
                Ok(ReadResult::EffectiveGraph(compose_effective(
                    base.graph(),
                    &manifest.base_generation,
                    revision,
                    &state,
                    profile,
                    crate::AgentGraphLimits::default(),
                )?))
            }
            ReadRequest::PrepareIngestion {
                overlay,
                base_generation,
                request,
            } => {
                let base = self.provider.open(&base_generation)?;
                let grounding_base = OverlayAwareBase {
                    repository: self,
                    base: base.as_ref(),
                };
                let expected_revision = match self.active_revision(&overlay)? {
                    Some(revision) => {
                        let (manifest, state) = self.read_revision(&revision)?;
                        if manifest.overlay != overlay {
                            return corrupt(
                                "active ingestion overlay revision belongs to another overlay",
                            );
                        }
                        if manifest.base_generation != base_generation {
                            return Err(AgentGraphError::new(
                                AgentGraphErrorCode::RebaseRequired,
                                "active overlay belongs to a different Base Generation",
                            ));
                        }
                        self.validate_reopened(&manifest, &state, &grounding_base)?;
                        Some(revision)
                    }
                    None => None,
                };
                Ok(ReadResult::IngestionPreparation(prepare_ingestion(
                    &grounding_base,
                    overlay,
                    expected_revision,
                    &request,
                    crate::AgentGraphLimits::default(),
                )?))
            }
            ReadRequest::History { overlay, limit } => {
                if limit == 0 || limit > 1_000 {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::LimitExceeded,
                        "history limit must be between 1 and 1000",
                    ));
                }
                let mut cursor = self.active_revision(&overlay)?;
                let mut revisions = Vec::new();
                while let Some(revision) = cursor {
                    if revisions.len() == limit {
                        return Ok(ReadResult::History(HistoryResult {
                            schema: "compass.agent-graph.history/1".to_owned(),
                            overlay,
                            revisions,
                            truncated: true,
                        }));
                    }
                    let (manifest, _) = self.read_revision(&revision)?;
                    cursor = manifest.parent_revision.clone();
                    revisions.push(revision);
                }
                Ok(ReadResult::History(HistoryResult {
                    schema: "compass.agent-graph.history/1".to_owned(),
                    overlay,
                    revisions,
                    truncated: false,
                }))
            }
            ReadRequest::Diff { overlay, old, new } => {
                let (old_manifest, old_state) = self.read_revision(&old)?;
                let (new_manifest, new_state) = self.read_revision(&new)?;
                if old_manifest.overlay != overlay || new_manifest.overlay != overlay {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::UnknownOverlay,
                        "diff revisions do not belong to the requested overlay",
                    ));
                }
                if old_manifest.base_generation != new_manifest.base_generation {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::RebaseRequired,
                        "overlay revisions use different Base Generations",
                    ));
                }
                Ok(ReadResult::Diff(diff_states(
                    overlay, old, new, &old_state, &new_state,
                )))
            }
            ReadRequest::PrepareRebase {
                overlay,
                source_revision,
                target_base_generation,
            } => {
                let (manifest, state) = self.read_revision(&source_revision)?;
                if manifest.overlay != overlay {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::UnknownOverlay,
                        "rebase source revision does not belong to the requested overlay",
                    ));
                }
                let target = self.provider.open(&target_base_generation)?;
                let grounding_target = OverlayAwareBase {
                    repository: self,
                    base: target.as_ref(),
                };
                Ok(ReadResult::RebasePlan(crate::rebase::prepare_rebase(
                    &state,
                    &source_revision,
                    &grounding_target,
                    crate::AgentGraphLimits::default(),
                )?))
            }
            ReadRequest::Audit { revision } => {
                let (manifest, _) = self.read_revision(&revision)?;
                let prefix = format!("{}:", revision.as_digest().as_str());
                let end = format!("{};", revision.as_digest().as_str());
                let page = self
                    .store
                    .scan(
                        &namespace()?,
                        &partition(AUDIT_PARTITION)?,
                        &KeyRange {
                            start_inclusive: Some(prefix.into_bytes()),
                            end_exclusive: Some(end.into_bytes()),
                        },
                        ScanLimits {
                            max_items: 64,
                            max_bytes: 1024 * 1024,
                        },
                        None,
                    )
                    .map_err(storage)?;
                let truncated = page.next.is_some();
                let mut records = Vec::with_capacity(page.entries.len());
                for entry in page.entries {
                    let record = serde_json::from_slice::<crate::AuditRecord>(&entry.value)
                        .map_err(|error| {
                            corrupt_error(format!("audit record is corrupt: {error}"))
                        })?;
                    record.validate().map_err(|error| {
                        corrupt_error(format!("audit record failed validation: {error}"))
                    })?;
                    if record.repository != self.repository
                        || record.overlay != manifest.overlay
                        || record.revision != revision
                    {
                        return corrupt(
                            "audit record identity or scope does not match its revision",
                        );
                    }
                    records.push(record);
                }
                if records.is_empty() {
                    return corrupt("overlay revision is missing its audit record");
                }
                Ok(ReadResult::Audit(crate::AuditResult {
                    schema: crate::AGENT_GRAPH_AUDIT_RESULT_SCHEMA_V1.to_owned(),
                    revision,
                    records,
                    truncated,
                }))
            }
        }
    }

    fn publish_audit(
        &self,
        revision: &OverlayRevisionId,
        overlay: &OverlayId,
        grant: &WriteGrant,
        mutation_digest: Digest,
    ) -> Result<(), AgentGraphError> {
        let record = grant.attestation().record(
            self.repository.clone(),
            overlay.clone(),
            grant.principal().clone(),
            revision.clone(),
            mutation_digest,
        );
        record.validate()?;
        let bytes = canonical_bytes(&record)?;
        if bytes.len() > grant.limits().max_audit_bytes {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                format!(
                    "operational audit record is {} bytes; maximum is {}",
                    bytes.len(),
                    grant.limits().max_audit_bytes
                ),
            ));
        }
        let record_digest = canonical_digest("compass.agent-graph.audit-record/1", &record)?;
        let address = format!(
            "{}:{}",
            revision.as_digest().as_str(),
            record_digest.as_str()
        );
        self.store
            .put_immutable(
                &namespace()?,
                &partition(AUDIT_PARTITION)?,
                &key(address.as_bytes())?,
                &bytes,
            )
            .map(|_| ())
            .map_err(storage)
    }

    fn delete_audits_for_revision(
        &self,
        revision: &OverlayRevisionId,
    ) -> Result<u64, AgentGraphError> {
        let prefix = format!("{}:", revision.as_digest().as_str());
        let end = format!("{};", revision.as_digest().as_str());
        let mut cursor = None;
        let mut keys = Vec::new();
        loop {
            let page = self
                .store
                .scan_keys(
                    &namespace()?,
                    &partition(AUDIT_PARTITION)?,
                    &KeyRange {
                        start_inclusive: Some(prefix.as_bytes().to_vec()),
                        end_exclusive: Some(end.as_bytes().to_vec()),
                    },
                    ScanLimits {
                        max_items: 1_000,
                        max_bytes: 1024 * 1024,
                    },
                    cursor.as_ref(),
                )
                .map_err(storage)?;
            for address in page.keys {
                keys.push(Key::new(address).map_err(storage)?);
                if keys.len() > MAX_TREE_OBJECTS {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::LimitExceeded,
                        "audit entries for one revision exceed the maintenance bound",
                    ));
                }
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        self.delete_key_batches(AUDIT_PARTITION, &keys)
    }

    fn read_head(
        &self,
        overlay: &OverlayId,
    ) -> Result<Option<(ActiveHead, VersionToken)>, AgentGraphError> {
        let entry = self
            .store
            .get(
                &namespace()?,
                &partition(HEAD_PARTITION)?,
                &key(overlay.as_str().as_bytes())?,
            )
            .map_err(storage)?;
        entry
            .map(|entry| {
                let head = serde_json::from_slice::<ActiveHead>(&entry.value)
                    .map_err(|error| corrupt_error(format!("active head is corrupt: {error}")))?;
                if head.schema != HEAD_SCHEMA_V1 || &head.overlay != overlay {
                    return corrupt("active head schema or overlay does not match");
                }
                Ok((head, entry.version))
            })
            .transpose()
    }

    fn publish_manifest(
        &self,
        revision: &OverlayRevisionId,
        manifest: &OverlayRevision,
    ) -> Result<(), AgentGraphError> {
        let bytes = canonical_bytes(manifest)?;
        self.store
            .put_immutable(
                &namespace()?,
                &partition(REVISION_PARTITION)?,
                &key(revision.as_digest().as_str().as_bytes())?,
                &bytes,
            )
            .map(|_| ())
            .map_err(storage)
    }

    fn read_revision(
        &self,
        revision: &OverlayRevisionId,
    ) -> Result<(OverlayRevision, OverlayState), AgentGraphError> {
        let entry = self
            .store
            .get(
                &namespace()?,
                &partition(REVISION_PARTITION)?,
                &key(revision.as_digest().as_str().as_bytes())?,
            )
            .map_err(storage)?
            .ok_or_else(|| {
                AgentGraphError::new(
                    AgentGraphErrorCode::UnknownOverlay,
                    "overlay revision does not exist",
                )
            })?;
        let manifest = serde_json::from_slice::<OverlayRevision>(&entry.value)
            .map_err(|error| corrupt_error(format!("revision manifest is corrupt: {error}")))?;
        if manifest.schema != OVERLAY_REVISION_SCHEMA_V1
            || OverlayRevisionId(canonical_digest(
                "compass.agent-graph.overlay-revision/1",
                &manifest,
            )?) != *revision
        {
            return corrupt("revision manifest identity does not match its address");
        }
        let state_bytes = self.read_tree(&manifest.state_root, MAX_STATE_BYTES)?;
        if state_bytes.len() as u64 != manifest.state_bytes
            || Digest::raw_bytes(&state_bytes) != manifest.state_digest
        {
            return corrupt("overlay state size or digest does not match its manifest");
        }
        let state = serde_json::from_slice::<OverlayState>(&state_bytes)
            .map_err(|error| corrupt_error(format!("overlay state is corrupt: {error}")))?;
        validate_manifest_counts(&manifest, &state)?;
        Ok((manifest, state))
    }

    fn validate_reopened(
        &self,
        manifest: &OverlayRevision,
        state: &OverlayState,
        base: &dyn BaseGenerationView,
    ) -> Result<(), AgentGraphError> {
        if manifest.overlay != state.overlay
            || manifest.base_generation != state.base_generation
            || manifest.sequence != state.sequence
            || manifest.parent_revision != state.parent_revision
            || base.identity() != &state.base_generation
        {
            return corrupt("revision manifest and materialized state disagree");
        }
        for assertion in state.assertions.values() {
            let draft = draft_from_active(assertion)?;
            let grounded = ground_assertion(
                &draft,
                base,
                GroundingPolicy::allowing_masks(),
                crate::AgentGraphLimits::default(),
            )?;
            if grounded.assertion_digest != assertion.assertion_digest
                || grounded.certificate_digest != assertion.certificate_digest
                || grounded.certificate != assertion.certificate
            {
                return corrupt("stored assertion Grounding certificate does not reverify");
            }
        }
        for challenge in state.challenges.values() {
            let grounded = ground_challenge(
                &challenge.target,
                &challenge.summary,
                &challenge.grounding,
                base,
                if challenge.effect == ChallengeEffect::Mask {
                    GroundingPolicy::allowing_masks()
                } else {
                    GroundingPolicy::default()
                },
                crate::AgentGraphLimits::default(),
            )?;
            if grounded.certificate_digest != challenge.certificate_digest
                || grounded.certificate != challenge.certificate
            {
                return corrupt("stored Challenge Grounding certificate does not reverify");
            }
        }
        compose_effective(
            base.graph(),
            &state.base_generation,
            OverlayRevisionId(Digest::raw_bytes(b"reopen-validation")),
            state,
            CompositionProfile::Augment,
            crate::AgentGraphLimits::default(),
        )?;
        Ok(())
    }

    fn write_tree(&self, bytes: &[u8]) -> Result<Digest, AgentGraphError> {
        let chunks = if bytes.is_empty() {
            vec![&[][..]]
        } else {
            bytes.chunks(LEAF_BYTES).collect::<Vec<_>>()
        };
        let mut level = Vec::with_capacity(chunks.len());
        let mut objects = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let mut value = Vec::with_capacity(chunk.len().saturating_add(1));
            value.push(LEAF_PREFIX);
            value.extend_from_slice(chunk);
            let digest = Digest::raw_bytes(&value);
            level.push(TreeChild {
                digest: digest.clone(),
                bytes: chunk.len() as u64,
            });
            objects.push((digest, value));
        }
        self.publish_objects(&objects)?;
        while level.len() > 1 {
            let mut next = Vec::with_capacity(level.len().div_ceil(BRANCH_FANOUT));
            let mut branches = Vec::new();
            for children in level.chunks(BRANCH_FANOUT) {
                let branch = TreeBranch {
                    schema: "compass.agent-graph.object-tree/1".to_owned(),
                    children: children.to_vec(),
                };
                let encoded = canonical_bytes(&branch)?;
                let mut value = Vec::with_capacity(encoded.len().saturating_add(1));
                value.push(BRANCH_PREFIX);
                value.extend_from_slice(&encoded);
                let digest = Digest::raw_bytes(&value);
                let byte_count = children
                    .iter()
                    .fold(0_u64, |total, child| total.saturating_add(child.bytes));
                next.push(TreeChild {
                    digest: digest.clone(),
                    bytes: byte_count,
                });
                branches.push((digest, value));
            }
            self.publish_objects(&branches)?;
            level = next;
        }
        level
            .into_iter()
            .next()
            .map(|child| child.digest)
            .ok_or_else(|| corrupt_error("object tree omitted its root"))
    }

    fn publish_objects(&self, objects: &[(Digest, Vec<u8>)]) -> Result<(), AgentGraphError> {
        let mut batch = Vec::new();
        let mut batch_bytes = 0_usize;
        for (digest, value) in objects {
            if !batch.is_empty()
                && (batch.len() == MAX_IMMUTABLE_BATCH_ITEMS
                    || batch_bytes.saturating_add(value.len()) > MAX_IMMUTABLE_BATCH_BYTES)
            {
                self.store
                    .put_immutable_batch(&namespace()?, &batch)
                    .map_err(storage)?;
                batch.clear();
                batch_bytes = 0;
            }
            batch.push(
                ImmutableWrite::new(
                    partition(OBJECT_PARTITION)?,
                    key(digest.as_str().as_bytes())?,
                    value.clone(),
                )
                .map_err(storage)?,
            );
            batch_bytes = batch_bytes.saturating_add(value.len());
        }
        if !batch.is_empty() {
            self.store
                .put_immutable_batch(&namespace()?, &batch)
                .map_err(storage)?;
        }
        Ok(())
    }

    fn read_tree(&self, root: &Digest, maximum: usize) -> Result<Vec<u8>, AgentGraphError> {
        let mut output = Vec::new();
        let mut objects = 0_usize;
        self.read_tree_object(root, 0, maximum, &mut objects, &mut output)?;
        Ok(output)
    }

    fn read_tree_object(
        &self,
        digest: &Digest,
        depth: usize,
        maximum: usize,
        objects: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), AgentGraphError> {
        if depth > MAX_TREE_DEPTH || *objects >= MAX_TREE_OBJECTS {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                "object tree exceeds depth or object-count limits",
            ));
        }
        *objects = objects.saturating_add(1);
        let entry = self
            .store
            .get(
                &namespace()?,
                &partition(OBJECT_PARTITION)?,
                &key(digest.as_str().as_bytes())?,
            )
            .map_err(storage)?
            .ok_or_else(|| corrupt_error("object tree references a missing object"))?;
        if Digest::raw_bytes(&entry.value) != *digest {
            return corrupt("object tree content digest does not match its address");
        }
        let Some((prefix, body)) = entry.value.split_first() else {
            return corrupt("object tree contains an empty object");
        };
        match *prefix {
            LEAF_PREFIX => {
                if output.len().saturating_add(body.len()) > maximum {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::LimitExceeded,
                        "materialized object tree exceeds its byte limit",
                    ));
                }
                output.extend_from_slice(body);
            }
            BRANCH_PREFIX => {
                let branch = serde_json::from_slice::<TreeBranch>(body).map_err(|error| {
                    corrupt_error(format!("object tree branch is corrupt: {error}"))
                })?;
                if branch.schema != "compass.agent-graph.object-tree/1"
                    || branch.children.is_empty()
                    || branch.children.len() > BRANCH_FANOUT
                {
                    return corrupt("object tree branch schema or fanout is invalid");
                }
                for child in &branch.children {
                    self.read_tree_object(
                        child.as_ref_digest(),
                        depth + 1,
                        maximum,
                        objects,
                        output,
                    )?;
                }
                let actual = branch
                    .children
                    .iter()
                    .fold(0_u64, |total, child| total.saturating_add(child.bytes));
                if actual > maximum as u64 {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::LimitExceeded,
                        "object tree branch declares more bytes than permitted",
                    ));
                }
            }
            _ => return corrupt("object tree contains an unknown object kind"),
        }
        Ok(())
    }

    fn read_idempotency(
        &self,
        root: &Digest,
    ) -> Result<BTreeMap<String, IdempotencyRecord>, AgentGraphError> {
        let bytes = self.read_tree(root, MAX_STATE_BYTES)?;
        serde_json::from_slice(&bytes)
            .map_err(|error| corrupt_error(format!("idempotency index is corrupt: {error}")))
    }
}

impl<P> OverlayRepository<compass_store::SqliteStore, P>
where
    P: BaseGenerationProvider,
{
    pub fn open_local(
        paths: &crate::AgentGraphPaths,
        provider: P,
        repository: RepositoryId,
    ) -> Result<Self, AgentGraphError> {
        let store = compass_store::SqliteStore::open(paths.database()).map_err(storage)?;
        paths.secure_database_permissions()?;
        let capabilities = store.capabilities();
        if !capabilities.strong_point_reads
            || !capabilities.ordered_partition_scans
            || !capabilities.conditional_single_key_writes
            || !capabilities.durable_acknowledgements
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::StorageFailure,
                "local agent graph store lacks required durable read, scan, or CAS semantics",
            ));
        }
        Ok(Self::new(store, provider, repository))
    }
}

impl<S, P> AgentGraphOverlay for OverlayRepository<S, P>
where
    S: Store + Send + Sync,
    P: BaseGenerationProvider,
{
    fn read(&self, request: ReadRequest) -> Result<ReadResult, AgentGraphError> {
        self.read_inner(request)
    }

    fn apply(
        &self,
        grant: &WriteGrant,
        batch: ChangeBatch,
    ) -> Result<CommitReceipt, AgentGraphError> {
        self.apply_inner(grant, batch)
    }

    fn commit_rebase(
        &self,
        grant: &WriteGrant,
        request: RebaseCommitRequest,
    ) -> Result<CommitReceipt, AgentGraphError> {
        self.commit_rebase_inner(grant, request)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActiveHead {
    schema: String,
    overlay: OverlayId,
    revision: OverlayRevisionId,
    idempotency_root: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TreeChild {
    digest: Digest,
    bytes: u64,
}

impl TreeChild {
    fn as_ref_digest(&self) -> &Digest {
        &self.digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TreeBranch {
    schema: String,
    children: Vec<TreeChild>,
}

fn authorize_operations(
    grant: &WriteGrant,
    operations: &[ChangeOperation],
) -> Result<(), AgentGraphError> {
    for operation in operations {
        let permission = match operation {
            ChangeOperation::PutAssertion { .. } => OperationPermission::PutAssertion,
            ChangeOperation::RetractAssertion { .. } => OperationPermission::RetractAssertion,
            ChangeOperation::PutChallenge { .. } => OperationPermission::PutChallenge,
            ChangeOperation::RetractChallenge { .. } => OperationPermission::RetractChallenge,
        };
        grant.authorize_operation(permission)?;
    }
    Ok(())
}

fn apply_operations(
    mut state: OverlayState,
    batch: &ChangeBatch,
    grant: &WriteGrant,
    base: &dyn BaseGenerationView,
    repository: &RepositoryId,
) -> Result<OverlayState, AgentGraphError> {
    let sequence = state.sequence.saturating_add(1);
    let mut puts = BTreeMap::<String, &AssertionDraft>::new();
    let mut retracts = BTreeMap::new();
    let mut challenge_puts = BTreeMap::new();
    let mut challenge_retracts = BTreeMap::new();
    for operation in &batch.operations {
        match operation {
            ChangeOperation::PutAssertion { assertion } => {
                let target = assertion_target(&assertion.selector);
                if puts.insert(target.clone(), assertion).is_some()
                    || retracts.contains_key(&target)
                {
                    return duplicate(&target);
                }
            }
            ChangeOperation::RetractAssertion {
                assertion,
                expected_assertion_digest,
                reason_code,
                explanation,
            } => {
                let target = assertion.as_str().to_owned();
                if retracts
                    .insert(
                        target.clone(),
                        (
                            assertion,
                            expected_assertion_digest,
                            reason_code,
                            explanation,
                        ),
                    )
                    .is_some()
                    || puts.contains_key(&target)
                {
                    return duplicate(&target);
                }
            }
            ChangeOperation::PutChallenge { challenge } => {
                let id = challenge_id(&challenge.selector);
                let target = id.as_str().to_owned();
                if challenge_puts.insert(target.clone(), challenge).is_some()
                    || challenge_retracts.contains_key(&target)
                {
                    return duplicate(&target);
                }
            }
            ChangeOperation::RetractChallenge {
                challenge,
                expected_challenge_digest,
                reason_code,
                explanation,
            } => {
                let target = challenge.as_str().to_owned();
                if challenge_retracts
                    .insert(
                        target.clone(),
                        (
                            challenge,
                            expected_challenge_digest,
                            reason_code,
                            explanation,
                        ),
                    )
                    .is_some()
                    || challenge_puts.contains_key(&target)
                {
                    return duplicate(&target);
                }
            }
        }
    }

    for (_, (id, expected, reason, explanation)) in retracts {
        let Some(existing) = state.assertions.remove(id) else {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::AssertionNotFound,
                format!("assertion {} is not active", id.as_str()),
            ));
        };
        enforce_owner(&existing.owner, grant.principal())?;
        if &existing.assertion_digest != expected {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::AssertionDigestConflict,
                format!("assertion {} digest is stale", id.as_str()),
            ));
        }
        state.retractions.insert(
            id.clone(),
            Retraction {
                assertion: id.clone(),
                key: existing.key,
                owner: existing.owner,
                retracted_digest: existing.assertion_digest,
                reason_code: reason.clone(),
                explanation: explanation.clone(),
                sequence,
            },
        );
    }

    for (_, (id, expected, reason, explanation)) in challenge_retracts {
        let Some(existing) = state.challenges.remove(id) else {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::AssertionNotFound,
                format!("challenge {} is not active", id.as_str()),
            ));
        };
        enforce_owner(&existing.owner, grant.principal())?;
        if &existing.challenge_digest != expected {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::AssertionDigestConflict,
                format!("challenge {} digest is stale", id.as_str()),
            ));
        }
        state.challenge_retractions.insert(
            id.clone(),
            crate::overlay::ChallengeRetraction {
                challenge: id.clone(),
                owner: existing.owner,
                retracted_digest: existing.challenge_digest,
                reason_code: reason.clone(),
                explanation: explanation.clone(),
                sequence,
            },
        );
    }

    let historical_keys = state
        .assertions
        .values()
        .map(|assertion| assertion.key.clone())
        .chain(state.retractions.values().map(|entry| entry.key.clone()))
        .collect::<BTreeSet<_>>();
    let mut created = BTreeMap::<AssertionKey, (AssertionId, &'static str)>::new();
    for draft in puts.values() {
        if let AssertionSelector::New { key } = &draft.selector {
            if historical_keys.contains(key) || created.contains_key(key) {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::InvalidTransition,
                    format!("assertion key {} was already used", key.as_str()),
                ));
            }
            let id = derive_assertion_id(
                repository,
                &batch.overlay,
                grant.principal(),
                draft.fact.class(),
                key,
            )?;
            created.insert(key.clone(), (id, draft.fact.class()));
        }
    }
    let prospective_nodes = prospective_node_ids(&state, &puts, &created)?;
    for draft in puts.values() {
        let (id, key, version) = match &draft.selector {
            AssertionSelector::New { key } => {
                let Some((id, _)) = created.get(key) else {
                    return corrupt("new assertion ID was not derived");
                };
                (id.clone(), key.clone(), 1)
            }
            AssertionSelector::Existing {
                id,
                expected_assertion_digest,
            } => {
                let Some(existing) = state.assertions.get(id) else {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::AssertionNotFound,
                        format!("assertion {} is not active", id.as_str()),
                    ));
                };
                enforce_owner(&existing.owner, grant.principal())?;
                if &existing.assertion_digest != expected_assertion_digest {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::AssertionDigestConflict,
                        format!("assertion {} digest is stale", id.as_str()),
                    ));
                }
                if existing.fact.class() != draft.fact.class() {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::InvalidTransition,
                        "an assertion replacement cannot change node/edge class",
                    ));
                }
                (
                    id.clone(),
                    existing.key.clone(),
                    existing.version.saturating_add(1),
                )
            }
        };
        let fact = resolve_fact(&draft.fact, &created, &prospective_nodes, base)?;
        let normalized_draft = draft_with_resolved_fact(draft, &fact);
        let grounded = ground_assertion(
            &normalized_draft,
            base,
            if grant.mask_permitted() {
                GroundingPolicy::allowing_masks()
            } else {
                GroundingPolicy::default()
            },
            grant.limits(),
        )?;
        state.assertions.insert(
            id.clone(),
            ActiveAssertion {
                id,
                key,
                owner: grant.principal().clone(),
                version,
                assertion_digest: grounded.assertion_digest,
                certificate_digest: grounded.certificate_digest,
                certificate: grounded.certificate,
                fact,
                summary: grounded.summary,
                grounding: grounded.grounding,
                projection_provenance: grounded.projection_provenance,
            },
        );
    }

    for challenge in challenge_puts.values() {
        if challenge.effect == ChallengeEffect::Mask && !grant.mask_permitted() {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::MaskNotPermitted,
                "write grant does not permit curated masks",
            ));
        }
        let (id, version) = match &challenge.selector {
            ChallengeSelector::New { id } => {
                if state.challenges.contains_key(id) || state.challenge_retractions.contains_key(id)
                {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::InvalidTransition,
                        format!("challenge ID {} was already used", id.as_str()),
                    ));
                }
                (id.clone(), 1)
            }
            ChallengeSelector::Existing {
                id,
                expected_challenge_digest,
            } => {
                let Some(existing) = state.challenges.get(id) else {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::AssertionNotFound,
                        format!("challenge {} is not active", id.as_str()),
                    ));
                };
                enforce_owner(&existing.owner, grant.principal())?;
                if &existing.challenge_digest != expected_challenge_digest {
                    return Err(AgentGraphError::new(
                        AgentGraphErrorCode::AssertionDigestConflict,
                        format!("challenge {} digest is stale", id.as_str()),
                    ));
                }
                (id.clone(), existing.version.saturating_add(1))
            }
        };
        let grounded = ground_challenge(
            &challenge.target,
            &challenge.summary,
            &challenge.grounding,
            base,
            if challenge.effect == ChallengeEffect::Mask {
                GroundingPolicy::allowing_masks()
            } else {
                GroundingPolicy::default()
            },
            grant.limits(),
        )?;
        let required_effect = if challenge.effect == ChallengeEffect::Mask {
            GroundedEffect::MaskBaseFact
        } else {
            GroundedEffect::FlagBaseFact
        };
        if !grounded
            .certificate
            .permitted_effects()
            .contains(&required_effect)
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::GroundingFailed,
                "Grounding certificate does not permit the requested Challenge effect",
            ));
        }
        let challenge_digest = canonical_digest(
            "compass.agent-graph.challenge-version/1",
            &(
                &id,
                grant.principal(),
                &challenge.target,
                challenge.effect,
                &challenge.summary,
                &grounded.certificate_digest,
            ),
        )?;
        state.challenges.insert(
            id.clone(),
            ActiveChallenge {
                id,
                owner: grant.principal().clone(),
                version,
                challenge_digest,
                target: challenge.target.clone(),
                effect: challenge.effect,
                summary: challenge.summary.clone(),
                grounding: grounded.grounding,
                certificate: grounded.certificate,
                certificate_digest: grounded.certificate_digest,
            },
        );
    }
    let node_count = state
        .assertions
        .values()
        .filter(|assertion| matches!(assertion.fact, ResolvedAgentFact::Node(_)))
        .count();
    let edge_count = state.assertions.len().saturating_sub(node_count);
    if node_count > grant.limits().max_agent_nodes || edge_count > grant.limits().max_agent_edges {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::LimitExceeded,
            "post-state exceeds the granted agent node or edge limit",
        ));
    }
    validate_agent_dependents(&state)?;
    compose_effective(
        base.graph(),
        &batch.base_generation,
        OverlayRevisionId(Digest::raw_bytes(b"pre-publication-validation")),
        &state,
        CompositionProfile::Augment,
        grant.limits(),
    )?;
    if state
        .challenges
        .values()
        .any(|challenge| challenge.effect == ChallengeEffect::Mask)
    {
        compose_effective(
            base.graph(),
            &batch.base_generation,
            OverlayRevisionId(Digest::raw_bytes(b"pre-publication-curated-validation")),
            &state,
            CompositionProfile::Curated,
            grant.limits(),
        )?;
    }
    Ok(state)
}

fn prospective_node_ids(
    state: &OverlayState,
    puts: &BTreeMap<String, &AssertionDraft>,
    created: &BTreeMap<AssertionKey, (AssertionId, &'static str)>,
) -> Result<BTreeSet<AssertionId>, AgentGraphError> {
    let mut nodes = state
        .assertions
        .values()
        .filter(|assertion| matches!(assertion.fact, ResolvedAgentFact::Node(_)))
        .map(|assertion| assertion.id.clone())
        .collect::<BTreeSet<_>>();
    for draft in puts.values() {
        match &draft.selector {
            AssertionSelector::New { key } if matches!(draft.fact, AgentFactDraft::Node(_)) => {
                let Some((id, class)) = created.get(key) else {
                    return corrupt("new node assertion was not assigned an ID");
                };
                if *class != "node" {
                    return corrupt("new assertion class does not match its draft");
                }
                nodes.insert(id.clone());
            }
            AssertionSelector::Existing { id, .. } => {
                if matches!(draft.fact, AgentFactDraft::Node(_)) {
                    nodes.insert(id.clone());
                } else {
                    nodes.remove(id);
                }
            }
            AssertionSelector::New { .. } => {}
        }
    }
    Ok(nodes)
}

fn resolve_fact(
    fact: &AgentFactDraft,
    created: &BTreeMap<AssertionKey, (AssertionId, &'static str)>,
    prospective_nodes: &BTreeSet<AssertionId>,
    base: &dyn BaseGenerationView,
) -> Result<ResolvedAgentFact, AgentGraphError> {
    match fact {
        AgentFactDraft::Node(node) => Ok(ResolvedAgentFact::Node(node.clone())),
        AgentFactDraft::Edge(edge) => Ok(ResolvedAgentFact::Edge(ResolvedAgentEdge {
            source: resolve_node_ref(&edge.source, created, prospective_nodes, base)?,
            target: resolve_node_ref(&edge.target, created, prospective_nodes, base)?,
            kind: edge.kind,
            relationship_site: edge.relationship_site.clone(),
            details: edge.details.clone(),
            context: edge.context.clone(),
        })),
    }
}

fn resolve_node_ref(
    reference: &NodeRef,
    created: &BTreeMap<AssertionKey, (AssertionId, &'static str)>,
    prospective_nodes: &BTreeSet<AssertionId>,
    base: &dyn BaseGenerationView,
) -> Result<ResolvedNodeRef, AgentGraphError> {
    match reference {
        NodeRef::Base { node } => {
            verify_base_node(node, base)?;
            Ok(ResolvedNodeRef::Base { node: node.clone() })
        }
        NodeRef::Agent { assertion } => {
            if !prospective_nodes.contains(assertion) {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::MissingEndpoint,
                    format!(
                        "agent endpoint {} is not an active node",
                        assertion.as_str()
                    ),
                ));
            }
            Ok(ResolvedNodeRef::Agent {
                assertion: assertion.clone(),
            })
        }
        NodeRef::CreatedInThisBatch { key } => {
            let Some((id, class)) = created.get(key) else {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::MissingEndpoint,
                    format!("within-batch endpoint {} was not created", key.as_str()),
                ));
            };
            if *class != "node" {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::MissingEndpoint,
                    format!("within-batch endpoint {} is not a node", key.as_str()),
                ));
            }
            Ok(ResolvedNodeRef::Agent {
                assertion: id.clone(),
            })
        }
    }
}

fn verify_base_node(
    reference: &crate::BaseNodeRef,
    base: &dyn BaseGenerationView,
) -> Result<(), AgentGraphError> {
    if &reference.base_generation != base.identity() {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::UnknownBaseGeneration,
            "base endpoint belongs to a different Base Generation",
        ));
    }
    let Some(node) = base
        .graph()
        .nodes
        .iter()
        .find(|node| node.id == reference.id)
    else {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::MissingEndpoint,
            format!("base node {} does not exist", reference.id),
        ));
    };
    let digest = canonical_digest("compass.agent-graph.base-node-record/1", node)?;
    if node.kind != reference.kind || digest != reference.record_digest {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            format!("base node {} kind or record digest is stale", reference.id),
        ));
    }
    Ok(())
}

fn validate_agent_dependents(state: &OverlayState) -> Result<(), AgentGraphError> {
    let nodes = state
        .assertions
        .values()
        .filter(|assertion| matches!(assertion.fact, ResolvedAgentFact::Node(_)))
        .map(|assertion| assertion.id.clone())
        .collect::<BTreeSet<_>>();
    let mut missing = Vec::new();
    for assertion in state.assertions.values() {
        let ResolvedAgentFact::Edge(edge) = &assertion.fact else {
            continue;
        };
        for endpoint in [&edge.source, &edge.target] {
            if let ResolvedNodeRef::Agent {
                assertion: endpoint_assertion,
            } = endpoint
                && !nodes.contains(endpoint_assertion)
            {
                missing.push(assertion.id.as_str().to_owned());
            }
        }
    }
    missing.sort();
    missing.dedup();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(AgentGraphError::new(
            AgentGraphErrorCode::ActiveDependents,
            format!(
                "node Retraction would leave active incident edges: {}",
                missing.join(", ")
            ),
        ))
    }
}

fn draft_from_active(assertion: &ActiveAssertion) -> Result<AssertionDraft, AgentGraphError> {
    let fact = match &assertion.fact {
        ResolvedAgentFact::Node(node) => AgentFactDraft::Node(node.clone()),
        ResolvedAgentFact::Edge(edge) => AgentFactDraft::Edge(crate::AgentEdgeDraft {
            source: unresolved_node_ref(&edge.source),
            target: unresolved_node_ref(&edge.target),
            kind: edge.kind,
            relationship_site: edge.relationship_site.clone(),
            details: edge.details.clone(),
            context: edge.context.clone(),
        }),
    };
    Ok(AssertionDraft {
        selector: AssertionSelector::Existing {
            id: assertion.id.clone(),
            expected_assertion_digest: assertion.assertion_digest.clone(),
        },
        fact,
        grounding: assertion.grounding.clone(),
        summary: assertion.summary.clone(),
    })
}

fn draft_with_resolved_fact(draft: &AssertionDraft, fact: &ResolvedAgentFact) -> AssertionDraft {
    let normalized_fact = match fact {
        ResolvedAgentFact::Node(node) => AgentFactDraft::Node(node.clone()),
        ResolvedAgentFact::Edge(edge) => AgentFactDraft::Edge(crate::AgentEdgeDraft {
            source: unresolved_node_ref(&edge.source),
            target: unresolved_node_ref(&edge.target),
            kind: edge.kind,
            relationship_site: edge.relationship_site.clone(),
            details: edge.details.clone(),
            context: edge.context.clone(),
        }),
    };
    AssertionDraft {
        selector: draft.selector.clone(),
        fact: normalized_fact,
        grounding: draft.grounding.clone(),
        summary: draft.summary.clone(),
    }
}

fn unresolved_node_ref(reference: &ResolvedNodeRef) -> NodeRef {
    match reference {
        ResolvedNodeRef::Base { node } => NodeRef::Base { node: node.clone() },
        ResolvedNodeRef::Agent { assertion } => NodeRef::Agent {
            assertion: assertion.clone(),
        },
    }
}

fn derive_assertion_id(
    repository: &RepositoryId,
    overlay: &OverlayId,
    owner: &PrincipalId,
    class: &str,
    key: &AssertionKey,
) -> Result<AssertionId, AgentGraphError> {
    let digest = canonical_digest(
        "compass.agent-graph.assertion-id/1",
        &(repository, overlay, owner, class, key),
    )?;
    AssertionId::parse(format!("assertion:{}", digest.as_str()))
}

fn assertion_target(selector: &AssertionSelector) -> String {
    match selector {
        AssertionSelector::New { key } => key.as_str().to_owned(),
        AssertionSelector::Existing { id, .. } => id.as_str().to_owned(),
    }
}

fn challenge_id(selector: &ChallengeSelector) -> &ChallengeId {
    match selector {
        ChallengeSelector::New { id } | ChallengeSelector::Existing { id, .. } => id,
    }
}

fn enforce_owner(actual: &PrincipalId, caller: &PrincipalId) -> Result<(), AgentGraphError> {
    if actual == caller {
        Ok(())
    } else {
        Err(AgentGraphError::new(
            AgentGraphErrorCode::OwnershipViolation,
            "principal does not own the targeted assertion",
        ))
    }
}

fn receipt_for(
    revision: OverlayRevisionId,
    manifest: &OverlayRevision,
    batch_digest: Digest,
    idempotent_replay: bool,
) -> CommitReceipt {
    CommitReceipt {
        schema: AGENT_GRAPH_RECEIPT_SCHEMA_V1.to_owned(),
        overlay: manifest.overlay.clone(),
        revision,
        parent_revision: manifest.parent_revision.clone(),
        base_generation: manifest.base_generation.clone(),
        sequence: manifest.sequence,
        batch_digest,
        active_assertions: manifest.active_assertions,
        active_challenges: manifest.active_challenges,
        retractions: manifest.retractions,
        idempotent_replay,
    }
}

fn idempotency_address(principal: &PrincipalId, key: &IdempotencyKey) -> String {
    format!("{}|{}", principal.as_str(), key.as_str())
}

fn validate_manifest_counts(
    manifest: &OverlayRevision,
    state: &OverlayState,
) -> Result<(), AgentGraphError> {
    if manifest.active_assertions != state.assertions.len() as u64
        || manifest.active_challenges != state.challenges.len() as u64
        || manifest.retractions != state.retractions.len() as u64
        || manifest.challenge_retractions != state.challenge_retractions.len() as u64
    {
        return corrupt("revision manifest counts do not match materialized state");
    }
    Ok(())
}

fn diff_states(
    overlay: OverlayId,
    old: OverlayRevisionId,
    new: OverlayRevisionId,
    old_state: &OverlayState,
    new_state: &OverlayState,
) -> DiffResult {
    let (added_assertions, removed_assertions, changed_assertions) =
        diff_map(&old_state.assertions, &new_state.assertions, |value| {
            value.assertion_digest.as_digest()
        });
    let (added_challenges, removed_challenges, changed_challenges) =
        diff_map(&old_state.challenges, &new_state.challenges, |value| {
            &value.challenge_digest
        });
    DiffResult {
        schema: "compass.agent-graph.diff/1".to_owned(),
        overlay,
        old,
        new,
        added_assertions,
        removed_assertions,
        changed_assertions,
        added_challenges,
        removed_challenges,
        changed_challenges,
    }
}

fn ensure_rebase_resolved(
    source: &OverlayState,
    rebased: &OverlayState,
    unresolved: &BTreeSet<String>,
) -> Result<(), AgentGraphError> {
    let mut remaining = Vec::new();
    for address in unresolved {
        let assertion_unchanged = source
            .assertions
            .values()
            .find(|entry| entry.id.as_str() == address)
            .is_some_and(|old| {
                rebased
                    .assertions
                    .get(&old.id)
                    .is_some_and(|new| new.assertion_digest == old.assertion_digest)
            });
        let challenge_unchanged = source
            .challenges
            .values()
            .find(|entry| entry.id.as_str() == address)
            .is_some_and(|old| {
                rebased
                    .challenges
                    .get(&old.id)
                    .is_some_and(|new| new.challenge_digest == old.challenge_digest)
            });
        if assertion_unchanged || challenge_unchanged {
            remaining.push(address.clone());
        }
    }
    if remaining.is_empty() {
        return Ok(());
    }
    remaining.sort();
    let omitted = remaining.len().saturating_sub(100);
    remaining.truncate(100);
    Err(
        AgentGraphError::new(
            AgentGraphErrorCode::RebaseUnresolved,
            "rebase commit must replace with newly GROUNDED content or retract every unresolved subject",
        )
        .with_diagnostic(crate::AgentGraphDiagnostic {
            code: "unresolved_rebase_subjects".to_owned(),
            field: "resolutionOperations".to_owned(),
            message: "one or more source assertions remain bound to stale evidence".to_owned(),
            related_ids: remaining,
            omitted_count: omitted as u64,
        }),
    )
}

fn diff_map<K, V, F>(
    old: &BTreeMap<K, V>,
    new: &BTreeMap<K, V>,
    digest: F,
) -> (Vec<K>, Vec<K>, Vec<K>)
where
    K: Clone + Ord,
    F: Fn(&V) -> &Digest,
{
    let added = new
        .keys()
        .filter(|key| !old.contains_key(*key))
        .cloned()
        .collect();
    let removed = old
        .keys()
        .filter(|key| !new.contains_key(*key))
        .cloned()
        .collect();
    let changed = new
        .iter()
        .filter_map(|(key, new_value)| {
            old.get(key)
                .filter(|old_value| digest(old_value) != digest(new_value))
                .map(|_| key.clone())
        })
        .collect();
    (added, removed, changed)
}

fn duplicate<T>(target: &str) -> Result<T, AgentGraphError> {
    Err(AgentGraphError::new(
        AgentGraphErrorCode::DuplicateOperation,
        format!("batch targets {target} more than once"),
    ))
}

fn truncated_gc_plan(
    repository: RepositoryId,
    reachability_digest: Digest,
    scanned_keys: usize,
    scanned_key_bytes: usize,
) -> GcPlan {
    GcPlan {
        schema: AGENT_GRAPH_GC_PLAN_SCHEMA_V1.to_owned(),
        repository,
        reachability_digest,
        // A truncated scan must never expose a partial deletion set.
        unreachable_revisions: Vec::new(),
        unreachable_objects: Vec::new(),
        scanned_keys: scanned_keys as u64,
        scanned_key_bytes: scanned_key_bytes as u64,
        truncated: true,
    }
}

fn namespace() -> Result<NamespaceId, AgentGraphError> {
    NamespaceId::new(NAMESPACE).map_err(storage)
}

fn partition(value: &[u8]) -> Result<PartitionKey, AgentGraphError> {
    PartitionKey::new(value).map_err(storage)
}

fn key(value: &[u8]) -> Result<Key, AgentGraphError> {
    Key::new(value).map_err(storage)
}

fn storage(error: StoreError) -> AgentGraphError {
    AgentGraphError::new(
        AgentGraphErrorCode::StorageFailure,
        format!("agent graph storage operation failed: {error}"),
    )
}

fn corrupt<T>(message: impl Into<String>) -> Result<T, AgentGraphError> {
    Err(corrupt_error(message))
}

fn corrupt_error(message: impl Into<String>) -> AgentGraphError {
    AgentGraphError::new(AgentGraphErrorCode::CorruptOverlay, message)
}
