use compass_model::code_graph::{EdgeDetails, EdgeKind, NodeDetails, NodeKind, NodeRole};
use compass_model::provenance::SourceAnchor;
use serde::{Deserialize, Serialize};

use crate::contract::validate_bounded_text;
use crate::{
    AGENT_GRAPH_BATCH_SCHEMA_V1, AgentGraphError, AgentGraphErrorCode, AgentGraphLimits,
    AssertionDigest, AssertionId, AssertionKey, BaseFactRef, BaseGenerationId, BaseNodeRef,
    ChallengeId, Digest, GroundingSubmission, IdempotencyKey, OverlayId, OverlayRevisionId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeBatch {
    pub schema: String,
    pub overlay: OverlayId,
    pub base_generation: BaseGenerationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<OverlayRevisionId>,
    pub idempotency_key: IdempotencyKey,
    pub operations: Vec<ChangeOperation>,
}

impl ChangeBatch {
    pub fn validate(&self, limits: AgentGraphLimits) -> Result<(), AgentGraphError> {
        let limits = limits.validate()?;
        if self.schema != AGENT_GRAPH_BATCH_SCHEMA_V1 {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnsupportedSchema,
                format!(
                    "batch schema must be {AGENT_GRAPH_BATCH_SCHEMA_V1}; got {}",
                    self.schema
                ),
            ));
        }
        self.base_generation.validate()?;
        if self.operations.is_empty() {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::InvalidInput,
                "batch must contain at least one operation",
            ));
        }
        if self.operations.len() > limits.max_operations {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                format!(
                    "batch has {} operations; maximum is {}",
                    self.operations.len(),
                    limits.max_operations
                ),
            ));
        }
        let bytes = crate::canonical_bytes(self)?;
        if bytes.len() > limits.max_batch_bytes {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                format!(
                    "encoded batch is {} bytes; maximum is {}",
                    bytes.len(),
                    limits.max_batch_bytes
                ),
            ));
        }
        for operation in &self.operations {
            operation.validate(limits)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChangeOperation {
    PutAssertion {
        assertion: AssertionDraft,
    },
    RetractAssertion {
        assertion: AssertionId,
        expected_assertion_digest: AssertionDigest,
        reason_code: String,
        explanation: String,
    },
    PutChallenge {
        challenge: ChallengeDraft,
    },
    RetractChallenge {
        challenge: ChallengeId,
        expected_challenge_digest: Digest,
        reason_code: String,
        explanation: String,
    },
}

impl ChangeOperation {
    fn validate(&self, limits: AgentGraphLimits) -> Result<(), AgentGraphError> {
        match self {
            Self::PutAssertion { assertion } => assertion.validate(limits),
            Self::PutChallenge { challenge } => challenge.validate(limits),
            Self::RetractAssertion {
                reason_code,
                explanation,
                ..
            }
            | Self::RetractChallenge {
                reason_code,
                explanation,
                ..
            } => {
                validate_bounded_text("reasonCode", reason_code, 128, true)?;
                validate_bounded_text("explanation", explanation, limits.max_text_bytes, false)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssertionDraft {
    pub selector: AssertionSelector,
    pub fact: AgentFactDraft,
    pub grounding: GroundingSubmission,
    pub summary: String,
}

impl AssertionDraft {
    pub fn validate(&self, limits: AgentGraphLimits) -> Result<(), AgentGraphError> {
        validate_bounded_text("summary", &self.summary, limits.max_text_bytes, false)?;
        self.fact.validate()?;
        self.grounding.validate(limits)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "selector",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AssertionSelector {
    New {
        key: AssertionKey,
    },
    Existing {
        id: AssertionId,
        expected_assertion_digest: AssertionDigest,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "factType", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentFactDraft {
    Node(AgentNodeDraft),
    Edge(AgentEdgeDraft),
}

impl AgentFactDraft {
    pub(crate) fn validate(&self) -> Result<(), AgentGraphError> {
        match self {
            Self::Node(node) => node.validate(),
            Self::Edge(edge) => edge.validate(),
        }
    }

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
pub struct AgentNodeDraft {
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<NodeRole>,
    pub name: String,
    pub qualified_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<NodeDetails>,
}

impl AgentNodeDraft {
    fn validate(&self) -> Result<(), AgentGraphError> {
        validate_bounded_text("name", &self.name, 2_048, false)?;
        validate_bounded_text("qualifiedName", &self.qualified_name, 4_096, false)?;
        if self.roles.len() > 32 {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                "node roles exceed the maximum of 32",
            ));
        }
        let mut roles = self.roles.clone();
        roles.sort_by_key(|role| format!("{role:?}"));
        roles.dedup();
        if roles.len() != self.roles.len() {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::InvalidInput,
                "node roles must not contain duplicates",
            ));
        }
        if let Some(language) = &self.language {
            validate_bounded_text("language", language, 128, false)?;
        }
        if let Some(framework) = &self.framework {
            validate_bounded_text("framework", framework, 128, false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "referenceType",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NodeRef {
    Base { node: BaseNodeRef },
    Agent { assertion: AssertionId },
    CreatedInThisBatch { key: AssertionKey },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentEdgeDraft {
    pub source: NodeRef,
    pub target: NodeRef,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship_site: Option<SourceAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<EdgeDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl AgentEdgeDraft {
    fn validate(&self) -> Result<(), AgentGraphError> {
        if let Some(anchor) = &self.relationship_site
            && (!anchor.is_valid() || anchor.start_byte == anchor.end_byte)
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::InvalidInput,
                "relationshipSite must be a non-empty portable source range",
            ));
        }
        if let Some(context) = &self.context {
            validate_bounded_text("context", context, 2_048, false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeEffect {
    Flag,
    Mask,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChallengeDraft {
    pub selector: ChallengeSelector,
    pub target: BaseFactRef,
    pub effect: ChallengeEffect,
    pub grounding: GroundingSubmission,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "selector",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChallengeSelector {
    New {
        id: ChallengeId,
    },
    Existing {
        id: ChallengeId,
        expected_challenge_digest: Digest,
    },
}

impl ChallengeDraft {
    fn validate(&self, limits: AgentGraphLimits) -> Result<(), AgentGraphError> {
        validate_bounded_text("summary", &self.summary, limits.max_text_bytes, false)?;
        self.grounding.validate(limits)
    }
}
