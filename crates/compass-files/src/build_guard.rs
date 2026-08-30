use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::atomic::{copy_file_atomic, sync_directory};
use crate::{FileError, io_error, write_text_atomic};

const SNAPSHOTS_DIRECTORY: &str = "snapshots";
const CURRENT_SNAPSHOT: &str = "current-snapshot";
const INCOMPLETE_MARKER: &str = "build-incomplete";
const ROOT_ARTIFACTS_COMPLETE: &str = "root-artifacts-complete";
const RETAINED_COMPLETE_SNAPSHOTS: usize = 2;
const MAX_SNAPSHOT_DIRECTORY_ENTRIES: usize = 1_024;
static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Owns an unpublished output snapshot until every authoritative artifact is sealed.
#[derive(Debug)]
pub struct BuildGuard {
    output_directory: PathBuf,
    snapshot_directory: PathBuf,
    marker: PathBuf,
    committed: bool,
}

impl BuildGuard {
    pub fn begin(output_directory: &Path) -> Result<Self, FileError> {
        Self::begin_excluding(output_directory, &[])
    }

    /// Return complete published snapshots in deterministic newest-first
    /// order. In-progress directories are deliberately excluded so callers
    /// can make retention decisions without treating partial artifacts as
    /// roots.
    pub fn complete_snapshot_directories(
        output_directory: &Path,
    ) -> Result<Vec<PathBuf>, FileError> {
        let snapshots = output_directory.join(SNAPSHOTS_DIRECTORY);
        if !snapshots.is_dir() {
            return Ok(Vec::new());
        }
        let mut complete = fs::read_dir(&snapshots)
            .map_err(|source| io_error(&snapshots, source))?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && !entry.path().join(INCOMPLETE_MARKER).exists()
            })
            .map(|entry| entry.path())
            .take(MAX_SNAPSHOT_DIRECTORY_ENTRIES.saturating_add(1))
            .collect::<Vec<_>>();
        if complete.len() > MAX_SNAPSHOT_DIRECTORY_ENTRIES {
            return Err(FileError::InvalidSnapshotArtifact(snapshots));
        }
        complete.sort();
        complete.reverse();
        Ok(complete)
    }

    /// Start a snapshot without copying selected top-level artifacts from
    /// the current snapshot.
    ///
    /// Large shared sidecars can live outside immutable snapshots and be
    /// addressed by a small snapshot-local reference. Exclusions are exact
    /// file names rather than patterns so callers cannot accidentally omit an
    /// unrelated artifact family.
    pub fn begin_excluding(
        output_directory: &Path,
        excluded_artifacts: &[&str],
    ) -> Result<Self, FileError> {
        validate_exclusions(excluded_artifacts)?;
        fs::create_dir_all(output_directory)
            .map_err(|source| io_error(output_directory, source))?;
        let active = match output_directory.join(CURRENT_SNAPSHOT).try_exists() {
            Ok(true) => Some(Self::resolve_current_snapshot_directory(output_directory)?),
            Ok(false) => None,
            Err(source) => {
                return Err(io_error(output_directory.join(CURRENT_SNAPSHOT), source));
            }
        };
        let snapshots = output_directory.join(SNAPSHOTS_DIRECTORY);
        fs::create_dir_all(&snapshots).map_err(|source| io_error(&snapshots, source))?;
        let snapshot_directory = snapshots.join(snapshot_name());
        fs::create_dir(&snapshot_directory)
            .map_err(|source| io_error(&snapshot_directory, source))?;

        if let Some(active) = active {
            copy_snapshot(&active, &snapshot_directory, excluded_artifacts, true)?;
        }
        let marker = snapshot_directory.join(INCOMPLETE_MARKER);
        write_text_atomic(&marker, "1")?;
        Ok(Self {
            output_directory: output_directory.to_path_buf(),
            snapshot_directory,
            marker,
            committed: false,
        })
    }

    #[must_use]
    pub fn staging_directory(&self) -> &Path {
        &self.snapshot_directory
    }

    /// Return the snapshot selected by the current output layout.
    pub fn resolve_current_snapshot_directory(
        output_directory: &Path,
    ) -> Result<PathBuf, FileError> {
        let pointer = output_directory.join(CURRENT_SNAPSHOT);
        match fs::read_to_string(&pointer) {
            Ok(value) => {
                let snapshot = value.trim();
                let relative = Path::new(snapshot);
                if snapshot.is_empty()
                    || relative.components().count() != 1
                    || !matches!(relative.components().next(), Some(Component::Normal(_)))
                    || !snapshot.starts_with("snapshot-")
                {
                    return Err(FileError::InvalidSnapshotArtifact(pointer));
                }
                let active = output_directory.join(SNAPSHOTS_DIRECTORY).join(relative);
                if !active.is_dir() || active.join(INCOMPLETE_MARKER).exists() {
                    return Err(FileError::IncompleteBuild(active));
                }
                Ok(active)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(FileError::InvalidSnapshotArtifact(pointer))
            }
            Err(source) => Err(io_error(pointer, source)),
        }
    }

    pub fn resolve_artifact(
        output_directory: &Path,
        relative: impl AsRef<Path>,
    ) -> Result<PathBuf, FileError> {
        let relative = relative.as_ref();
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(FileError::InvalidSnapshotArtifact(relative.to_path_buf()));
        }
        Ok(Self::resolve_current_snapshot_directory(output_directory)?.join(relative))
    }

    /// Resolve a managed output artifact through its current snapshot.
    /// Standalone artifact files outside managed snapshots remain valid inputs.
    pub fn resolve_requested_artifact(path: &Path) -> Result<PathBuf, FileError> {
        let Some(name) = path.file_name() else {
            return Err(FileError::InvalidSnapshotArtifact(path.to_path_buf()));
        };
        let output_directory = path.parent().unwrap_or_else(|| Path::new("."));
        let pointer = output_directory.join(CURRENT_SNAPSHOT);
        match pointer.try_exists() {
            Ok(true) => Self::resolve_artifact(output_directory, Path::new(name)),
            Ok(false) if output_directory.join(SNAPSHOTS_DIRECTORY).is_dir() => {
                Err(FileError::InvalidSnapshotArtifact(pointer))
            }
            Ok(false) => Ok(path.to_path_buf()),
            Err(source) => Err(io_error(pointer, source)),
        }
    }

    pub fn ensure_complete(output_directory: &Path) -> Result<(), FileError> {
        let active = Self::resolve_current_snapshot_directory(output_directory)?;
        let marker = active.join(INCOMPLETE_MARKER);
        if marker.exists() {
            Err(FileError::IncompleteBuild(marker))
        } else {
            Ok(())
        }
    }

    /// Return the stable output container that owns a generated artifact.
    ///
    /// Paths inside the current immutable snapshot map back to their public
    /// output root. Standalone artifacts keep their immediate parent directory.
    #[must_use]
    pub fn output_container_for_artifact(path: &Path) -> PathBuf {
        let artifact_directory = path.parent().unwrap_or_else(|| Path::new("."));
        let Some(snapshots_directory) = artifact_directory.parent() else {
            return artifact_directory.to_path_buf();
        };
        if snapshots_directory
            .file_name()
            .and_then(|name| name.to_str())
            != Some(SNAPSHOTS_DIRECTORY)
        {
            return artifact_directory.to_path_buf();
        }
        let Some(output_directory) = snapshots_directory.parent() else {
            return artifact_directory.to_path_buf();
        };
        if Self::resolve_current_snapshot_directory(output_directory)
            .is_ok_and(|active| active == artifact_directory)
        {
            output_directory.to_path_buf()
        } else {
            artifact_directory.to_path_buf()
        }
    }

    /// Materialize selected snapshot files directly under the output root.
    ///
    /// The immutable snapshot remains authoritative for Compass-aware
    /// readers. These independently atomic copies provide stable conventional
    /// paths for browsers, scripts, archives, and other file-based consumers.
    /// Missing optional artifacts remove an older root copy. A failed or
    /// interrupted projection leaves no completion marker, so the next build
    /// performs a full repair.
    pub fn publish_root_artifacts(
        output_directory: &Path,
        artifacts: &[&str],
        refresh: bool,
    ) -> Result<(), FileError> {
        let resolve_started = Instant::now();
        validate_exclusions(artifacts)?;
        let active = Self::resolve_current_snapshot_directory(output_directory)?;
        profile_internal_duration(
            "root projection resolve active snapshot",
            resolve_started.elapsed(),
        );

        let completion_marker = output_directory.join(ROOT_ARTIFACTS_COMPLETE);
        let repair_all = refresh || !completion_marker.is_file();
        let marker_started = Instant::now();
        remove_file_if_exists(&completion_marker)?;
        profile_internal_duration(
            "root projection remove completion marker",
            marker_started.elapsed(),
        );

        for artifact in artifacts {
            let artifact_started = Instant::now();
            let source = active.join(artifact);
            let destination = output_directory.join(artifact);
            if source.is_file() {
                if repair_all || !destination.is_file() {
                    copy_file_atomic(&source, &destination)?;
                }
            } else {
                remove_file_if_exists(&destination)?;
            }
            profile_internal_duration(
                &format!("root projection artifact {artifact}"),
                artifact_started.elapsed(),
            );
        }

        let snapshot = active
            .file_name()
            .ok_or_else(|| FileError::InvalidSnapshotArtifact(active.clone()))?
            .to_string_lossy();
        let completion_started = Instant::now();
        write_text_atomic(completion_marker, &snapshot)?;
        profile_internal_duration(
            "root projection publish completion marker",
            completion_started.elapsed(),
        );
        Ok(())
    }

    pub fn commit(self) -> Result<(), FileError> {
        self.commit_with_artifacts(&[])
    }

    pub fn commit_with_artifacts(self, artifacts: &[&str]) -> Result<(), FileError> {
        self.commit_with_artifacts_inner(artifacts, true)
    }

    /// Publish a snapshot whose listed artifacts were already written by an
    /// atomic writer in this snapshot. The files are still checked before
    /// publication, but are not flushed a second time during the commit.
    pub fn commit_with_presealed_artifacts(self, artifacts: &[&str]) -> Result<(), FileError> {
        self.commit_with_artifacts_inner(artifacts, false)
    }

    fn commit_with_artifacts_inner(
        mut self,
        artifacts: &[&str],
        sync_artifacts: bool,
    ) -> Result<(), FileError> {
        for artifact in artifacts {
            let path = self.snapshot_directory.join(artifact);
            if !path.is_file() {
                return Err(FileError::InvalidSnapshotArtifact(path));
            }
            if sync_artifacts {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .and_then(|file| file.sync_all())
                    .map_err(|source| io_error(&path, source))?;
            }
        }
        // Atomic writers used for presealed artifacts already sync the file
        // and its parent directory. The final directory sync below orders the
        // incomplete-marker removal with those durable files, so a second
        // pre-commit directory flush is unnecessary on that path.
        if sync_artifacts {
            sync_directory(&self.snapshot_directory)?;
        }
        fs::remove_file(&self.marker).map_err(|source| io_error(&self.marker, source))?;
        sync_directory(&self.snapshot_directory)?;
        let pointer = self.output_directory.join(CURRENT_SNAPSHOT);
        let snapshot = self
            .snapshot_directory
            .file_name()
            .ok_or_else(|| FileError::InvalidSnapshotArtifact(self.snapshot_directory.clone()))?
            .to_string_lossy();
        write_text_atomic(&pointer, &snapshot)?;
        prune_complete_snapshots(
            &self.output_directory.join(SNAPSHOTS_DIRECTORY),
            snapshot.as_ref(),
        )?;
        self.committed = true;
        Ok(())
    }
}

