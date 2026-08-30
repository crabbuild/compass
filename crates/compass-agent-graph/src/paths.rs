use std::fs;
use std::path::{Path, PathBuf};

use crate::{AgentGraphError, AgentGraphErrorCode};

pub const AGENT_GRAPH_DATABASE_NAME: &str = "agent-graph.sqlite3";

/// Validated local paths for an agent graph store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentGraphPaths {
    root: PathBuf,
    database: PathBuf,
}

impl AgentGraphPaths {
    /// Create or reopen storage below an exact canonical Git common directory.
    pub fn for_git_common_dir(common_dir: &Path) -> Result<Self, AgentGraphError> {
        let canonical = validate_existing_directory(common_dir, "Git common directory")?;
        let root = canonical.join("compass");
        create_owner_directory(&root)?;
        let database = root.join(AGENT_GRAPH_DATABASE_NAME);
        reject_symlink(&database)?;
        Ok(Self { root, database })
    }

    /// Create or reopen storage below an explicit non-Git state directory.
    ///
    /// The caller, normally `compass-core`, selects and confines `state_root`;
    /// no path is accepted from an agent mutation request.
    pub fn for_explicit_state_root(state_root: &Path) -> Result<Self, AgentGraphError> {
        if !state_root.is_absolute() {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::StorageFailure,
                "non-Git agent graph state root must be absolute",
            ));
        }
        reject_symlink(state_root)?;
        let name = state_root.file_name().ok_or_else(|| {
            path_error(
                state_root,
                "non-Git state root must name a directory below an existing parent",
            )
        })?;
        let parent = state_root
            .parent()
            .ok_or_else(|| path_error(state_root, "non-Git state root has no parent"))?
            .canonicalize()
            .map_err(|error| {
                path_error(
                    state_root,
                    format!("cannot canonicalize non-Git state parent: {error}"),
                )
            })?;
        let root = parent.join(name);
        create_owner_directory(&root)?;
        let database = root.join(AGENT_GRAPH_DATABASE_NAME);
        reject_symlink(&database)?;
        Ok(Self { root, database })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn database(&self) -> &Path {
        &self.database
    }

    pub fn secure_database_permissions(&self) -> Result<(), AgentGraphError> {
        reject_symlink(&self.database)?;
        if !self.database.is_file() {
            return Err(path_error(
                &self.database,
                "database was not created as a regular file",
            ));
        }
        set_owner_file(&self.database)
    }
}

fn validate_existing_directory(path: &Path, label: &str) -> Result<PathBuf, AgentGraphError> {
    reject_symlink(path)?;
    let metadata = fs::metadata(path)
        .map_err(|error| path_error(path, format!("cannot inspect {label}: {error}")))?;
    if !metadata.is_dir() {
        return Err(path_error(path, format!("{label} is not a directory")));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| path_error(path, format!("cannot canonicalize {label}: {error}")))?;
    if canonical != path {
        return Err(path_error(
            path,
            format!("{label} must be supplied as its exact canonical path"),
        ));
    }
    Ok(canonical)
}

fn create_owner_directory(path: &Path) -> Result<(), AgentGraphError> {
    reject_symlink(path)?;
    if path.exists() {
        if !path.is_dir() {
            return Err(path_error(path, "storage root is not a directory"));
        }
        return set_owner_directory(path);
    }
    let parent = path
        .parent()
        .ok_or_else(|| path_error(path, "storage root has no parent"))?;
    validate_existing_directory(parent, "storage parent")?;
    create_directory(path)?;
    reject_symlink(path)?;
    set_owner_directory(path)
}

#[cfg(unix)]
fn create_directory(path: &Path) -> Result<(), AgentGraphError> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(path)
        .map_err(|error| path_error(path, format!("cannot create owner-only directory: {error}")))
}

#[cfg(not(unix))]
fn create_directory(path: &Path) -> Result<(), AgentGraphError> {
    fs::create_dir(path)
        .map_err(|error| path_error(path, format!("cannot create storage directory: {error}")))
}

#[cfg(unix)]
fn set_owner_directory(path: &Path) -> Result<(), AgentGraphError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        path_error(
            path,
            format!("cannot set owner-only directory permissions: {error}"),
        )
    })
}

#[cfg(not(unix))]
fn set_owner_directory(_path: &Path) -> Result<(), AgentGraphError> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_file(path: &Path) -> Result<(), AgentGraphError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        path_error(
            path,
            format!("cannot set owner-only database permissions: {error}"),
        )
    })
}

#[cfg(not(unix))]
fn set_owner_file(_path: &Path) -> Result<(), AgentGraphError> {
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), AgentGraphError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(path_error(path, "storage path must not be a symlink"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(path_error(
            path,
            format!("cannot inspect storage path: {error}"),
        )),
    }
}

fn path_error(path: &Path, message: impl Into<String>) -> AgentGraphError {
    AgentGraphError::new(
        AgentGraphErrorCode::StorageFailure,
        format!("{}: {}", path.display(), message.into()),
    )
}
