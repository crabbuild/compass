use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::process::Command;

use prolly::{ManifestStoreScan, NamedRootRetention, TransactionUpdate};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::now_millis;
use crate::durable::remove_file_durable;
use crate::store::{STORE_FORMAT_ROOT, reject_directory, reject_symlink, version_root_name};
use crate::{HistoryError, HistoryQueue, HistoryStore, JobState, RealizationId};

const GC_SCHEMA_VERSION: u32 = 1;
const TERMINAL_JOB_RETENTION_MILLIS: u64 = 30 * 24 * 60 * 60 * 1_000;
const TEMP_RETENTION_MILLIS: u64 = 24 * 60 * 60 * 1_000;
const REALIZATION_ROOT_KINDS: [&[u8]; 6] = [
    b"nodes",
    b"edges",
    b"hyperedges",
    b"analysis",
    b"metadata",
    b"manifest",
];

/// Immutable, checked garbage-collection plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GcPlan {
    pub schema_version: u32,
    pub catalog_digest: String,
    pub node_plan_digest: String,
    pub cleanup_digest: String,
    pub prune_non_preferred: bool,
    pub prunable_realizations: usize,
    pub prunable_realization_ids: Vec<RealizationId>,
    pub prunable_named_roots: Vec<String>,
    pub candidate_nodes: usize,
    pub retained_nodes: usize,
    pub reclaimable_nodes: usize,
    pub reclaimable_bytes: usize,
    pub expired_job_records: Vec<String>,
    pub expired_temp_directories: Vec<String>,
    planned_at_millis: u64,
}

/// Applied logical reclamation result. SQLite file size is deliberately not reported.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GcSweep {
    pub plan: GcPlan,
    pub deleted_nodes: usize,
    pub deleted_bytes: usize,
    pub deleted_named_roots: usize,
    pub deleted_job_records: usize,
    pub deleted_temp_directories: usize,
    pub reusable_bytes: Option<u64>,
    pub reusable_pages: Option<u64>,
}

impl HistoryStore {
    /// Plan logical SQLite reclamation while retaining every published realization by default.
    pub fn plan_gc(&self, prune_non_preferred: bool) -> Result<GcPlan, HistoryError> {
        let _maintenance = self.maintenance()?;
        self.plan_gc_locked(prune_non_preferred, now_millis())
    }

    /// Apply an immutable plan after rechecking the exact catalog and cleanup observations.
    pub fn sweep_gc(&self, plan: GcPlan) -> Result<GcSweep, HistoryError> {
        let _maintenance = self.maintenance()?;
        let current = self.plan_gc_locked(plan.prune_non_preferred, plan.planned_at_millis)?;
        if current != plan {
            return Err(HistoryError::OperationalState(
                "garbage-collection plan is stale; plan again before sweeping".to_owned(),
            ));
        }

        let root_names = roots_for_realizations(&plan.prunable_realization_ids);
        if !root_names.is_empty() {
            let transaction = self.prolly.begin_transaction()?;
            for name in &root_names {
                if transaction.load_named_root(name)?.is_none() {
                    return Err(HistoryError::OperationalState(
                        "garbage-collection plan lost a named root".to_owned(),
                    ));
                }
                transaction.delete_named_root(name)?;
            }
            if !matches!(transaction.commit()?, TransactionUpdate::Applied { .. }) {
                return Err(HistoryError::OperationalState(
                    "garbage-collection root transaction conflicted".to_owned(),
                ));
            }
        }

        let swept = self
            .prolly
            .sweep_store_gc_for_retention(&NamedRootRetention::all())?;
        let deleted_job_records = self.delete_expired_jobs(&plan.expired_job_records)?;
        let deleted_temp_directories =
            self.delete_expired_temp_directories(&plan.expired_temp_directories)?;
        Ok(GcSweep {
            deleted_nodes: swept.deleted_nodes,
            deleted_bytes: swept.deleted_bytes,
            deleted_named_roots: root_names.len(),
            deleted_job_records,
            deleted_temp_directories,
            reusable_bytes: None,
            reusable_pages: None,
            plan,
        })
    }

