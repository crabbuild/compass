use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::atomic::sync_directory;
use crate::{FileError, io_error, write_text_atomic};

const GENERATIONS_DIRECTORY: &str = ".compass-generations";
const ACTIVE_GENERATION: &str = ".compass-active-generation";
const INCOMPLETE_MARKER: &str = ".compass-build-incomplete";
const RETAINED_COMPLETE_GENERATIONS: usize = 2;
const MAX_GENERATION_DIRECTORY_ENTRIES: usize = 1_024;
static GENERATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Owns an unpublished output generation until every authoritative artifact is sealed.
#[derive(Debug)]
pub struct BuildGuard {
    output_directory: PathBuf,
    generation_directory: PathBuf,
    marker: PathBuf,
    committed: bool,
}

impl BuildGuard {
    pub fn begin(output_directory: &Path) -> Result<Self, FileError> {
        Self::begin_excluding(output_directory, &[])
    }

    /// Return complete published generations in deterministic newest-first
    /// order. In-progress directories are deliberately excluded so callers
    /// can make retention decisions without treating partial artifacts as
    /// roots.
    pub fn complete_generation_directories(
        output_directory: &Path,
    ) -> Result<Vec<PathBuf>, FileError> {
        let generations = output_directory.join(GENERATIONS_DIRECTORY);
        if !generations.is_dir() {
            return Ok(Vec::new());
        }
        let mut complete = fs::read_dir(&generations)
            .map_err(|source| io_error(&generations, source))?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && !entry.path().join(INCOMPLETE_MARKER).exists()
            })
            .map(|entry| entry.path())
            .take(MAX_GENERATION_DIRECTORY_ENTRIES.saturating_add(1))
            .collect::<Vec<_>>();
        if complete.len() > MAX_GENERATION_DIRECTORY_ENTRIES {
            return Err(FileError::InvalidGenerationArtifact(generations));
        }
        complete.sort();
        complete.reverse();
        Ok(complete)
    }

    /// Start a generation without copying selected top-level artifacts from
    /// the active generation.
    ///
    /// Large shared sidecars can live outside immutable generations and be
    /// addressed by a small generation-local reference. Exclusions are exact
    /// file names rather than patterns so callers cannot accidentally omit an
    /// unrelated artifact family.
    pub fn begin_excluding(
        output_directory: &Path,
        excluded_artifacts: &[&str],
    ) -> Result<Self, FileError> {
        validate_exclusions(excluded_artifacts)?;
        fs::create_dir_all(output_directory)
            .map_err(|source| io_error(output_directory, source))?;
        let generations = output_directory.join(GENERATIONS_DIRECTORY);
        fs::create_dir_all(&generations).map_err(|source| io_error(&generations, source))?;
        let generation_directory = generations.join(generation_name());
        fs::create_dir(&generation_directory)
            .map_err(|source| io_error(&generation_directory, source))?;

        let active = Self::resolve_active_directory(output_directory)?;
        copy_generation(&active, &generation_directory, excluded_artifacts, true)?;
        let marker = generation_directory.join(INCOMPLETE_MARKER);
        write_text_atomic(&marker, "1")?;
        Ok(Self {
            output_directory: output_directory.to_path_buf(),
            generation_directory,
            marker,
            committed: false,
        })
    }

    #[must_use]
    pub fn staging_directory(&self) -> &Path {
        &self.generation_directory
    }

    /// Return the stable active-generation path, with a legacy-root fallback.
    pub fn resolve_active_directory(output_directory: &Path) -> Result<PathBuf, FileError> {
        let pointer = output_directory.join(ACTIVE_GENERATION);
        match fs::read_to_string(&pointer) {
            Ok(value) => {
                let generation = value.trim();
                let relative = Path::new(generation);
                if generation.is_empty()
                    || relative.components().count() != 1
                    || !matches!(relative.components().next(), Some(Component::Normal(_)))
                    || !generation.starts_with("generation-")
                {
                    return Err(FileError::InvalidGenerationArtifact(pointer));
                }
                let active = output_directory.join(GENERATIONS_DIRECTORY).join(relative);
                if !active.is_dir() || active.join(INCOMPLETE_MARKER).exists() {
                    return Err(FileError::IncompleteBuild(active));
                }
                Ok(active)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(output_directory.to_path_buf())
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
            return Err(FileError::InvalidGenerationArtifact(relative.to_path_buf()));
        }
        Ok(Self::resolve_active_directory(output_directory)?.join(relative))
    }

    /// Resolve a public artifact path while honoring a published generation before
    /// any stale legacy-root file. Legacy files remain readable only when no active
    /// generation pointer exists.
    pub fn resolve_requested_artifact(path: &Path) -> Result<PathBuf, FileError> {
        let Some(name) = path.file_name() else {
            return Err(FileError::InvalidGenerationArtifact(path.to_path_buf()));
        };
        let output_directory = path.parent().unwrap_or_else(|| Path::new("."));
        let pointer = output_directory.join(ACTIVE_GENERATION);
        match pointer.try_exists() {
            Ok(true) => Self::resolve_artifact(output_directory, Path::new(name)),
            Ok(false) if path.is_file() => Ok(path.to_path_buf()),
            Ok(false) => Self::resolve_artifact(output_directory, Path::new(name)),
            Err(source) => Err(io_error(pointer, source)),
        }
    }

    pub fn ensure_complete(output_directory: &Path) -> Result<(), FileError> {
        let active = Self::resolve_active_directory(output_directory)?;
        let marker = active.join(INCOMPLETE_MARKER);
        if marker.exists() {
            Err(FileError::IncompleteBuild(marker))
        } else {
            Ok(())
        }
    }

    pub fn commit(self) -> Result<(), FileError> {
        self.commit_with_artifacts(&[])
    }

    pub fn commit_with_artifacts(self, artifacts: &[&str]) -> Result<(), FileError> {
        self.commit_with_artifacts_inner(artifacts, true)
    }

    /// Publish a generation whose listed artifacts were already written by an
    /// atomic writer in this generation. The files are still checked before
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
            let path = self.generation_directory.join(artifact);
            if !path.is_file() {
                return Err(FileError::InvalidGenerationArtifact(path));
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
            sync_directory(&self.generation_directory)?;
        }
        fs::remove_file(&self.marker).map_err(|source| io_error(&self.marker, source))?;
        sync_directory(&self.generation_directory)?;
        let pointer = self.output_directory.join(ACTIVE_GENERATION);
        let generation = self
            .generation_directory
            .file_name()
            .ok_or_else(|| FileError::InvalidGenerationArtifact(self.generation_directory.clone()))?
            .to_string_lossy();
        write_text_atomic(&pointer, &generation)?;
        prune_complete_generations(
            &self.output_directory.join(GENERATIONS_DIRECTORY),
            generation.as_ref(),
        )?;
        self.committed = true;
        Ok(())
    }
}

