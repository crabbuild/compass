use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    ActiveAssertion, ActiveChallenge, AgentEdgeDraft, AgentFactDraft, AgentGraphError,
    AgentGraphErrorCode, AgentGraphLimits, AssertionDraft, AssertionId, AssertionSelector,
    BaseEdgeRef, BaseFactRef, BaseGenerationId, BaseGenerationView, BaseNodeRef, ChallengeEffect,
    ChallengeId, ChangeOperation, Digest, GroundedEffect, GroundingEvidence, GroundingPolicy,
    GroundingSubmission, IdempotencyKey, NodeRef, OverlayId, OverlayRevisionId, OverlayState,
    ResolvedAgentEdge, ResolvedAgentFact, ResolvedNodeRef, canonical_digest, ground_assertion,
    ground_challenge,
};

pub const AGENT_GRAPH_REBASE_PLAN_SCHEMA_V1: &str = "compass.agent-graph.rebase-plan/1";
pub const AGENT_GRAPH_REBASE_COMMIT_SCHEMA_V1: &str = "compass.agent-graph.rebase-commit/1";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "subjectType", rename_all = "snake_case", deny_unknown_fields)]
pub enum RebaseSubject {
    Assertion { id: AssertionId },
    Challenge { id: ChallengeId },
}

impl RebaseSubject {
    #[must_use]
    pub fn address(&self) -> &str {
        match self {
            Self::Assertion { id } => id.as_str(),
            Self::Challenge { id } => id.as_str(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebaseDisposition {
    RetainedExact,
    RequiresGroundedReplacementOrRetraction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RebaseItem {
    pub subject: RebaseSubject,
    pub disposition: RebaseDisposition,
    pub reason_codes: Vec<String>,
    pub exact_candidates: Vec<String>,
    pub omitted_candidates: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RebasePlan {
    pub schema: String,
    pub overlay: OverlayId,
    pub source_revision: OverlayRevisionId,
    pub source_base_generation: BaseGenerationId,
    pub target_base_generation: BaseGenerationId,
    pub items: Vec<RebaseItem>,
    pub unresolved_count: u64,
    pub ambiguous_count: u64,
    pub plan_digest: Digest,
}

impl RebasePlan {
    pub(crate) fn new(
        overlay: OverlayId,
        source_revision: OverlayRevisionId,
        source_base_generation: BaseGenerationId,
        target_base_generation: BaseGenerationId,
        mut items: Vec<RebaseItem>,
    ) -> Result<Self, AgentGraphError> {
        items.sort_by(|left, right| left.subject.cmp(&right.subject));
        let unresolved_count = items
            .iter()
            .filter(|item| item.disposition != RebaseDisposition::RetainedExact)
            .count() as u64;
        let ambiguous_count = items
            .iter()
            .filter(|item| item.exact_candidates.len() > 1)
            .count() as u64;
        let plan_digest = canonical_digest(
            "compass.agent-graph.rebase-plan-body/1",
            &(
                &overlay,
                &source_revision,
                &source_base_generation,
                &target_base_generation,
                &items,
                unresolved_count,
                ambiguous_count,
            ),
        )?;
        Ok(Self {
            schema: AGENT_GRAPH_REBASE_PLAN_SCHEMA_V1.to_owned(),
            overlay,
            source_revision,
            source_base_generation,
            target_base_generation,
            items,
            unresolved_count,
            ambiguous_count,
            plan_digest,
        })
    }

    pub fn validate(&self) -> Result<(), AgentGraphError> {
        if self.schema != AGENT_GRAPH_REBASE_PLAN_SCHEMA_V1 {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnsupportedSchema,
                "rebase plan has an unsupported schema",
            ));
        }
        let expected = Self::new(
            self.overlay.clone(),
            self.source_revision.clone(),
            self.source_base_generation.clone(),
            self.target_base_generation.clone(),
            self.items.clone(),
        )?;
        if expected.plan_digest != self.plan_digest
            || expected.unresolved_count != self.unresolved_count
            || expected.ambiguous_count != self.ambiguous_count
            || expected.items != self.items
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::RebasePlanStale,
                "rebase plan digest, counts, or canonical item order is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RebaseCommitRequest {
    pub schema: String,
    pub plan: RebasePlan,
    pub idempotency_key: IdempotencyKey,
    pub resolution_operations: Vec<ChangeOperation>,
}

impl RebaseCommitRequest {
    pub fn validate(&self, limits: AgentGraphLimits) -> Result<(), AgentGraphError> {
        if self.schema != AGENT_GRAPH_REBASE_COMMIT_SCHEMA_V1 {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnsupportedSchema,
                "rebase commit has an unsupported schema",
            ));
        }
        self.plan.validate()?;
        if self.resolution_operations.len() > limits.validate()?.max_operations {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                "rebase resolution operation count exceeds the granted maximum",
            ));
        }
        let bytes = crate::canonical_bytes(self)?;
        if bytes.len() > limits.max_batch_bytes {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                "rebase commit exceeds the granted batch size",
            ));
        }
        Ok(())
    }
}

