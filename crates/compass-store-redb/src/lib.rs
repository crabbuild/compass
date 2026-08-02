#![forbid(unsafe_code)]

//! redb implementation of the backend-neutral [`compass_store::Store`] contract.
//!
//! This crate deliberately owns the redb file and envelope format. The common
//! store crate, graph records, and query contracts never depend on redb types.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use compass_store::{
    Entry, Key, KeyRange, MAX_KEY_BYTES, MAX_SCAN_BYTES, MAX_SCAN_ITEMS, MAX_VALUE_BYTES,
    NamespaceId, PartitionKey, ScanCursor, ScanLimits, ScanPage, Store, StoreCapabilities,
    StoreError, VersionToken, WriteCondition,
};
use redb::{
    Database, ReadOnlyDatabase, ReadTransaction, ReadableDatabase, ReadableTable, Table,
    TableDefinition, WriteTransaction,
};
use sha2::{Digest, Sha256};

const REDB_SCHEMA_V1: &str = "compass.store.redb/1";
const REDB_ADAPTER: &str = "redb";
const REDB_VALUE_VERSION: u8 = 1;
const REDB_VALUE_HEADER_BYTES: usize = 1 + std::mem::size_of::<u64>() + 32;
const REDB_METADATA: TableDefinition<&str, &[u8]> = TableDefinition::new("compass.metadata.v1");
type RedbKey = (&'static [u8], &'static [u8], &'static [u8]);
type RedbValue = &'static [u8];
const REDB_KV: TableDefinition<RedbKey, RedbValue> = TableDefinition::new("compass.kv.v1");

/// Conventional filename for an explicit redb local backend.
pub const REDB_FILE_NAME: &str = "compass-store.redb";

enum RedbDatabase {
    ReadWrite(Database),
    ReadOnly(ReadOnlyDatabase),
}

impl RedbDatabase {
    fn begin_read(&self) -> Result<ReadTransaction, StoreError> {
        match self {
            Self::ReadWrite(database) => database
                .begin_read()
                .map_err(|error| backend_error("begin_read", error)),
            Self::ReadOnly(database) => database
                .begin_read()
                .map_err(|error| backend_error("begin_read", error)),
        }
    }

    fn begin_write(&self) -> Result<WriteTransaction, StoreError> {
        match self {
            Self::ReadWrite(database) => database
                .begin_write()
                .map_err(|error| backend_error("begin_write", error)),
            Self::ReadOnly(_) => Err(StoreError::Unsupported(
                "cannot write through a read-only redb store".to_owned(),
            )),
        }
    }
}

/// A durable redb implementation of the portable Compass store contract.
///
/// redb serializes write transactions. This adapter adds a non-blocking
/// process-local writer gate: a second writer receives a typed backend error
/// immediately instead of accumulating an unbounded queue. Readers use redb's
/// snapshot transactions and can run concurrently with a writer.
pub struct RedbStore {
    path: PathBuf,
    database: RedbDatabase,
    writer: Mutex<()>,
    read_only: bool,
}

impl std::fmt::Debug for RedbStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RedbStore")
            .field("path", &self.path)
            .field("read_only", &self.read_only)
            .finish()
    }
}

