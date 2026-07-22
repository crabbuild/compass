use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use prolly::{Config, NamedRootUpdate, Prolly};
use prolly_store_sqlite::{SqliteStore, SqliteStoreConfig};

use crate::{ActivityGuard, HistoryError, MaintenanceGuard, Repository};

const STORE_FORMAT_ROOT: &[u8] = b"compass/store-format/v1";
const STORE_FORMAT_KEY: &[u8] = b"format";
const STORE_FORMAT_VALUE: &[u8] = br#"{"adapter":"prolly-store-sqlite","canonical_encoding":1,"history_schema":1,"typed_keys":1}"#;

/// Project-owned wrapper around the pinned SQLite Prolly adapter.
pub struct HistoryStore {
    root: PathBuf,
    database_path: PathBuf,
    lock_path: PathBuf,
    pub(crate) prolly: Prolly<Arc<SqliteStore>>,
}

impl HistoryStore {
    /// Create or open the repository's shared history store.
    pub fn create(repository: &Repository) -> Result<Self, HistoryError> {
        let paths = HistoryPaths::create(repository)?;
        let existed = paths.database_path.exists();
        if existed {
            let guard = ActivityGuard::acquire(&paths.lock_path, false)?;
            let store = Self::open(paths)?;
            store.verify_store_format()?;
            drop(guard);
            Ok(store)
        } else {
            let guard = MaintenanceGuard::acquire(&paths.lock_path, false)?;
            let appeared_while_waiting = paths.database_path.exists();
            let store = Self::open(paths)?;
            if appeared_while_waiting {
                store.verify_store_format()?;
            } else {
                store.initialize_store_format()?;
            }
            drop(guard);
            Ok(store)
        }
    }

    /// Open an existing store without creating any file or directory.
    pub fn open_existing(repository: &Repository) -> Result<Option<Self>, HistoryError> {
        let Some(paths) = HistoryPaths::existing(repository)? else {
            return Ok(None);
        };
        let guard = ActivityGuard::acquire(&paths.lock_path, false)?;
        let store = Self::open(paths)?;
        store.verify_store_format()?;
        drop(guard);
        Ok(Some(store))
    }

    /// Return the owner-protected history resource directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the SQLite database path.
    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Acquire the shared activity lock.
    pub fn activity(&self) -> Result<ActivityGuard, HistoryError> {
        ActivityGuard::acquire(&self.lock_path, false)
    }

    /// Acquire the exclusive maintenance lock.
    pub fn maintenance(&self) -> Result<MaintenanceGuard, HistoryError> {
        MaintenanceGuard::acquire(&self.lock_path, false)
    }

    fn open(paths: HistoryPaths) -> Result<Self, HistoryError> {
        reject_symlink(&paths.database_path, true)?;
        let backend = Arc::new(SqliteStore::open_with_config(
            &paths.database_path,
            SqliteStoreConfig {
                busy_timeout_ms: 10_000,
                enable_wal: true,
                synchronous_normal: false,
            },
        )?);
        set_owner_file(&paths.database_path)?;
        secure_sqlite_sidecars(&paths.database_path)?;
        reject_symlink(&paths.database_path, false)?;
        Ok(Self {
            root: paths.root,
            database_path: paths.database_path,
            lock_path: paths.lock_path,
            prolly: Prolly::new(backend, Config::default()),
        })
    }

    fn initialize_store_format(&self) -> Result<(), HistoryError> {
        let tree = self.prolly.put(
            &self.prolly.create(),
            STORE_FORMAT_KEY.to_vec(),
            STORE_FORMAT_VALUE.to_vec(),
        )?;
        match self
            .prolly
            .compare_and_swap_named_root(STORE_FORMAT_ROOT, None, Some(&tree))?
        {
            NamedRootUpdate::Applied => Ok(()),
            NamedRootUpdate::Conflict { .. } => self.verify_store_format(),
        }
    }

