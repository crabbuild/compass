use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const BACKUP_ARTIFACTS: &[&str] = &[
    "graph.json",
    "program.json",
    "GRAPH_REPORT.md",
    "orientation.json",
    "labels.json",
    "analysis.json",
    "manifest.json",
    "semantic-marker.json",
    "cost.json",
];
const BACKUP_COMPLETE: &str = "backup-complete.json";
const BACKUP_COMPLETE_SCHEMA: &str = "compass.backup-complete/1";
const BACKUP_LOCK: &str = ".compass-backup.lock";
const BACKUP_STAGING_PREFIX: &str = ".compass-backup-staging-";
const MAX_BACKUP_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_BACKUP_CANDIDATES: usize = 100;
const MAX_BACKUP_ROOT_ENTRIES: usize = 4_096;
const HASH_BUFFER_BYTES: usize = 1024 * 1024;
const MAX_LABELS_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BACKUP_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_BACKUP_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const BACKUP_LOCK_WAIT: Duration = Duration::from_secs(10);
const BACKUP_LOCK_RETRY: Duration = Duration::from_millis(10);
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct BackupSeal {
    bytes: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct BackupManifest {
    schema: String,
    artifacts: BTreeMap<String, BackupSeal>,
}

#[derive(Debug)]
struct BackupLock {
    file: File,
}

impl BackupLock {
    fn acquire(backup_root: &Path) -> std::io::Result<Self> {
        Self::acquire_with_timeout(backup_root, BACKUP_LOCK_WAIT)
    }

    fn acquire_with_timeout(backup_root: &Path, timeout: Duration) -> std::io::Result<Self> {
        let path = backup_root.join(BACKUP_LOCK);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { file }),
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(BACKUP_LOCK_RETRY);
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("timed out acquiring backup lock {}", path.display()),
                    ));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error),
            }
        }
    }
}

impl Drop for BackupLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackupResult {
    pub path: Option<PathBuf>,
    pub message: Option<String>,
    pub warning: Option<String>,
}

/// Snapshot non-regenerable graph artifacts before an overwrite.
///
/// Failures are reported but deliberately never block the graph write, matching
/// Compass's recovery contract.
#[must_use]
pub fn backup_if_protected(output_dir: &Path) -> BackupResult {
    backup_if_protected_to(output_dir, output_dir)
}

/// Snapshot protected artifacts from `source_dir` beneath `backup_root`.
///
/// Managed graph builds keep their authoritative artifacts in immutable
/// snapshot directories. This split form lets an updater read that active
/// snapshot while placing the recovery copy in the mutable public output
/// container.
#[must_use]
pub fn backup_if_protected_to(source_dir: &Path, backup_root: &Path) -> BackupResult {
    backup_if_protected_to_with_copy(source_dir, backup_root, |source, destination, expected| {
        copy_artifact_bounded(source, destination, expected.bytes)
    })
}