fn prune_complete_generations(generations: &Path, active: &str) -> Result<(), FileError> {
    let mut complete = fs::read_dir(generations)
        .map_err(|source| io_error(generations, source))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && !entry.path().join(INCOMPLETE_MARKER).exists()
        })
        .take(MAX_GENERATION_DIRECTORY_ENTRIES.saturating_add(1))
        .collect::<Vec<_>>();
    if complete.len() > MAX_GENERATION_DIRECTORY_ENTRIES {
        return Err(FileError::InvalidGenerationArtifact(
            generations.to_path_buf(),
        ));
    }
    complete.sort_by_key(|entry| entry.file_name());
    complete.reverse();
    let retained = complete
        .iter()
        .take(RETAINED_COMPLETE_GENERATIONS)
        .map(|entry| entry.file_name())
        .chain(std::iter::once(std::ffi::OsString::from(active)))
        .collect::<std::collections::BTreeSet<_>>();
    for entry in complete {
        if !retained.contains(&entry.file_name()) {
            fs::remove_dir_all(entry.path()).map_err(|source| io_error(entry.path(), source))?;
        }
    }
    sync_directory(generations)
}

impl Drop for BuildGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.generation_directory);
        }
    }
}

fn generation_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = GENERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("generation-{nanos}-{}-{sequence}", std::process::id())
}

fn validate_exclusions(excluded_artifacts: &[&str]) -> Result<(), FileError> {
    for artifact in excluded_artifacts {
        let path = Path::new(artifact);
        if artifact.is_empty()
            || path.components().count() != 1
            || !matches!(path.components().next(), Some(Component::Normal(_)))
        {
            return Err(FileError::InvalidGenerationArtifact(path.to_path_buf()));
        }
    }
    Ok(())
}

fn copy_generation(
    source: &Path,
    destination: &Path,
    excluded_artifacts: &[&str],
    top_level: bool,
) -> Result<(), FileError> {
    for entry in fs::read_dir(source).map_err(|error| io_error(source, error))? {
        let entry = entry.map_err(|error| io_error(source, error))?;
        let name = entry.file_name();
        if name == INCOMPLETE_MARKER
            || name == GENERATIONS_DIRECTORY
            || name == ACTIVE_GENERATION
            || name
                .to_string_lossy()
                .starts_with(&format!("{ACTIVE_GENERATION}.tmp-"))
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
            copy_generation(&from, &to, excluded_artifacts, false)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to).map_err(|error| io_error(&to, error))?;
        }
    }
    Ok(())
}