    fn verify_store_format(&self) -> Result<(), HistoryError> {
        let tree = self
            .prolly
            .load_named_root(STORE_FORMAT_ROOT)?
            .ok_or(HistoryError::IncompatibleStoreFormat)?;
        let value = self.prolly.get(&tree, STORE_FORMAT_KEY)?;
        if value.as_deref() == Some(STORE_FORMAT_VALUE) {
            Ok(())
        } else {
            Err(HistoryError::IncompatibleStoreFormat)
        }
    }
}

struct HistoryPaths {
    root: PathBuf,
    database_path: PathBuf,
    lock_path: PathBuf,
}

impl HistoryPaths {
    fn create(repository: &Repository) -> Result<Self, HistoryError> {
        let root = repository.common_dir().join("compass");
        create_owner_dir(&root)?;
        let locks = root.join("locks");
        create_owner_dir(&locks)?;
        let lock_path = locks.join("maintenance.lock");
        create_owner_file(&lock_path)?;
        Ok(Self {
            database_path: root.join("history.sqlite"),
            root,
            lock_path,
        })
    }

    fn existing(repository: &Repository) -> Result<Option<Self>, HistoryError> {
        let root = repository.common_dir().join("compass");
        if !root.exists() {
            reject_symlink(&root, true)?;
            return Ok(None);
        }
        reject_directory(&root)?;
        let database_path = root.join("history.sqlite");
        if !database_path.exists() {
            reject_symlink(&database_path, true)?;
            return Ok(None);
        }
        reject_regular_file(&database_path)?;
        let locks = root.join("locks");
        reject_directory(&locks)?;
        let lock_path = locks.join("maintenance.lock");
        reject_regular_file(&lock_path)?;
        Ok(Some(Self {
            root,
            database_path,
            lock_path,
        }))
    }
}

fn create_owner_dir(path: &Path) -> Result<(), HistoryError> {
    reject_symlink(path, true)?;
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        if let Err(source) = builder.create(path)
            && source.kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(crate::error::io_error(path, source));
        }
    }
    reject_directory(path)?;
    set_owner_dir(path)
}

fn create_owner_file(path: &Path) -> Result<(), HistoryError> {
    reject_symlink(path, true)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| crate::error::io_error(path, source))?;
    reject_regular_file(path)?;
    set_owner_file(path)
}

fn reject_directory(path: &Path) -> Result<(), HistoryError> {
    reject_symlink(path, false)?;
    let metadata = fs::metadata(path).map_err(|source| crate::error::io_error(path, source))?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(HistoryError::UnsafePath {
            path: path.to_path_buf(),
            reason: "expected a directory".to_owned(),
        })
    }
}

fn reject_regular_file(path: &Path) -> Result<(), HistoryError> {
    reject_symlink(path, false)?;
    let metadata = fs::metadata(path).map_err(|source| crate::error::io_error(path, source))?;
    if metadata.is_file() {
        Ok(())
    } else {
        Err(HistoryError::UnsafePath {
            path: path.to_path_buf(),
            reason: "expected a regular file".to_owned(),
        })
    }
}

fn reject_symlink(path: &Path, missing_ok: bool) -> Result<(), HistoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(HistoryError::UnsafePath {
            path: path.to_path_buf(),
            reason: "symbolic links are not allowed".to_owned(),
        }),
        Ok(_) => Ok(()),
        Err(error) if missing_ok && error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(crate::error::io_error(path, source)),
    }
}

#[cfg(unix)]
fn set_owner_dir(path: &Path) -> Result<(), HistoryError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|source| crate::error::io_error(path, source))
}

#[cfg(not(unix))]
fn set_owner_dir(_path: &Path) -> Result<(), HistoryError> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_file(path: &Path) -> Result<(), HistoryError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|source| crate::error::io_error(path, source))
}

#[cfg(not(unix))]
fn set_owner_file(_path: &Path) -> Result<(), HistoryError> {
    Ok(())
}

fn secure_sqlite_sidecars(database_path: &Path) -> Result<(), HistoryError> {
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", database_path.display()));
        if sidecar.exists() {
            reject_regular_file(&sidecar)?;
            set_owner_file(&sidecar)?;
        } else {
            reject_symlink(&sidecar, true)?;
        }
    }
    Ok(())
}
