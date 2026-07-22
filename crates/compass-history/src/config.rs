use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::durable::{read_json_bounded, write_json_atomic};
use crate::store::{create_owner_dir, reject_directory, reject_symlink};
use crate::{BuildProfile, HistoryError, Repository};

const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Explicit repository-wide eager-history configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryConfig {
    pub schema_version: u32,
    pub enabled: bool,
    pub profile: Option<BuildProfile>,
    pub profile_digest: Option<String>,
    pub updated_at_millis: u64,
}

impl HistoryConfig {
    /// Load configuration without creating any operational path.
    pub fn load(repository: &Repository) -> Result<Self, HistoryError> {
        let root = repository.common_dir().join("compass");
        if !root.exists() {
            reject_symlink(&root, true)?;
            return Ok(Self::absent());
        }
        reject_directory(&root)?;
        let path = root.join("config.json");
        if !path.exists() {
            reject_symlink(&path, true)?;
            return Ok(Self::absent());
        }
        let config: Self = read_json_bounded(&path)?;
        config.validate()?;
        Ok(config)
    }

    /// Atomically enable eager history with a normalized non-secret profile.
    pub fn enable(repository: &Repository, profile: BuildProfile) -> Result<Self, HistoryError> {
        let digest = hex(&profile.digest()?);
        let config = Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            enabled: true,
            profile: Some(profile),
            profile_digest: Some(digest),
            updated_at_millis: now_millis(),
        };
        config.validate()?;
        let root = repository.common_dir().join("compass");
        create_owner_dir(&root)?;
        write_json_atomic(&root.join("config.json"), &config)?;
        Ok(config)
    }

    /// Atomically disable eager history while retaining all graph and job data.
    pub fn disable(repository: &Repository) -> Result<Self, HistoryError> {
        let mut config = Self::load(repository)?;
        if config.profile.is_none() || !config.enabled {
            return Ok(config);
        }
        config.enabled = false;
        config.updated_at_millis = now_millis();
        write_json_atomic(
            &repository.common_dir().join("compass/config.json"),
            &config,
        )?;
        Ok(config)
    }

    #[must_use]
    pub fn configured(&self) -> bool {
        self.profile.is_some()
    }

    fn absent() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            enabled: false,
            profile: None,
            profile_digest: None,
            updated_at_millis: 0,
        }
    }

    fn validate(&self) -> Result<(), HistoryError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(HistoryError::OperationalState(format!(
                "unsupported history config schema {}",
                self.schema_version
            )));
        }
        match (&self.profile, &self.profile_digest) {
            (Some(profile), Some(digest)) if hex(&profile.digest()?) == *digest => Ok(()),
            (None, None) if !self.enabled => Ok(()),
            _ => Err(HistoryError::OperationalState(
                "history config profile and digest do not agree".to_owned(),
            )),
        }
    }
}

#[must_use]
pub(crate) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

#[must_use]
pub(crate) fn operational_root(repository: &Repository) -> PathBuf {
    repository.common_dir().join("compass")
}
