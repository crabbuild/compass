use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{FileError, io_error, write_text_atomic};

const GENERATIONS_DIRECTORY: &str = ".compass-generations";
const ACTIVE_GENERATION: &str = ".compass-active-generation";
const INCOMPLETE_MARKER: &str = ".compass-build-incomplete";
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
        fs::create_dir_all(output_directory)
            .map_err(|source| io_error(output_directory, source))?;
        let generations = output_directory.join(GENERATIONS_DIRECTORY);
        fs::create_dir_all(&generations).map_err(|source| io_error(&generations, source))?;
        let generation_directory = generations.join(generation_name());
        fs::create_dir(&generation_directory)
            .map_err(|source| io_error(&generation_directory, source))?;

        let active = Self::resolve_active_directory(output_directory)?;
        copy_generation(&active, &generation_directory)?;
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

    pub fn commit_with_artifacts(mut self, artifacts: &[&str]) -> Result<(), FileError> {
        for artifact in artifacts {
            let path = self.generation_directory.join(artifact);
            if !path.is_file() {
                return Err(FileError::InvalidGenerationArtifact(path));
            }
        }
        fs::remove_file(&self.marker).map_err(|source| io_error(&self.marker, source))?;
        let pointer = self.output_directory.join(ACTIVE_GENERATION);
        let generation = self
            .generation_directory
            .file_name()
            .expect("generation directory has a name")
            .to_string_lossy();
        write_text_atomic(&pointer, &generation)?;
        self.committed = true;
        Ok(())
    }
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

fn copy_generation(source: &Path, destination: &Path) -> Result<(), FileError> {
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
        let from = entry.path();
        let to = destination.join(name);
        let file_type = entry.file_type().map_err(|error| io_error(&from, error))?;
        if file_type.is_dir() {
            fs::create_dir(&to).map_err(|error| io_error(&to, error))?;
            copy_generation(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to).map_err(|error| io_error(&to, error))?;
        }
    }
    Ok(())
}