    fn plan_gc_locked(
        &self,
        prune_non_preferred: bool,
        planned_at_millis: u64,
    ) -> Result<GcPlan, HistoryError> {
        let roots = self.prolly.store().list_roots()?;
        let preferred_commits = validate_root_listing(&roots)?;
        let catalog_digest = digest_roots(&roots)?;
        let versions = self.list_unlocked_for_gc()?;
        for commit in preferred_commits {
            if !versions
                .iter()
                .any(|version| version.preferred && version.version.git_commit == commit.as_str())
            {
                return Err(HistoryError::CorruptHistory(format!(
                    "preferred root for {commit} has no complete catalog realization"
                )));
            }
        }
        let mut prunable_realizations = versions
            .into_iter()
            .filter(|version| prune_non_preferred && !version.preferred)
            .map(|version| version.id)
            .collect::<Vec<_>>();
        prunable_realizations.sort_by_key(RealizationId::as_hex);
        let roots_to_remove = roots_for_realizations(&prunable_realizations);
        let removed = roots_to_remove.iter().collect::<BTreeSet<_>>();
        let retained_names = roots
            .iter()
            .filter(|root| !removed.contains(&root.name))
            .map(|root| root.name.clone())
            .collect::<Vec<_>>();
        let retention = if prune_non_preferred {
            NamedRootRetention::exact(&retained_names)
        } else {
            NamedRootRetention::all()
        };
        let node_plan = self.prolly.plan_store_gc_for_retention(&retention)?;
        let (expired_job_records, expired_temp_directories, cleanup_digest) =
            self.cleanup_plan(planned_at_millis)?;
        Ok(GcPlan {
            schema_version: GC_SCHEMA_VERSION,
            catalog_digest,
            node_plan_digest: digest_node_plan(&node_plan),
            cleanup_digest,
            prune_non_preferred,
            prunable_named_roots: roots_to_remove.iter().map(|name| hex(name)).collect(),
            prunable_realizations: prunable_realizations.len(),
            prunable_realization_ids: prunable_realizations,
            candidate_nodes: node_plan.candidate_nodes,
            retained_nodes: node_plan.retained_candidate_nodes(),
            reclaimable_nodes: node_plan.reclaimable_nodes,
            reclaimable_bytes: node_plan.reclaimable_bytes,
            expired_job_records,
            expired_temp_directories,
            planned_at_millis,
        })
    }

    fn cleanup_plan(
        &self,
        planned_at_millis: u64,
    ) -> Result<(Vec<String>, Vec<String>, String), HistoryError> {
        let mut digest = Sha256::new();
        let mut expired_jobs = Vec::new();
        if let Some(queue) = HistoryQueue::open_root_existing(&self.root)? {
            for job in queue.list()? {
                digest.update(job.id.as_bytes());
                digest.update(job.updated_at_millis.to_le_bytes());
                digest.update(match job.state {
                    JobState::Queued => b"queued".as_slice(),
                    JobState::Building => b"building".as_slice(),
                    JobState::Validating => b"validating".as_slice(),
                    JobState::Published => b"published".as_slice(),
                    JobState::Failed => b"failed".as_slice(),
                    JobState::Incomplete => b"incomplete".as_slice(),
                });
                if job.state.terminal()
                    && job
                        .updated_at_millis
                        .saturating_add(TERMINAL_JOB_RETENTION_MILLIS)
                        <= planned_at_millis
                {
                    expired_jobs.push(format!("{}.json", job.id));
                }
            }
        }
        expired_jobs.sort();

        let mut expired_temp = Vec::new();
        let leases = self.root.join("leases");
        let live_lease = crate::leases::has_live_lease(&leases)?;
        digest.update([u8::from(live_lease)]);
        let tmp = self.root.join("tmp");
        if tmp.exists() {
            reject_directory(&tmp)?;
            for entry in
                fs::read_dir(&tmp).map_err(|source| crate::error::io_error(&tmp, source))?
            {
                let entry = entry.map_err(|source| crate::error::io_error(&tmp, source))?;
                let path = entry.path();
                reject_symlink(&path, false)?;
                reject_directory(&path)?;
                let name = entry.file_name().into_string().map_err(|_| {
                    HistoryError::OperationalState("temporary directory name is not UTF-8".into())
                })?;
                if !name.starts_with("worktree-") || path.parent() != Some(tmp.as_path()) {
                    return Err(HistoryError::UnsafePath {
                        path,
                        reason: "unexpected history temporary directory".to_owned(),
                    });
                }
                let modified = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .map_err(|source| crate::error::io_error(&path, source))?
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| {
                        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
                    });
                digest.update(name.as_bytes());
                digest.update(modified.to_le_bytes());
                if !live_lease
                    && modified.saturating_add(TEMP_RETENTION_MILLIS) <= planned_at_millis
                {
                    expired_temp.push(name);
                }
            }
        } else {
            reject_symlink(&tmp, true)?;
        }
        expired_temp.sort();
        Ok((expired_jobs, expired_temp, hex(&digest.finalize())))
    }

    fn delete_expired_jobs(&self, names: &[String]) -> Result<usize, HistoryError> {
        let root = self.root.join("jobs");
        let mut deleted = 0;
        for name in names {
            validate_cleanup_name(name, ".json")?;
            let path = root.join(name);
            if path.parent() != Some(root.as_path()) {
                return Err(HistoryError::UnsafePath {
                    path,
                    reason: "job cleanup target escaped its root".to_owned(),
                });
            }
            remove_file_durable(&path)?;
            deleted += 1;
        }
        Ok(deleted)
    }

    fn delete_expired_temp_directories(&self, names: &[String]) -> Result<usize, HistoryError> {
        if names.is_empty() {
            return Ok(0);
        }
        let root = self.root.join("tmp");
        let canonical_root = root
            .canonicalize()
            .map_err(|source| crate::error::io_error(&root, source))?;
        let mut deleted = 0;
        for name in names {
            validate_cleanup_name(name, "")?;
            if !name.starts_with("worktree-") {
                return Err(HistoryError::OperationalState(
                    "invalid temporary cleanup target".to_owned(),
                ));
            }
            let path = root.join(name);
            reject_symlink(&path, false)?;
            reject_directory(&path)?;
            let canonical = path
                .canonicalize()
                .map_err(|source| crate::error::io_error(&path, source))?;
            if canonical.parent() != Some(canonical_root.as_path()) {
                return Err(HistoryError::UnsafePath {
                    path,
                    reason: "temporary cleanup target escaped its root".to_owned(),
                });
            }
            let checkout = canonical.join("checkout");
            if checkout.exists() {
                let output = Command::new("git")
                    .args(["-C"])
                    .arg(&self.repository_root)
                    .args(["worktree", "remove", "--force", "--"])
                    .arg(&checkout)
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .output()
                    .map_err(|source| crate::error::io_error(&checkout, source))?;
                if !output.status.success() {
                    return Err(HistoryError::WorktreeCleanup(
                        String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                    ));
                }
            }
            if canonical.exists() {
                fs::remove_dir_all(&canonical)
                    .map_err(|source| crate::error::io_error(&canonical, source))?;
            }
            deleted += 1;
        }
        Ok(deleted)
    }

    fn list_unlocked_for_gc(&self) -> Result<Vec<crate::PublishedVersion>, HistoryError> {
        // The exclusive maintenance guard held by the caller is stronger than an activity guard.
        self.list_without_activity(None)
    }
}

