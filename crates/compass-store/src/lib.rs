#![forbid(unsafe_code)]

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

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const STORE_SCHEMA_V1: &str = "compass.store/1";
pub const GRAPH_SNAPSHOT_SCHEMA_V1: &str = "compass.store.graph-snapshot/1";
pub const STORE_REF_SCHEMA_V1: &str = "compass.store.ref/1";
pub const STORE_RETENTION_SCHEMA_V1: &str = "compass.store.retention/1";
pub const GRAPH_SCHEMA_V1: &str = "compass.graph/1";
pub const STORE_FILE_NAME: &str = "compass-store.sqlite3";
pub const STORE_REF_FILE_NAME: &str = "store.ref";
pub const STORE_DIRECTORY_NAME: &str = ".compass-store";
pub const KEY_ENCODING_V1: u8 = 1;
pub const MAX_KEY_SEGMENTS: usize = 32;
pub const MAX_NAMESPACE_BYTES: usize = 128;
pub const MAX_PARTITION_BYTES: usize = 256;
pub const MAX_KEY_BYTES: usize = 1_024;
pub const MAX_VALUE_BYTES: usize = 256 * 1024;
pub const MAX_SCAN_ITEMS: usize = 1_000;
pub const MAX_SCAN_BYTES: usize = 1024 * 1024;
pub const MAX_IMMUTABLE_BATCH_ITEMS: usize = 1_024;
pub const MAX_IMMUTABLE_BATCH_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_GRAPH_BYTES: usize = 1024 * 1024 * 1024;
const GRAPH_NAMESPACE: &[u8] = b"compass.current.graph.v1";
const CATALOG_PARTITION: &[u8] = b"catalog";
const OBJECT_PARTITION: &[u8] = b"object";
const ACTIVE_KEY: &[u8] = b"active";
const GRAPH_SNAPSHOT_CATALOG_PARTITION: &[u8] = b"graph-snapshot/catalog";
const GRAPH_SNAPSHOT_OBJECT_PARTITION: &[u8] = b"graph-snapshot/objects";
const MANIFEST_PREFIX: &[u8] = b"manifest/";
const CHUNK_PREFIX: &[u8] = b"chunk/";
const CHUNK_BYTES: usize = MAX_VALUE_BYTES - 1_024;
const GRAPH_SNAPSHOT_LAYOUT_V2: &str = "compass.store.graph-index/2";
const GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1: &str = "compass.store.graph-selector/1";
const GRAPH_SNAPSHOT_ACTIVE_KEY: &[u8] = b"active";
const RETENTION_METADATA_KEY: &str = "retention.v1";

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
    #[error("{adapter} operation failed during {operation}: {message}")]
    Backend {
        adapter: &'static str,
        operation: &'static str,
        message: String,
    },
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
    #[error("immutable batch is invalid: {0}")]
    InvalidBatch(String),
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

    /// Construct a token from an adapter-owned monotonically increasing value.
    ///
    /// Storage adapters must reject zero and overflow before publishing a
    /// token. The value remains opaque to callers; this bridge only lets
    /// adapter crates preserve the common token type.
    #[must_use]
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Return the adapter-owned token representation.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
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
    pub fn from_last_key(last_key: Vec<u8>) -> Result<Self, StoreError> {
        validate_component("cursor", &last_key, MAX_KEY_BYTES)?;
        Ok(Self { last_key })
    }

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

/// One bounded page of keys without materializing stored values.
///
/// Maintenance operations such as reachability-based garbage collection use
/// this projection so large immutable values do not have to cross the adapter
/// boundary merely to discover their addresses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyPage {
    pub keys: Vec<Vec<u8>>,
    pub next: Option<ScanCursor>,
    pub bytes_read: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreCapabilities {
    pub strong_point_reads: bool,
    pub ordered_partition_scans: bool,
    pub conditional_single_key_writes: bool,
    pub durable_acknowledgements: bool,
    /// Maximum number of immutable values accepted by one bounded request.
    pub max_immutable_batch_items: usize,
    /// Maximum aggregate value bytes accepted by one bounded request.
    pub max_immutable_batch_bytes: usize,
    /// Whether a failed batch leaves every address in its pre-request state.
    pub atomic_immutable_batches: bool,
}

/// One namespace-scoped immutable write in a bounded backend-neutral batch.
///
/// A batch may span partitions but never namespaces. Values are owned so an
/// adapter can prepare a native transaction without borrowing graph-builder
/// scratch buffers. Immutable batches are idempotent: retrying an acknowledged
/// or partially completed non-atomic batch must return the same values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImmutableWrite {
    partition: PartitionKey,
    key: Key,
    value: Vec<u8>,
}

impl ImmutableWrite {
    pub fn new(
        partition: PartitionKey,
        key: Key,
        value: impl Into<Vec<u8>>,
    ) -> Result<Self, StoreError> {
        let value = value.into();
        validate_value(&value)?;
        Ok(Self {
            partition,
            key,
            value,
        })
    }

    #[must_use]
    pub fn partition(&self) -> &PartitionKey {
        &self.partition
    }

    #[must_use]
    pub fn key(&self) -> &Key {
        &self.key
    }

    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }
}

