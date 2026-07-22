use std::path::{Path, PathBuf};
use std::process::Command;

use compass_history::{HistoryStore, Repository};
use std::sync::Arc;

use prolly::{
    Config, Error, ManifestStore, ManifestStoreScan, NodeStoreScan, Prolly, TransactionalStore,
};
use prolly_store_sqlite::{SqliteStore, SqliteStoreConfig};

fn requires_store_contract<T>()
where
    T: ManifestStore + ManifestStoreScan + NodeStoreScan + TransactionalStore,
{
}

struct CommittedRepository {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl CommittedRepository {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "--quiet"])?;
        run_git(directory.path(), &["config", "user.name", "Compass Test"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "compass@example.invalid"],
        )?;
        std::fs::write(directory.path().join("README.md"), "fixture\n")?;
        run_git(directory.path(), &["add", "README.md"])?;
        run_git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;
        let path = directory.path().to_path_buf();
        Ok(Self {
            _directory: directory,
            path,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn run_git(directory: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned().into())
    }
}

#[test]
fn sqlite_store_has_every_publication_capability() -> Result<(), Box<dyn std::error::Error>> {
    requires_store_contract::<prolly_store_sqlite::SqliteStore>();
    let fixture = CommittedRepository::new()?;
    let repository = Repository::discover(fixture.path())?;
    assert!(HistoryStore::open_existing(&repository)?.is_none());
    assert!(!repository.common_dir().join("compass").exists());

    let history = HistoryStore::create(&repository)?;
    assert_eq!(
        history.database_path(),
        repository.common_dir().join("compass/history.sqlite")
    );
    drop(history);
    assert!(HistoryStore::open_existing(&repository)?.is_some());
    Ok(())
}

#[cfg(unix)]
#[test]
fn history_resources_are_owner_only_and_reject_symlinks() -> Result<(), Box<dyn std::error::Error>>
{
    use std::os::unix::fs::{PermissionsExt, symlink};

    let fixture = CommittedRepository::new()?;
    let repository = Repository::discover(fixture.path())?;
    let history = HistoryStore::create(&repository)?;
    assert_eq!(
        std::fs::metadata(history.root())?.permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(history.database_path())?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(history);

    let bad_fixture = CommittedRepository::new()?;
    let bad_repository = Repository::discover(bad_fixture.path())?;
    let outside = tempfile::tempdir()?;
    symlink(outside.path(), bad_repository.common_dir().join("compass"))?;
    assert!(HistoryStore::create(&bad_repository).is_err());
    Ok(())
}

#[test]
fn concurrent_first_open_initializes_one_compatible_store() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = CommittedRepository::new()?;
    let repository = Repository::discover(fixture.path())?;
    std::thread::scope(|scope| {
        let handles = (0..4)
            .map(|_| {
                let repository = repository.clone();
                scope.spawn(move || {
                    HistoryStore::create(&repository).map(|store| store.database_path().is_file())
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            let opened = handle.join().map_err(|_| {
                compass_history::HistoryError::Git("history opener panicked".to_owned())
            })??;
            assert!(opened);
        }
        Ok::<(), compass_history::HistoryError>(())
    })?;
    assert!(HistoryStore::open_existing(&repository)?.is_some());
    Ok(())
}

#[test]
fn incompatible_store_format_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = CommittedRepository::new()?;
    let repository = Repository::discover(fixture.path())?;
    let history = HistoryStore::create(&repository)?;
    let database_path = history.database_path().to_path_buf();
    drop(history);

    let backend = Arc::new(SqliteStore::open_with_config(
        &database_path,
        sqlite_config(),
    )?);
    let manager = Prolly::new(backend, Config::default());
    let tree = manager.put(
        &manager.create(),
        b"format".to_vec(),
        br#"{"history_schema":999}"#.to_vec(),
    )?;
    manager.publish_named_root(b"compass/store-format/v1", &tree)?;
    drop(manager);

    assert!(HistoryStore::open_existing(&repository).is_err());
    Ok(())
}

#[test]
fn sqlite_adapter_rolls_back_strict_transactions() -> Result<(), Box<dyn std::error::Error>> {
    let backend = Arc::new(SqliteStore::open_in_memory()?);
    let manager = Prolly::new(backend, Config::default());
    let committed = manager.transaction(|transaction| {
        let tree = transaction.put(
            &transaction.create(),
            b"key".to_vec(),
            b"committed".to_vec(),
        )?;
        transaction.publish_named_root(b"main", &tree)?;
        Ok(tree)
    })?;
    let result: Result<(), Error> = manager.transaction(|transaction| {
        let changed = transaction.put(&committed, b"key".to_vec(), b"rolled-back".to_vec())?;
        transaction.publish_named_root(b"main", &changed)?;
        Err(Error::Serialize("forced rollback".to_owned()))
    });
    assert!(result.is_err());
    assert_eq!(manager.load_named_root(b"main")?, Some(committed));
    Ok(())
}

#[test]
fn separate_sqlite_connections_resolve_root_cas_contention()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database_path = directory.path().join("history.sqlite");
    let first = Prolly::new(
        Arc::new(SqliteStore::open_with_config(
            &database_path,
            sqlite_config(),
        )?),
        Config::default(),
    );
    let second = Prolly::new(
        Arc::new(SqliteStore::open_with_config(
            &database_path,
            sqlite_config(),
        )?),
        Config::default(),
    );
    let first_tree = first.put(&first.create(), b"winner".to_vec(), b"one".to_vec())?;
    let second_tree = second.put(&second.create(), b"winner".to_vec(), b"two".to_vec())?;
    assert!(
        first
            .compare_and_swap_named_root(b"preferred", None, Some(&first_tree))?
            .is_applied()
    );
    assert!(
        !second
            .compare_and_swap_named_root(b"preferred", None, Some(&second_tree))?
            .is_applied()
    );
    assert_eq!(second.load_named_root(b"preferred")?, Some(first_tree));
    Ok(())
}

fn sqlite_config() -> SqliteStoreConfig {
    SqliteStoreConfig {
        busy_timeout_ms: 10_000,
        enable_wal: true,
        synchronous_normal: false,
    }
}
