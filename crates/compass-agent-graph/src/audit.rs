use serde::{Deserialize, Serialize};

use crate::{
    AgentGraphError, AgentGraphErrorCode, Digest, HardLimits, OverlayId, OverlayRevisionId,
    PrincipalId, RepositoryId, canonical_bytes,
};

pub const AGENT_GRAPH_AUDIT_SCHEMA_V1: &str = "compass.agent-graph.audit/1";
pub const AGENT_GRAPH_AUDIT_RESULT_SCHEMA_V1: &str = "compass.agent-graph.audit-result/1";

/// Trusted, bounded operational metadata. It deliberately has no prompt, response,
/// credential, token, source excerpt, or chain-of-thought fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteAttestation {
    adapter: String,
    session_digest: Option<Digest>,
    request_digest: Option<Digest>,
    model_id: Option<String>,
}

impl WriteAttestation {
    pub fn new(adapter: impl Into<String>) -> Result<Self, AgentGraphError> {
        let adapter = adapter.into();
        validate_label("audit adapter", &adapter, 128)?;
        Ok(Self {
            adapter,
            session_digest: None,
            request_digest: None,
            model_id: None,
        })
    }

    #[must_use]
    pub fn with_session_digest(mut self, digest: Digest) -> Self {
        self.session_digest = Some(digest);
        self
    }

    #[must_use]
    pub fn with_request_digest(mut self, digest: Digest) -> Self {
        self.request_digest = Some(digest);
        self
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Result<Self, AgentGraphError> {
        let model_id = model_id.into();
        validate_label("audit model ID", &model_id, 256)?;
        self.model_id = Some(model_id);
        Ok(self)
    }

    pub(crate) fn record(
        &self,
        repository: RepositoryId,
        overlay: OverlayId,
        principal: PrincipalId,
        revision: OverlayRevisionId,
        mutation_digest: Digest,
    ) -> AuditRecord {
        AuditRecord {
            schema: AGENT_GRAPH_AUDIT_SCHEMA_V1.to_owned(),
            repository,
            overlay,
            principal,
            revision,
            adapter: self.adapter.clone(),
            session_digest: self.session_digest.clone(),
            request_digest: self.request_digest.clone(),
            model_id: self.model_id.clone(),
            mutation_digest,
            outcome: AuditOutcome::PublicationPrepared,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    PublicationPrepared,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditRecord {
    pub schema: String,
    pub repository: RepositoryId,
    pub overlay: OverlayId,
    pub principal: PrincipalId,
    pub revision: OverlayRevisionId,
    pub adapter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_digest: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_digest: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub mutation_digest: Digest,
    pub outcome: AuditOutcome,
}

impl AuditRecord {
    /// Validate operational metadata loaded from untrusted persistent storage.
    pub fn validate(&self) -> Result<(), AgentGraphError> {
        if self.schema != AGENT_GRAPH_AUDIT_SCHEMA_V1 {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnsupportedSchema,
                format!(
                    "audit schema must be {AGENT_GRAPH_AUDIT_SCHEMA_V1}; got {}",
                    self.schema
                ),
            ));
        }
        validate_label("audit adapter", &self.adapter, 128)?;
        if let Some(model_id) = &self.model_id {
            validate_label("audit model ID", model_id, 256)?;
        }
        let bytes = canonical_bytes(self)?;
        if bytes.len() > HardLimits::AUDIT_BYTES {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                format!(
                    "operational audit record is {} bytes; maximum is {}",
                    bytes.len(),
                    HardLimits::AUDIT_BYTES
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditResult {
    pub schema: String,
    pub revision: OverlayRevisionId,
    pub records: Vec<AuditRecord>,
    pub truncated: bool,
}

fn validate_label(label: &str, value: &str, maximum: usize) -> Result<(), AgentGraphError> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidInput,
            format!("{label} is empty, oversized, or contains control characters"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit_record() -> Result<AuditRecord, AgentGraphError> {
        Ok(WriteAttestation::new("mcp")?.record(
            RepositoryId::parse("repository:test")?,
            OverlayId::parse("overlay:test")?,
            PrincipalId::parse("principal:test")?,
            OverlayRevisionId(Digest::parse("1".repeat(64))?),
            Digest::parse("2".repeat(64))?,
        ))
    }

    #[test]
    fn persisted_audit_metadata_is_revalidated() -> Result<(), AgentGraphError> {
        let record = audit_record()?;
        record.validate()?;

        let mut invalid_adapter = record.clone();
        invalid_adapter.adapter = "x".repeat(129);
        assert_eq!(
            invalid_adapter.validate().err().map(|error| error.code),
            Some(AgentGraphErrorCode::InvalidInput)
        );

        let mut invalid_schema = record;
        invalid_schema.schema = "compass.agent-graph.audit/2".to_owned();
        assert_eq!(
            invalid_schema.validate().err().map(|error| error.code),
            Some(AgentGraphErrorCode::UnsupportedSchema)
        );
        Ok(())
    }
}
