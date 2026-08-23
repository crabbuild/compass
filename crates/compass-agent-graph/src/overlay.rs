use std::collections::BTreeMap;

use compass_model::code_graph::{EdgeDetails, EdgeKind};
use compass_model::provenance::{Provenance, SourceAnchor};
use serde::{Deserialize, Serialize};

use crate::{
    AgentNodeDraft, AssertionDigest, AssertionId, AssertionKey, BaseFactRef, BaseGenerationId,
    BaseNodeRef, ChallengeEffect, ChallengeId, Digest, GroundingCertificate,
    GroundingCertificateDigest, GroundingSubmission, OverlayId, OverlayRevisionId, PrincipalId,
};

pub const AGENT_GRAPH_OVERLAY_SCHEMA_V1: &str = "compass.agent-graph.overlay/1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveAssertion {
    pub id: AssertionId,
    pub key: AssertionKey,
    pub owner: PrincipalId,
    pub version: u64,
    pub assertion_digest: AssertionDigest,
    pub certificate_digest: GroundingCertificateDigest,
    pub certificate: GroundingCertificate,
    pub fact: ResolvedAgentFact,
    pub summary: String,
    pub grounding: GroundingSubmission,
    pub projection_provenance: Provenance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "factType", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResolvedAgentFact {
    Node(AgentNodeDraft),
    Edge(ResolvedAgentEdge),
}

impl ResolvedAgentFact {
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Node(_) => "node",
            Self::Edge(_) => "edge",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedAgentEdge {
    pub source: ResolvedNodeRef,
    pub target: ResolvedNodeRef,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship_site: Option<SourceAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<EdgeDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "referenceType", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResolvedNodeRef {
    Base { node: BaseNodeRef },
    Agent { assertion: AssertionId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Retraction {
    pub assertion: AssertionId,
    pub key: AssertionKey,
    pub owner: PrincipalId,
    pub retracted_digest: AssertionDigest,
    pub reason_code: String,
    pub explanation: String,
    pub sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveChallenge {
    pub id: ChallengeId,
    pub owner: PrincipalId,
    pub version: u64,
    pub challenge_digest: Digest,
    pub target: BaseFactRef,
    pub effect: ChallengeEffect,
    pub summary: String,
    pub grounding: GroundingSubmission,
    pub certificate: GroundingCertificate,
    pub certificate_digest: GroundingCertificateDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChallengeRetraction {
    pub challenge: ChallengeId,
    pub owner: PrincipalId,
    pub retracted_digest: Digest,
    pub reason_code: String,
    pub explanation: String,
    pub sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdempotencyRecord {
    pub batch_digest: Digest,
    pub revision: OverlayRevisionId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayState {
    pub schema: String,
    pub overlay: OverlayId,
    pub base_generation: BaseGenerationId,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<OverlayRevisionId>,
    pub assertions: BTreeMap<AssertionId, ActiveAssertion>,
    pub challenges: BTreeMap<ChallengeId, ActiveChallenge>,
    pub retractions: BTreeMap<AssertionId, Retraction>,
    pub challenge_retractions: BTreeMap<ChallengeId, ChallengeRetraction>,
}

impl OverlayState {
    #[must_use]
    pub fn empty(overlay: OverlayId, base_generation: BaseGenerationId) -> Self {
        Self {
            schema: AGENT_GRAPH_OVERLAY_SCHEMA_V1.to_owned(),
            overlay,
            base_generation,
            sequence: 0,
            parent_revision: None,
            assertions: BTreeMap::new(),
            challenges: BTreeMap::new(),
            retractions: BTreeMap::new(),
            challenge_retractions: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayRevision {
    pub schema: String,
    pub overlay: OverlayId,
    pub base_generation: BaseGenerationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<OverlayRevisionId>,
    pub sequence: u64,
    pub state_root: Digest,
    pub state_digest: Digest,
    pub state_bytes: u64,
    pub active_assertions: u64,
    pub active_challenges: u64,
    pub retractions: u64,
    pub challenge_retractions: u64,
    pub mutation_digest: Digest,
    pub composition_version: String,
}