fn validate_root_listing(
    roots: &[prolly::NamedRootManifest],
) -> Result<Vec<crate::CommitId>, HistoryError> {
    let mut realization_roots = BTreeMap::<String, BTreeSet<Vec<u8>>>::new();
    let mut preferred_commits = Vec::new();
    for root in roots {
        if root.name == STORE_FORMAT_ROOT {
            continue;
        }
        let segments = prolly::decode_segments(&root.name).map_err(|error| {
            HistoryError::CorruptHistory(format!("malformed named root: {error}"))
        })?;
        let valid = match segments.as_slice() {
            [compass, version, kind, id, root_kind]
                if compass == b"compass" && version == b"v1" && kind == b"version" =>
            {
                let parsed = std::str::from_utf8(id)
                    .ok()
                    .and_then(|value| value.parse::<RealizationId>().ok());
                if let Some(parsed) = parsed.as_ref()
                    && REALIZATION_ROOT_KINDS.contains(&root_kind.as_slice())
                {
                    realization_roots
                        .entry(parsed.as_hex())
                        .or_default()
                        .insert(root_kind.clone());
                    true
                } else {
                    false
                }
            }
            [compass, version, kind, commit]
                if compass == b"compass" && version == b"v1" && kind == b"preferred" =>
            {
                let parsed = std::str::from_utf8(commit)
                    .ok()
                    .and_then(|value| value.parse::<crate::CommitId>().ok());
                if let Some(parsed) = parsed {
                    preferred_commits.push(parsed);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if !valid {
            return Err(HistoryError::CorruptHistory(format!(
                "unsupported named root {}",
                hex(&root.name)
            )));
        }
    }
    let expected = REALIZATION_ROOT_KINDS
        .iter()
        .map(|kind| kind.to_vec())
        .collect::<BTreeSet<_>>();
    for (id, kinds) in realization_roots {
        if kinds != expected {
            return Err(HistoryError::CorruptHistory(format!(
                "realization {id} does not have exactly six catalog roots"
            )));
        }
    }
    preferred_commits.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    preferred_commits.dedup();
    Ok(preferred_commits)
}

fn roots_for_realizations(ids: &[RealizationId]) -> Vec<Vec<u8>> {
    ids.iter()
        .flat_map(|id| {
            REALIZATION_ROOT_KINDS
                .iter()
                .map(move |kind| version_root_name(id, kind))
        })
        .collect()
}

fn digest_roots(roots: &[prolly::NamedRootManifest]) -> Result<String, HistoryError> {
    let mut digest = Sha256::new();
    for root in roots {
        digest.update((root.name.len() as u64).to_le_bytes());
        digest.update(&root.name);
        let manifest = root.manifest.to_bytes()?;
        digest.update((manifest.len() as u64).to_le_bytes());
        digest.update(manifest);
    }
    Ok(hex(&digest.finalize()))
}

fn digest_node_plan(plan: &prolly::GcPlan) -> String {
    let mut digest = Sha256::new();
    digest.update(stable_usize(plan.candidate_nodes).to_le_bytes());
    digest.update(stable_usize(plan.reclaimable_nodes).to_le_bytes());
    digest.update(stable_usize(plan.reclaimable_bytes).to_le_bytes());
    for cid in &plan.reclaimable_cids {
        digest.update(cid.as_bytes());
    }
    hex(&digest.finalize())
}

fn stable_usize(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn validate_cleanup_name(name: &str, suffix: &str) -> Result<(), HistoryError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        || (!suffix.is_empty() && !name.ends_with(suffix))
    {
        return Err(HistoryError::OperationalState(
            "invalid cleanup basename".to_owned(),
        ));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}
