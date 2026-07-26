use std::collections::HashMap;
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{now_millis, operational_root};
use crate::durable::{read_json_bounded, remove_file_durable, write_json_atomic};
use crate::leases;
use crate::store::{create_owner_dir, reject_directory, reject_symlink};
use crate::validate::exceeds_limit;
use crate::{
    BuildProfile, CommitId, HistoryError, LeaseGuard, MAX_DIAGNOSTIC_BYTES, RealizationId,
    Repository,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Building,
    Validating,
    Published,
    Failed,
    Incomplete,
}

impl JobState {
    #[must_use]
    pub fn terminal(self) -> bool {
        matches!(self, Self::Published | Self::Failed | Self::Incomplete)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JobRecord {
    pub id: String,
    pub commit: CommitId,
    pub profile: BuildProfile,
    pub profile_digest: String,
    pub rebuild: bool,
    pub replace_corrupt: bool,
    pub resolved_fingerprint: Option<String>,
    pub state: JobState,
    pub attempts: u32,
    pub diagnostic: Option<String>,
    pub candidate_realization: Option<RealizationId>,
    pub observed_preferred: Option<RealizationId>,
    pub preferred: Option<bool>,
    pub lease_generation: u64,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
}

#[derive(Clone)]
pub struct ClaimedJob {
    record: JobRecord,
    lease: LeaseGuard,
}

impl Deref for ClaimedJob {
    type Target = JobRecord;

    fn deref(&self) -> &Self::Target {
        &self.record
    }
}

impl ClaimedJob {
    #[must_use]
    pub fn lease(&self) -> &LeaseGuard {
        &self.lease
    }
}

#[derive(Clone, Debug)]
pub struct JobRequest {
    pub commit: CommitId,
    pub profile: BuildProfile,
}

pub struct HistoryQueue {
    root: PathBuf,
    jobs: PathBuf,
    latest: PathBuf,
    leases: PathBuf,
    lock_root: PathBuf,
}

#[derive(Deserialize, Serialize)]
struct LatestJobPointer {
    id: String,
}

#[derive(Deserialize, Serialize)]
struct LatestIndexState {
    complete: bool,
}

impl HistoryQueue {
    /// Open or create operational queue directories below a protected Compass root.
    pub fn open(root: &Path) -> Result<Self, HistoryError> {
        create_owner_dir(root)?;
        let jobs = root.join("jobs");
        let latest = root.join("latest");
        let leases = root.join("leases");
        let lock_root = root.join("locks");
        create_owner_dir(&jobs)?;
        create_owner_dir(&latest)?;
        create_owner_dir(&leases)?;
        create_owner_dir(&lock_root)?;
        let queue = Self {
            root: root.to_path_buf(),
            jobs,
            latest,
            leases,
            lock_root,
        };
        queue.ensure_latest_index()?;
        Ok(queue)
    }

    pub fn for_repository(repository: &Repository) -> Result<Self, HistoryError> {
        Self::open(&operational_root(repository))
    }

    /// Open an existing queue without creating operational paths.
    pub fn open_existing(repository: &Repository) -> Result<Option<Self>, HistoryError> {
        let root = operational_root(repository);
        Self::open_root_existing(&root)
    }

    pub(crate) fn open_root_existing(root: &Path) -> Result<Option<Self>, HistoryError> {
        let root = root.to_path_buf();
        if !root.exists() {
            reject_symlink(&root, true)?;
            return Ok(None);
        }
        reject_directory(&root)?;
        let jobs = root.join("jobs");
        if !jobs.exists() {
            reject_symlink(&jobs, true)?;
            return Ok(None);
        }
        let leases = root.join("leases");
        let lock_root = root.join("locks");
        let latest = root.join("latest");
        reject_directory(&jobs)?;
        if latest.exists() {
            reject_directory(&latest)?;
        } else {
            reject_symlink(&latest, true)?;
        }
        reject_directory(&leases)?;
        reject_directory(&lock_root)?;
        Ok(Some(Self {
            root,
            jobs,
            latest,
            leases,
            lock_root,
        }))
    }

    pub fn enqueue(&self, request: JobRequest) -> Result<String, HistoryError> {
        self.enqueue_inner(request, false, false)
    }

    pub fn enqueue_rebuild(
        &self,
        request: JobRequest,
        replace_corrupt: bool,
    ) -> Result<String, HistoryError> {
        self.enqueue_inner(request, true, replace_corrupt)
    }

    fn enqueue_inner(
        &self,
        request: JobRequest,
        rebuild: bool,
        replace_corrupt: bool,
    ) -> Result<String, HistoryError> {
        let _lock = QueueLock::acquire(&self.lock_root)?;
        let profile_digest = hex(&request.profile.digest()?);
        let existing_jobs = self.list_unlocked()?;
        if !rebuild
            && let Some(existing) = existing_jobs.iter().find(|job| {
                job.commit == request.commit
                    && job.profile_digest == profile_digest
                    && !job.state.terminal()
            })
        {
            return Ok(existing.id.clone());
        }
        // The wall clock has millisecond resolution and can repeat or move backwards. Advancing
        // past the latest durable enqueue timestamp gives claim_next a stable FIFO order.
        let created_at_millis = existing_jobs
            .iter()
            .map(|job| job.created_at_millis)
            .max()
            .map_or_else(now_millis, |latest| {
                now_millis().max(latest.saturating_add(1))
            });
        let id = (0..1024)
            .map(|_| job_id(&request.commit, &profile_digest, created_at_millis))
            .find(|candidate| !self.jobs.join(format!("{candidate}.json")).exists())
            .ok_or_else(|| {
                HistoryError::OperationalState("could not allocate a unique job ID".to_owned())
            })?;
        let record = JobRecord {
            id: id.clone(),
            commit: request.commit,
            profile: request.profile,
            profile_digest,
            rebuild,
            replace_corrupt,
            resolved_fingerprint: None,
            state: JobState::Queued,
            attempts: 0,
            diagnostic: None,
            candidate_realization: None,
            observed_preferred: None,
            preferred: None,
            lease_generation: 0,
            created_at_millis,
            updated_at_millis: created_at_millis,
        };
        self.write_job(&record)?;
        Ok(id)
    }

    pub fn get(&self, id: &str) -> Result<Option<JobRecord>, HistoryError> {
        let path = self.job_path(id)?;
        if !path.exists() {
            reject_symlink(&path, true)?;
            return Ok(None);
        }
        self.read_job(&path).map(Some)
    }

    pub fn list(&self) -> Result<Vec<JobRecord>, HistoryError> {
        let _lock = QueueLock::acquire(&self.lock_root)?;
        self.list_unlocked()
    }

    /// Load only the latest indexed job for each requested commit.
    ///
    /// Legacy queues are scanned once to populate the derived index. Subsequent calls read one
    /// pointer per requested commit, and a damaged or stale index is repaired from job records.
    pub fn latest_for_commits(&self, commits: &[CommitId]) -> Result<Vec<JobRecord>, HistoryError> {
        if self.latest_index_complete()
            && let Some(jobs) = self.read_latest_index(commits)
        {
            return Ok(jobs);
        }

        let latest = self.list_unlocked()?.into_iter().fold(
            HashMap::<CommitId, JobRecord>::new(),
            |mut latest, job| {
                let replace = latest
                    .get(&job.commit)
                    .is_none_or(|current| job_is_newer(&job, current));
                if replace {
                    latest.insert(job.commit.clone(), job);
                }
                latest
            },
        );
        Ok(commits
            .iter()
            .filter_map(|commit| latest.get(commit).cloned())
            .collect())
    }

    fn read_latest_index(&self, commits: &[CommitId]) -> Option<Vec<JobRecord>> {
        if !self.latest.is_dir() {
            return None;
        }
        let mut jobs = Vec::with_capacity(commits.len());
        for commit in commits {
            let path = self.latest.join(format!("{commit}.json"));
            if !path.exists() {
                if reject_symlink(&path, true).is_err() {
                    return None;
                }
                continue;
            }
            let pointer: LatestJobPointer = read_json_bounded(&path).ok()?;
            let job = self.get(&pointer.id).ok()??;
            if &job.commit != commit {
                return None;
            }
            jobs.push(job);
        }
        Some(jobs)
    }

    pub fn claim_next(&self) -> Result<Option<ClaimedJob>, HistoryError> {
        let _lock = QueueLock::acquire(&self.lock_root)?;
        let mut jobs = self.list_unlocked()?;
        jobs.sort_by_key(|job| (job.created_at_millis, job.id.clone()));
        for mut job in jobs {
            let lease_path = self.lease_path(&job);
            let claimable = job.state == JobState::Queued
                || (matches!(job.state, JobState::Building | JobState::Validating)
                    && leases::expired(&lease_path)?);
            if !claimable {
                continue;
            }
            let Some(lease) = leases::claim(&lease_path)? else {
                continue;
            };
            job.state = JobState::Building;
            job.attempts = job.attempts.saturating_add(1);
            job.lease_generation = lease.generation();
            job.updated_at_millis = now_millis();
            job.diagnostic = None;
            self.write_job(&job)?;
            return Ok(Some(ClaimedJob { record: job, lease }));
        }
        Ok(None)
    }

    /// Claim one exact queued attempt, or join its still-live lease by returning `None`.
    pub fn claim_or_join(&self, id: &str) -> Result<Option<ClaimedJob>, HistoryError> {
        let _lock = QueueLock::acquire(&self.lock_root)?;
        let mut job = self
            .get(id)?
            .ok_or_else(|| HistoryError::OperationalState("queued job disappeared".to_owned()))?;
        if job.state.terminal() {
            return Ok(None);
        }
        let lease_path = self.lease_path(&job);
        if matches!(job.state, JobState::Building | JobState::Validating)
            && !leases::expired(&lease_path)?
        {
            return Ok(None);
        }
        let Some(lease) = leases::claim(&lease_path)? else {
            return Ok(None);
        };
        job.state = JobState::Building;
        job.attempts = job.attempts.saturating_add(1);
        job.lease_generation = lease.generation();
        job.updated_at_millis = now_millis();
        job.diagnostic = None;
        self.write_job(&job)?;
        Ok(Some(ClaimedJob { record: job, lease }))
    }

    pub fn transition(
        &self,
        claimed: &ClaimedJob,
        state: JobState,
        diagnostic: Option<&str>,
    ) -> Result<JobRecord, HistoryError> {
        let _lock = QueueLock::acquire(&self.lock_root)?;
        leases::validate(&claimed.lease)?;
        let mut current = self
            .get(&claimed.id)?
            .ok_or_else(|| HistoryError::OperationalState("claimed job disappeared".to_owned()))?;
        if current.lease_generation != claimed.lease.generation()
            || !valid_transition(current.state, state)
        {
            return Err(HistoryError::OperationalState(format!(
                "invalid or stale job transition {:?} -> {:?}",
                current.state, state
            )));
        }
        current.state = state;
        current.updated_at_millis = now_millis();
        current.diagnostic = diagnostic.map(redact_diagnostic);
        self.write_job(&current)?;
        Ok(current)
    }

    pub fn annotate(
        &self,
        claimed: &ClaimedJob,
        resolved_fingerprint: Option<String>,
        candidate_realization: Option<RealizationId>,
        observed_preferred: Option<RealizationId>,
    ) -> Result<JobRecord, HistoryError> {
        let _lock = QueueLock::acquire(&self.lock_root)?;
        leases::validate(&claimed.lease)?;
        let mut current = self
            .get(&claimed.id)?
            .ok_or_else(|| HistoryError::OperationalState("claimed job disappeared".to_owned()))?;
        if current.lease_generation != claimed.lease.generation() || current.state.terminal() {
            return Err(HistoryError::OperationalState(
                "late worker cannot annotate a job".to_owned(),
            ));
        }
        if resolved_fingerprint.is_some() {
            current.resolved_fingerprint = resolved_fingerprint;
        }
        if candidate_realization.is_some() {
            current.candidate_realization = candidate_realization;
        }
        if observed_preferred.is_some() {
            current.observed_preferred = observed_preferred;
        }
        current.updated_at_millis = now_millis();
        self.write_job(&current)?;
        Ok(current)
    }

    pub fn finish(
        &self,
        claimed: &ClaimedJob,
        state: JobState,
        preferred: Option<bool>,
        diagnostic: Option<&str>,
    ) -> Result<JobRecord, HistoryError> {
        let _lock = QueueLock::acquire(&self.lock_root)?;
        leases::validate(&claimed.lease)?;
        let mut record = self
            .get(&claimed.id)?
            .ok_or_else(|| HistoryError::OperationalState("claimed job disappeared".to_owned()))?;
        if record.lease_generation != claimed.lease.generation()
            || !valid_transition(record.state, state)
        {
            return Err(HistoryError::OperationalState(format!(
                "invalid or stale job transition {:?} -> {:?}",
                record.state, state
            )));
        }
        record.state = state;
        record.updated_at_millis = now_millis();
        record.diagnostic = diagnostic.map(redact_diagnostic);
        record.preferred = preferred;
        self.write_job(&record)?;
        leases::release(&claimed.lease)?;
        Ok(record)
    }

    pub fn heartbeat(&self, claimed: &ClaimedJob) -> Result<(), HistoryError> {
        let _lock = QueueLock::acquire(&self.lock_root)?;
        leases::heartbeat(&claimed.lease)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn list_unlocked(&self) -> Result<Vec<JobRecord>, HistoryError> {
        let mut entries = fs::read_dir(&self.jobs)
            .map_err(|source| crate::error::io_error(&self.jobs, source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| crate::error::io_error(&self.jobs, source))?;
        entries.sort_by_key(fs::DirEntry::file_name);
        entries
            .into_iter()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "json")
            })
            .map(|entry| self.read_job(&entry.path()))
            .collect()
    }

    fn read_job(&self, path: &Path) -> Result<JobRecord, HistoryError> {
        let job: JobRecord = read_json_bounded(path)?;
        let expected_name = format!("{}.json", job.id);
        if path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str())
            || job.id.len() != 64
            || !job
                .id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || job.profile_digest != hex(&job.profile.digest()?)
            || job.replace_corrupt && !job.rebuild
            || job.created_at_millis > job.updated_at_millis
            || job
                .diagnostic
                .as_ref()
                .is_some_and(|value| exceeds_limit(value.len() as u64, MAX_DIAGNOSTIC_BYTES as u64))
            || (job.state == JobState::Queued && job.lease_generation != 0)
            || (job.state != JobState::Queued && job.lease_generation == 0)
        {
            return Err(HistoryError::OperationalState(format!(
                "{} contains an inconsistent job record",
                path.display()
            )));
        }
        Ok(job)
    }

