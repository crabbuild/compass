//! Backend-neutral, namespace-scoped storage for Compass current graph snapshots.
//!
//! The portable address is `(namespace, partition, key)`. The SQLite adapter
//! in this crate is deliberately a normal key-value store; graph-specific
//! snapshot encoding is layered on top through [`SqliteStore::publish_snapshot`].
//! Other backends can implement [`Store`] without importing Compass graph
//! records or query code.
//!
//! The v1 trait is synchronous and runtime-neutral. A remote adapter may put a
//! bounded blocking boundary around its client, while a future async facade
//! can preserve these same request, ordering, and error semantics without
//! forcing an executor into this contract crate.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const STORE_SCHEMA_V1: &str = "compass.store/1";
pub const GRAPH_SNAPSHOT_SCHEMA_V1: &str = "compass.store.graph-snapshot/1";
pub const GRAPH_SCHEMA_V1: &str = "compass.graph/1";
pub const STORE_FILE_NAME: &str = "compass-store.sqlite3";
pub const MAX_NAMESPACE_BYTES: usize = 128;
pub const MAX_PARTITION_BYTES: usize = 256;
pub const MAX_KEY_BYTES: usize = 1_024;
pub const MAX_VALUE_BYTES: usize = 256 * 1024;
pub const MAX_SCAN_ITEMS: usize = 1_000;
pub const MAX_SCAN_BYTES: usize = 1024 * 1024;
pub const MAX_GRAPH_BYTES: usize = 1024 * 1024 * 1024;
const GRAPH_NAMESPACE: &[u8] = b"compass.current.graph.v1";
const CATALOG_PARTITION: &[u8] = b"catalog";
const OBJECT_PARTITION: &[u8] = b"object";
const ACTIVE_KEY: &[u8] = b"active";
const MANIFEST_PREFIX: &[u8] = b"manifest/";
const CHUNK_PREFIX: &[u8] = b"chunk/";
const CHUNK_BYTES: usize = MAX_VALUE_BYTES - 1_024;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store I/O failed during {operation} at {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{component} is empty")]
    EmptyComponent { component: &'static str },
    #[error("{component} is {actual} bytes; maximum is {maximum}")]
    ComponentTooLarge {
        component: &'static str,
        actual: usize,
        maximum: usize,
    },
    #[error("value is {actual} bytes; maximum is {maximum}")]
    ValueTooLarge { actual: usize, maximum: usize },
    #[error("scan limit is invalid: {0}")]
    InvalidScanLimit(String),
    #[error("store format is unsupported or corrupt: {0}")]
    InvalidFormat(String),
    #[error("store value is corrupt: {0}")]
    Corrupt(String),
    #[error("conditional store write conflicted at the requested address")]
    Conflict,
    #[error("store operation is not supported: {0}")]
    Unsupported(String),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NamespaceId(Vec<u8>);

impl NamespaceId {
    pub fn new(value: impl AsRef<[u8]>) -> Result<Self, StoreError> {
        Self::from_component("namespace", value.as_ref(), MAX_NAMESPACE_BYTES)
    }

    #[must_use]
    pub fn graph() -> Self {
        Self(GRAPH_NAMESPACE.to_vec())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn from_component(
        component: &'static str,
        value: &[u8],
        maximum: usize,
    ) -> Result<Self, StoreError> {
        validate_component(component, value, maximum)?;
        Ok(Self(value.to_vec()))
    }
}

impl AsRef<[u8]> for NamespaceId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PartitionKey(Vec<u8>);

impl PartitionKey {
    pub fn new(value: impl AsRef<[u8]>) -> Result<Self, StoreError> {
        validate_component("partition", value.as_ref(), MAX_PARTITION_BYTES)?;
        Ok(Self(value.as_ref().to_vec()))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for PartitionKey {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Key(Vec<u8>);

impl Key {
    pub fn new(value: impl AsRef<[u8]>) -> Result<Self, StoreError> {
        validate_component("key", value.as_ref(), MAX_KEY_BYTES)?;
        Ok(Self(value.as_ref().to_vec()))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for Key {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VersionToken(u64);

impl VersionToken {
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub version: VersionToken,
    pub digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteCondition {
    Any,
    Missing,
    Version(VersionToken),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyRange {
    pub start_inclusive: Option<Vec<u8>>,
    pub end_exclusive: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanLimits {
    pub max_items: usize,
    pub max_bytes: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_items: MAX_SCAN_ITEMS,
            max_bytes: MAX_SCAN_BYTES,
        }
    }
}

impl ScanLimits {
    fn validate(self) -> Result<Self, StoreError> {
        if self.max_items == 0 || self.max_items > MAX_SCAN_ITEMS {
            return Err(StoreError::InvalidScanLimit(format!(
                "max_items must be between 1 and {MAX_SCAN_ITEMS}"
            )));
        }
        if self.max_bytes == 0 || self.max_bytes > MAX_SCAN_BYTES {
            return Err(StoreError::InvalidScanLimit(format!(
                "max_bytes must be between 1 and {MAX_SCAN_BYTES}"
            )));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanCursor {
    last_key: Vec<u8>,
}

impl ScanCursor {
    #[must_use]
    pub fn last_key(&self) -> &[u8] {
        &self.last_key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanPage {
    pub entries: Vec<Entry>,
    pub next: Option<ScanCursor>,
    pub bytes_read: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreCapabilities {
    pub strong_point_reads: bool,
    pub ordered_partition_scans: bool,
    pub conditional_single_key_writes: bool,
    pub durable_acknowledgements: bool,
}

pub trait Store {
    /// Bind all subsequent operations to one namespace boundary.
    fn scope(&self, namespace: NamespaceId) -> ScopedStore<'_, Self>
    where
        Self: Sized,
    {
        ScopedStore::new(self, namespace)
    }

    fn capabilities(&self) -> StoreCapabilities;

    fn get(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
    ) -> Result<Option<Entry>, StoreError>;

    fn scan(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        range: &KeyRange,
        limits: ScanLimits,
        cursor: Option<&ScanCursor>,
    ) -> Result<ScanPage, StoreError>;

    fn put(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        value: &[u8],
        condition: WriteCondition,
    ) -> Result<Entry, StoreError>;

    fn delete(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        condition: WriteCondition,
    ) -> Result<bool, StoreError>;

    fn put_immutable(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        value: &[u8],
    ) -> Result<Entry, StoreError> {
        if let Some(existing) = self.get(namespace, partition, key)? {
            if existing.digest == digest(value) {
                return Ok(existing);
            }
            return Err(StoreError::Conflict);
        }
        match self.put(namespace, partition, key, value, WriteCondition::Missing) {
            Ok(entry) => Ok(entry),
            Err(StoreError::Conflict) => {
                let Some(existing) = self.get(namespace, partition, key)? else {
                    return Err(StoreError::Conflict);
                };
                if existing.digest == digest(value) {
                    Ok(existing)
                } else {
                    Err(StoreError::Conflict)
                }
            }
            Err(error) => Err(error),
        }
    }
}

pub struct ScopedStore<'a, S: Store + ?Sized> {
    store: &'a S,
    namespace: NamespaceId,
}

impl<'a, S: Store + ?Sized> ScopedStore<'a, S> {
    pub fn new(store: &'a S, namespace: NamespaceId) -> Self {
        Self { store, namespace }
    }

    pub fn get(&self, partition: &PartitionKey, key: &Key) -> Result<Option<Entry>, StoreError> {
        self.store.get(&self.namespace, partition, key)
    }

    pub fn scan(
        &self,
        partition: &PartitionKey,
        range: &KeyRange,
        limits: ScanLimits,
        cursor: Option<&ScanCursor>,
    ) -> Result<ScanPage, StoreError> {
        self.store
            .scan(&self.namespace, partition, range, limits, cursor)
    }

    pub fn put(
        &self,
        partition: &PartitionKey,
        key: &Key,
        value: &[u8],
        condition: WriteCondition,
    ) -> Result<Entry, StoreError> {
        self.store
            .put(&self.namespace, partition, key, value, condition)
    }

    pub fn delete(
        &self,
        partition: &PartitionKey,
        key: &Key,
        condition: WriteCondition,
    ) -> Result<bool, StoreError> {
        self.store
            .delete(&self.namespace, partition, key, condition)
    }

    pub fn put_immutable(
        &self,
        partition: &PartitionKey,
        key: &Key,
        value: &[u8],
    ) -> Result<Entry, StoreError> {
        self.store
            .put_immutable(&self.namespace, partition, key, value)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotManifest {
    pub schema: String,
    pub snapshot_id: String,
    pub graph_schema: String,
    pub graph_digest: String,
    pub payload_bytes: u64,
    pub chunk_count: u32,
    pub node_count: u64,
    pub edge_count: u64,
}

pub struct SqliteStore {
    path: PathBuf,
    connection: Mutex<Connection>,
    read_only: bool,
}

impl fmt::Debug for SqliteStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SqliteStore")
            .field("path", &self.path)
            .field("read_only", &self.read_only)
            .finish()
    }
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                operation: "create_store_parent",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let connection = Connection::open(&path)?;
        configure_connection(&connection)?;
        initialize_schema(&connection)?;
        Ok(Self {
            path,
            connection: Mutex::new(connection),
            read_only: false,
        })
    }

    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        verify_schema(&connection)?;
        Ok(Self {
            path,
            connection: Mutex::new(connection),
            read_only: true,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read_snapshot(&self) -> Result<(SnapshotManifest, Vec<u8>), StoreError> {
        let namespace = NamespaceId::graph();
        let catalog = PartitionKey::new(CATALOG_PARTITION)?;
        let active = Key::new(ACTIVE_KEY)?;
        let Some(entry) = self.get(&namespace, &catalog, &active)? else {
            return Err(StoreError::Corrupt("active snapshot is missing".to_owned()));
        };
        let manifest: SnapshotManifest = serde_json::from_slice(&entry.value)
            .map_err(|error| StoreError::Corrupt(format!("active manifest: {error}")))?;
        validate_manifest(&manifest)?;
        let object = PartitionKey::new(OBJECT_PARTITION)?;
        let capacity = usize::try_from(manifest.payload_bytes).unwrap_or(MAX_GRAPH_BYTES);
        let mut bytes = Vec::with_capacity(capacity);
        for index in 0..manifest.chunk_count {
            let key = Key::new(chunk_key(&manifest.snapshot_id, index))?;
            let Some(chunk) = self.get(&namespace, &object, &key)? else {
                return Err(StoreError::Corrupt(format!(
                    "snapshot chunk {index} is missing"
                )));
            };
            bytes.extend_from_slice(&chunk.value);
            if bytes.len() > MAX_GRAPH_BYTES {
                return Err(StoreError::ValueTooLarge {
                    actual: bytes.len(),
                    maximum: MAX_GRAPH_BYTES,
                });
            }
        }
        if bytes.len() as u64 != manifest.payload_bytes {
            return Err(StoreError::Corrupt(
                "snapshot payload length does not match its manifest".to_owned(),
            ));
        }
        if hex_digest(&bytes) != manifest.graph_digest {
            return Err(StoreError::Corrupt(
                "snapshot payload digest does not match its manifest".to_owned(),
            ));
        }
        Ok((manifest, bytes))
    }

    pub fn publish_snapshot(
        &self,
        graph_bytes: &[u8],
        graph_schema: &str,
        node_count: usize,
        edge_count: usize,
    ) -> Result<SnapshotManifest, StoreError> {
        if self.read_only {
            return Err(StoreError::Unsupported(
                "cannot publish through a read-only store".to_owned(),
            ));
        }
        if graph_bytes.is_empty() || graph_bytes.len() > MAX_GRAPH_BYTES {
            return Err(StoreError::ValueTooLarge {
                actual: graph_bytes.len(),
                maximum: MAX_GRAPH_BYTES,
            });
        }
        if graph_schema != GRAPH_SCHEMA_V1 {
            return Err(StoreError::InvalidFormat(format!(
                "unsupported graph schema {graph_schema}"
            )));
        }
        let snapshot_id = hex_digest(graph_bytes);
        let chunk_count = u32::try_from(graph_bytes.len().div_ceil(CHUNK_BYTES)).map_err(|_| {
            StoreError::ValueTooLarge {
                actual: graph_bytes.len(),
                maximum: MAX_GRAPH_BYTES,
            }
        })?;
        let manifest = SnapshotManifest {
            schema: GRAPH_SNAPSHOT_SCHEMA_V1.to_owned(),
            snapshot_id: snapshot_id.clone(),
            graph_schema: graph_schema.to_owned(),
            graph_digest: snapshot_id.clone(),
            payload_bytes: graph_bytes.len() as u64,
            chunk_count,
            node_count: node_count as u64,
            edge_count: edge_count as u64,
        };
        validate_manifest(&manifest)?;
        let namespace = NamespaceId::graph();
        let object = PartitionKey::new(OBJECT_PARTITION)?;
        for index in 0..chunk_count {
            let start = usize::try_from(index).unwrap_or(usize::MAX) * CHUNK_BYTES;
            let end = start.saturating_add(CHUNK_BYTES).min(graph_bytes.len());
            let key = Key::new(chunk_key(&snapshot_id, index))?;
            self.put_immutable(&namespace, &object, &key, &graph_bytes[start..end])?;
        }
        let manifest_key = Key::new(manifest_key(&snapshot_id))?;
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|error| StoreError::Corrupt(format!("manifest encode: {error}")))?;
        self.put_immutable(&namespace, &object, &manifest_key, &manifest_bytes)?;

        let catalog = PartitionKey::new(CATALOG_PARTITION)?;
        let active = Key::new(ACTIVE_KEY)?;
        let observed = self.get(&namespace, &catalog, &active)?;
        let condition = observed.as_ref().map_or(WriteCondition::Missing, |entry| {
            WriteCondition::Version(entry.version)
        });
        self.put(&namespace, &catalog, &active, &manifest_bytes, condition)?;
        Ok(manifest)
    }

    pub fn validate_snapshot(&self) -> Result<SnapshotManifest, StoreError> {
        self.read_snapshot().map(|(manifest, _)| manifest)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.connection
            .lock()
            .map_err(|_| StoreError::Corrupt("store connection lock is poisoned".to_owned()))
    }
}

impl Store for SqliteStore {
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
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT key, value, digest, version FROM kv
                 WHERE namespace = ?1 AND partition = ?2 AND key = ?3",
                params![namespace.as_bytes(), partition.as_bytes(), key.as_bytes()],
                read_entry,
            )
            .optional()
            .map_err(StoreError::from)
    }

    fn scan(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        range: &KeyRange,
        limits: ScanLimits,
        cursor: Option<&ScanCursor>,
    ) -> Result<ScanPage, StoreError> {
        let limits = limits.validate()?;
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
            validate_component("cursor", &cursor.last_key, MAX_KEY_BYTES)?;
        }
        let connection = self.connection()?;
        let mut query = String::from(
            "SELECT key, value, digest, version FROM kv
             WHERE namespace = ?1 AND partition = ?2",
        );
        let mut values = vec![
            rusqlite::types::Value::Blob(namespace.as_bytes().to_vec()),
            rusqlite::types::Value::Blob(partition.as_bytes().to_vec()),
        ];
        if let Some(start) = range.start_inclusive.as_ref() {
            query.push_str(" AND key >= ?3");
            values.push(rusqlite::types::Value::Blob(start.clone()));
        }
        if let Some(end) = range.end_exclusive.as_ref() {
            let index = values.len() + 1;
            query.push_str(&format!(" AND key < ?{index}"));
            values.push(rusqlite::types::Value::Blob(end.clone()));
        }
        if let Some(cursor) = cursor {
            let index = values.len() + 1;
            query.push_str(&format!(" AND key > ?{index}"));
            values.push(rusqlite::types::Value::Blob(cursor.last_key.clone()));
        }
        query.push_str(" ORDER BY key ASC");
        let mut statement = connection.prepare(&query)?;
        let mut rows = statement.query(rusqlite::params_from_iter(values))?;
        let mut entries = Vec::new();
        let mut bytes_read = 0_usize;
        let mut has_more = false;
        while let Some(row) = rows.next()? {
            let entry = read_entry(row)?;
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
        let next = has_more.then(|| ScanCursor {
            last_key: entries
                .last()
                .map_or_else(Vec::new, |entry| entry.key.clone()),
        });
        Ok(ScanPage {
            entries,
            next,
            bytes_read,
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
        if self.read_only {
            return Err(StoreError::Unsupported(
                "cannot write through a read-only store".to_owned(),
            ));
        }
        validate_value(value)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT digest, version FROM kv WHERE namespace = ?1 AND partition = ?2 AND key = ?3",
                params![namespace.as_bytes(), partition.as_bytes(), key.as_bytes()],
                |row| {
                    let digest: Vec<u8> = row.get(0)?;
                    let version: i64 = row.get(1)?;
                    Ok((digest, version))
                },
            )
            .optional()?;
        if !condition_matches(condition, existing.as_ref())? {
            return Err(StoreError::Conflict);
        }
        let version = existing
            .as_ref()
            .map_or(1_i64, |(_, version)| version.saturating_add(1));
        if version <= 0 {
            return Err(StoreError::Corrupt("store version overflow".to_owned()));
        }
        let digest = digest(value);
        transaction.execute(
            "INSERT INTO kv(namespace, partition, key, value, digest, version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(namespace, partition, key) DO UPDATE SET
               value = excluded.value, digest = excluded.digest, version = excluded.version",
            params![
                namespace.as_bytes(),
                partition.as_bytes(),
                key.as_bytes(),
                value,
                digest.as_slice(),
                version,
            ],
        )?;
        transaction.commit()?;
        Ok(Entry {
            key: key.as_bytes().to_vec(),
            value: value.to_vec(),
            version: VersionToken(version as u64),
            digest,
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
                "cannot delete through a read-only store".to_owned(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT digest, version FROM kv WHERE namespace = ?1 AND partition = ?2 AND key = ?3",
                params![namespace.as_bytes(), partition.as_bytes(), key.as_bytes()],
                |row| {
                    let digest: Vec<u8> = row.get(0)?;
                    let version: i64 = row.get(1)?;
                    Ok((digest, version))
                },
            )
            .optional()?;
        if !condition_matches(condition, existing.as_ref())? {
            return Err(StoreError::Conflict);
        }
        let deleted = transaction.execute(
            "DELETE FROM kv WHERE namespace = ?1 AND partition = ?2 AND key = ?3",
            params![namespace.as_bytes(), partition.as_bytes(), key.as_bytes()],
        )?;
        transaction.commit()?;
        Ok(deleted != 0)
    }
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

fn validate_value(value: &[u8]) -> Result<(), StoreError> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(StoreError::ValueTooLarge {
            actual: value.len(),
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

fn condition_matches(
    condition: WriteCondition,
    existing: Option<&(Vec<u8>, i64)>,
) -> Result<bool, StoreError> {
    match condition {
        WriteCondition::Any => Ok(true),
        WriteCondition::Missing => Ok(existing.is_none()),
        WriteCondition::Version(version) => {
            let Some((_, actual)) = existing else {
                return Ok(false);
            };
            let expected = i64::try_from(version.0)
                .map_err(|_| StoreError::Corrupt("invalid version token".to_owned()))?;
            Ok(*actual == expected)
        }
    }
}

fn read_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let key: Vec<u8> = row.get(0)?;
    let value: Vec<u8> = row.get(1)?;
    let digest_bytes: Vec<u8> = row.get(2)?;
    let version: i64 = row.get(3)?;
    let stored_digest: [u8; 32] = digest_bytes.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::other("invalid digest length")),
        )
    })?;
    if value.len() > MAX_VALUE_BYTES {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::other("value exceeds portable store limit")),
        ));
    }
    if stored_digest != digest(&value) {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::other("value digest does not match")),
        ));
    }
    let version = u64::try_from(version).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::other("invalid version")),
        )
    })?;
    Ok(Entry {
        key,
        value,
        version: VersionToken(version),
        digest: stored_digest,
    })
}

fn digest(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

fn hex_digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn configure_connection(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "PRAGMA journal_mode=DELETE;
         PRAGMA synchronous=FULL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;",
    )?;
    Ok(())
}

fn initialize_schema(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS metadata(
            key TEXT PRIMARY KEY NOT NULL,
            value BLOB NOT NULL
         );
         CREATE TABLE IF NOT EXISTS kv(
            namespace BLOB NOT NULL,
            partition BLOB NOT NULL,
            key BLOB NOT NULL,
            value BLOB NOT NULL,
            digest BLOB NOT NULL,
            version INTEGER NOT NULL,
            PRIMARY KEY(namespace, partition, key)
         ) WITHOUT ROWID;
         INSERT OR IGNORE INTO metadata(key, value) VALUES('schema', 'compass.store/1');",
    )?;
    verify_schema(connection)
}

fn verify_schema(connection: &Connection) -> Result<(), StoreError> {
    let schema = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'schema'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if schema.as_deref() != Some(STORE_SCHEMA_V1) {
        return Err(StoreError::InvalidFormat(format!(
            "expected metadata schema {STORE_SCHEMA_V1}"
        )));
    }
    Ok(())
}

fn manifest_key(snapshot_id: &str) -> Vec<u8> {
    let mut key = MANIFEST_PREFIX.to_vec();
    key.extend_from_slice(snapshot_id.as_bytes());
    key
}

fn chunk_key(snapshot_id: &str, index: u32) -> Vec<u8> {
    let mut key = CHUNK_PREFIX.to_vec();
    key.extend_from_slice(snapshot_id.as_bytes());
    key.push(b'/');
    key.extend_from_slice(format!("{index:08}").as_bytes());
    key
}

fn validate_manifest(manifest: &SnapshotManifest) -> Result<(), StoreError> {
    if manifest.schema != GRAPH_SNAPSHOT_SCHEMA_V1 {
        return Err(StoreError::InvalidFormat(format!(
            "expected snapshot schema {GRAPH_SNAPSHOT_SCHEMA_V1}"
        )));
    }
    if manifest.graph_schema != GRAPH_SCHEMA_V1 {
        return Err(StoreError::InvalidFormat(format!(
            "expected graph schema {GRAPH_SCHEMA_V1}"
        )));
    }
    let expected_chunks = manifest
        .payload_bytes
        .checked_add(CHUNK_BYTES as u64 - 1)
        .and_then(|bytes| u32::try_from(bytes / CHUNK_BYTES as u64).ok());
    let digest_shape = |value: &str| {
        value.len() == 64 && value.as_bytes().iter().all(|byte| byte.is_ascii_hexdigit())
    };
    if !digest_shape(&manifest.snapshot_id)
        || manifest.snapshot_id != manifest.graph_digest
        || manifest.chunk_count == 0
        || manifest.payload_bytes == 0
        || manifest.payload_bytes > MAX_GRAPH_BYTES as u64
        || expected_chunks != Some(manifest.chunk_count)
    {
        return Err(StoreError::Corrupt(
            "snapshot manifest has invalid identity or bounds".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{
        Key, KeyRange, NamespaceId, PartitionKey, ScanLimits, SqliteStore, Store, StoreError,
        WriteCondition,
    };

    #[test]
    fn namespace_partition_and_key_are_isolated_and_ordered() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let first = NamespaceId::new("first")?;
        let second = NamespaceId::new("second")?;
        let partition = PartitionKey::new("nodes")?;
        let other_partition = PartitionKey::new("edges")?;
        let one = Key::new("a")?;
        let two = Key::new("b")?;
        store.put(&first, &partition, &two, b"two", WriteCondition::Missing)?;
        store.put(&first, &partition, &one, b"one", WriteCondition::Missing)?;
        assert!(store.get(&second, &partition, &one)?.is_none());
        assert!(store.get(&first, &other_partition, &one)?.is_none());
        let page = store.scan(
            &first,
            &partition,
            &KeyRange::default(),
            ScanLimits {
                max_items: 1,
                max_bytes: 32,
            },
            None,
        )?;
        assert_eq!(page.entries[0].key, b"a");
        let cursor = page.next.as_ref().ok_or("expected cursor")?;
        let next = store.scan(
            &first,
            &partition,
            &KeyRange::default(),
            ScanLimits::default(),
            Some(cursor),
        )?;
        assert_eq!(next.entries[0].key, b"b");
        Ok(())
    }

    #[test]
    fn compare_and_swap_and_immutable_writes_are_checked() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let namespace = NamespaceId::new("tenant")?;
        let partition = PartitionKey::new("catalog")?;
        let key = Key::new("active")?;
        let entry = store.put(
            &namespace,
            &partition,
            &key,
            b"one",
            WriteCondition::Missing,
        )?;
        let replaced = store.put(
            &namespace,
            &partition,
            &key,
            b"two",
            WriteCondition::Version(entry.version),
        )?;
        assert!(!replaced.version.is_zero());
        assert!(matches!(
            store.put(
                &namespace,
                &partition,
                &key,
                b"three",
                WriteCondition::Version(entry.version),
            ),
            Err(StoreError::Conflict)
        ));
        let immutable = Key::new("object")?;
        store.put_immutable(&namespace, &partition, &immutable, b"payload")?;
        store.put_immutable(&namespace, &partition, &immutable, b"payload")?;
        assert!(matches!(
            store.put_immutable(&namespace, &partition, &immutable, b"different"),
            Err(StoreError::Conflict)
        ));
        Ok(())
    }

    #[test]
    fn snapshots_reopen_and_verify_digest() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("store.sqlite3");
        let bytes = br#"{"graph":{"schema":"compass.graph/1"},"nodes":[],"links":[]}"#;
        {
            let store = SqliteStore::open(&path)?;
            assert!(store.publish_snapshot(bytes, "other", 0, 0).is_err());
        }
        let bytes = br#"{"graph":{"schema":"compass.graph/1"},"nodes":[1],"links":[]}"#;
        {
            let store = SqliteStore::open(&path)?;
            let manifest = store.publish_snapshot(bytes, "compass.graph/1", 1, 0)?;
            assert_eq!(manifest.payload_bytes, bytes.len() as u64);
        }
        let store = SqliteStore::open_read_only(&path)?;
        let (_, loaded) = store.read_snapshot()?;
        assert_eq!(loaded, bytes);
        Ok(())
    }

    #[test]
    fn invalid_components_are_rejected() {
        assert!(NamespaceId::new([]).is_err());
        assert!(PartitionKey::new([0_u8; super::MAX_PARTITION_BYTES + 1]).is_err());
        assert!(Key::new([0_u8; super::MAX_KEY_BYTES + 1]).is_err());
    }
}