fn backup_if_protected_to_with_copy<F>(
    source_dir: &Path,
    backup_root: &Path,
    mut copy: F,
) -> BackupResult
where
    F: FnMut(&Path, &Path, &BackupSeal) -> std::io::Result<()>,
{
    if std::env::var_os("COMPASS_NO_BACKUP").is_some_and(|value| !value.is_empty()) {
        return BackupResult::default();
    }
    let graph_path = source_dir.join("graph.json");
    if !is_regular_file(&graph_path) {
        return BackupResult::default();
    }
    let semantic = is_regular_file(&source_dir.join("semantic-marker.json"));
    let curated = match labels_are_curated(&source_dir.join("labels.json")) {
        Ok(curated) => curated,
        Err(_) if semantic => false,
        Err(error) => return backup_warning(error),
    };
    if !semantic && !curated {
        return BackupResult::default();
    }
    let reason = match (semantic, curated) {
        (true, true) => "semantic+curated",
        (true, false) => "semantic",
        (false, true) => "curated",
        (false, false) => return BackupResult::default(),
    };
    let inventory = match artifact_inventory(source_dir) {
        Ok(inventory) => inventory,
        Err(error) => return backup_warning(error),
    };
    let manifest = BackupManifest {
        schema: BACKUP_COMPLETE_SCHEMA.to_owned(),
        artifacts: inventory,
    };
    let date = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .date()
        .to_string();

    if let Err(error) = fs::create_dir_all(backup_root) {
        return backup_warning(error);
    }
    let backup_lock = match BackupLock::acquire(backup_root) {
        Ok(lock) => lock,
        Err(error) => return backup_warning(error),
    };
    if let Err(error) = reclaim_stale_staging_directories(backup_root, &backup_lock) {
        return backup_warning(error);
    }
    if let Some(existing) = find_completed_backup(backup_root, &date, &manifest) {
        return BackupResult {
            path: Some(existing),
            ..BackupResult::default()
        };
    }

    let staging = match create_staging_directory(backup_root, &backup_lock) {
        Ok(staging) => staging,
        Err(error) => return backup_warning(error),
    };

    let publication = (|| -> Result<PathBuf, String> {
        for (artifact, expected) in &manifest.artifacts {
            copy(
                &source_dir.join(artifact),
                &staging.join(artifact),
                expected,
            )
            .map_err(|error| format!("copy {artifact}: {error}"))?;
        }
        verify_artifacts(&staging, &manifest)?;
        write_completion_manifest(&staging.join(BACKUP_COMPLETE), &manifest)
            .map_err(|error| format!("write completion manifest: {error}"))?;
        verify_completed_backup(&staging, &manifest)?;

        for candidate in backup_candidates(backup_root, &date, &manifest) {
            if candidate.exists() {
                continue;
            }
            match fs::rename(&staging, &candidate) {
                Ok(()) => return Ok(candidate),
                Err(_error) if candidate.exists() => continue,
                Err(error) => return Err(format!("publish backup: {error}")),
            }
        }
        Err(format!(
            "no free backup destination after {MAX_BACKUP_CANDIDATES} attempts"
        ))
    })();

    match publication {
        Ok(path) => BackupResult {
            message: Some(format!(
                "[compass] backed up {reason} graph ({} files) -> {}/",
                manifest.artifacts.len(),
                path.file_name()
                    .map_or_else(|| date.clone(), |name| name.to_string_lossy().into_owned())
            )),
            path: Some(path),
            warning: None,
        },
        Err(error) => {
            let cleanup = remove_validated_staging_directory(&staging);
            match cleanup {
                Ok(()) => backup_warning(error),
                Err(cleanup_error) => backup_warning(format!(
                    "{error}; could not reclaim staging directory: {cleanup_error}"
                )),
            }
        }
    }
}

fn artifact_inventory(directory: &Path) -> Result<BTreeMap<String, BackupSeal>, String> {
    artifact_inventory_with_limits(
        directory,
        MAX_BACKUP_ARTIFACT_BYTES,
        MAX_BACKUP_TOTAL_BYTES,
        file_seal,
    )
}

fn artifact_inventory_with_limits<F>(
    directory: &Path,
    artifact_cap: u64,
    total_cap: u64,
    mut seal_file: F,
) -> Result<BTreeMap<String, BackupSeal>, String>
where
    F: FnMut(&Path, u64) -> std::io::Result<BackupSeal>,
{
    let mut total = 0_u64;
    let mut candidates = Vec::new();
    for artifact in BACKUP_ARTIFACTS {
        let entry = {
            let path = directory.join(artifact);
            is_regular_file(&path).then_some(path)
        };
        let Some(path) = entry else {
            continue;
        };
        let size = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect {artifact}: {error}"))?
            .len();
        if size > artifact_cap {
            return Err(format!(
                "{artifact} is {size} bytes; maximum is {artifact_cap}"
            ));
        }
        total = total
            .checked_add(size)
            .ok_or_else(|| "backup artifact byte count overflow".to_owned())?;
        if total > total_cap {
            return Err(format!(
                "backup artifact set is {total} bytes; maximum is {total_cap}"
            ));
        }
        candidates.push((*artifact, path));
    }
    let mut inventory = BTreeMap::new();
    let mut sealed_total = 0_u64;
    for (artifact, path) in candidates {
        let remaining_total = total_cap.saturating_sub(sealed_total);
        let stream_cap = artifact_cap.min(remaining_total);
        let seal = seal_file(&path, stream_cap).map_err(|error| {
            format!(
                "read {artifact} within the remaining {remaining_total}-byte aggregate limit: {error}"
            )
        })?;
        sealed_total = sealed_total
            .checked_add(seal.bytes)
            .ok_or_else(|| "backup artifact byte count overflow".to_owned())?;
        if sealed_total > total_cap {
            return Err(format!(
                "backup artifact set grew to {sealed_total} bytes; maximum is {total_cap}"
            ));
        }
        inventory.insert(artifact.to_owned(), seal);
    }
    Ok(inventory)
}