    fn write_job(&self, job: &JobRecord) -> Result<(), HistoryError> {
        let restore_complete = self.invalidate_latest_index()?;
        write_json_atomic(&self.job_path(&job.id)?, job)?;
        // Latest-job pointers are a repairable cache. Once the authoritative record is durable,
        // cache maintenance must not make a state transition appear to have failed or retain a
        // worker lease that should have been released.
        let _index_result = self.update_latest_index(job, restore_complete);
        Ok(())
    }

    pub(crate) fn invalidate_latest_index(&self) -> Result<bool, HistoryError> {
        let was_complete = self.latest_index_complete();
        // This must become durable before the authoritative job write. If the process stops
        // after that write but before its pointer is published, readers will scan job records
        // instead of trusting a stale complete index.
        self.write_latest_index_state(false)?;
        Ok(was_complete)
    }

    fn update_latest_index(
        &self,
        job: &JobRecord,
        restore_complete: bool,
    ) -> Result<(), HistoryError> {
        create_owner_dir(&self.latest)?;
        let pointer_path = self.latest.join(format!("{}.json", job.commit));
        let replace = if pointer_path.exists() {
            let pointer: LatestJobPointer = read_json_bounded(&pointer_path)?;
            self.get(&pointer.id)?
                .is_none_or(|current| job_is_newer(job, &current))
        } else {
            reject_symlink(&pointer_path, true)?;
            true
        };
        if replace {
            write_json_atomic(&pointer_path, &LatestJobPointer { id: job.id.clone() })?;
        }
        if restore_complete {
            self.write_latest_index_state(true)?;
        }
        Ok(())
    }

