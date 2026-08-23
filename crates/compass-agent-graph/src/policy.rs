use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    AgentGraphError, AgentGraphErrorCode, AgentGraphLimits, BaseGenerationId, OverlayId,
    OverlayRevisionId, PrincipalId, RepositoryId,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OperationPermission {
    PutAssertion,
    RetractAssertion,
    PutChallenge,
    RetractChallenge,
    CommitRebase,
}

/// Trusted adapter-side authority. It is never request-deserializable.
#[derive(Clone, Debug)]
pub struct WriteAuthority {
    enabled: bool,
    repository: RepositoryId,
}

impl WriteAuthority {
    #[must_use]
    pub const fn disabled(repository: RepositoryId) -> Self {
        Self {
            enabled: false,
            repository,
        }
    }

    #[must_use]
    pub const fn explicitly_enabled(repository: RepositoryId) -> Self {
        Self {
            enabled: true,
            repository,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mint(
        &self,
        principal: PrincipalId,
        overlay: OverlayId,
        base_generation: BaseGenerationId,
        expected_revision: Option<OverlayRevisionId>,
        permissions: BTreeSet<OperationPermission>,
        mask_permitted: bool,
        expires_at_unix_seconds: u64,
        limits: AgentGraphLimits,
    ) -> Result<WriteGrant, AgentGraphError> {
        self.mint_attested(
            principal,
            overlay,
            base_generation,
            expected_revision,
            permissions,
            mask_permitted,
            expires_at_unix_seconds,
            limits,
            crate::WriteAttestation::new("local")?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mint_attested(
        &self,
        principal: PrincipalId,
        overlay: OverlayId,
        base_generation: BaseGenerationId,
        expected_revision: Option<OverlayRevisionId>,
        permissions: BTreeSet<OperationPermission>,
        mask_permitted: bool,
        expires_at_unix_seconds: u64,
        limits: AgentGraphLimits,
        attestation: crate::WriteAttestation,
    ) -> Result<WriteGrant, AgentGraphError> {
        if !self.enabled {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::WritesDisabled,
                "agent graph writes are disabled",
            ));
        }
        limits.validate()?;
        if permissions.is_empty() {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::Unauthorized,
                "write grant must contain at least one operation permission",
            ));
        }
        let now = unix_seconds()?;
        if expires_at_unix_seconds <= now {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::Unauthorized,
                "write grant expiry must be in the future",
            ));
        }
        Ok(WriteGrant {
            repository: self.repository.clone(),
            principal,
            overlay,
            base_generation,
            expected_revision,
            permissions,
            mask_permitted,
            expires_at_unix_seconds,
            limits,
            attestation,
        })
    }
}

#[derive(Clone, Debug)]
pub struct WriteGrant {
    repository: RepositoryId,
    principal: PrincipalId,
    overlay: OverlayId,
    base_generation: BaseGenerationId,
    expected_revision: Option<OverlayRevisionId>,
    permissions: BTreeSet<OperationPermission>,
    mask_permitted: bool,
    expires_at_unix_seconds: u64,
    limits: AgentGraphLimits,
    attestation: crate::WriteAttestation,
}

impl WriteGrant {
    #[must_use]
    pub fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    #[must_use]
    pub fn limits(&self) -> AgentGraphLimits {
        self.limits
    }

    #[must_use]
    pub const fn mask_permitted(&self) -> bool {
        self.mask_permitted
    }

    pub(crate) fn attestation(&self) -> &crate::WriteAttestation {
        &self.attestation
    }

    pub(crate) fn authorize_scope(
        &self,
        repository: &RepositoryId,
        overlay: &OverlayId,
        base_generation: &BaseGenerationId,
        expected_revision: &Option<OverlayRevisionId>,
    ) -> Result<(), AgentGraphError> {
        if &self.repository != repository
            || &self.overlay != overlay
            || &self.base_generation != base_generation
        {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::Unauthorized,
                "write grant scope does not match repository, overlay, or Base Generation",
            ));
        }
        if &self.expected_revision != expected_revision {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::Unauthorized,
                "write grant expected revision does not match the batch",
            ));
        }
        if unix_seconds()? >= self.expires_at_unix_seconds {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::Unauthorized,
                "write grant has expired",
            ));
        }
        Ok(())
    }

    pub(crate) fn authorize_operation(
        &self,
        permission: OperationPermission,
    ) -> Result<(), AgentGraphError> {
        if self.permissions.contains(&permission) {
            Ok(())
        } else {
            Err(AgentGraphError::new(
                AgentGraphErrorCode::Unauthorized,
                "write grant does not permit one or more requested operations",
            ))
        }
    }
}

fn unix_seconds() -> Result<u64, AgentGraphError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| {
            AgentGraphError::new(
                AgentGraphErrorCode::Unauthorized,
                format!("system clock is before the Unix epoch: {error}"),
            )
        })
}