pub(crate) fn prepare_rebase(
    state: &OverlayState,
    source_revision: &OverlayRevisionId,
    target: &dyn BaseGenerationView,
    limits: AgentGraphLimits,
) -> Result<RebasePlan, AgentGraphError> {
    let mut items = Vec::with_capacity(
        state
            .assertions
            .len()
            .saturating_add(state.challenges.len()),
    );
    for assertion in state.assertions.values() {
        let result = rebase_assertion(assertion, target, limits);
        items.push(item(
            RebaseSubject::Assertion {
                id: assertion.id.clone(),
            },
            result.as_ref().err(),
        ));
    }
    for challenge in state.challenges.values() {
        let result = rebase_challenge(challenge, target, limits);
        items.push(item(
            RebaseSubject::Challenge {
                id: challenge.id.clone(),
            },
            result.as_ref().err(),
        ));
    }
    RebasePlan::new(
        state.overlay.clone(),
        source_revision.clone(),
        state.base_generation.clone(),
        target.identity().clone(),
        items,
    )
}

fn item(subject: RebaseSubject, error: Option<&AgentGraphError>) -> RebaseItem {
    let (disposition, reason_codes) = error.map_or_else(
        || (RebaseDisposition::RetainedExact, Vec::new()),
        |error| {
            (
                RebaseDisposition::RequiresGroundedReplacementOrRetraction,
                vec![error.code.as_str().to_owned()],
            )
        },
    );
    RebaseItem {
        subject,
        disposition,
        reason_codes,
        // V1 deliberately performs exact-ID/digest retention only. It does not search by
        // labels, so a miss has zero candidates and can never become a first-match attach.
        exact_candidates: Vec::new(),
        omitted_candidates: 0,
    }
}

pub(crate) fn materialize_rebase(
    source: &OverlayState,
    plan: &RebasePlan,
    target: &dyn BaseGenerationView,
    limits: AgentGraphLimits,
) -> Result<(OverlayState, BTreeSet<String>), AgentGraphError> {
    let mut state = source.clone();
    state.base_generation = target.identity().clone();
    state.parent_revision = Some(plan.source_revision.clone());
    let unresolved = plan
        .items
        .iter()
        .filter(|item| item.disposition != RebaseDisposition::RetainedExact)
        .map(|item| item.subject.address().to_owned())
        .collect::<BTreeSet<_>>();
    for item in &plan.items {
        if item.disposition != RebaseDisposition::RetainedExact {
            continue;
        }
        match &item.subject {
            RebaseSubject::Assertion { id } => {
                let source = source
                    .assertions
                    .get(id)
                    .ok_or_else(|| stale("rebase assertion disappeared"))?;
                state
                    .assertions
                    .insert(id.clone(), rebase_assertion(source, target, limits)?);
            }
            RebaseSubject::Challenge { id } => {
                let source = source
                    .challenges
                    .get(id)
                    .ok_or_else(|| stale("rebase Challenge disappeared"))?;
                state
                    .challenges
                    .insert(id.clone(), rebase_challenge(source, target, limits)?);
            }
        }
    }
    Ok((state, unresolved))
}

fn rebase_assertion(
    assertion: &ActiveAssertion,
    target: &dyn BaseGenerationView,
    limits: AgentGraphLimits,
) -> Result<ActiveAssertion, AgentGraphError> {
    let fact = rebind_resolved_fact(&assertion.fact, target)?;
    let grounding = rebind_grounding(&assertion.grounding, target)?;
    let fact_draft = draft_fact(&fact);
    let draft = AssertionDraft {
        selector: AssertionSelector::Existing {
            id: assertion.id.clone(),
            expected_assertion_digest: assertion.assertion_digest.clone(),
        },
        fact: fact_draft,
        grounding,
        summary: assertion.summary.clone(),
    };
    let grounded = ground_assertion(&draft, target, GroundingPolicy::allowing_masks(), limits)?;
    Ok(ActiveAssertion {
        id: assertion.id.clone(),
        key: assertion.key.clone(),
        owner: assertion.owner.clone(),
        version: assertion.version.saturating_add(1),
        assertion_digest: grounded.assertion_digest,
        certificate_digest: grounded.certificate_digest,
        certificate: grounded.certificate,
        fact,
        summary: grounded.summary,
        grounding: grounded.grounding,
        projection_provenance: grounded.projection_provenance,
    })
}