    pub(crate) fn ensure_latest_index(&self) -> Result<(), HistoryError> {
        if self.latest_index_complete() {
            return Ok(());
        }
        let _lock = QueueLock::acquire(&self.lock_root)?;
        if self.latest_index_complete() {
            return Ok(());
        }
        self.rebuild_latest_index_from_jobs()
    }

    pub(crate) fn delete_job_records(&self, names: &[String]) -> Result<usize, HistoryError> {
        let paths = names
            .iter()
            .map(|name| {
                let id = name.strip_suffix(".json").ok_or_else(|| {
                    HistoryError::OperationalState("invalid job cleanup target".to_owned())
                })?;
                let path = self.job_path(id)?;
                if path.file_name().and_then(|value| value.to_str()) != Some(name) {
                    return Err(HistoryError::OperationalState(
                        "invalid job cleanup target".to_owned(),
                    ));
                }
                Ok(path)
            })
            .collect::<Result<Vec<_>, HistoryError>>()?;
        let _lock = QueueLock::acquire(&self.lock_root)?;
        self.invalidate_latest_index()?;
        for path in &paths {
            remove_file_durable(path)?;
        }
        self.rebuild_latest_index_from_jobs()?;
        Ok(paths.len())
    }

    fn rebuild_latest_index_from_jobs(&self) -> Result<(), HistoryError> {
        let latest = self.list_unlocked()?.into_iter().fold(
            HashMap::<CommitId, JobRecord>::new(),
            |mut latest, job| {
                let replace = latest
                    .get(&job.commit)
                    .is_none_or(|current| job_is_newer(&job, current));
                if replace {
                    latest.insert(job.commit.clone(), job);
                }
                latest
            },
        );
        self.rebuild_latest_index(&latest)
    }