/// Exact work performed by one immutable batch request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImmutableBatchOutcome {
    pub entries: Vec<Entry>,
    pub new_entries: u64,
    pub reused_entries: u64,
    pub transactions: u64,
    pub bytes_written: u64,
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

    /// Scan only keys in deterministic byte order. `max_bytes` applies to the
    /// returned key bytes, not to values hidden by the projection.
    ///
    /// The portable implementation is correct but may read values internally;
    /// database adapters should override it with a native key projection.
    fn scan_keys(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        range: &KeyRange,
        limits: ScanLimits,
        cursor: Option<&ScanCursor>,
    ) -> Result<KeyPage, StoreError> {
        let limits = limits.validate()?;
        let page = self.scan(
            namespace,
            partition,
            range,
            ScanLimits {
                max_items: limits.max_items,
                max_bytes: MAX_SCAN_BYTES,
            },
            cursor,
        )?;
        let mut keys = Vec::new();
        let mut bytes_read = 0_usize;
        let mut has_more = false;
        for entry in page.entries {
            let key_bytes = entry.key.len();
            if keys.is_empty() && key_bytes > limits.max_bytes {
                return Err(StoreError::InvalidScanLimit(
                    "the first matching key exceeds max_bytes".to_owned(),
                ));
            }
            if keys.len() == limits.max_items
                || bytes_read.saturating_add(key_bytes) > limits.max_bytes
            {
                has_more = true;
                break;
            }
            bytes_read = bytes_read.saturating_add(key_bytes);
            keys.push(entry.key);
        }
        let next = if has_more {
            keys.last().cloned().map(|last_key| ScanCursor { last_key })
        } else {
            page.next
        };
        Ok(KeyPage {
            keys,
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
    ) -> Result<Entry, StoreError>;

    fn delete(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        condition: WriteCondition,
    ) -> Result<bool, StoreError>;

    /// Delete a bounded set of keys from one partition. Adapters with native
    /// transactions override this to avoid one durable commit per key.
    fn delete_batch(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        keys: &[Key],
    ) -> Result<u64, StoreError> {
        validate_delete_batch(keys)?;
        let mut deleted = 0_u64;
        for key in keys {
            if self.delete(namespace, partition, key, WriteCondition::Any)? {
                deleted = deleted.saturating_add(1);
            }
        }
        Ok(deleted)
    }

    /// Publish a bounded group of immutable values.
    ///
    /// The portable guarantee is ordered, idempotent processing rather than
    /// cross-address atomicity. Adapters advertise stronger all-or-nothing
    /// behavior through [`StoreCapabilities::atomic_immutable_batches`]. This
    /// keeps remote backends portable while allowing embedded databases to
    /// collapse thousands of durable commits into bounded transactions.
    fn put_immutable_batch(
        &self,
        namespace: &NamespaceId,
        writes: &[ImmutableWrite],
    ) -> Result<ImmutableBatchOutcome, StoreError> {
        validate_immutable_batch(writes)?;
        let mut outcome = ImmutableBatchOutcome::default();
        for write in writes {
            let expected_digest = digest(write.value());
            let entry = if let Some(existing) =
                self.get(namespace, write.partition(), write.key())?
            {
                if existing.digest != expected_digest || existing.value != write.value() {
                    return Err(StoreError::Conflict);
                }
                outcome.reused_entries = outcome.reused_entries.saturating_add(1);
                existing
            } else {
                match self.put(
                    namespace,
                    write.partition(),
                    write.key(),
                    write.value(),
                    WriteCondition::Missing,
                ) {
                    Ok(entry) => {
                        outcome.new_entries = outcome.new_entries.saturating_add(1);
                        outcome.bytes_written = outcome
                            .bytes_written
                            .saturating_add(write.value().len() as u64);
                        entry
                    }
                    Err(StoreError::Conflict) => {
                        let Some(existing) = self.get(namespace, write.partition(), write.key())?
                        else {
                            return Err(StoreError::Conflict);
                        };
                        if existing.digest != expected_digest || existing.value != write.value() {
                            return Err(StoreError::Conflict);
                        }
                        outcome.reused_entries = outcome.reused_entries.saturating_add(1);
                        existing
                    }
                    Err(error) => return Err(error),
                }
            };
            outcome.entries.push(entry);
        }
        outcome.transactions = writes.len() as u64;
        Ok(outcome)
    }

    fn put_immutable(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        value: &[u8],
    ) -> Result<Entry, StoreError> {
        let write = ImmutableWrite::new(partition.clone(), key.clone(), value.to_vec())?;
        self.put_immutable_batch(namespace, std::slice::from_ref(&write))?
            .entries
            .into_iter()
            .next()
            .ok_or_else(|| StoreError::Corrupt("immutable batch omitted its result".to_owned()))
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

    pub fn scan_keys(
        &self,
        partition: &PartitionKey,
        range: &KeyRange,
        limits: ScanLimits,
        cursor: Option<&ScanCursor>,
    ) -> Result<KeyPage, StoreError> {
        self.store
            .scan_keys(&self.namespace, partition, range, limits, cursor)
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

    pub fn put_immutable_batch(
        &self,
        writes: &[ImmutableWrite],
    ) -> Result<ImmutableBatchOutcome, StoreError> {
        self.store.put_immutable_batch(&self.namespace, writes)
    }

    pub fn delete_batch(&self, partition: &PartitionKey, keys: &[Key]) -> Result<u64, StoreError> {
        self.store.delete_batch(&self.namespace, partition, keys)
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

/// A small, typed pointer to one validated store snapshot.
///
/// The reference is an application artifact rather than a SQLite implementation
/// detail. It lets a reader validate that the selected database and snapshot are
/// the ones published with the active generation before opening query state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoreRef {
    pub schema: String,
    pub store_schema: String,
    pub adapter: String,
    pub store_id: String,
    pub namespace: String,
    pub snapshot_id: String,
    pub manifest_digest: String,
    pub graph_digest: String,
}

/// Bounded, backend-neutral maintenance state. It records what a future
/// garbage collector may retain without embedding a path, clock value, or
/// deletion policy in the graph snapshot contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetentionMetadata {
    pub schema: String,
    pub active_snapshot_id: String,
    pub active_manifest_digest: String,
    pub orphan_scan_limit: u32,
}

impl RetentionMetadata {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.schema != STORE_RETENTION_SCHEMA_V1 {
            return Err(StoreError::InvalidFormat(format!(
                "expected retention schema {STORE_RETENTION_SCHEMA_V1}"
            )));
        }
        validate_digest("retention snapshot", &self.active_snapshot_id)?;
        validate_digest("retention manifest", &self.active_manifest_digest)?;
        if self.orphan_scan_limit == 0 || self.orphan_scan_limit as usize > MAX_SCAN_ITEMS {
            return Err(StoreError::InvalidFormat(format!(
                "retention orphan scan limit must be between 1 and {MAX_SCAN_ITEMS}"
            )));
        }
        Ok(())
    }
}

impl StoreRef {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.schema != STORE_REF_SCHEMA_V1 {
            return Err(StoreError::InvalidFormat(format!(
                "expected store reference schema {STORE_REF_SCHEMA_V1}"
            )));
        }
        if self.store_schema != STORE_SCHEMA_V1 {
            return Err(StoreError::InvalidFormat(format!(
                "expected store schema {STORE_SCHEMA_V1}"
            )));
        }
        if self.adapter.is_empty() || self.store_id.is_empty() || self.namespace.is_empty() {
            return Err(StoreError::InvalidFormat(
                "store reference identity fields must be non-empty".to_owned(),
            ));
        }
        for (name, value) in [
            ("snapshot_id", self.snapshot_id.as_str()),
            ("manifest_digest", self.manifest_digest.as_str()),
            ("graph_digest", self.graph_digest.as_str()),
        ] {
            if value.len() != 64 || !value.as_bytes().iter().all(u8::is_ascii_hexdigit) {
                return Err(StoreError::InvalidFormat(format!(
                    "store reference {name} is not a SHA-256 hex digest"
                )));
            }
        }
        Ok(())
    }
}

/// Resolve the local SQLite realization selected by a graph artifact.
///
/// Current build generations keep the database once under the output root and
/// publish only a small `store.ref` beside `graph.json`. An adjacent database
/// remains supported for standalone restored bundles and older generations.
#[must_use]
pub fn local_sqlite_store_path(graph_path: &Path) -> PathBuf {
    let graph_directory = graph_path.parent().unwrap_or_else(|| Path::new("."));
    let adjacent = graph_directory.join(STORE_FILE_NAME);
    if adjacent.is_file() {
        return adjacent;
    }
    let Some(generations_directory) = graph_directory.parent() else {
        return adjacent;
    };
    if generations_directory
        .file_name()
        .and_then(|name| name.to_str())
        != Some(".compass-generations")
    {
        return adjacent;
    }
    let Some(output_root) = generations_directory.parent() else {
        return adjacent;
    };
    output_root.join(STORE_DIRECTORY_NAME).join(STORE_FILE_NAME)
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
        configure_read_only_connection(&connection)?;
        verify_schema(&connection)?;
        Ok(Self {
            path,
            connection: Mutex::new(connection),
            read_only: true,
        })
    }

    /// Pin subsequent reads on this connection to one SQLite MVCC snapshot.
    /// The transaction remains open for the lifetime of the read-only store,
    /// allowing concurrent graph GC to reclaim current rows without breaking
    /// an already-running query.
    pub fn begin_read_snapshot(&self) -> Result<(), StoreError> {
        if !self.read_only {
            return Err(StoreError::Unsupported(
                "read snapshots require a read-only store".to_owned(),
            ));
        }
        self.connection()?.execute_batch("BEGIN DEFERRED;")?;
        Ok(())
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

    /// Flush all acknowledged WAL frames before a filesystem generation is
    /// committed.  The main database file is the only authoritative artifact
    /// named by `BuildGuard`; checkpointing makes it self-contained for copy,
    /// backup, and recovery operations.
    pub fn checkpoint(&self) -> Result<(), StoreError> {
        if self.read_only {
            return Err(StoreError::Unsupported(
                "cannot checkpoint a read-only store".to_owned(),
            ));
        }
        let connection = self.connection()?;
        connection.execute_batch("PRAGMA optimize; PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// Reclaim a bounded number of free pages after immutable-object GC.
    /// New stores use incremental auto-vacuum so maintenance never triggers an
    /// unbounded full-database rewrite on the build path.
    pub fn reclaim_unused_pages(&self, max_pages: u32) -> Result<(), StoreError> {
        if self.read_only {
            return Err(StoreError::Unsupported(
                "cannot reclaim pages through a read-only store".to_owned(),
            ));
        }
        if max_pages == 0 || max_pages > 65_536 {
            return Err(StoreError::InvalidScanLimit(
                "incremental vacuum page bound must be between 1 and 65536".to_owned(),
            ));
        }
        self.connection()?
            .execute_batch(&format!("PRAGMA incremental_vacuum({max_pages});"))?;
        Ok(())
    }

    /// Copy a validated, checkpointed SQLite store to a new backup path.
    ///
    /// The destination must not be the live store. The copy is reopened and
    /// validated before this method returns so callers never receive a backup
    /// that only looked complete at the filesystem level.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<(), StoreError> {
        let destination = destination.as_ref();
        if destination == self.path {
            return Err(StoreError::InvalidFormat(
                "store backup destination must differ from the live store".to_owned(),
            ));
        }
        if destination.exists() {
            return Err(StoreError::InvalidFormat(format!(
                "store backup destination already exists: {}",
                destination.display()
            )));
        }
        self.checkpoint()?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                operation: "create_store_backup_parent",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&self.path, destination).map_err(|source| StoreError::Io {
            operation: "copy_store_backup",
            path: destination.to_path_buf(),
            source,
        })?;
        let validation =
            Self::open_read_only(destination).and_then(|store| store.snapshot_reference());
        if let Err(error) = validation {
            let _ = fs::remove_file(destination);
            return Err(error);
        }
        Ok(())
    }

    /// Restore a validated SQLite backup into a new path.
    ///
    /// Restores intentionally refuse to overwrite an existing path. A caller
    /// that needs replacement must move the old generation aside first, which
    /// preserves a recoverable rollback copy.
    pub fn restore_from(
        backup: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<(), StoreError> {
        let backup = backup.as_ref();
        let destination = destination.as_ref();
        if backup == destination {
            return Err(StoreError::InvalidFormat(
                "store restore destination must differ from the backup".to_owned(),
            ));
        }
        if destination.exists() {
            return Err(StoreError::InvalidFormat(format!(
                "store restore destination already exists: {}",
                destination.display()
            )));
        }
        Self::open_read_only(backup)?.snapshot_reference()?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                operation: "create_store_restore_parent",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(backup, destination).map_err(|source| StoreError::Io {
            operation: "copy_store_restore",
            path: destination.to_path_buf(),
            source,
        })?;
        let validation =
            Self::open_read_only(destination).and_then(|store| store.snapshot_reference());
        if let Err(error) = validation {
            let _ = fs::remove_file(destination);
            return Err(error);
        }
        Ok(())
    }

    /// Return bounded manifest keys that are not selected by either the
    /// payload snapshot or the immutable graph-index selector.  This is a
    /// discovery API only; Phase 8 owns retention policy and deletion.
    pub fn discover_orphan_manifests(&self, limits: ScanLimits) -> Result<Vec<String>, StoreError> {
        let namespace = NamespaceId::graph();
        let object = PartitionKey::new(OBJECT_PARTITION)?;
        let page = self.scan(
            &namespace,
            &object,
            &KeyRange {
                start_inclusive: Some(MANIFEST_PREFIX.to_vec()),
                end_exclusive: Some(b"manifest0".to_vec()),
            },
            limits,
            None,
        )?;
        if page.next.is_some() {
            return Err(StoreError::InvalidScanLimit(
                "orphan manifest discovery exceeds the supplied bounded page".to_owned(),
            ));
        }
        let mut active = BTreeMap::new();
        if let Some(entry) = self.get(
            &namespace,
            &PartitionKey::new(CATALOG_PARTITION)?,
            &Key::new(ACTIVE_KEY)?,
        )? {
            let manifest = serde_json::from_slice::<SnapshotManifest>(&entry.value)
                .map_err(|error| StoreError::Corrupt(format!("active manifest: {error}")))?;
            validate_manifest(&manifest)?;
            active.insert(manifest.snapshot_id, ());
        }
        if let Some(entry) = self.get(
            &namespace,
            &PartitionKey::new(GRAPH_SNAPSHOT_CATALOG_PARTITION)?,
            &Key::new(GRAPH_SNAPSHOT_ACTIVE_KEY)?,
        )? {
            let selector =
                serde_json::from_slice::<GraphSnapshotSelector>(&entry.value).map_err(|error| {
                    StoreError::Corrupt(format!("graph snapshot selector: {error}"))
                })?;
            if selector.schema != GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1 {
                return Err(StoreError::InvalidFormat(
                    "selected graph snapshot uses an unsupported format; rebuild the store"
                        .to_owned(),
                ));
            }
            validate_digest("snapshot selector manifest", &selector.manifest_digest)?;
            active.insert(selector.manifest_digest, ());
        }
        let mut orphans = Vec::new();
        for entry in page.entries {
            let Some(digest) = entry
                .key
                .strip_prefix(MANIFEST_PREFIX)
                .and_then(|value| std::str::from_utf8(value).ok())
            else {
                continue;
            };
            if digest.len() == 64
                && digest.as_bytes().iter().all(u8::is_ascii_hexdigit)
                && !active.contains_key(digest)
            {
                orphans.push(digest.to_owned());
            }
        }
        Ok(orphans)
    }

    /// Record the active graph snapshot and the bounded maintenance budget
    /// used by orphan discovery. The value is operational metadata and does
    /// not participate in graph identity.
    pub fn record_retention_metadata(
        &self,
        reference: &StoreRef,
        orphan_scan_limit: usize,
    ) -> Result<RetentionMetadata, StoreError> {
        if self.read_only {
            return Err(StoreError::Unsupported(
                "cannot update retention metadata through a read-only store".to_owned(),
            ));
        }
        reference.validate()?;
        let orphan_scan_limit = u32::try_from(orphan_scan_limit).map_err(|_| {
            StoreError::InvalidScanLimit("retention scan limit does not fit u32".to_owned())
        })?;
        let metadata = RetentionMetadata {
            schema: STORE_RETENTION_SCHEMA_V1.to_owned(),
            active_snapshot_id: reference.snapshot_id.clone(),
            active_manifest_digest: reference.manifest_digest.clone(),
            orphan_scan_limit,
        };
        metadata.validate()?;
        let bytes = serde_json::to_vec(&metadata)
            .map_err(|error| StoreError::Corrupt(format!("retention metadata encode: {error}")))?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO metadata(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![RETENTION_METADATA_KEY, bytes],
        )?;
        Ok(metadata)
    }

    pub fn retention_metadata(&self) -> Result<Option<RetentionMetadata>, StoreError> {
        let connection = self.connection()?;
        let bytes = connection
            .query_row(
                "SELECT value FROM metadata WHERE key = ?1",
                params![RETENTION_METADATA_KEY],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        bytes
            .map(|bytes| {
                let metadata = serde_json::from_slice::<RetentionMetadata>(&bytes)
                    .map_err(|error| StoreError::Corrupt(format!("retention metadata: {error}")))?;
                metadata.validate()?;
                Ok(metadata)
            })
            .transpose()
    }

    /// Return the typed reference for the currently selected snapshot.
    pub fn snapshot_reference(&self) -> Result<StoreRef, StoreError> {
        if let Some(reference) = self.graph_snapshot_reference()? {
            return Ok(reference);
        }
        let (manifest, _) = self.read_snapshot()?;
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|error| StoreError::Corrupt(format!("manifest encode: {error}")))?;
        let reference = StoreRef {
            schema: STORE_REF_SCHEMA_V1.to_owned(),
            store_schema: STORE_SCHEMA_V1.to_owned(),
            adapter: "sqlite".to_owned(),
            store_id: "sqlite-local-v1".to_owned(),
            namespace: String::from_utf8_lossy(GRAPH_NAMESPACE).into_owned(),
            snapshot_id: manifest.snapshot_id.clone(),
            manifest_digest: hex_digest(&manifest_bytes),
            graph_digest: manifest.graph_digest,
        };
        reference.validate()?;
        Ok(reference)
    }

    /// Return a typed reference for a specific immutable graph snapshot rather
    /// than consulting the mutable active selector.
    pub fn graph_snapshot_reference_for(
        &self,
        snapshot_id: &str,
        manifest_digest: &str,
    ) -> Result<StoreRef, StoreError> {
        validate_digest("snapshot reference", snapshot_id)?;
        validate_digest("snapshot reference manifest", manifest_digest)?;
        let namespace = NamespaceId::graph();
        let object = PartitionKey::new(GRAPH_SNAPSHOT_OBJECT_PARTITION)?;
        let manifest_key = Key::new(format!("manifest/{manifest_digest}"))?;
        let Some(manifest_entry) = self.get(&namespace, &object, &manifest_key)? else {
            return Err(StoreError::Corrupt(
                "referenced graph snapshot manifest is missing".to_owned(),
            ));
        };
        if hex_digest(&manifest_entry.value) != manifest_digest {
            return Err(StoreError::Corrupt(
                "referenced graph snapshot manifest digest does not match".to_owned(),
            ));
        }
        let manifest: GraphSnapshotManifest = serde_json::from_slice(&manifest_entry.value)
            .map_err(|error| StoreError::Corrupt(format!("graph snapshot manifest: {error}")))?;
        if manifest.schema != GRAPH_SNAPSHOT_LAYOUT_V2
            || manifest.graph_schema != GRAPH_SCHEMA_V1
            || manifest.snapshot_id != snapshot_id
        {
            return Err(StoreError::InvalidFormat(
                "referenced graph snapshot uses an unsupported or mismatched format; rebuild the store"
                    .to_owned(),
            ));
        }
        validate_digest("graph snapshot graph", &manifest.graph_digest)?;
        let reference = StoreRef {
            schema: STORE_REF_SCHEMA_V1.to_owned(),
            store_schema: STORE_SCHEMA_V1.to_owned(),
            adapter: "sqlite".to_owned(),
            store_id: "sqlite-local-v1".to_owned(),
            namespace: String::from_utf8_lossy(GRAPH_NAMESPACE).into_owned(),
            snapshot_id: snapshot_id.to_owned(),
            manifest_digest: manifest_digest.to_owned(),
            graph_digest: manifest.graph_digest,
        };
        reference.validate()?;
        Ok(reference)
    }

    fn graph_snapshot_reference(&self) -> Result<Option<StoreRef>, StoreError> {
        let namespace = NamespaceId::graph();
        let catalog = PartitionKey::new(GRAPH_SNAPSHOT_CATALOG_PARTITION)?;
        let active = Key::new(GRAPH_SNAPSHOT_ACTIVE_KEY)?;
        let Some(entry) = self.get(&namespace, &catalog, &active)? else {
            return Ok(None);
        };
        let selector: GraphSnapshotSelector = serde_json::from_slice(&entry.value)
            .map_err(|error| StoreError::Corrupt(format!("graph snapshot selector: {error}")))?;
        if selector.schema != GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1 {
            return Err(StoreError::InvalidFormat(
                "selected graph snapshot uses an unsupported format; rebuild the store".to_owned(),
            ));
        }
        validate_digest("snapshot selector", &selector.snapshot_id)?;
        validate_digest("snapshot selector manifest", &selector.manifest_digest)?;
        self.graph_snapshot_reference_for(&selector.snapshot_id, &selector.manifest_digest)
            .map(Some)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.connection
            .lock()
            .map_err(|_| StoreError::Corrupt("store connection lock is poisoned".to_owned()))
    }
}

#[derive(Clone, Debug)]
struct MemoryRecord {
    value: Vec<u8>,
    digest: [u8; 32],
    version: VersionToken,
}

type MemoryAddress = (Vec<u8>, Vec<u8>, Vec<u8>);
type MemoryRecords = BTreeMap<MemoryAddress, MemoryRecord>;

/// Deterministic in-memory reference implementation of [`Store`].
///
/// It is intentionally small and synchronous: its purpose is to make address,
/// ordering, limits, conditional-write, and immutable-write semantics
/// executable without a database. Adapter conformance tests can run against
/// it and use it as the logical oracle for later backends.
#[derive(Debug, Default)]
pub struct MemoryStore {
    records: Mutex<MemoryRecords>,
}

/// Compatibility alias for callers that use the more explicit name.
pub type InMemoryStore = MemoryStore;

impl MemoryStore {
    fn lock(&self) -> Result<MutexGuard<'_, MemoryRecords>, StoreError> {
        self.records
            .lock()
            .map_err(|_| StoreError::Corrupt("memory store lock is poisoned".to_owned()))
    }
}

impl Store for MemoryStore {
    fn capabilities(&self) -> StoreCapabilities {
        StoreCapabilities {
            strong_point_reads: true,
            ordered_partition_scans: true,
            conditional_single_key_writes: true,
            durable_acknowledgements: false,
            max_immutable_batch_items: MAX_IMMUTABLE_BATCH_ITEMS,
            max_immutable_batch_bytes: MAX_IMMUTABLE_BATCH_BYTES,
            atomic_immutable_batches: true,
        }
    }

    fn get(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
    ) -> Result<Option<Entry>, StoreError> {
        let records = self.lock()?;
        Ok(records
            .get(&(
                namespace.as_bytes().to_vec(),
                partition.as_bytes().to_vec(),
                key.as_bytes().to_vec(),
            ))
            .map(|record| Entry {
                key: key.as_bytes().to_vec(),
                value: record.value.clone(),
                version: record.version,
                digest: record.digest,
            }))
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
        let records = self.lock()?;
        let mut entries = Vec::new();
        let mut bytes_read = 0_usize;
        let mut has_more = false;
        for ((found_namespace, found_partition, found_key), record) in records.iter() {
            if found_namespace.as_slice() != namespace.as_bytes()
                || found_partition.as_slice() != partition.as_bytes()
            {
                continue;
            }
            if range
                .start_inclusive
                .as_ref()
                .is_some_and(|start| found_key < start)
                || range
                    .end_exclusive
                    .as_ref()
                    .is_some_and(|end| found_key >= end)
                || cursor.is_some_and(|cursor| found_key <= &cursor.last_key)
            {
                continue;
            }
            let entry_bytes = record.value.len();
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
            entries.push(Entry {
                key: found_key.clone(),
                value: record.value.clone(),
                version: record.version,
                digest: record.digest,
            });
        }
        Ok(ScanPage {
            next: has_more.then(|| ScanCursor {
                last_key: entries
                    .last()
                    .map_or_else(Vec::new, |entry| entry.key.clone()),
            }),
            entries,
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
        validate_value(value)?;
        let mut records = self.lock()?;
        let address = (
            namespace.as_bytes().to_vec(),
            partition.as_bytes().to_vec(),
            key.as_bytes().to_vec(),
        );
        let existing = records.get(&address);
        if !memory_condition_matches(condition, existing)? {
            return Err(StoreError::Conflict);
        }
        let version = existing.map_or(1, |record| record.version.0.saturating_add(1));
        if version == 0 {
            return Err(StoreError::Corrupt(
                "memory store version overflow".to_owned(),
            ));
        }
        let digest = digest(value);
        let entry = Entry {
            key: key.as_bytes().to_vec(),
            value: value.to_vec(),
            version: VersionToken(version),
            digest,
        };
        records.insert(
            address,
            MemoryRecord {
                value: entry.value.clone(),
                digest,
                version: entry.version,
            },
        );
        Ok(entry)
    }

    fn delete(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        condition: WriteCondition,
    ) -> Result<bool, StoreError> {
        let mut records = self.lock()?;
        let address = (
            namespace.as_bytes().to_vec(),
            partition.as_bytes().to_vec(),
            key.as_bytes().to_vec(),
        );
        if !memory_condition_matches(condition, records.get(&address))? {
            return Err(StoreError::Conflict);
        }
        Ok(records.remove(&address).is_some())
    }

    fn put_immutable_batch(
        &self,
        namespace: &NamespaceId,
        writes: &[ImmutableWrite],
    ) -> Result<ImmutableBatchOutcome, StoreError> {
        validate_immutable_batch(writes)?;
        let mut records = self.lock()?;
        for write in writes {
            let address = (
                namespace.as_bytes().to_vec(),
                write.partition().as_bytes().to_vec(),
                write.key().as_bytes().to_vec(),
            );
            if let Some(existing) = records.get(&address)
                && (existing.digest != digest(write.value()) || existing.value != write.value())
            {
                return Err(StoreError::Conflict);
            }
        }

        let mut outcome = ImmutableBatchOutcome {
            entries: Vec::with_capacity(writes.len()),
            transactions: 1,
            ..ImmutableBatchOutcome::default()
        };
        for write in writes {
            let address = (
                namespace.as_bytes().to_vec(),
                write.partition().as_bytes().to_vec(),
                write.key().as_bytes().to_vec(),
            );
            if let Some(existing) = records.get(&address) {
                outcome.reused_entries = outcome.reused_entries.saturating_add(1);
                outcome.entries.push(Entry {
                    key: write.key().as_bytes().to_vec(),
                    value: existing.value.clone(),
                    version: existing.version,
                    digest: existing.digest,
                });
                continue;
            }
            let digest = digest(write.value());
            let entry = Entry {
                key: write.key().as_bytes().to_vec(),
                value: write.value().to_vec(),
                version: VersionToken(1),
                digest,
            };
            records.insert(
                address,
                MemoryRecord {
                    value: entry.value.clone(),
                    digest,
                    version: entry.version,
                },
            );
            outcome.new_entries = outcome.new_entries.saturating_add(1);
            outcome.bytes_written = outcome
                .bytes_written
                .saturating_add(write.value().len() as u64);
            outcome.entries.push(entry);
        }
        Ok(outcome)
    }

    fn delete_batch(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        keys: &[Key],
    ) -> Result<u64, StoreError> {
        validate_delete_batch(keys)?;
        let mut records = self.lock()?;
        let mut deleted = 0_u64;
        for key in keys {
            let address = (
                namespace.as_bytes().to_vec(),
                partition.as_bytes().to_vec(),
                key.as_bytes().to_vec(),
            );
            if records.remove(&address).is_some() {
                deleted = deleted.saturating_add(1);
            }
        }
        Ok(deleted)
    }
}

impl Store for SqliteStore {
    fn capabilities(&self) -> StoreCapabilities {
        StoreCapabilities {
            strong_point_reads: true,
            ordered_partition_scans: true,
            conditional_single_key_writes: true,
            durable_acknowledgements: true,
            max_immutable_batch_items: MAX_IMMUTABLE_BATCH_ITEMS,
            max_immutable_batch_bytes: MAX_IMMUTABLE_BATCH_BYTES,
            atomic_immutable_batches: true,
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

    fn scan_keys(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        range: &KeyRange,
        limits: ScanLimits,
        cursor: Option<&ScanCursor>,
    ) -> Result<KeyPage, StoreError> {
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
            validate_component("cursor", cursor.last_key(), MAX_KEY_BYTES)?;
        }
        let connection = self.connection()?;
        let mut query = String::from("SELECT key FROM kv WHERE namespace = ?1 AND partition = ?2");
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
            values.push(rusqlite::types::Value::Blob(cursor.last_key().to_vec()));
        }
        query.push_str(" ORDER BY key ASC");
        let mut statement = connection.prepare(&query)?;
        let mut rows = statement.query(rusqlite::params_from_iter(values))?;
        let mut keys = Vec::new();
        let mut bytes_read = 0_usize;
        let mut has_more = false;
        while let Some(row) = rows.next()? {
            let key = row.get::<_, Vec<u8>>(0)?;
            let key_bytes = key.len();
            if keys.is_empty() && key_bytes > limits.max_bytes {
                return Err(StoreError::InvalidScanLimit(
                    "the first matching key exceeds max_bytes".to_owned(),
                ));
            }
            if keys.len() == limits.max_items
                || bytes_read.saturating_add(key_bytes) > limits.max_bytes
            {
                has_more = true;
                break;
            }
            bytes_read = bytes_read.saturating_add(key_bytes);
            keys.push(key);
        }
        let next = has_more.then(|| ScanCursor {
            last_key: keys.last().cloned().unwrap_or_default(),
        });
        Ok(KeyPage {
            keys,
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

    fn put_immutable_batch(
        &self,
        namespace: &NamespaceId,
        writes: &[ImmutableWrite],
    ) -> Result<ImmutableBatchOutcome, StoreError> {
        if self.read_only {
            return Err(StoreError::Unsupported(
                "cannot write through a read-only store".to_owned(),
            ));
        }
        validate_immutable_batch(writes)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut outcome = ImmutableBatchOutcome {
            entries: Vec::with_capacity(writes.len()),
            transactions: 1,
            ..ImmutableBatchOutcome::default()
        };
        for write in writes {
            let expected_digest = digest(write.value());
            let inserted = transaction.execute(
                "INSERT INTO kv(namespace, partition, key, value, digest, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1)
                 ON CONFLICT(namespace, partition, key) DO NOTHING",
                params![
                    namespace.as_bytes(),
                    write.partition().as_bytes(),
                    write.key().as_bytes(),
                    write.value(),
                    expected_digest.as_slice(),
                ],
            )?;
            if inserted == 1 {
                outcome.new_entries = outcome.new_entries.saturating_add(1);
                outcome.bytes_written = outcome
                    .bytes_written
                    .saturating_add(write.value().len() as u64);
                outcome.entries.push(Entry {
                    key: write.key().as_bytes().to_vec(),
                    value: write.value().to_vec(),
                    version: VersionToken(1),
                    digest: expected_digest,
                });
                continue;
            }
            let existing = transaction.query_row(
                "SELECT key, value, digest, version FROM kv
                 WHERE namespace = ?1 AND partition = ?2 AND key = ?3",
                params![
                    namespace.as_bytes(),
                    write.partition().as_bytes(),
                    write.key().as_bytes(),
                ],
                read_entry,
            )?;
            if existing.digest != expected_digest || existing.value != write.value() {
                return Err(StoreError::Conflict);
            }
            outcome.reused_entries = outcome.reused_entries.saturating_add(1);
            outcome.entries.push(existing);
        }
        transaction.commit()?;
        Ok(outcome)
    }

    fn delete_batch(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        keys: &[Key],
    ) -> Result<u64, StoreError> {
        if self.read_only {
            return Err(StoreError::Unsupported(
                "cannot delete through a read-only store".to_owned(),
            ));
        }
        validate_delete_batch(keys)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut deleted = 0_u64;
        for key in keys {
            deleted = deleted.saturating_add(transaction.execute(
                "DELETE FROM kv WHERE namespace = ?1 AND partition = ?2 AND key = ?3",
                params![namespace.as_bytes(), partition.as_bytes(), key.as_bytes()],
            )? as u64);
        }
        transaction.commit()?;
        Ok(deleted)
    }
}

fn validate_immutable_batch(writes: &[ImmutableWrite]) -> Result<(), StoreError> {
    if writes.is_empty() || writes.len() > MAX_IMMUTABLE_BATCH_ITEMS {
        return Err(StoreError::InvalidBatch(format!(
            "immutable batch item count must be between 1 and {MAX_IMMUTABLE_BATCH_ITEMS}"
        )));
    }
    let mut value_bytes = 0_usize;
    let mut addresses = BTreeSet::new();
    for write in writes {
        validate_value(write.value())?;
        value_bytes = value_bytes.saturating_add(write.value().len());
        if value_bytes > MAX_IMMUTABLE_BATCH_BYTES {
            return Err(StoreError::ValueTooLarge {
                actual: value_bytes,
                maximum: MAX_IMMUTABLE_BATCH_BYTES,
            });
        }
        if !addresses.insert((
            write.partition().as_bytes().to_vec(),
            write.key().as_bytes().to_vec(),
        )) {
            return Err(StoreError::InvalidBatch(
                "immutable batch contains a duplicate address".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_delete_batch(keys: &[Key]) -> Result<(), StoreError> {
    if keys.is_empty() || keys.len() > MAX_IMMUTABLE_BATCH_ITEMS {
        return Err(StoreError::InvalidBatch(format!(
            "delete batch item count must be between 1 and {MAX_IMMUTABLE_BATCH_ITEMS}"
        )));
    }
    let mut unique = BTreeSet::new();
    for key in keys {
        if !unique.insert(key.as_bytes()) {
            return Err(StoreError::InvalidBatch(
                "delete batch contains a duplicate key".to_owned(),
            ));
        }
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

/// Encode opaque key segments in the portable v1 binary format.
///
/// The format is deliberately independent of backend text collations:
/// `major || segment_count || (u32-be length || bytes)*`.  A caller can use the
/// resulting bytes as a range key without introducing separator collisions.
pub fn encode_key_segments(segments: &[&[u8]]) -> Result<Vec<u8>, StoreError> {
    if segments.is_empty() || segments.len() > MAX_KEY_SEGMENTS {
        return Err(StoreError::InvalidFormat(format!(
            "key segment count must be between 1 and {MAX_KEY_SEGMENTS}"
        )));
    }
    let mut encoded = Vec::new();
    encoded.push(KEY_ENCODING_V1);
    encoded.push(u8::try_from(segments.len()).map_err(|_| {
        StoreError::InvalidFormat("key segment count does not fit encoding".to_owned())
    })?);
    for segment in segments {
        validate_component("key segment", segment, MAX_KEY_BYTES)?;
        let length = u32::try_from(segment.len()).map_err(|_| StoreError::ComponentTooLarge {
            component: "key segment",
            actual: segment.len(),
            maximum: u32::MAX as usize,
        })?;
        encoded.extend_from_slice(&length.to_be_bytes());
        encoded.extend_from_slice(segment);
    }
    validate_component("key", &encoded, MAX_KEY_BYTES)?;
    Ok(encoded)
}

/// Decode a v1 binary key into its opaque segments.
pub fn decode_key_segments(encoded: &[u8]) -> Result<Vec<Vec<u8>>, StoreError> {
    validate_component("key", encoded, MAX_KEY_BYTES)?;
    if encoded.len() < 2 || encoded[0] != KEY_ENCODING_V1 {
        return Err(StoreError::InvalidFormat(
            "unsupported key encoding major".to_owned(),
        ));
    }
    let count = usize::from(encoded[1]);
    if count == 0 || count > MAX_KEY_SEGMENTS {
        return Err(StoreError::InvalidFormat(
            "invalid key segment count".to_owned(),
        ));
    }
    let mut offset = 2_usize;
    let mut segments = Vec::with_capacity(count);
    for _ in 0..count {
        let end_length = offset
            .checked_add(4)
            .ok_or_else(|| StoreError::InvalidFormat("key encoding length overflow".to_owned()))?;
        let length_bytes = encoded
            .get(offset..end_length)
            .ok_or_else(|| StoreError::InvalidFormat("truncated key segment length".to_owned()))?;
        let length =
            usize::try_from(u32::from_be_bytes(length_bytes.try_into().map_err(
                |_| StoreError::InvalidFormat("invalid key segment length".to_owned()),
            )?))
            .map_err(|_| StoreError::InvalidFormat("key segment length overflow".to_owned()))?;
        offset = end_length;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| StoreError::InvalidFormat("key segment end overflow".to_owned()))?;
        let segment = encoded
            .get(offset..end)
            .ok_or_else(|| StoreError::InvalidFormat("truncated key segment".to_owned()))?;
        validate_component("key segment", segment, MAX_KEY_BYTES)?;
        segments.push(segment.to_vec());
        offset = end;
    }
    if offset != encoded.len() {
        return Err(StoreError::InvalidFormat(
            "key encoding has trailing bytes".to_owned(),
        ));
    }
    Ok(segments)
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

fn memory_condition_matches(
    condition: WriteCondition,
    existing: Option<&MemoryRecord>,
) -> Result<bool, StoreError> {
    match condition {
        WriteCondition::Any => Ok(true),
        WriteCondition::Missing => Ok(existing.is_none()),
        WriteCondition::Version(version) => {
            Ok(existing.is_some_and(|record| record.version == version))
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
    if key.is_empty() || key.len() > MAX_KEY_BYTES || version == 0 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::other("invalid key or version")),
        ));
    }
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
        "PRAGMA page_size=16384;
         PRAGMA auto_vacuum=INCREMENTAL;
         PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;
         PRAGMA cache_size=-131072;
         PRAGMA mmap_size=536870912;
         PRAGMA temp_store=MEMORY;
         PRAGMA wal_autocheckpoint=0;",
    )?;
    Ok(())
}

fn configure_read_only_connection(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;
         PRAGMA cache_size=-131072;
         PRAGMA mmap_size=536870912;
         PRAGMA query_only=ON;",
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
            "expected metadata schema {STORE_SCHEMA_V1}; remove the store and rebuild it"
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct GraphSnapshotSelector {
    schema: String,
    snapshot_id: String,
    manifest_digest: String,
}

#[derive(Debug, Deserialize)]
struct GraphSnapshotManifest {
    schema: String,
    snapshot_id: String,
    graph_schema: String,
    graph_digest: String,
}

fn validate_digest(component: &'static str, value: &str) -> Result<(), StoreError> {
    if value.len() != 64 || !value.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(StoreError::Corrupt(format!(
            "{component} is not a SHA-256 hex digest"
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

/// Reusable adapter conformance checks for backend crates.
///
/// The helper is feature-gated so production binaries do not carry test-only
/// assertions. Every adapter test invokes the same contract checks rather than
/// maintaining a backend-shaped copy of the semantics.
#[cfg(feature = "test-support")]
pub mod test_support {
    use super::{
        ImmutableWrite, Key, KeyRange, NamespaceId, PartitionKey, ScanLimits, Store, StoreError,
        WriteCondition,
    };

    pub fn assert_store_contract<S: Store + ?Sized>(store: &S) -> Result<(), StoreError> {
        let first_namespace = NamespaceId::new(b"conformance/first")?;
        let second_namespace = NamespaceId::new(b"conformance/second")?;
        let partition = PartitionKey::new(b"records")?;
        let other_partition = PartitionKey::new(b"other")?;
        let first = Key::new([0_u8, 1_u8])?;
        let second = Key::new([0_u8, 2_u8])?;

        store.put(
            &first_namespace,
            &partition,
            &second,
            b"two",
            WriteCondition::Missing,
        )?;
        store.put(
            &first_namespace,
            &partition,
            &first,
            b"one",
            WriteCondition::Missing,
        )?;
        if store.get(&second_namespace, &partition, &first)?.is_some()
            || store
                .get(&first_namespace, &other_partition, &first)?
                .is_some()
        {
            return Err(StoreError::Corrupt(
                "adapter leaked a value across namespace or partition boundaries".to_owned(),
            ));
        }

        let first_page = store.scan(
            &first_namespace,
            &partition,
            &KeyRange::default(),
            ScanLimits {
                max_items: 1,
                max_bytes: 32,
            },
            None,
        )?;
        if first_page.entries.len() != 1 || first_page.entries[0].key != first.as_bytes() {
            return Err(StoreError::Corrupt(
                "adapter did not preserve unsigned key ordering".to_owned(),
            ));
        }
        let cursor = first_page.next.as_ref().ok_or_else(|| {
            StoreError::Corrupt("adapter omitted a continuation cursor".to_owned())
        })?;
        let second_page = store.scan(
            &first_namespace,
            &partition,
            &KeyRange::default(),
            ScanLimits::default(),
            Some(cursor),
        )?;
        if second_page.entries.len() != 1 || second_page.entries[0].key != second.as_bytes() {
            return Err(StoreError::Corrupt(
                "adapter cursor did not continue after the last key".to_owned(),
            ));
        }
        let key_page = store.scan_keys(
            &first_namespace,
            &partition,
            &KeyRange::default(),
            ScanLimits {
                max_items: 1,
                max_bytes: 32,
            },
            None,
        )?;
        if key_page.keys != [first.as_bytes()] || key_page.next.is_none() {
            return Err(StoreError::Corrupt(
                "adapter key projection did not preserve scan ordering or continuation".to_owned(),
            ));
        }

        let version = store
            .get(&first_namespace, &partition, &first)?
            .ok_or_else(|| StoreError::Corrupt("inserted value is missing".to_owned()))?
            .version;
        store.put(
            &first_namespace,
            &partition,
            &first,
            b"updated",
            WriteCondition::Version(version),
        )?;
        if !matches!(
            store.put(
                &first_namespace,
                &partition,
                &first,
                b"stale",
                WriteCondition::Version(version),
            ),
            Err(StoreError::Conflict)
        ) {
            return Err(StoreError::Corrupt(
                "adapter accepted a stale compare-and-swap write".to_owned(),
            ));
        }

        let immutable = Key::new(b"immutable")?;
        let first_immutable =
            store.put_immutable(&first_namespace, &partition, &immutable, b"payload")?;
        let second_immutable =
            store.put_immutable(&first_namespace, &partition, &immutable, b"payload")?;
        if first_immutable != second_immutable
            || !matches!(
                store.put_immutable(&first_namespace, &partition, &immutable, b"different",),
                Err(StoreError::Conflict)
            )
        {
            return Err(StoreError::Corrupt(
                "adapter violated immutable-write semantics".to_owned(),
            ));
        }

        let batch = [
            ImmutableWrite::new(partition.clone(), Key::new(b"batch/one")?, b"one".to_vec())?,
            ImmutableWrite::new(
                other_partition.clone(),
                Key::new(b"batch/two")?,
                b"two".to_vec(),
            )?,
        ];
        let first_batch = store.put_immutable_batch(&first_namespace, &batch)?;
        let second_batch = store.put_immutable_batch(&first_namespace, &batch)?;
        if first_batch.entries.len() != batch.len()
            || first_batch.new_entries != batch.len() as u64
            || second_batch.reused_entries != batch.len() as u64
            || first_batch.transactions == 0
            || second_batch.transactions == 0
        {
            return Err(StoreError::Corrupt(
                "adapter violated bounded immutable-batch semantics".to_owned(),
            ));
        }

        let deleted_version = store
            .get(&first_namespace, &partition, &first)?
            .ok_or_else(|| StoreError::Corrupt("updated value is missing".to_owned()))?
            .version;
        if !store.delete(
            &first_namespace,
            &partition,
            &first,
            WriteCondition::Version(deleted_version),
        )? {
            return Err(StoreError::Corrupt(
                "adapter did not delete an existing value".to_owned(),
            ));
        }
        let delete_keys = [Key::new(b"delete/one")?, Key::new(b"delete/two")?];
        for key in &delete_keys {
            store.put(
                &first_namespace,
                &partition,
                key,
                b"delete",
                WriteCondition::Missing,
            )?;
        }
        if store.delete_batch(&first_namespace, &partition, &delete_keys)? != 2
            || store.delete_batch(&first_namespace, &partition, &delete_keys)? != 0
        {
            return Err(StoreError::Corrupt(
                "adapter violated bounded delete-batch semantics".to_owned(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::sync::Arc;
    use std::thread;

    use rusqlite::Connection;
    use sha2::{Digest, Sha256};

    use super::{
        GRAPH_SCHEMA_V1, ImmutableWrite, Key, KeyRange, MemoryStore, NamespaceId, PartitionKey,
        ScanLimits, SqliteStore, Store, StoreError, VersionToken, WriteCondition,
        decode_key_segments, encode_key_segments,
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
    fn sqlite_primary_key_serves_point_and_partition_queries() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("store.sqlite3");
        let store = SqliteStore::open(&path)?;
        drop(store);
        let connection = Connection::open(path)?;
        for sql in [
            "EXPLAIN QUERY PLAN SELECT value FROM kv \
             WHERE namespace = ?1 AND partition = ?2 AND key = ?3",
            "EXPLAIN QUERY PLAN SELECT key FROM kv \
             WHERE namespace = ?1 AND partition = ?2 AND key >= ?3 ORDER BY key",
        ] {
            let mut statement = connection.prepare(sql)?;
            let details = statement
                .query_map(
                    rusqlite::params![b"namespace", b"partition", b"key"],
                    |row| row.get::<_, String>(3),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            assert!(
                details.iter().any(|detail| detail.contains("PRIMARY KEY")),
                "expected the WITHOUT ROWID primary key in query plan: {details:?}"
            );
        }
        let page_size = connection.query_row("PRAGMA page_size", [], |row| row.get::<_, u32>(0))?;
        let auto_vacuum =
            connection.query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, u32>(0))?;
        assert_eq!(page_size, 16_384);
        assert_eq!(auto_vacuum, 2);
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
    fn sqlite_batches_immutable_objects_in_one_atomic_transaction() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        let namespace = NamespaceId::new("tenant")?;
        let partition = PartitionKey::new("objects")?;
        let writes = (0..super::MAX_IMMUTABLE_BATCH_ITEMS)
            .map(|index| {
                ImmutableWrite::new(
                    partition.clone(),
                    Key::new(format!("object-{index:04}"))?,
                    format!("value-{index}").into_bytes(),
                )
            })
            .collect::<Result<Vec<_>, StoreError>>()?;

        let first = store.put_immutable_batch(&namespace, &writes)?;
        assert_eq!(first.entries.len(), writes.len());
        assert_eq!(first.new_entries, writes.len() as u64);
        assert_eq!(first.reused_entries, 0);
        assert_eq!(first.transactions, 1);
        assert_eq!(
            first.bytes_written,
            writes
                .iter()
                .map(|write| write.value().len() as u64)
                .sum::<u64>()
        );

        let second = store.put_immutable_batch(&namespace, &writes)?;
        assert_eq!(second.new_entries, 0);
        assert_eq!(second.reused_entries, writes.len() as u64);
        assert_eq!(second.transactions, 1);
        assert_eq!(second.bytes_written, 0);

        let conflicting = vec![
            ImmutableWrite::new(
                partition.clone(),
                Key::new("must-not-commit")?,
                b"new".to_vec(),
            )?,
            ImmutableWrite::new(
                partition.clone(),
                writes[0].key().clone(),
                b"different".to_vec(),
            )?,
        ];
        assert!(matches!(
            store.put_immutable_batch(&namespace, &conflicting),
            Err(StoreError::Conflict)
        ));
        assert!(
            store
                .get(&namespace, &partition, &Key::new("must-not-commit")?)?
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn sqlite_serializes_concurrent_conditional_writers() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let store = Arc::new(SqliteStore::open(directory.path().join("store.sqlite3"))?);
        let namespace = NamespaceId::new("tenant")?;
        let partition = PartitionKey::new("records")?;
        let key = Key::new("active")?;
        let mut writers = Vec::new();
        for index in 0..8_u8 {
            let store = Arc::clone(&store);
            let namespace = namespace.clone();
            let partition = partition.clone();
            let key = key.clone();
            writers.push(thread::spawn(move || {
                store.put(
                    &namespace,
                    &partition,
                    &key,
                    &[index],
                    WriteCondition::Missing,
                )
            }));
        }
        let mut winners = 0;
        let mut conflicts = 0;
        for writer in writers {
            match writer.join().map_err(|_| "SQLite writer thread panicked")? {
                Ok(_) => winners += 1,
                Err(StoreError::Conflict) => conflicts += 1,
                Err(error) => return Err(error.into()),
            }
        }
        assert_eq!(winners, 1);
        assert_eq!(conflicts, 7);
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
            store.checkpoint()?;
        }
        let store = SqliteStore::open_read_only(&path)?;
        let (_, loaded) = store.read_snapshot()?;
        assert_eq!(loaded, bytes);
        let copied_path = directory.path().join("copied.sqlite3");
        std::fs::copy(&path, &copied_path)?;
        let copied = SqliteStore::open_read_only(&copied_path)?;
        assert_eq!(copied.read_snapshot()?.1, bytes);
        let connection = Connection::open(&path)?;
        let journal_mode =
            connection.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?;
        assert_eq!(journal_mode, "wal");
        let synchronous =
            connection.query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0))?;
        assert_eq!(synchronous, 2);
        Ok(())
    }

    #[test]
    fn backup_and_restore_validate_the_complete_sqlite_file() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source.sqlite3");
        let backup = directory.path().join("backup.sqlite3");
        let restored = directory.path().join("restored.sqlite3");
        let bytes = br#"{"graph":{"schema":"compass.graph/1"},"nodes":[],"links":[]}"#;
        {
            let store = SqliteStore::open(&source)?;
            store.publish_snapshot(bytes, GRAPH_SCHEMA_V1, 0, 0)?;
            store.backup_to(&backup)?;
        }
        SqliteStore::restore_from(&backup, &restored)?;
        assert_eq!(
            SqliteStore::open_read_only(&restored)?.read_snapshot()?.1,
            bytes
        );

        let corrupt = directory.path().join("corrupt.sqlite3");
        fs::copy(&backup, &corrupt)?;
        let connection = Connection::open(&corrupt)?;
        connection.execute("DELETE FROM kv", [])?;
        drop(connection);
        assert!(
            SqliteStore::restore_from(&corrupt, directory.path().join("rejected.sqlite3")).is_err()
        );
        Ok(())
    }

    #[test]
    fn orphan_manifest_discovery_is_bounded_and_does_not_delete_data() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("store.sqlite3");
        let first = br#"{"graph":{"schema":"compass.graph/1"},"nodes":[1],"links":[]}"#;
        let second = br#"{"graph":{"schema":"compass.graph/1"},"nodes":[2],"links":[]}"#;
        let store = SqliteStore::open(&path)?;
        store.publish_snapshot(first, "compass.graph/1", 1, 0)?;
        store.publish_snapshot(second, "compass.graph/1", 1, 0)?;
        let reference = store.snapshot_reference()?;
        let retention = store.record_retention_metadata(&reference, 16)?;
        assert_eq!(store.retention_metadata()?, Some(retention));
        let orphans = store.discover_orphan_manifests(ScanLimits::default())?;
        let first_digest = format!("{:x}", Sha256::digest(first));
        assert_eq!(orphans, vec![first_digest]);
        assert_eq!(store.read_snapshot()?.1, second);
        Ok(())
    }

    #[test]
    fn stale_schema_reports_rebuild_instruction() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("store.sqlite3");
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE metadata(key TEXT PRIMARY KEY NOT NULL, value BLOB NOT NULL);
             INSERT INTO metadata(key, value) VALUES('schema', 'compass.store/0');",
        )?;
        drop(connection);
        let error = match SqliteStore::open(&path) {
            Ok(_) => return Err("stale schema unexpectedly opened".into()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("rebuild"));
        Ok(())
    }

    #[test]
    fn invalid_components_are_rejected() {
        assert!(NamespaceId::new([]).is_err());
        assert!(PartitionKey::new([0_u8; super::MAX_PARTITION_BYTES + 1]).is_err());
        assert!(Key::new([0_u8; super::MAX_KEY_BYTES + 1]).is_err());
    }

    #[test]
    fn memory_store_matches_portable_order_and_cas_semantics() -> Result<(), Box<dyn Error>> {
        let store = MemoryStore::default();
        let namespace = NamespaceId::new("tenant")?;
        let partition = PartitionKey::new("nodes")?;
        let first = Key::new([0_u8, 1_u8])?;
        let second = Key::new([0_u8, 2_u8])?;
        let first_entry = store.put(
            &namespace,
            &partition,
            &first,
            b"one",
            WriteCondition::Missing,
        )?;
        store.put(
            &namespace,
            &partition,
            &second,
            b"two",
            WriteCondition::Missing,
        )?;
        assert!(matches!(
            store.put(
                &namespace,
                &partition,
                &first,
                b"changed",
                WriteCondition::Version(VersionToken(first_entry.version.0.saturating_add(1))),
            ),
            Err(StoreError::Conflict)
        ));
        let page = store.scan(
            &namespace,
            &partition,
            &KeyRange::default(),
            ScanLimits {
                max_items: 1,
                max_bytes: 32,
            },
            None,
        )?;
        assert_eq!(page.entries[0].key, vec![0, 1]);
        assert!(page.next.is_some());
        Ok(())
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn sqlite_passes_the_shared_adapter_conformance_contract() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let store = SqliteStore::open(directory.path().join("store.sqlite3"))?;
        super::test_support::assert_store_contract(&store)?;
        Ok(())
    }

    #[test]
    fn key_encoding_has_stable_golden_vector_and_round_trip() -> Result<(), Box<dyn Error>> {
        let encoded = encode_key_segments(&[b"node", &[0, 1, 2]])?;
        assert_eq!(
            encoded,
            vec![
                1, 2, 0, 0, 0, 4, b'n', b'o', b'd', b'e', 0, 0, 0, 3, 0, 1, 2,
            ]
        );
        assert_eq!(
            decode_key_segments(&encoded)?,
            vec![b"node".to_vec(), vec![0, 1, 2]]
        );
        assert!(decode_key_segments(&encoded[..encoded.len() - 1]).is_err());
        Ok(())
    }
}
