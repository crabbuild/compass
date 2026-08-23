use std::collections::BTreeMap;

use compass_model::code_graph::{EdgeRecord, GraphDocument, NodeRecord};
use compass_model::identity::edge_id;
use compass_model::provenance::OccurrenceRule;
use serde::{Deserialize, Serialize};

use crate::{
    ActiveAssertion, AgentGraphError, AgentGraphErrorCode, AgentGraphLimits, AssertionId,
    BaseFactRef, BaseGenerationId, ChallengeEffect, ChallengeId, Digest,
    GroundingCertificateDigest, OverlayRevisionId, OverlayState, ResolvedAgentFact,
    ResolvedNodeRef, canonical_bytes, canonical_digest,
};

pub const AGENT_GRAPH_EFFECTIVE_SCHEMA_V1: &str = "compass.agent-graph.effective/1";
pub const COMPOSITION_VERSION_V1: &str = "compass.agent-graph.composition/1";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionProfile {
    Augment,
    Curated,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveAgentFact {
    pub assertion: AssertionId,
    pub projected_id: String,
    pub certificate_digest: GroundingCertificateDigest,
    pub owner: crate::PrincipalId,
    pub version: u64,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveChallenge {
    pub challenge: ChallengeId,
    pub target_id: String,
    pub effect: ChallengeEffect,
    pub masked: bool,
    pub certificate_digest: GroundingCertificateDigest,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveRetractionKind {
    Assertion,
    Challenge,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveRetraction {
    pub kind: EffectiveRetractionKind,
    pub id: String,
    pub reason_code: String,
    pub explanation: String,
    pub sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveRetractions {
    pub total: u64,
    pub examples: Vec<EffectiveRetraction>,
    pub omitted_examples: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompositionOmission {
    pub id: String,
    pub kind: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge: Option<ChallengeId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompositionOmissions {
    pub total: u64,
    pub direct: u64,
    pub cascaded: u64,
    pub examples: Vec<CompositionOmission>,
    pub omitted_examples: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveGraph {
    pub schema: String,
    pub base_generation: BaseGenerationId,
    pub overlay_revision: OverlayRevisionId,
    pub composition_profile: CompositionProfile,
    pub composition_version: String,
    pub effective_identity: Digest,
    pub graph: GraphDocument,
    pub agent_facts: Vec<EffectiveAgentFact>,
    pub challenges: Vec<EffectiveChallenge>,
    pub retractions: EffectiveRetractions,
    pub omissions: CompositionOmissions,
}

pub fn compose_effective(
    base: &GraphDocument,
    base_generation: &BaseGenerationId,
    overlay_revision: OverlayRevisionId,
    state: &OverlayState,
    profile: CompositionProfile,
    limits: AgentGraphLimits,
) -> Result<EffectiveGraph, AgentGraphError> {
    let limits = limits.validate()?;
    if &state.base_generation != base_generation
        || base.graph.build.generation_id != base_generation.generation_id
    {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::RebaseRequired,
            "overlay and Base Graph do not name the same exact Base Generation",
        ));
    }
    compass_model::validate_code_graph(base).map_err(|error| {
        AgentGraphError::new(
            AgentGraphErrorCode::UnknownBaseGeneration,
            format!("Base Graph validation failed: {error}"),
        )
    })?;

    let mut masked_nodes = BTreeMap::<String, ChallengeId>::new();
    let mut masked_edges = BTreeMap::<String, ChallengeId>::new();
    let mut challenges = Vec::with_capacity(state.challenges.len());
    for challenge in state.challenges.values() {
        let target_id = base_fact_id(&challenge.target).to_owned();
        let masked =
            profile == CompositionProfile::Curated && challenge.effect == ChallengeEffect::Mask;
        if masked {
            match &challenge.target {
                BaseFactRef::Node(_) => {
                    masked_nodes.insert(target_id.clone(), challenge.id.clone());
                }
                BaseFactRef::Edge(_) => {
                    masked_edges.insert(target_id.clone(), challenge.id.clone());
                }
            }
        }
        challenges.push(EffectiveChallenge {
            challenge: challenge.id.clone(),
            target_id,
            effect: challenge.effect,
            masked,
            certificate_digest: challenge.certificate_digest.clone(),
            summary: challenge.summary.clone(),
        });
    }

    let mut examples = Vec::new();
    let mut direct = 0_u64;
    let mut cascaded = 0_u64;
    let mut nodes = Vec::with_capacity(base.nodes.len().saturating_add(state.assertions.len()));
    for node in &base.nodes {
        if let Some(challenge) = masked_nodes.get(&node.id) {
            direct = direct.saturating_add(1);
            push_omission(
                &mut examples,
                limits.max_diagnostics,
                &node.id,
                "node",
                "challenge_mask",
                Some(challenge.clone()),
            );
        } else {
            nodes.push(node.clone());
        }
    }

    let mut links = Vec::with_capacity(base.links.len().saturating_add(state.assertions.len()));
    for edge in &base.links {
        if let Some(challenge) = masked_edges.get(&edge.id) {
            direct = direct.saturating_add(1);
            push_omission(
                &mut examples,
                limits.max_diagnostics,
                &edge.id,
                "edge",
                "challenge_mask",
                Some(challenge.clone()),
            );
        } else if masked_nodes.contains_key(&edge.source) || masked_nodes.contains_key(&edge.target)
        {
            cascaded = cascaded.saturating_add(1);
            push_omission(
                &mut examples,
                limits.max_diagnostics,
                &edge.id,
                "edge",
                "masked_endpoint",
                None,
            );
        } else {
            links.push(edge.clone());
        }
    }

    let agent_node_ids = state
        .assertions
        .values()
        .filter_map(|assertion| match assertion.fact {
            ResolvedAgentFact::Node(_) => {
                Some((assertion.id.clone(), projected_node_id(&assertion.id)))
            }
            ResolvedAgentFact::Edge(_) => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut agent_facts = Vec::with_capacity(state.assertions.len());
    for assertion in state.assertions.values() {
        match &assertion.fact {
            ResolvedAgentFact::Node(node) => {
                let projected_id = projected_node_id(&assertion.id);
                nodes.push(project_node(assertion, node, projected_id.clone()));
                agent_facts.push(agent_metadata(assertion, projected_id));
            }
            ResolvedAgentFact::Edge(edge) => {
                let source = projected_endpoint(&edge.source, &agent_node_ids)?;
                let target = projected_endpoint(&edge.target, &agent_node_ids)?;
                if masked_nodes.contains_key(&source) || masked_nodes.contains_key(&target) {
                    cascaded = cascaded.saturating_add(1);
                    let projected_id = projected_edge_id(assertion, &source, &target)?;
                    push_omission(
                        &mut examples,
                        limits.max_diagnostics,
                        &projected_id,
                        "edge",
                        "masked_endpoint",
                        None,
                    );
                    continue;
                }
                let projected = project_edge(assertion, edge, source, target)?;
                let projected_id = projected.id.clone();
                links.push(projected);
                agent_facts.push(agent_metadata(assertion, projected_id));
            }
        }
    }
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    links.sort_by(|left, right| left.id.cmp(&right.id));
    agent_facts.sort_by(|left, right| left.assertion.cmp(&right.assertion));
    challenges.sort_by(|left, right| left.challenge.cmp(&right.challenge));
    examples.sort_by(|left, right| (&left.kind, &left.id).cmp(&(&right.kind, &right.id)));

    let graph = GraphDocument {
        directed: true,
        multigraph: true,
        graph: base.graph.clone(),
        nodes,
        links,
    };
    compass_model::validate_code_graph(&graph).map_err(|error| {
        AgentGraphError::new(
            AgentGraphErrorCode::InvalidTransition,
            format!("Effective Graph validation failed: {error}"),
        )
    })?;
    let total = direct.saturating_add(cascaded);
    let omissions = CompositionOmissions {
        total,
        direct,
        cascaded,
        omitted_examples: total.saturating_sub(examples.len() as u64),
        examples,
    };
    let retractions = effective_retractions(state, limits.max_diagnostics);
    let semantic_graph_digest = Digest::raw_bytes(&canonical_bytes(&graph)?);
    let effective_identity = canonical_digest(
        "compass.agent-graph.effective-identity/1",
        &(
            base_generation,
            &overlay_revision,
            profile,
            COMPOSITION_VERSION_V1,
            &semantic_graph_digest,
            &agent_facts,
            &challenges,
            &retractions,
            &omissions,
        ),
    )?;
    Ok(EffectiveGraph {
        schema: AGENT_GRAPH_EFFECTIVE_SCHEMA_V1.to_owned(),
        base_generation: base_generation.clone(),
        overlay_revision,
        composition_profile: profile,
        composition_version: COMPOSITION_VERSION_V1.to_owned(),
        effective_identity,
        graph,
        agent_facts,
        challenges,
        retractions,
        omissions,
    })
}

fn effective_retractions(state: &OverlayState, limit: usize) -> EffectiveRetractions {
    let total_entries = state
        .retractions
        .len()
        .saturating_add(state.challenge_retractions.len());
    let total = u64::try_from(total_entries).unwrap_or(u64::MAX);
    let mut examples = Vec::with_capacity(limit.min(total_entries));
    examples.extend(
        state
            .retractions
            .values()
            .take(limit)
            .map(|retraction| EffectiveRetraction {
                kind: EffectiveRetractionKind::Assertion,
                id: retraction.assertion.as_str().to_owned(),
                reason_code: retraction.reason_code.clone(),
                explanation: retraction.explanation.clone(),
                sequence: retraction.sequence,
            }),
    );
    let remaining = limit.saturating_sub(examples.len());
    examples.extend(
        state
            .challenge_retractions
            .values()
            .take(remaining)
            .map(|retraction| EffectiveRetraction {
                kind: EffectiveRetractionKind::Challenge,
                id: retraction.challenge.as_str().to_owned(),
                reason_code: retraction.reason_code.clone(),
                explanation: retraction.explanation.clone(),
                sequence: retraction.sequence,
            }),
    );
    EffectiveRetractions {
        total,
        omitted_examples: total.saturating_sub(examples.len() as u64),
        examples,
    }
}

fn project_node(
    assertion: &ActiveAssertion,
    node: &crate::AgentNodeDraft,
    id: String,
) -> NodeRecord {
    let source = assertion.projection_provenance.anchors.first().cloned();
    NodeRecord {
        id,
        kind: node.kind,
        roles: node.roles.clone(),
        name: node.name.clone(),
        qualified_name: node.qualified_name.clone(),
        language: node.language.clone(),
        framework: node.framework.clone(),
        source,
        details: node.details.clone(),
        evidence: vec![assertion.projection_provenance.clone()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    }
}

fn project_edge(
    assertion: &ActiveAssertion,
    edge: &crate::ResolvedAgentEdge,
    source: String,
    target: String,
) -> Result<EdgeRecord, AgentGraphError> {
    let relationship_site = edge
        .relationship_site
        .clone()
        .or_else(|| assertion.projection_provenance.anchors.first().cloned());
    let rule = OccurrenceRule::new(format!("grounded-agent:{}", assertion.id.as_str()))
        .ok_or_else(|| {
            AgentGraphError::new(
                AgentGraphErrorCode::InvalidIdentifier,
                "assertion ID cannot form an occurrence rule",
            )
        })?;
    let id = edge_id(
        &source,
        edge.kind,
        &target,
        relationship_site.as_ref(),
        Some(rule.as_str()),
    );
    Ok(EdgeRecord {
        key: id.clone(),
        id,
        source,
        target,
        kind: edge.kind,
        occurrence_rule: Some(rule),
        relationship_site,
        details: edge.details.clone(),
        evidence: vec![assertion.projection_provenance.clone()],
        weight: None,
        context: edge.context.clone(),
        deferred: false,
        diagnostics: Vec::new(),
    })
}

fn projected_edge_id(
    assertion: &ActiveAssertion,
    source: &str,
    target: &str,
) -> Result<String, AgentGraphError> {
    let ResolvedAgentFact::Edge(edge) = &assertion.fact else {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidTransition,
            "node assertion cannot be projected as an edge",
        ));
    };
    Ok(project_edge(assertion, edge, source.to_owned(), target.to_owned())?.id)
}

fn agent_metadata(assertion: &ActiveAssertion, projected_id: String) -> EffectiveAgentFact {
    EffectiveAgentFact {
        assertion: assertion.id.clone(),
        projected_id,
        certificate_digest: assertion.certificate_digest.clone(),
        owner: assertion.owner.clone(),
        version: assertion.version,
        summary: assertion.summary.clone(),
    }
}

fn projected_endpoint(
    endpoint: &ResolvedNodeRef,
    agent_node_ids: &BTreeMap<AssertionId, String>,
) -> Result<String, AgentGraphError> {
    match endpoint {
        ResolvedNodeRef::Base { node } => Ok(node.id.clone()),
        ResolvedNodeRef::Agent { assertion } => {
            agent_node_ids.get(assertion).cloned().ok_or_else(|| {
                AgentGraphError::new(
                    AgentGraphErrorCode::MissingEndpoint,
                    format!(
                        "agent endpoint {} is not an active node assertion",
                        assertion.as_str()
                    ),
                )
            })
        }
    }
}

fn projected_node_id(assertion: &AssertionId) -> String {
    let digest = Digest::of_bytes(
        "compass.agent-graph.projected-node/1",
        assertion.as_str().as_bytes(),
    );
    format!("agent-node:{}", digest.as_str())
}

fn base_fact_id(fact: &BaseFactRef) -> &str {
    match fact {
        BaseFactRef::Node(node) => &node.id,
        BaseFactRef::Edge(edge) => &edge.id,
    }
}

fn push_omission(
    examples: &mut Vec<CompositionOmission>,
    limit: usize,
    id: &str,
    kind: &str,
    reason: &str,
    challenge: Option<ChallengeId>,
) {
    if examples.len() < limit {
        examples.push(CompositionOmission {
            id: id.to_owned(),
            kind: kind.to_owned(),
            reason: reason.to_owned(),
            challenge,
        });
    }
}