    fn rebuild_latest_index(
        &self,
        latest: &HashMap<CommitId, JobRecord>,
    ) -> Result<(), HistoryError> {
        self.write_latest_index_state(false)?;
        create_owner_dir(&self.latest)?;
        for job in latest.values() {
            write_json_atomic(
                &self.latest.join(format!("{}.json", job.commit)),
                &LatestJobPointer { id: job.id.clone() },
            )?;
        }
        for entry in fs::read_dir(&self.latest)
            .map_err(|source| crate::error::io_error(&self.latest, source))?
        {
            let entry = entry.map_err(|source| crate::error::io_error(&self.latest, source))?;
            let path = entry.path();
            let Some(commit) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<CommitId>().ok())
            else {
                continue;
            };
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
                && !latest.contains_key(&commit)
            {
                remove_file_durable(&path)?;
            }
        }
        self.write_latest_index_state(true)
    }

    fn latest_index_complete(&self) -> bool {
        let path = self.latest_index_state_path();
        path.exists()
            && read_json_bounded::<LatestIndexState>(&path).is_ok_and(|state| state.complete)
    }

    fn write_latest_index_state(&self, complete: bool) -> Result<(), HistoryError> {
        write_json_atomic(
            &self.latest_index_state_path(),
            &LatestIndexState { complete },
        )
    }

    fn latest_index_state_path(&self) -> PathBuf {
        self.root.join("latest-index.json")
    }

    fn job_path(&self, id: &str) -> Result<PathBuf, HistoryError> {
        if id.len() != 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(HistoryError::OperationalState("invalid job ID".to_owned()));
        }
        Ok(self.jobs.join(format!("{id}.json")))
    }

    fn lease_path(&self, job: &JobRecord) -> PathBuf {
        self.leases
            .join(format!("{}-{}.lease", job.commit, job.profile_digest))
    }
}