fn profile_internal_duration(label: &str, elapsed: Duration) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!("[compass internal] {label}: {:.3}s", elapsed.as_secs_f64());
    }
}

fn remove_file_if_exists(path: &Path) -> Result<(), FileError> {
    match fs::remove_file(path) {
        Ok(()) => sync_directory(path.parent().unwrap_or_else(|| Path::new("."))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, error)),
    }
}

fn prune_complete_snapshots(snapshots: &Path, current: &str) -> Result<(), FileError> {
    let mut complete = fs::read_dir(snapshots)
        .map_err(|source| io_error(snapshots, source))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && !entry.path().join(INCOMPLETE_MARKER).exists()
        })
        .take(MAX_SNAPSHOT_DIRECTORY_ENTRIES.saturating_add(1))
        .collect::<Vec<_>>();
    if complete.len() > MAX_SNAPSHOT_DIRECTORY_ENTRIES {
        return Err(FileError::InvalidSnapshotArtifact(snapshots.to_path_buf()));
    }
    complete.sort_by_key(|entry| entry.file_name());
    complete.reverse();
    let retained = complete
        .iter()
        .take(RETAINED_COMPLETE_SNAPSHOTS)
        .map(|entry| entry.file_name())
        .chain(std::iter::once(std::ffi::OsString::from(current)))
        .collect::<std::collections::BTreeSet<_>>();
    for entry in complete {
        if !retained.contains(&entry.file_name()) {
            fs::remove_dir_all(entry.path()).map_err(|source| io_error(entry.path(), source))?;
        }
    }
    sync_directory(snapshots)
}