fn rebase_challenge(
    challenge: &ActiveChallenge,
    target: &dyn BaseGenerationView,
    limits: AgentGraphLimits,
) -> Result<ActiveChallenge, AgentGraphError> {
    let target_ref = rebind_base_fact(&challenge.target, target)?;
    let grounding = rebind_grounding(&challenge.grounding, target)?;
    let grounded = ground_challenge(
        &target_ref,
        &challenge.summary,
        &grounding,
        target,
        if challenge.effect == ChallengeEffect::Mask {
            GroundingPolicy::allowing_masks()
        } else {
            GroundingPolicy::default()
        },
        limits,
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
            "regrounded Challenge certificate does not permit its effect",
        ));
    }
    let challenge_digest = canonical_digest(
        "compass.agent-graph.challenge-version/1",
        &(
            &challenge.id,
            &challenge.owner,
            &target_ref,
            challenge.effect,
            &challenge.summary,
            &grounded.certificate_digest,
        ),
    )?;
    Ok(ActiveChallenge {
        id: challenge.id.clone(),
        owner: challenge.owner.clone(),
        version: challenge.version.saturating_add(1),
        challenge_digest,
        target: target_ref,
        effect: challenge.effect,
        summary: challenge.summary.clone(),
        grounding: grounded.grounding,
        certificate: grounded.certificate,
        certificate_digest: grounded.certificate_digest,
    })
}

fn draft_fact(fact: &ResolvedAgentFact) -> AgentFactDraft {
    match fact {
        ResolvedAgentFact::Node(node) => AgentFactDraft::Node(node.clone()),
        ResolvedAgentFact::Edge(edge) => AgentFactDraft::Edge(AgentEdgeDraft {
            source: draft_node_ref(&edge.source),
            target: draft_node_ref(&edge.target),
            kind: edge.kind,
            relationship_site: edge.relationship_site.clone(),
            details: edge.details.clone(),
            context: edge.context.clone(),
        }),
    }
}

fn draft_node_ref(reference: &ResolvedNodeRef) -> NodeRef {
    match reference {
        ResolvedNodeRef::Base { node } => NodeRef::Base { node: node.clone() },
        ResolvedNodeRef::Agent { assertion } => NodeRef::Agent {
            assertion: assertion.clone(),
        },
    }
}

fn rebind_resolved_fact(
    fact: &ResolvedAgentFact,
    target: &dyn BaseGenerationView,
) -> Result<ResolvedAgentFact, AgentGraphError> {
    match fact {
        ResolvedAgentFact::Node(node) => Ok(ResolvedAgentFact::Node(node.clone())),
        ResolvedAgentFact::Edge(edge) => Ok(ResolvedAgentFact::Edge(ResolvedAgentEdge {
            source: rebind_resolved_node(&edge.source, target)?,
            target: rebind_resolved_node(&edge.target, target)?,
            kind: edge.kind,
            relationship_site: edge.relationship_site.clone(),
            details: edge.details.clone(),
            context: edge.context.clone(),
        })),
    }
}

fn rebind_resolved_node(
    reference: &ResolvedNodeRef,
    target: &dyn BaseGenerationView,
) -> Result<ResolvedNodeRef, AgentGraphError> {
    match reference {
        ResolvedNodeRef::Base { node } => Ok(ResolvedNodeRef::Base {
            node: rebind_base_node(node, target)?,
        }),
        ResolvedNodeRef::Agent { assertion } => Ok(ResolvedNodeRef::Agent {
            assertion: assertion.clone(),
        }),
    }
}

