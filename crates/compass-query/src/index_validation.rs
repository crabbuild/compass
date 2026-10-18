//! Full validation of a disposable JSON query index, including slow file pagers.
//! Images are inspection-only and must preserve the exact original file bytes.
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rusqlite::serialize::Data;
use rusqlite::{Connection, DatabaseName, OpenFlags};

use crate::cql::{QueryError, QueryErrorKind};

#[derive(Clone, Copy)]
struct Limits {
    file_probe: Duration,
    total: Duration,
    minimum_image_bytes: u64,
    maximum_image_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_probe: Duration::from_secs(2),
            total: Duration::from_secs(30),
            minimum_image_bytes: 16 * 1024 * 1024,
            maximum_image_bytes: 768 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Validation {
    File,
    Image,
    Invalid,
}

enum Check {
    Complete(bool),
    Interrupted,
}

fn timeout() -> QueryError {
    QueryError::new(
        QueryErrorKind::Timeout,
        "query_index_validation_timeout",
        "local query index validation exceeded its startup budget; retry, or build a narrower graph with compass update <root>",
    )
}

fn within(deadline: Instant) -> Result<(), QueryError> {
    if Instant::now() >= deadline {
        Err(timeout())
    } else {
        Ok(())
    }
}

/// Cancel and join before the connection can start another operation. In
/// particular, no late interrupt may escape into image inspection or queries.
struct DeadlineGuard<'a> {
    connection: &'a Connection,
    cancel: Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for DeadlineGuard<'_> {
    fn drop(&mut self) {
        self.connection.progress_handler(0, None::<fn() -> bool>);
        let _ = self.cancel.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn bounded<T>(
    connection: &Connection,
    deadline: Instant,
    run: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> Result<rusqlite::Result<T>, QueryError> {
    within(deadline)?;
    let interrupted = Arc::new(AtomicBool::new(false));
    let timer_flag = Arc::clone(&interrupted);
    let interrupt = connection.get_interrupt_handle();
    let (cancel, cancellation) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("compass-index-deadline".to_owned())
        .spawn(move || {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if matches!(
                cancellation.recv_timeout(remaining),
                Err(RecvTimeoutError::Timeout)
            ) {
                timer_flag.store(true, Ordering::Relaxed);
                // SQLite checks this flag within its B-tree integrity walk,
                // independently of the coarser VM/page progress interval.
                interrupt.interrupt();
            }
        })
        .ok();
    // If an OS thread cannot be started, retain deadline checks at each
    // progress step rather than silently losing the independent timer.
    let progress_interval = if worker.is_some() { 1000 } else { 1 };
    let guard = DeadlineGuard {
        connection,
        cancel,
        worker,
    };
    let flag = Arc::clone(&interrupted);
    connection.progress_handler(
        progress_interval,
        Some(move || {
            let expired = Instant::now() >= deadline;
            if expired {
                flag.store(true, Ordering::Relaxed);
            }
            expired
        }),
    );
    within(deadline)?;
    let result = run(connection);
    drop(guard);
    if interrupted.load(Ordering::Relaxed) || Instant::now() >= deadline {
        Err(timeout())
    } else {
        Ok(result)
    }
}

fn full_check(connection: &Connection, deadline: Instant) -> Check {
    match bounded(connection, deadline, |connection| {
        connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
    }) {
        Ok(result) => Check::Complete(result.is_ok_and(|text| text == "ok")),
        Err(_) => Check::Interrupted,
    }
}

fn metadata_matches(
    connection: &Connection,
    key: &str,
    format: &str,
    deadline: Instant,
) -> Result<bool, QueryError> {
    bounded(connection, deadline, |connection| {
        connection.query_row(
            "SELECT
               (SELECT value FROM metadata WHERE key='format'),
               (SELECT value FROM metadata WHERE key='key'),
               (SELECT value FROM metadata WHERE key='complete')",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
    })
    .map(|result| {
        result.is_ok_and(|(found_format, found_key, complete)| {
            found_format == format && found_key == key && complete == "1"
        })
    })
}

/// `None` means the original file must still be validated. Ineligibility or a
/// different serialized image is never evidence that the original is corrupt.
fn image_check(
    connection: &Connection,
    path: &Path,
    limits: Limits,
    deadline: Instant,
) -> Result<Option<bool>, QueryError> {
    within(deadline)?;
    let Some(mut file) = File::open(path).ok() else {
        return Ok(None);
    };
    let Some(metadata) = file.metadata().ok() else {
        return Ok(None);
    };
    let size = metadata.len();
    if !metadata.is_file()
        || size < limits.minimum_image_bytes.max(100)
        || size > limits.maximum_image_bytes
    {
        return Ok(None);
    }
    let mut header = [0_u8; 100];
    if file.read_exact(&mut header).is_err()
        || &header[..16] != b"SQLite format 3\0"
        // A byte-equal WAL image does not preserve the original pager context.
        || header[18] != 1
        || header[19] != 1
    {
        return Ok(None);
    }
    let pages = connection.query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0));
    let page_size = connection.query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0));
    let (Ok(pages), Ok(page_size)) = (pages, page_size) else {
        return Ok(None);
    };
    if !(512..=65536).contains(&page_size)
        || !page_size.is_power_of_two()
        || pages == 0
        || pages.checked_mul(page_size) != Some(size)
    {
        return Ok(None);
    }
    within(deadline)?;
    // This safe API allocates at most the checked page_count * page_size for
    // the read transaction. Never construct OwnedData from raw pointers.
    let Ok(image) = connection.serialize(DatabaseName::Main) else {
        return Ok(None);
    };
    within(deadline)?;
    if u64::try_from(image.len()).ok() != Some(size) || file.rewind().is_err() {
        return Ok(None);
    }
    let mut buffer = [0_u8; 65536];
    let mut start = 0;
    while start < image.len() {
        within(deadline)?;
        let end = start.saturating_add(buffer.len()).min(image.len());
        let bytes = &mut buffer[..end - start];
        if file.read_exact(bytes).is_err() || image.get(start..end) != Some(bytes) {
            return Ok(None);
        }
        start = end;
    }
    // Serialize may normalize trailing pages or zero-fill read failures. Exact
    // physical equality, including EOF, is required before inspecting the copy.
    if file.read(&mut [0_u8; 1]).ok() != Some(0) {
        return Ok(None);
    }
    let Data::Owned(image) = image else {
        return Ok(None);
    };
    let Ok(mut memory) = Connection::open_in_memory() else {
        return Ok(None);
    };
    // FTS5's integrity operation needs a writable handle even for inspection.
    if memory
        .deserialize(DatabaseName::Main, image, false)
        .is_err()
    {
        return Ok(None);
    }
    match full_check(&memory, deadline) {
        Check::Complete(valid) => Ok(Some(valid)),
        Check::Interrupted => Err(timeout()),
    }
}