impl RedbStore {
    /// Create or reopen a read/write redb database and validate its metadata.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                operation: "create_redb_parent",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let database = Database::create(&path).map_err(|error| backend_error("open", error))?;
        let store = Self {
            path,
            database: RedbDatabase::ReadWrite(database),
            writer: Mutex::new(()),
            read_only: false,
        };
        store.initialize_schema()?;
        Ok(store)
    }

    /// Open an existing database without write permissions.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let database = ReadOnlyDatabase::open(&path)
            .map_err(|error| backend_error("open_read_only", error))?;
        let store = Self {
            path,
            database: RedbDatabase::ReadOnly(database),
            writer: Mutex::new(()),
            read_only: true,
        };
        store.verify_schema()?;
        Ok(store)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Copy a validated redb database to a new backup path.
    ///
    /// Callers should stop application writers before invoking this method.
    /// The copied database is reopened read-only and its schema is validated
    /// before the method returns.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<(), StoreError> {
        if !self.read_only {
            return Err(StoreError::Unsupported(
                "redb backup requires a read-only reopen after writers close".to_owned(),
            ));
        }
        let destination = destination.as_ref();
        if destination == self.path {
            return Err(StoreError::InvalidFormat(
                "redb backup destination must differ from the live store".to_owned(),
            ));
        }
        if destination.exists() {
            return Err(StoreError::InvalidFormat(format!(
                "redb backup destination already exists: {}",
                destination.display()
            )));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                operation: "create_redb_backup_parent",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&self.path, destination).map_err(|source| StoreError::Io {
            operation: "copy_redb_backup",
            path: destination.to_path_buf(),
            source,
        })?;
        if let Err(error) =
            Self::open_read_only(destination).and_then(|store| store.verify_schema())
        {
            let _ = fs::remove_file(destination);
            return Err(error);
        }
        Ok(())
    }

    /// Restore a validated redb backup into a new path without overwriting it.
    pub fn restore_from(
        backup: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<(), StoreError> {
        let backup = backup.as_ref();
        let destination = destination.as_ref();
        if backup == destination {
            return Err(StoreError::InvalidFormat(
                "redb restore destination must differ from the backup".to_owned(),
            ));
        }
        if destination.exists() {
            return Err(StoreError::InvalidFormat(format!(
                "redb restore destination already exists: {}",
                destination.display()
            )));
        }
        Self::open_read_only(backup)?.verify_schema()?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                operation: "create_redb_restore_parent",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(backup, destination).map_err(|source| StoreError::Io {
            operation: "copy_redb_restore",
            path: destination.to_path_buf(),
            source,
        })?;
        if let Err(error) =
            Self::open_read_only(destination).and_then(|store| store.verify_schema())
        {
            let _ = fs::remove_file(destination);
            return Err(error);
        }
        Ok(())
    }

    fn initialize_schema(&self) -> Result<(), StoreError> {
        self.with_write(|transaction| {
            let mut metadata = transaction
                .open_table(REDB_METADATA)
                .map_err(|error| backend_error("open_metadata", error))?;
            let existing_schema = metadata
                .get("schema")
                .map_err(|error| backend_error("read_schema", error))?
                .map(|value| value.value().to_vec());
            if let Some(value) = existing_schema {
                if value != REDB_SCHEMA_V1.as_bytes() {
                    return Err(StoreError::InvalidFormat(format!(
                        "expected redb metadata schema {REDB_SCHEMA_V1}"
                    )));
                }
            } else {
                metadata
                    .insert("schema", REDB_SCHEMA_V1.as_bytes())
                    .map_err(|error| backend_error("write_schema", error))?;
            }
            transaction
                .open_table(REDB_KV)
                .map_err(|error| backend_error("open_kv", error))?;
            Ok(())
        })
    }

    fn verify_schema(&self) -> Result<(), StoreError> {
        self.with_read(|transaction| {
            let metadata = transaction
                .open_table(REDB_METADATA)
                .map_err(|error| backend_error("open_metadata", error))?;
            let Some(value) = metadata
                .get("schema")
                .map_err(|error| backend_error("read_schema", error))?
            else {
                return Err(StoreError::InvalidFormat(
                    "redb metadata schema is missing".to_owned(),
                ));
            };
            if value.value() != REDB_SCHEMA_V1.as_bytes() {
                return Err(StoreError::InvalidFormat(format!(
                    "expected redb metadata schema {REDB_SCHEMA_V1}; remove the store and rebuild it"
                )));
            }
            transaction
                .open_table(REDB_KV)
                .map_err(|error| backend_error("open_kv", error))?;
            Ok(())
        })
    }

    fn with_read<T>(
        &self,
        operation: impl FnOnce(&ReadTransaction) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let transaction = self.database.begin_read()?;
        operation(&transaction)
    }

    fn with_write<T>(
        &self,
        operation: impl FnOnce(&WriteTransaction) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        if self.read_only {
            return Err(StoreError::Unsupported(
                "cannot write through a read-only redb store".to_owned(),
            ));
        }
        let _writer = self.writer.try_lock().map_err(|_| StoreError::Backend {
            adapter: REDB_ADAPTER,
            operation: "begin_write",
            message: "the bounded single-writer gate is full".to_owned(),
        })?;
        let transaction = self.database.begin_write()?;
        let result = operation(&transaction);
        match result {
            Ok(value) => {
                transaction
                    .commit()
                    .map_err(|error| backend_error("commit", error))?;
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }

    fn read_entry(
        table: &redb::ReadOnlyTable<RedbKey, RedbValue>,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
    ) -> Result<Option<Entry>, StoreError> {
        let Some(value) = table
            .get((namespace.as_bytes(), partition.as_bytes(), key.as_bytes()))
            .map_err(|error| backend_error("get", error))?
        else {
            return Ok(None);
        };
        decode_entry(key.as_bytes(), value.value())
            .map(Some)
            .map_err(|error| StoreError::Corrupt(format!("redb value: {error}")))
    }

    fn write_entry(
        table: &mut Table<'_, RedbKey, RedbValue>,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        value: &[u8],
        condition: WriteCondition,
    ) -> Result<Entry, StoreError> {
        validate_value(value)?;
        let address = (namespace.as_bytes(), partition.as_bytes(), key.as_bytes());
        let existing_decoded = {
            let existing = table
                .get(address)
                .map_err(|error| backend_error("get_for_write", error))?;
            existing
                .as_ref()
                .map(|entry| decode_entry(key.as_bytes(), entry.value()))
                .transpose()
                .map_err(|error| StoreError::Corrupt(format!("redb value: {error}")))?
        };
        if !condition_matches(condition, existing_decoded.as_ref()) {
            return Err(StoreError::Conflict);
        }
        let version = existing_decoded.map_or(1, |entry| entry.version.raw().saturating_add(1));
        if version == 0 {
            return Err(StoreError::Corrupt("redb version overflow".to_owned()));
        }
        let digest = digest(value);
        let envelope = encode_value(version, digest, value);
        table
            .insert(address, envelope.as_slice())
            .map_err(|error| backend_error("put", error))?;
        Ok(Entry {
            key: key.as_bytes().to_vec(),
            value: value.to_vec(),
            version: VersionToken::from_raw(version),
            digest,
        })
    }
}

impl Store for RedbStore {
    fn capabilities(&self) -> StoreCapabilities {
        StoreCapabilities {
            strong_point_reads: true,
            ordered_partition_scans: true,
            conditional_single_key_writes: true,
            durable_acknowledgements: true,
        }
    }

    fn get(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
    ) -> Result<Option<Entry>, StoreError> {
        self.with_read(|transaction| {
            let table = transaction
                .open_table(REDB_KV)
                .map_err(|error| backend_error("open_kv", error))?;
            Self::read_entry(&table, namespace, partition, key)
        })
    }

    fn scan(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        range: &KeyRange,
        limits: ScanLimits,
        cursor: Option<&ScanCursor>,
    ) -> Result<ScanPage, StoreError> {
        let limits = validate_limits(limits)?;
        validate_range(range, cursor)?;
        let max_key = vec![u8::MAX; MAX_KEY_BYTES];
        let start = range.start_inclusive.as_deref().unwrap_or(&[]);
        let end = range.end_exclusive.as_deref().unwrap_or(&max_key);
        let cursor_key = cursor.map(ScanCursor::last_key);
        self.with_read(|transaction| {
            let table = transaction
                .open_table(REDB_KV)
                .map_err(|error| backend_error("open_kv", error))?;
            let rows = table
                .range(
                    (namespace.as_bytes(), partition.as_bytes(), start)
                        ..=(namespace.as_bytes(), partition.as_bytes(), end),
                )
                .map_err(|error| backend_error("scan", error))?;
            let mut entries = Vec::new();
            let mut bytes_read = 0_usize;
            let mut has_more = false;
            for row in rows {
                let (found_key, found_value) =
                    row.map_err(|error| backend_error("scan_row", error))?;
                let (_, _, key) = found_key.value();
                if range
                    .start_inclusive
                    .as_deref()
                    .is_some_and(|start| key < start)
                    || range.end_exclusive.as_deref().is_some_and(|end| key >= end)
                    || cursor_key.is_some_and(|cursor| key <= cursor)
                {
                    continue;
                }
                let key = Key::new(key)?;
                let entry = decode_entry(key.as_bytes(), found_value.value())
                    .map_err(|error| StoreError::Corrupt(format!("redb value: {error}")))?;
                let entry_bytes = entry.value.len();
                if entries.is_empty() && entry_bytes > limits.max_bytes {
                    return Err(StoreError::InvalidScanLimit(
                        "the first matching value exceeds max_bytes".to_owned(),
                    ));
                }
                if entries.len() == limits.max_items
                    || bytes_read.saturating_add(entry_bytes) > limits.max_bytes
                {
                    has_more = true;
                    break;
                }
                bytes_read = bytes_read.saturating_add(entry_bytes);
                entries.push(entry);
            }
            let next = has_more
                .then(|| {
                    ScanCursor::from_last_key(
                        entries
                            .last()
                            .map_or_else(Vec::new, |entry| entry.key.clone()),
                    )
                })
                .transpose()?;
            Ok(ScanPage {
                entries,
                next,
                bytes_read,
            })
        })
    }

    fn put(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        value: &[u8],
        condition: WriteCondition,
    ) -> Result<Entry, StoreError> {
        self.with_write(|transaction| {
            let mut table = transaction
                .open_table(REDB_KV)
                .map_err(|error| backend_error("open_kv", error))?;
            Self::write_entry(&mut table, namespace, partition, key, value, condition)
        })
    }

    fn delete(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        condition: WriteCondition,
    ) -> Result<bool, StoreError> {
        if self.read_only {
            return Err(StoreError::Unsupported(
                "cannot delete through a read-only redb store".to_owned(),
            ));
        }
        self.with_write(|transaction| {
            let mut table = transaction
                .open_table(REDB_KV)
                .map_err(|error| backend_error("open_kv", error))?;
            let existing_decoded = {
                let existing = table
                    .get((namespace.as_bytes(), partition.as_bytes(), key.as_bytes()))
                    .map_err(|error| backend_error("get_for_delete", error))?;
                existing
                    .as_ref()
                    .map(|entry| decode_entry(key.as_bytes(), entry.value()))
                    .transpose()
                    .map_err(|error| StoreError::Corrupt(format!("redb value: {error}")))?
            };
            if !condition_matches(condition, existing_decoded.as_ref()) {
                return Err(StoreError::Conflict);
            }
            let deleted = table
                .remove((namespace.as_bytes(), partition.as_bytes(), key.as_bytes()))
                .map_err(|error| backend_error("delete", error))?;
            Ok(deleted.is_some())
        })
    }
}

fn validate_value(value: &[u8]) -> Result<(), StoreError> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(StoreError::ValueTooLarge {
            actual: value.len(),
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

fn validate_limits(limits: ScanLimits) -> Result<ScanLimits, StoreError> {
    if limits.max_items == 0 || limits.max_items > MAX_SCAN_ITEMS {
        return Err(StoreError::InvalidScanLimit(format!(
            "max_items must be between 1 and {MAX_SCAN_ITEMS}"
        )));
    }
    if limits.max_bytes == 0 || limits.max_bytes > MAX_SCAN_BYTES {
        return Err(StoreError::InvalidScanLimit(format!(
            "max_bytes must be between 1 and {MAX_SCAN_BYTES}"
        )));
    }
    Ok(limits)
}

fn validate_range(range: &KeyRange, cursor: Option<&ScanCursor>) -> Result<(), StoreError> {
    if let Some(start) = &range.start_inclusive {
        validate_component("key", start, MAX_KEY_BYTES)?;
    }
    if let Some(end) = &range.end_exclusive {
        validate_component("key", end, MAX_KEY_BYTES)?;
    }
    if let (Some(start), Some(end)) = (&range.start_inclusive, &range.end_exclusive)
        && start > end
    {
        return Err(StoreError::InvalidScanLimit(
            "range start must be smaller than range end".to_owned(),
        ));
    }
    if let Some(cursor) = cursor {
        validate_component("cursor", cursor.last_key(), MAX_KEY_BYTES)?;
    }
    Ok(())
}

fn validate_component(
    component: &'static str,
    value: &[u8],
    maximum: usize,
) -> Result<(), StoreError> {
    if value.is_empty() {
        return Err(StoreError::EmptyComponent { component });
    }
    if value.len() > maximum {
        return Err(StoreError::ComponentTooLarge {
            component,
            actual: value.len(),
            maximum,
        });
    }
    Ok(())
}

fn condition_matches(condition: WriteCondition, existing: Option<&Entry>) -> bool {
    match condition {
        WriteCondition::Any => true,
        WriteCondition::Missing => existing.is_none(),
        WriteCondition::Version(version) => existing.is_some_and(|entry| entry.version == version),
    }
}

fn encode_value(version: u64, digest: [u8; 32], value: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(REDB_VALUE_HEADER_BYTES + value.len());
    encoded.push(REDB_VALUE_VERSION);
    encoded.extend_from_slice(&version.to_be_bytes());
    encoded.extend_from_slice(&digest);
    encoded.extend_from_slice(value);
    encoded
}

fn decode_entry(key: &[u8], encoded: &[u8]) -> Result<Entry, &'static str> {
    if encoded.len() < REDB_VALUE_HEADER_BYTES || encoded[0] != REDB_VALUE_VERSION {
        return Err("unsupported or truncated value envelope");
    }
    let version = u64::from_be_bytes(
        encoded[1..9]
            .try_into()
            .map_err(|_| "invalid value version")?,
    );
    if version == 0 {
        return Err("value version is zero");
    }
    let digest: [u8; 32] = encoded[9..41]
        .try_into()
        .map_err(|_| "invalid value digest")?;
    let value = &encoded[REDB_VALUE_HEADER_BYTES..];
    if value.len() > MAX_VALUE_BYTES {
        return Err("value exceeds portable store limit");
    }
    if digest != digest_bytes(value) {
        return Err("value digest does not match");
    }
    Ok(Entry {
        key: key.to_vec(),
        value: value.to_vec(),
        version: VersionToken::from_raw(version),
        digest,
    })
}

fn digest(value: &[u8]) -> [u8; 32] {
    digest_bytes(value)
}

fn digest_bytes(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

fn backend_error(operation: &'static str, error: impl std::fmt::Display) -> StoreError {
    StoreError::Backend {
        adapter: REDB_ADAPTER,
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_gate_rejects_a_second_writer_without_waiting() -> Result<(), StoreError> {
        let directory = tempfile::tempdir().map_err(|error| StoreError::Backend {
            adapter: REDB_ADAPTER,
            operation: "test_tempdir",
            message: error.to_string(),
        })?;
        let store = RedbStore::open(directory.path().join(REDB_FILE_NAME))?;
        let namespace = NamespaceId::new(b"test")?;
        let partition = PartitionKey::new(b"records")?;
        let key = Key::new(b"one")?;
        let _writer = store
            .writer
            .try_lock()
            .map_err(|_| StoreError::Corrupt("test writer gate was already held".to_owned()))?;
        assert!(matches!(
            store.put(
                &namespace,
                &partition,
                &key,
                b"value",
                WriteCondition::Missing,
            ),
            Err(StoreError::Backend {
                operation: "begin_write",
                ..
            })
        ));
        Ok(())
    }
}