fn job_is_newer(candidate: &JobRecord, current: &JobRecord) -> bool {
    (
        candidate.updated_at_millis,
        candidate.created_at_millis,
        &candidate.id,
    ) >= (
        current.updated_at_millis,
        current.created_at_millis,
        &current.id,
    )
}

fn valid_transition(old: JobState, new: JobState) -> bool {
    matches!(
        (old, new),
        (JobState::Building, JobState::Validating)
            | (
                JobState::Building | JobState::Validating,
                JobState::Failed | JobState::Incomplete
            )
            | (JobState::Validating, JobState::Published)
    )
}

fn redact_diagnostic(value: &str) -> String {
    let mut redacted = value.to_owned();
    for (name, secret) in std::env::vars().filter(|(name, value)| {
        !value.is_empty()
            && ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"]
                .iter()
                .any(|needle| name.to_ascii_uppercase().contains(needle))
    }) {
        let _ = name;
        redacted = redacted.replace(&secret, "[REDACTED]");
    }
    if exceeds_limit(redacted.len() as u64, MAX_DIAGNOSTIC_BYTES as u64) {
        let mut end = MAX_DIAGNOSTIC_BYTES;
        while !redacted.is_char_boundary(end) {
            end -= 1;
        }
        redacted.truncate(end);
    }
    redacted
}

fn job_id(commit: &CommitId, profile_digest: &str, created_at_millis: u64) -> String {
    let mut nonce = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let mut digest = Sha256::new();
    digest.update(commit.as_str().as_bytes());
    digest.update(profile_digest.as_bytes());
    digest.update(created_at_millis.to_le_bytes());
    digest.update(nonce);
    hex(&digest.finalize())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

struct QueueLock {
    path: PathBuf,
}

impl QueueLock {
    fn acquire(root: &Path) -> Result<Self, HistoryError> {
        let path = root.join("queue.lock");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match fs::create_dir(&path) {
                Ok(()) => {
                    create_owner_dir(&path)?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    match fs::symlink_metadata(&path) {
                        Ok(metadata) if metadata.file_type().is_symlink() => {
                            return Err(HistoryError::UnsafePath {
                                path,
                                reason: "symbolic queue locks are not allowed".to_owned(),
                            });
                        }
                        Ok(metadata) if !metadata.is_dir() => {
                            return Err(HistoryError::UnsafePath {
                                path,
                                reason: "queue lock is not a directory".to_owned(),
                            });
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(source) => return Err(crate::error::io_error(&path, source)),
                    }
                    if path
                        .metadata()
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(120))
                    {
                        fs::remove_dir(&path)
                            .map_err(|source| crate::error::io_error(&path, source))?;
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Err(HistoryError::LockTimeout {
                            kind: "queue",
                            path,
                        });
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(source) => return Err(crate::error::io_error(&path, source)),
            }
        }
    }
}

impl Drop for QueueLock {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir(&self.path);
    }
}
