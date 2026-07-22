use std::path::{Path, PathBuf};

use rand::RngCore as _;
use serde::{Deserialize, Serialize};

use crate::HistoryError;
use crate::config::now_millis;
use crate::durable::{read_json_bounded, remove_file_durable, write_json_atomic};
use crate::store::reject_symlink;

pub const LEASE_DURATION_MILLIS: u64 = 120_000;
pub const LEASE_HEARTBEAT_MILLIS: u64 = 30_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LeaseRecord {
    owner: String,
    generation: u64,
    expires_at_millis: u64,
}

/// Capability held by exactly one live queue worker generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaseGuard {
    pub(crate) path: PathBuf,
    pub(crate) owner: String,
    pub(crate) generation: u64,
}

impl LeaseGuard {
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

pub(crate) fn claim(path: &Path) -> Result<Option<LeaseGuard>, HistoryError> {
    reject_symlink(path, true)?;
    let now = now_millis();
    let generation = if path.exists() {
        let existing = read_lease(path)?;
        if existing.expires_at_millis > now {
            return Ok(None);
        }
        existing.generation.saturating_add(1)
    } else {
        1
    };
    let mut random = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut random);
    let owner: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let record = LeaseRecord {
        owner: owner.clone(),
        generation,
        expires_at_millis: now.saturating_add(LEASE_DURATION_MILLIS),
    };
    write_json_atomic(path, &record)?;
    Ok(Some(LeaseGuard {
        path: path.to_path_buf(),
        owner,
        generation,
    }))
}

pub(crate) fn validate(lease: &LeaseGuard) -> Result<(), HistoryError> {
    let current = read_lease(&lease.path)?;
    if current.owner != lease.owner
        || current.generation != lease.generation
        || current.expires_at_millis <= now_millis()
    {
        return Err(HistoryError::OperationalState(
            "history worker lease is stale".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn heartbeat(lease: &LeaseGuard) -> Result<(), HistoryError> {
    validate(lease)?;
    write_json_atomic(
        &lease.path,
        &LeaseRecord {
            owner: lease.owner.clone(),
            generation: lease.generation,
            expires_at_millis: now_millis().saturating_add(LEASE_DURATION_MILLIS),
        },
    )
}

pub(crate) fn release(lease: &LeaseGuard) -> Result<(), HistoryError> {
    validate(lease)?;
    remove_file_durable(&lease.path)
}

pub(crate) fn expired(path: &Path) -> Result<bool, HistoryError> {
    if !path.exists() {
        reject_symlink(path, true)?;
        return Ok(true);
    }
    let lease = read_lease(path)?;
    Ok(lease.expires_at_millis <= now_millis())
}

fn read_lease(path: &Path) -> Result<LeaseRecord, HistoryError> {
    let lease: LeaseRecord = read_json_bounded(path)?;
    if lease.owner.len() != 32
        || !lease
            .owner
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || lease.generation == 0
    {
        return Err(HistoryError::OperationalState(format!(
            "{} contains an invalid lease",
            path.display()
        )));
    }
    Ok(lease)
}
