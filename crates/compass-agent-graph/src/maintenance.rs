use serde::{Deserialize, Serialize};

use crate::{
    AgentGraphError, AgentGraphErrorCode, Digest, OverlayId, OverlayRevisionId, PinId, RepositoryId,
};

pub const AGENT_GRAPH_PIN_SCHEMA_V1: &str = "compass.agent-graph.pin/1";
pub const AGENT_GRAPH_GC_PLAN_SCHEMA_V1: &str = "compass.agent-graph.gc-plan/1";
pub const AGENT_GRAPH_GC_RECEIPT_SCHEMA_V1: &str = "compass.agent-graph.gc-receipt/1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevisionPin {
    pub schema: String,
    pub pin: PinId,
    pub overlay: OverlayId,
    pub revision: OverlayRevisionId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GcPlan {
    pub schema: String,
    pub repository: RepositoryId,
    pub reachability_digest: Digest,
    pub unreachable_revisions: Vec<OverlayRevisionId>,
    pub unreachable_objects: Vec<Digest>,
    pub scanned_keys: u64,
    pub scanned_key_bytes: u64,
    pub truncated: bool,
}

impl GcPlan {
    pub fn digest(&self) -> Result<Digest, AgentGraphError> {
        crate::canonical_digest("compass.agent-graph.gc-plan/1", self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GcReceipt {
    pub schema: String,
    pub plan_digest: Digest,
    pub deleted_revisions: u64,
    pub deleted_objects: u64,
    pub deleted_audits: u64,
}

/// Capability for a sweep performed while every writer using this repository is stopped.
///
/// The storage interface provides point-key CAS, not a portable cross-process read/write
/// lock. Consequently an online collector can safely plan reachability but cannot safely
/// delete a content object that an in-flight writer has published but not activated. This
/// non-serializable grant makes that precondition explicit at the owning boundary.
pub struct QuiescentGcGrant {
    repository: RepositoryId,
    _private: (),
}

impl QuiescentGcGrant {
    pub(crate) fn authorize(&self, repository: &RepositoryId) -> Result<(), AgentGraphError> {
        if &self.repository != repository {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::Unauthorized,
                "GC grant does not belong to this repository",
            ));
        }
        Ok(())
    }
}

pub struct GcAuthority {
    repository: RepositoryId,
    quiescence_confirmed: bool,
}

impl GcAuthority {
    #[must_use]
    pub const fn disabled(repository: RepositoryId) -> Self {
        Self {
            repository,
            quiescence_confirmed: false,
        }
    }

    /// Construct an authority only after the adapter has stopped all processes that can write
    /// this repository and prevented new writers from starting until the sweep returns.
    #[must_use]
    pub const fn explicitly_quiescent(repository: RepositoryId) -> Self {
        Self {
            repository,
            quiescence_confirmed: true,
        }
    }

    pub fn mint(self) -> Result<QuiescentGcGrant, AgentGraphError> {
        if !self.quiescence_confirmed {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::WritesDisabled,
                "GC sweep requires an explicitly quiescent repository",
            ));
        }
        Ok(QuiescentGcGrant {
            repository: self.repository,
            _private: (),
        })
    }
}