fn reclaim_stale_staging_directories(
    backup_root: &Path,
    _backup_lock: &BackupLock,
) -> std::io::Result<()> {
    let mut staging_directories = Vec::new();
    for (index, entry) in fs::read_dir(backup_root)?.enumerate() {
        if index >= MAX_BACKUP_ROOT_ENTRIES {
            return Err(std::io::Error::other(format!(
                "backup root contains more than {MAX_BACKUP_ROOT_ENTRIES} entries; refusing an unbounded staging scan"
            )));
        }
        let entry = entry?;
        if !is_owned_staging_name(&entry.file_name()) {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            return Err(std::io::Error::other(format!(
                "backup staging path is not a directory: {}",
                entry.path().display()
            )));
        }
        let directory = entry.path();
        let contents = validate_staging_contents(&directory)?;
        staging_directories.push((directory, contents));
    }
    for (staging, contents) in staging_directories {
        remove_staging_contents(&staging, contents)?;
    }
    Ok(())
}

fn remove_validated_staging_directory(staging: &Path) -> std::io::Result<()> {
    let contents = validate_staging_contents(staging)?;
    remove_staging_contents(staging, contents)
}

fn remove_staging_contents(staging: &Path, contents: Vec<PathBuf>) -> std::io::Result<()> {
    for artifact in contents {
        fs::remove_file(artifact)?;
    }
    fs::remove_dir(staging)
}

fn validate_staging_contents(staging: &Path) -> std::io::Result<Vec<PathBuf>> {
    let max_entries = BACKUP_ARTIFACTS.len() + 1;
    let mut contents = Vec::new();
    let mut total_bytes = 0_u64;
    for (index, entry) in fs::read_dir(staging)?.enumerate() {
        if index >= max_entries {
            return Err(std::io::Error::other(format!(
                "backup staging directory contains more than {max_entries} entries: {}",
                staging.display()
            )));
        }
        let entry = entry?;
        let name = entry.file_name();
        let is_completion = name == BACKUP_COMPLETE;
        if !is_completion && !BACKUP_ARTIFACTS.iter().any(|artifact| name == *artifact) {
            return Err(std::io::Error::other(format!(
                "backup staging directory contains an unexpected entry: {}",
                entry.path().display()
            )));
        }
        if !entry.file_type()?.is_file() {
            return Err(std::io::Error::other(format!(
                "backup staging entry is not a regular file: {}",
                entry.path().display()
            )));
        }
        let size = entry.metadata()?.len();
        let artifact_cap = if is_completion {
            MAX_BACKUP_MANIFEST_BYTES
        } else {
            MAX_BACKUP_ARTIFACT_BYTES
        };
        if size > artifact_cap {
            return Err(limit_error("backup staging artifact", size, artifact_cap));
        }
        if !is_completion {
            total_bytes = total_bytes
                .checked_add(size)
                .ok_or_else(|| std::io::Error::other("staging artifact byte count overflow"))?;
            if total_bytes > MAX_BACKUP_TOTAL_BYTES {
                return Err(limit_error(
                    "backup staging artifact set",
                    total_bytes,
                    MAX_BACKUP_TOTAL_BYTES,
                ));
            }
        }
        contents.push(entry.path());
    }
    Ok(contents)
}

fn is_owned_staging_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(suffix) = name.strip_prefix(BACKUP_STAGING_PREFIX) else {
        return false;
    };
    let Some((process_id, sequence)) = suffix.split_once('-') else {
        return false;
    };
    !process_id.is_empty()
        && !sequence.is_empty()
        && process_id.bytes().all(|byte| byte.is_ascii_digit())
        && sequence.bytes().all(|byte| byte.is_ascii_digit())
}