fn rebind_grounding(
    submission: &GroundingSubmission,
    target: &dyn BaseGenerationView,
) -> Result<GroundingSubmission, AgentGraphError> {
    let mut evidence = Vec::with_capacity(submission.evidence.len());
    for citation in &submission.evidence {
        evidence.push(match citation {
            GroundingEvidence::SourceSpan {
                file,
                anchor,
                file_digest,
                excerpt_digest,
            } => GroundingEvidence::SourceSpan {
                file: file.clone(),
                anchor: anchor.clone(),
                file_digest: file_digest.clone(),
                excerpt_digest: excerpt_digest.clone(),
            },
            GroundingEvidence::BaseFact {
                fact,
                record_digest,
            } => {
                let fact = rebind_base_fact(fact, target)?;
                GroundingEvidence::BaseFact {
                    fact,
                    record_digest: record_digest.clone(),
                }
            }
            GroundingEvidence::BasePath { nodes, edges, .. } => {
                let nodes = nodes
                    .iter()
                    .map(|node| rebind_base_node(node, target))
                    .collect::<Result<Vec<_>, _>>()?;
                let edges = edges
                    .iter()
                    .map(|edge| rebind_base_edge(edge, target))
                    .collect::<Result<Vec<_>, _>>()?;
                let path_digest =
                    canonical_digest("compass.agent-graph.base-path/1", &(&nodes, &edges))?;
                GroundingEvidence::BasePath {
                    nodes,
                    edges,
                    path_digest,
                }
            }
            GroundingEvidence::PriorAssertion { .. } => {
                return Err(AgentGraphError::new(
                    AgentGraphErrorCode::RebaseUnresolved,
                    "prior-assertion evidence must be submitted and verified again for a rebase",
                ));
            }
            GroundingEvidence::SnapshotArtifact {
                artifact,
                artifact_digest,
                json_pointer,
            } => GroundingEvidence::SnapshotArtifact {
                artifact: artifact.clone(),
                artifact_digest: artifact_digest.clone(),
                json_pointer: json_pointer.clone(),
            },
        });
    }
    Ok(GroundingSubmission {
        schema: submission.schema.clone(),
        policy_id: submission.policy_id.clone(),
        evidence,
    })
}

fn rebind_base_fact(
    fact: &BaseFactRef,
    target: &dyn BaseGenerationView,
) -> Result<BaseFactRef, AgentGraphError> {
    match fact {
        BaseFactRef::Node(node) => rebind_base_node(node, target).map(BaseFactRef::Node),
        BaseFactRef::Edge(edge) => rebind_base_edge(edge, target).map(BaseFactRef::Edge),
    }
}

fn rebind_base_node(
    reference: &BaseNodeRef,
    target: &dyn BaseGenerationView,
) -> Result<BaseNodeRef, AgentGraphError> {
    let node = target
        .graph()
        .nodes
        .iter()
        .find(|node| node.id == reference.id)
        .ok_or_else(|| unresolved("exact base node ID is absent from the target generation"))?;
    let digest = canonical_digest("compass.agent-graph.base-node-record/1", node)?;
    if node.kind != reference.kind || digest != reference.record_digest {
        return Err(unresolved(
            "exact base node kind or record digest changed in the target generation",
        ));
    }
    Ok(BaseNodeRef {
        base_generation: target.identity().clone(),
        id: reference.id.clone(),
        kind: reference.kind,
        record_digest: reference.record_digest.clone(),
    })
}

fn rebind_base_edge(
    reference: &BaseEdgeRef,
    target: &dyn BaseGenerationView,
) -> Result<BaseEdgeRef, AgentGraphError> {
    let edge = target
        .graph()
        .links
        .iter()
        .find(|edge| edge.id == reference.id)
        .ok_or_else(|| unresolved("exact base edge ID is absent from the target generation"))?;
    let digest = canonical_digest("compass.agent-graph.base-edge-record/1", edge)?;
    if edge.kind != reference.kind
        || edge.source != reference.source
        || edge.target != reference.target
        || digest != reference.record_digest
    {
        return Err(unresolved(
            "exact base edge endpoints, kind, or record digest changed in the target generation",
        ));
    }
    Ok(BaseEdgeRef {
        base_generation: target.identity().clone(),
        id: reference.id.clone(),
        kind: reference.kind,
        source: reference.source.clone(),
        target: reference.target.clone(),
        record_digest: reference.record_digest.clone(),
    })
}

fn unresolved(message: &'static str) -> AgentGraphError {
    AgentGraphError::new(AgentGraphErrorCode::RebaseUnresolved, message)
}

fn stale(message: &'static str) -> AgentGraphError {
    AgentGraphError::new(AgentGraphErrorCode::RebasePlanStale, message)
}
