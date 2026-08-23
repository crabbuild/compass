use std::fmt;

use compass_model::code_graph::{EdgeKind, NodeKind};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::Digest;

pub const AGENT_GRAPH_BATCH_SCHEMA_V1: &str = "compass.agent-graph.batch/1";
pub const AGENT_GRAPH_RECEIPT_SCHEMA_V1: &str = "compass.agent-graph.receipt/1";
const MAX_ID_BYTES: usize = 256;

macro_rules! text_id {
    ($name:ident, $label:literal, $prefix:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, AgentGraphError> {
                let value = value.into();
                validate_text_id($label, $prefix, &value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(de::Error::custom)
            }
        }
    };
}

text_id!(OverlayId, "overlay ID", "overlay:");
text_id!(AssertionKey, "assertion key", "key:");
text_id!(AssertionId, "assertion ID", "assertion:");
text_id!(ChallengeId, "challenge ID", "challenge:");
text_id!(PrincipalId, "principal ID", "principal:");
text_id!(IdempotencyKey, "idempotency key", "idempotency:");
text_id!(RepositoryId, "repository ID", "repository:");
text_id!(PinId, "pin ID", "pin:");

macro_rules! digest_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Digest);

        impl $name {
            #[must_use]
            pub fn as_digest(&self) -> &Digest {
                &self.0
            }
        }
    };
}

digest_id!(OverlayRevisionId);
digest_id!(AssertionDigest);
digest_id!(GroundingCertificateDigest);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaseGenerationId {
    pub generation_id: String,
    pub graph_digest: Digest,
}

impl BaseGenerationId {
    pub fn validate(&self) -> Result<(), AgentGraphError> {
        validate_bounded_text("generationId", &self.generation_id, MAX_ID_BYTES, false)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "factType",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BaseFactRef {
    Node(BaseNodeRef),
    Edge(BaseEdgeRef),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaseNodeRef {
    pub base_generation: BaseGenerationId,
    pub id: String,
    pub kind: NodeKind,
    pub record_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaseEdgeRef {
    pub base_generation: BaseGenerationId,
    pub id: String,
    pub kind: EdgeKind,
    pub source: String,
    pub target: String,
    pub record_digest: Digest,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentGraphErrorCode {
    UnsupportedSchema,
    WritesDisabled,
    Unauthenticated,
    Unauthorized,
    InvalidIdentifier,
    InvalidInput,
    LimitExceeded,
    UnknownBaseGeneration,
    UnknownOverlay,
    RevisionConflict,
    IdempotencyConflict,
    AssertionNotFound,
    AssertionDigestConflict,
    OwnershipViolation,
    DuplicateOperation,
    InvalidTransition,
    InvalidCitation,
    GroundingFailed,
    GroundingPolicyUnsupported,
    MaskNotPermitted,
    MissingEndpoint,
    ActiveDependents,
    AssertionCycle,
    RebaseRequired,
    RebasePlanStale,
    RebaseUnresolved,
    RebaseAmbiguous,
    CorruptOverlay,
    PublicationConflict,
    StorageFailure,
}

impl AgentGraphErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedSchema => "unsupported_schema",
            Self::WritesDisabled => "writes_disabled",
            Self::Unauthenticated => "unauthenticated",
            Self::Unauthorized => "unauthorized",
            Self::InvalidIdentifier => "invalid_identifier",
            Self::InvalidInput => "invalid_input",
            Self::LimitExceeded => "limit_exceeded",
            Self::UnknownBaseGeneration => "unknown_base_generation",
            Self::UnknownOverlay => "unknown_overlay",
            Self::RevisionConflict => "revision_conflict",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::AssertionNotFound => "assertion_not_found",
            Self::AssertionDigestConflict => "assertion_digest_conflict",
            Self::OwnershipViolation => "ownership_violation",
            Self::DuplicateOperation => "duplicate_operation",
            Self::InvalidTransition => "invalid_transition",
            Self::InvalidCitation => "invalid_citation",
            Self::GroundingFailed => "grounding_failed",
            Self::GroundingPolicyUnsupported => "grounding_policy_unsupported",
            Self::MaskNotPermitted => "mask_not_permitted",
            Self::MissingEndpoint => "missing_endpoint",
            Self::ActiveDependents => "active_dependents",
            Self::AssertionCycle => "assertion_cycle",
            Self::RebaseRequired => "rebase_required",
            Self::RebasePlanStale => "rebase_plan_stale",
            Self::RebaseUnresolved => "rebase_unresolved",
            Self::RebaseAmbiguous => "rebase_ambiguous",
            Self::CorruptOverlay => "corrupt_overlay",
            Self::PublicationConflict => "publication_conflict",
            Self::StorageFailure => "storage_failure",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentGraphDiagnostic {
    pub code: String,
    pub field: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_ids: Vec<String>,
    #[serde(default)]
    pub omitted_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphError {
    pub schema: String,
    pub code: AgentGraphErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<AgentGraphDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_revision: Option<OverlayRevisionId>,
}

impl AgentGraphError {
    #[must_use]
    pub fn new(code: AgentGraphErrorCode, message: impl Into<String>) -> Self {
        Self {
            schema: "compass.agent-graph.error/1".to_owned(),
            code,
            message: message.into(),
            diagnostics: Vec::new(),
            observed_revision: None,
        }
    }

    #[must_use]
    pub fn with_diagnostic(mut self, diagnostic: AgentGraphDiagnostic) -> Self {
        self.diagnostics.push(diagnostic);
        self.diagnostics.sort_by(|left, right| {
            (&left.field, &left.code, &left.message).cmp(&(
                &right.field,
                &right.code,
                &right.message,
            ))
        });
        self
    }
}

impl fmt::Display for AgentGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for AgentGraphError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitReceipt {
    pub schema: String,
    pub overlay: OverlayId,
    pub revision: OverlayRevisionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<OverlayRevisionId>,
    pub base_generation: BaseGenerationId,
    pub sequence: u64,
    pub batch_digest: Digest,
    pub active_assertions: u64,
    pub active_challenges: u64,
    pub retractions: u64,
    pub idempotent_replay: bool,
}

pub(crate) fn validate_bounded_text(
    field: &str,
    value: &str,
    maximum: usize,
    ascii_only: bool,
) -> Result<(), AgentGraphError> {
    if value.trim().is_empty() {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidInput,
            format!("{field} must not be empty"),
        ));
    }
    if value.len() > maximum {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::LimitExceeded,
            format!("{field} is {} bytes; maximum is {maximum}", value.len()),
        ));
    }
    if ascii_only && !value.is_ascii() {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidIdentifier,
            format!("{field} must contain only ASCII characters"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidInput,
            format!("{field} must not contain control characters"),
        ));
    }
    Ok(())
}

fn validate_text_id(label: &str, prefix: &str, value: &str) -> Result<(), AgentGraphError> {
    validate_bounded_text(label, value, MAX_ID_BYTES, true)?;
    if !value.starts_with(prefix)
        || value.len() == prefix.len()
        || !value[prefix.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidIdentifier,
            format!("{label} must use the {prefix} domain prefix and portable ID characters"),
        ));
    }
    Ok(())
}