fn create_staging_directory(
    backup_root: &Path,
    _backup_lock: &BackupLock,
) -> std::io::Result<PathBuf> {
    for _ in 0..MAX_BACKUP_CANDIDATES {
        let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let staging = backup_root.join(format!(
            "{BACKUP_STAGING_PREFIX}{}-{sequence}",
            std::process::id()
        ));
        match fs::create_dir(&staging) {
            Ok(()) => return Ok(staging),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("no free backup staging directory after {MAX_BACKUP_CANDIDATES} attempts"),
    ))
}

fn find_completed_backup(
    backup_root: &Path,
    date: &str,
    expected: &BackupManifest,
) -> Option<PathBuf> {
    backup_candidates(backup_root, date, expected)
        .into_iter()
        .find(|candidate| verify_completed_backup(candidate, expected).is_ok())
}

fn backup_candidates(backup_root: &Path, date: &str, manifest: &BackupManifest) -> Vec<PathBuf> {
    let digest_prefix = manifest
        .artifacts
        .get("graph.json")
        .map_or("graph", |seal| &seal.sha256[..12]);
    std::iter::once(backup_root.join(date))
        .chain((0..MAX_BACKUP_CANDIDATES - 1).map(|index| {
            if index == 0 {
                backup_root.join(format!("{date}-{digest_prefix}"))
            } else {
                backup_root.join(format!("{date}-{digest_prefix}-{index}"))
            }
        }))
        .collect()
}

fn verify_completed_backup(directory: &Path, expected: &BackupManifest) -> Result<(), String> {
    let marker = directory.join(BACKUP_COMPLETE);
    let file = File::open(&marker).map_err(|error| format!("open completion manifest: {error}"))?;
    let size = file
        .metadata()
        .map_err(|error| format!("read completion manifest metadata: {error}"))?
        .len();
    if size > MAX_BACKUP_MANIFEST_BYTES {
        return Err(format!(
            "completion manifest is {size} bytes; maximum is {MAX_BACKUP_MANIFEST_BYTES}"
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BACKUP_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read completion manifest: {error}"))?;
    if bytes.len() as u64 > MAX_BACKUP_MANIFEST_BYTES {
        return Err(format!(
            "completion manifest grew beyond the {MAX_BACKUP_MANIFEST_BYTES}-byte maximum"
        ));
    }
    let actual: BackupManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("decode completion manifest: {error}"))?;
    if actual != *expected || actual.schema != BACKUP_COMPLETE_SCHEMA {
        return Err("completion manifest does not match the source artifact set".to_owned());
    }
    verify_artifacts(directory, expected)
}

fn write_completion_manifest(path: &Path, manifest: &BackupManifest) -> std::io::Result<()> {
    let output = OpenOptions::new().create_new(true).write(true).open(path)?;
    let mut writer = BufWriter::new(output);
    serde_json::to_writer_pretty(&mut writer, manifest)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    writer.flush()?;
    writer.get_ref().sync_all()
}

fn verify_artifacts(directory: &Path, expected: &BackupManifest) -> Result<(), String> {
    let actual = artifact_inventory(directory)?;
    if actual != expected.artifacts {
        return Err("backup artifact set or content does not match its manifest".to_owned());
    }
    Ok(())
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

fn labels_are_curated(path: &Path) -> Result<bool, String> {
    if !is_regular_file(path) {
        return Ok(false);
    }
    let bytes = read_bounded(path, MAX_LABELS_BYTES)
        .map_err(|error| format!("read labels.json: {error}"))?;
    let curated = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|labels| {
            labels.iter().any(|(community, label)| {
                label
                    .as_str()
                    .is_none_or(|label| label != format!("Community {community}"))
            })
        });
    Ok(curated)
}

fn file_seal(path: &Path, cap: u64) -> std::io::Result<BackupSeal> {
    file_seal_after_metadata(path, cap, || Ok(()))
}

fn file_seal_after_metadata<F>(
    path: &Path,
    cap: u64,
    after_metadata: F,
) -> std::io::Result<BackupSeal>
where
    F: FnOnce() -> std::io::Result<()>,
{
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    if size > cap {
        return Err(limit_error("backup artifact", size, cap));
    }
    after_metadata()?;
    let mut reader = BufReader::new(file.take(cap.saturating_add(1)));
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .ok_or_else(|| std::io::Error::other("artifact byte count overflow"))?;
        if bytes > cap {
            return Err(limit_error("backup artifact", bytes, cap));
        }
        digest.update(&buffer[..read]);
    }
    Ok(BackupSeal {
        bytes,
        sha256: format!("{:x}", digest.finalize()),
    })
}

fn copy_artifact_bounded(source: &Path, destination: &Path, cap: u64) -> std::io::Result<()> {
    copy_artifact_bounded_after_metadata(source, destination, cap, || Ok(()))
}

fn copy_artifact_bounded_after_metadata<F>(
    source: &Path,
    destination: &Path,
    cap: u64,
    after_metadata: F,
) -> std::io::Result<()>
where
    F: FnOnce() -> std::io::Result<()>,
{
    let input = File::open(source)?;
    let size = input.metadata()?.len();
    if size > cap {
        return Err(limit_error("backup artifact", size, cap));
    }
    after_metadata()?;
    let output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
    let mut reader = BufReader::new(input.take(cap.saturating_add(1)));
    let mut writer = BufWriter::new(output);
    let mut copied = 0_u64;
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .ok_or_else(|| std::io::Error::other("artifact byte count overflow"))?;
        if copied > cap {
            return Err(limit_error("backup artifact", copied, cap));
        }
        writer.write_all(&buffer[..read])?;
    }
    writer.flush()?;
    writer.get_ref().sync_all()
}