impl Drop for BuildGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.snapshot_directory);
        }
    }
}

fn snapshot_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("snapshot-{nanos}-{}-{sequence}", std::process::id())
}

fn validate_exclusions(excluded_artifacts: &[&str]) -> Result<(), FileError> {
    for artifact in excluded_artifacts {
        let path = Path::new(artifact);
        if artifact.is_empty()
            || path.components().count() != 1
            || !matches!(path.components().next(), Some(Component::Normal(_)))
        {
            return Err(FileError::InvalidSnapshotArtifact(path.to_path_buf()));
        }
    }
    Ok(())
}

fn copy_snapshot(
    source: &Path,
    destination: &Path,
    excluded_artifacts: &[&str],
    top_level: bool,
) -> Result<(), FileError> {
    for entry in fs::read_dir(source).map_err(|error| io_error(source, error))? {
        let entry = entry.map_err(|error| io_error(source, error))?;
        let name = entry.file_name();
        if name == INCOMPLETE_MARKER
            || name == SNAPSHOTS_DIRECTORY
            || name == CURRENT_SNAPSHOT
            || name
                .to_string_lossy()
                .starts_with(&format!("{CURRENT_SNAPSHOT}.tmp-"))
        {
            continue;
        }
        if top_level
            && excluded_artifacts
                .iter()
                .any(|excluded| name == std::ffi::OsStr::new(excluded))
        {
            continue;
        }
        let from = entry.path();
        let to = destination.join(name);
        let file_type = entry.file_type().map_err(|error| io_error(&from, error))?;
        if file_type.is_dir() {
            fs::create_dir(&to).map_err(|error| io_error(&to, error))?;
            copy_snapshot(&from, &to, excluded_artifacts, false)?;
        } else if file_type.is_file() {
            // A staging snapshot is writable by the build pipeline. Hard
            // linking a published artifact would make an ordinary write to
            // the staging path mutate the active snapshot as well, violating
            // the one-complete-snapshot publication contract. Copy the bytes
            // instead; callers that need large sidecars can explicitly use
            // the excluded-artifact path and its immutable reference.
            fs::copy(&from, &to).map_err(|error| io_error(&to, error))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_staging_keeps_published_files_when_replaced_atomically() -> Result<(), FileError> {
        let directory = tempfile::tempdir().map_err(|source| io_error("tempdir", source))?;
        let source = directory.path().join("source");
        let destination = directory.path().join("destination");
        fs::create_dir(&source).map_err(|error| io_error(&source, error))?;
        fs::create_dir(&destination).map_err(|error| io_error(&destination, error))?;
        let published = source.join("graph.json");
        fs::write(&published, b"published").map_err(|source| io_error(&published, source))?;

        copy_snapshot(&source, &destination, &[], true)?;
        let staged = destination.join("graph.json");
        write_text_atomic(&staged, "staged")?;

        assert_eq!(
            fs::read(&published).map_err(|error| io_error(&published, error))?,
            b"published"
        );
        assert_eq!(
            fs::read(&staged).map_err(|error| io_error(&staged, error))?,
            b"staged"
        );
        Ok(())
    }
}