pub(crate) fn validate(path: &Path, key: &str, format: &str) -> Result<bool, QueryError> {
    validate_with_limits(path, key, format, Limits::default())
        .map(|result| result != Validation::Invalid)
}

fn validate_with_limits(
    path: &Path,
    key: &str,
    format: &str,
    limits: Limits,
) -> Result<Validation, QueryError> {
    if !path.is_file() {
        return Ok(Validation::Invalid);
    }
    let started = Instant::now();
    let deadline = started.checked_add(limits.total).ok_or_else(timeout)?;
    within(deadline)?;
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
    else {
        return Ok(Validation::Invalid);
    };
    // Pin one database snapshot across metadata, interrupted file inspection,
    // serialization and byte comparison. Validation never publishes the image.
    let Ok(transaction) = connection.unchecked_transaction() else {
        return Ok(Validation::Invalid);
    };
    if !metadata_matches(&transaction, key, format, deadline)? {
        return Ok(Validation::Invalid);
    }
    within(deadline)?;
    let probe_deadline = started
        .checked_add(limits.file_probe)
        .unwrap_or(deadline)
        .min(deadline);
    match full_check(&transaction, probe_deadline) {
        Check::Complete(true) => return Ok(Validation::File),
        Check::Complete(false) => return Ok(Validation::Invalid),
        Check::Interrupted => {}
    }
    within(deadline)?;
    if let Some(valid) = image_check(&transaction, path, limits, deadline)? {
        return Ok(if valid {
            Validation::Image
        } else {
            Validation::Invalid
        });
    }
    match full_check(&transaction, deadline) {
        Check::Complete(true) => Ok(Validation::File),
        Check::Complete(false) => Ok(Validation::Invalid),
        Check::Interrupted => Err(timeout()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::fs;

    const FORMAT: &str = "test-index/1";
    const KEY: &str = "test-key";

    fn fixture(path: &Path, page_size: u32, wal: bool) -> Result<(), Box<dyn Error>> {
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "page_size", page_size)?;
        if wal {
            connection.pragma_update(None, "journal_mode", "WAL")?;
        }
        connection.execute_batch(
            "CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);
             CREATE TABLE payload(id INTEGER PRIMARY KEY,body BLOB);
             CREATE INDEX payload_body ON payload(body);
             CREATE VIRTUAL TABLE words USING fts5(text);
             WITH RECURSIVE rows(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<2000)
               INSERT INTO payload SELECT n,zeroblob(100) FROM rows;
             INSERT INTO words VALUES('exact source identifier fixture');",
        )?;
        for (key, value) in [("format", FORMAT), ("key", KEY), ("complete", "1")] {
            connection.execute("INSERT INTO metadata VALUES(?1,?2)", [key, value])?;
        }
        Ok(())
    }

    fn forced_image() -> Limits {
        Limits {
            file_probe: Duration::ZERO,
            total: Duration::from_secs(10),
            minimum_image_bytes: 0,
            maximum_image_bytes: 2 * 1024 * 1024,
        }
    }

    #[test]
    fn adaptive_image_retains_fts_integrity_and_original_bytes() -> Result<(), Box<dyn Error>> {
        for page_size in [4096, 8192, 65536] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("index.sqlite3");
            fixture(&path, page_size, false)?;
            let original = fs::read(&path)?;
            assert_eq!(
                validate_with_limits(&path, KEY, FORMAT, Limits::default())?,
                Validation::File
            );
            assert_eq!(
                validate_with_limits(&path, KEY, FORMAT, forced_image())?,
                Validation::Image
            );
            assert_eq!(fs::read(&path)?, original);
        }
        Ok(())
    }

    #[test]
    fn byte_equal_image_preserves_corruption_rejection() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("index.sqlite3");
        fixture(&path, 4096, false)?;
        let root_page: usize = Connection::open(&path)?.query_row(
            "SELECT rootpage FROM sqlite_schema WHERE name='payload'",
            [],
            |row| row.get(0),
        )?;
        let mut bytes = fs::read(&path)?;
        *bytes.get_mut((root_page - 1) * 4096).ok_or("root page")? = 0;
        fs::write(&path, &bytes)?;
        let connection = Connection::open(&path)?;
        assert!(matches!(
            full_check(&connection, Instant::now() + Duration::from_secs(10)),
            Check::Complete(false)
        ));
        assert_eq!(
            image_check(
                &connection,
                &path,
                forced_image(),
                Instant::now() + Duration::from_secs(10)
            )?,
            Some(false)
        );
        drop(connection);
        assert_eq!(
            validate_with_limits(&path, KEY, FORMAT, forced_image())?,
            Validation::Invalid
        );
        assert_eq!(fs::read(&path)?, bytes);
        Ok(())
    }

    #[test]
    fn physical_size_mismatch_falls_back_to_original_full_check() -> Result<(), Box<dyn Error>> {
        for truncate in [false, true] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("index.sqlite3");
            fixture(&path, 4096, false)?;
            let mut bytes = fs::read(&path)?;
            if truncate {
                bytes.truncate(bytes.len().saturating_sub(41));
            } else {
                bytes.extend_from_slice(&[0_u8; 4096]);
            }
            fs::write(&path, &bytes)?;
            let connection = Connection::open(&path)?;
            let valid = matches!(
                full_check(&connection, Instant::now() + Duration::from_secs(10)),
                Check::Complete(true)
            );
            assert_eq!(
                image_check(
                    &connection,
                    &path,
                    forced_image(),
                    Instant::now() + Duration::from_secs(10)
                )?,
                None
            );
            drop(connection);
            assert_eq!(
                validate_with_limits(&path, KEY, FORMAT, forced_image())?,
                if valid {
                    Validation::File
                } else {
                    Validation::Invalid
                }
            );
            assert_eq!(fs::read(&path)?, bytes);
        }
        Ok(())
    }

    #[test]
    fn wal_and_image_size_limits_keep_original_validation() -> Result<(), Box<dyn Error>> {
        for wal in [false, true] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("index.sqlite3");
            fixture(&path, 4096, wal)?;
            let bytes = fs::read(&path)?;
            let limits = if wal {
                forced_image()
            } else {
                Limits {
                    maximum_image_bytes: 1,
                    ..forced_image()
                }
            };
            assert_eq!(
                image_check(
                    &Connection::open(&path)?,
                    &path,
                    limits,
                    Instant::now() + Duration::from_secs(10)
                )?,
                None
            );
            assert_eq!(
                validate_with_limits(&path, KEY, FORMAT, limits)?,
                Validation::File
            );
            assert_eq!(fs::read(&path)?, bytes);
        }
        Ok(())
    }

    #[test]
    fn metadata_guards_apply_before_image_acceptance() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("index.sqlite3");
        fixture(&path, 4096, false)?;
        assert!(!validate(&path, "wrong-key", FORMAT)?);
        assert!(!validate(&path, KEY, "unknown-format")?);
        Connection::open(&path)?
            .execute("UPDATE metadata SET value='0' WHERE key='complete'", [])?;
        assert_eq!(
            validate_with_limits(&path, KEY, FORMAT, forced_image())?,
            Validation::Invalid
        );
        Ok(())
    }

    #[test]
    fn deadline_is_a_typed_error_and_never_invalidates_the_index() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("index.sqlite3");
        fixture(&path, 4096, false)?;
        let bytes = fs::read(&path)?;
        let error = validate_with_limits(
            &path,
            KEY,
            FORMAT,
            Limits {
                total: Duration::ZERO,
                ..forced_image()
            },
        )
        .err()
        .ok_or("deadline unexpectedly accepted")?;
        assert_eq!(error.kind(), QueryErrorKind::Timeout);
        assert_eq!(error.code(), "query_index_validation_timeout");
        assert_eq!(fs::read(&path)?, bytes);
        assert!(validate(&path, KEY, FORMAT)?);
        Ok(())
    }

    #[test]
    fn byte_guard_rejects_a_different_physical_database() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("first.sqlite3");
        let second = directory.path().join("second.sqlite3");
        fixture(&first, 4096, false)?;
        fs::copy(&first, &second)?;
        Connection::open(&second)?
            .execute("UPDATE metadata SET value='new-value' WHERE key='key'", [])?;
        assert_eq!(fs::metadata(&first)?.len(), fs::metadata(&second)?.len());
        assert_eq!(
            image_check(
                &Connection::open(&first)?,
                &second,
                forced_image(),
                Instant::now() + Duration::from_secs(10)
            )?,
            None
        );
        Ok(())
    }

    #[test]
    fn interrupted_integrity_retains_the_read_snapshot_for_image_validation()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("index.sqlite3");
        fixture(&path, 4096, false)?;
        let bytes = fs::read(&path)?;
        let connection = Connection::open(&path)?;
        let transaction = connection.unchecked_transaction()?;
        assert!(metadata_matches(
            &transaction,
            KEY,
            FORMAT,
            Instant::now() + Duration::from_secs(10)
        )?);
        // Force an actual SQLite VM interruption, independently of wall timing.
        transaction.progress_handler(1, Some(|| true));
        let interrupted =
            transaction.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0));
        transaction.progress_handler(0, None::<fn() -> bool>);
        assert!(
            matches!(interrupted, Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::OperationInterrupted)
        );
        assert!(!transaction.is_autocommit());
        assert_eq!(
            image_check(
                &transaction,
                &path,
                forced_image(),
                Instant::now() + Duration::from_secs(10)
            )?,
            Some(true)
        );
        drop(transaction);
        drop(connection);
        assert_eq!(fs::read(&path)?, bytes);
        Ok(())
    }

    #[test]
    fn deadline_interrupts_sql_without_progress_callbacks() -> Result<(), Box<dyn Error>> {
        let connection = Connection::open_in_memory()?;
        let started = Instant::now();
        let result = bounded(
            &connection,
            started + Duration::from_millis(20),
            |connection| {
                // A long SQLite operation can spend substantial time between
                // progress callbacks. Exercise the independent interrupt path.
                connection.progress_handler(0, None::<fn() -> bool>);
                connection.query_row(
                    "WITH RECURSIVE rows(n) AS (
                       VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<50000000
                     ) SELECT sum(n) FROM rows",
                    [],
                    |row| row.get::<_, i64>(0),
                )
            },
        );
        let error = result.err().ok_or("deadline unexpectedly accepted")?;
        assert_eq!(error.code(), "query_index_validation_timeout");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            connection.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?,
            1
        );
        Ok(())
    }

    #[test]
    fn cancelled_deadline_cannot_interrupt_the_next_sql_operation() -> Result<(), Box<dyn Error>> {
        let connection = Connection::open_in_memory()?;
        let value = bounded(
            &connection,
            Instant::now() + Duration::from_millis(200),
            |connection| connection.query_row("SELECT 1", [], |row| row.get::<_, i64>(0)),
        )??;
        assert_eq!(value, 1);
        // This query crosses the first operation's deadline. A detached or late
        // interrupt must not escape into subsequent work on this connection.
        let mut delay_once = true;
        connection.progress_handler(
            1,
            Some(move || {
                if delay_once {
                    delay_once = false;
                    std::thread::sleep(Duration::from_millis(250));
                }
                false
            }),
        );
        let result = connection.query_row(
            "WITH RECURSIVE rows(n) AS (
               VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<100
             ) SELECT sum(n) FROM rows",
            [],
            |row| row.get::<_, i64>(0),
        );
        connection.progress_handler(0, None::<fn() -> bool>);
        assert_eq!(result?, 5050);
        Ok(())
    }
}