fn read_bounded(path: &Path, cap: u64) -> std::io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    if size > cap {
        return Err(limit_error("file", size, cap));
    }
    let mut bytes = Vec::new();
    file.take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(limit_error("file", bytes.len() as u64, cap));
    }
    Ok(bytes)
}

fn limit_error(kind: &str, size: u64, cap: u64) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{kind} is {size} bytes; maximum is {cap}"),
    )
}

fn backup_warning(error: impl std::fmt::Display) -> BackupResult {
    BackupResult {
        warning: Some(format!(
            "[compass] warning: backup failed ({error}) - continuing with overwrite"
        )),
        ..BackupResult::default()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn write_curated_source(directory: &Path) -> std::io::Result<()> {
        fs::write(directory.join("graph.json"), "graph")?;
        fs::write(directory.join("program.json"), "program")?;
        fs::write(directory.join("GRAPH_REPORT.md"), "report")?;
        fs::write(directory.join("labels.json"), r#"{"0":"Orders"}"#)
    }

    fn owned_staging_directories(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
        fs::read_dir(directory)?
            .filter_map(|entry| match entry {
                Ok(entry) if is_owned_staging_name(&entry.file_name()) => Some(Ok(entry.path())),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    fn contains_only_backup_lock(directory: &Path) -> std::io::Result<bool> {
        let entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        Ok(entries.len() == 1 && entries[0].file_name() == BACKUP_LOCK)
    }

    #[test]
    fn curated_backup_is_dated_deduplicated_and_complete() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        write_curated_source(directory.path())?;
        let first = backup_if_protected(directory.path());
        assert!(
            first
                .message
                .as_deref()
                .is_some_and(|message| message.contains("4 files"))
        );
        let backup = first.path.ok_or("backup path missing")?;
        assert_eq!(fs::read_to_string(backup.join("graph.json"))?, "graph");
        assert_eq!(fs::read_to_string(backup.join("program.json"))?, "program");
        assert!(backup.join(BACKUP_COMPLETE).is_file());
        let second = backup_if_protected(directory.path());
        assert_eq!(second.path.as_deref(), Some(backup.as_path()));
        assert!(second.message.is_none());
        Ok(())
    }

    #[test]
    fn partial_copy_is_not_published_or_used_for_deduplication()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = tempfile::tempdir()?;
        let backups = tempfile::tempdir()?;
        write_curated_source(source.path())?;
        let mut copies = 0;
        let failed = backup_if_protected_to_with_copy(
            source.path(),
            backups.path(),
            |source, destination, _expected| {
                copies += 1;
                if copies == 2 {
                    return Err(std::io::Error::other("injected copy failure"));
                }
                fs::copy(source, destination).map(|_| ())
            },
        );
        assert!(failed.path.is_none());
        assert!(failed.warning.is_some());
        assert!(contains_only_backup_lock(backups.path())?);

        let retried = backup_if_protected_to(source.path(), backups.path());
        let published = retried.path.ok_or("retry did not publish backup")?;
        assert!(published.join(BACKUP_COMPLETE).is_file());
        assert_eq!(
            fs::read_to_string(published.join("program.json"))?,
            "program"
        );
        Ok(())
    }

    #[test]
    fn interrupted_staging_directory_is_ignored() -> Result<(), Box<dyn std::error::Error>> {
        let source = tempfile::tempdir()?;
        let backups = tempfile::tempdir()?;
        write_curated_source(source.path())?;
        let interrupted = backups.path().join(".compass-backup-staging-old");
        fs::create_dir(&interrupted)?;
        fs::write(interrupted.join("graph.json"), "graph")?;

        let result = backup_if_protected_to(source.path(), backups.path());
        let published = result.path.ok_or("backup was not published")?;
        assert_ne!(published, interrupted);
        assert!(published.join(BACKUP_COMPLETE).is_file());
        assert!(
            interrupted
                .join(BACKUP_COMPLETE)
                .try_exists()
                .is_ok_and(|exists| !exists)
        );
        Ok(())
    }

    #[test]
    fn repeated_interrupted_staging_attempts_are_reclaimed_before_reuse()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = tempfile::tempdir()?;
        let backups = tempfile::tempdir()?;
        write_curated_source(source.path())?;

        for attempt in 0..8 {
            let backup_lock = BackupLock::acquire(backups.path())?;
            reclaim_stale_staging_directories(backups.path(), &backup_lock)?;
            let staging = create_staging_directory(backups.path(), &backup_lock)?;
            fs::write(staging.join("graph.json"), vec![0_u8; attempt + 1])?;
            drop(backup_lock);

            let owned = owned_staging_directories(backups.path())?;
            assert_eq!(owned.len(), 1, "attempt {attempt} accumulated staging");
        }

        let result = backup_if_protected_to(source.path(), backups.path());
        assert!(result.path.is_some());
        assert!(owned_staging_directories(backups.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn active_staging_is_not_reclaimed_until_its_lock_is_released()
    -> Result<(), Box<dyn std::error::Error>> {
        let backups = tempfile::tempdir()?;
        let active_lock = BackupLock::acquire(backups.path())?;
        let active_staging = create_staging_directory(backups.path(), &active_lock)?;
        let sentinel = active_staging.join("graph.json");
        fs::write(&sentinel, "active")?;

        let blocked = match BackupLock::acquire_with_timeout(backups.path(), Duration::ZERO) {
            Err(error) => error,
            Ok(_) => return Err("a concurrent backup acquired the active lock".into()),
        };
        assert_eq!(blocked.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(fs::read_to_string(&sentinel)?, "active");

        drop(active_lock);
        let replacement_lock = BackupLock::acquire(backups.path())?;
        reclaim_stale_staging_directories(backups.path(), &replacement_lock)?;
        assert!(!active_staging.exists());
        Ok(())
    }

    #[test]
    fn staging_reclamation_rejects_a_matching_non_directory()
    -> Result<(), Box<dyn std::error::Error>> {
        let backups = tempfile::tempdir()?;
        let unexpected = backups.path().join(".compass-backup-staging-123-456");
        fs::write(&unexpected, "do not delete")?;
        let backup_lock = BackupLock::acquire(backups.path())?;

        let error = match reclaim_stale_staging_directories(backups.path(), &backup_lock) {
            Err(error) => error,
            Ok(()) => return Err("a matching non-directory was accepted".into()),
        };

        assert!(error.to_string().contains("not a directory"));
        assert_eq!(fs::read_to_string(unexpected)?, "do not delete");
        Ok(())
    }

    #[test]
    fn staging_reclamation_rejects_nested_or_unexpected_contents()
    -> Result<(), Box<dyn std::error::Error>> {
        let backups = tempfile::tempdir()?;
        let nested_staging = backups.path().join(".compass-backup-staging-123-456");
        fs::create_dir(&nested_staging)?;
        fs::create_dir(nested_staging.join("graph.json"))?;
        let unexpected_staging = backups.path().join(".compass-backup-staging-123-457");
        fs::create_dir(&unexpected_staging)?;
        fs::write(unexpected_staging.join("surprise"), "do not delete")?;
        let backup_lock = BackupLock::acquire(backups.path())?;

        let error = match reclaim_stale_staging_directories(backups.path(), &backup_lock) {
            Err(error) => error,
            Ok(()) => return Err("nested or unexpected staging contents were accepted".into()),
        };

        assert!(
            error.to_string().contains("not a regular file")
                || error.to_string().contains("unexpected entry")
        );
        assert!(nested_staging.join("graph.json").is_dir());
        assert_eq!(
            fs::read_to_string(unexpected_staging.join("surprise"))?,
            "do not delete"
        );
        Ok(())
    }

    #[test]
    fn staging_validation_allows_a_manifest_beyond_the_artifact_total_limit()
    -> Result<(), Box<dyn std::error::Error>> {
        let staging = tempfile::tempdir()?;
        for artifact in ["graph.json", "program.json"] {
            let file = File::create(staging.path().join(artifact))?;
            file.set_len(MAX_BACKUP_ARTIFACT_BYTES)?;
        }
        fs::write(staging.path().join(BACKUP_COMPLETE), "{}")?;

        let contents = validate_staging_contents(staging.path())?;

        assert_eq!(contents.len(), 3);
        Ok(())
    }

    #[test]
    fn oversized_artifact_and_aggregate_sets_fail_before_copying()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = tempfile::tempdir()?;
        let backups = tempfile::tempdir()?;
        write_curated_source(source.path())?;
        OpenOptions::new()
            .write(true)
            .open(source.path().join("graph.json"))?
            .set_len(MAX_BACKUP_ARTIFACT_BYTES + 1)?;
        let oversized = backup_if_protected_to(source.path(), backups.path());
        assert!(oversized.path.is_none());
        assert!(
            oversized
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("maximum"))
        );
        assert!(fs::read_dir(backups.path())?.next().is_none());

        write_curated_source(source.path())?;
        for artifact in ["graph.json", "program.json", "GRAPH_REPORT.md"] {
            OpenOptions::new()
                .write(true)
                .open(source.path().join(artifact))?
                .set_len(6 * 1024 * 1024 * 1024)?;
        }
        let aggregate = backup_if_protected_to(source.path(), backups.path());
        assert!(aggregate.path.is_none());
        assert!(
            aggregate
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("artifact set"))
        );
        assert!(fs::read_dir(backups.path())?.next().is_none());
        Ok(())
    }

    #[test]
    fn aggregate_limit_uses_streamed_sizes_after_files_grow()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("graph.json"), b"x")?;
        fs::write(directory.path().join("program.json"), b"x")?;
        let result = artifact_inventory_with_limits(directory.path(), 4, 5, |path, cap| {
            OpenOptions::new()
                .append(true)
                .open(path)?
                .write_all(b"yz")?;
            file_seal(path, cap)
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => return Err("aggregate growth should exceed the streamed limit".into()),
        };
        assert!(
            error.contains("remaining 2-byte aggregate limit"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn artifact_growth_after_metadata_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        fs::write(&path, b"1234")?;
        let mut growth = OpenOptions::new().append(true).open(&path)?;
        let result = file_seal_after_metadata(&path, 8, move || growth.write_all(b"56789"));
        let error = match result {
            Err(error) => error,
            Ok(_) => return Err("growing artifact should exceed its seal limit".into()),
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        fs::write(&path, b"1234")?;
        let destination = directory.path().join("copied.json");
        let mut copy_growth = OpenOptions::new().append(true).open(&path)?;
        let copied = copy_artifact_bounded_after_metadata(&path, &destination, 4, move || {
            copy_growth.write_all(b"56789")
        });
        let copy_error = match copied {
            Err(error) => error,
            Ok(()) => return Err("growing artifact should exceed its copy limit".into()),
        };
        assert_eq!(copy_error.kind(), std::io::ErrorKind::InvalidData);
        Ok(())
    }

    #[test]
    fn oversized_labels_are_not_read_to_detect_curation() -> Result<(), Box<dyn std::error::Error>>
    {
        let source = tempfile::tempdir()?;
        let backups = tempfile::tempdir()?;
        write_curated_source(source.path())?;
        OpenOptions::new()
            .write(true)
            .open(source.path().join("labels.json"))?
            .set_len(MAX_LABELS_BYTES + 1)?;
        let result = backup_if_protected_to(source.path(), backups.path());
        assert!(result.path.is_none());
        assert!(
            result
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("labels.json"))
        );
        assert!(fs::read_dir(backups.path())?.next().is_none());
        Ok(())
    }
}
