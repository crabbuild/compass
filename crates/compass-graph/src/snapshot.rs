//! Backend-neutral immutable graph snapshots.
//!
//! The snapshot layer deliberately knows only the [`compass_store::Store`]
//! contract.  It stores canonical records in content-addressed ordered trees;
//! SQLite, redb, PostgreSQL, and a remote adapter therefore observe the same
//! logical layout.  The JSON artifact remains the compatibility engine and is
//! reconstructed from these records for export and differential testing.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use compass_model::code_graph::{
    CODE_GRAPH_SCHEMA_V1, EdgeRecord, FileRecord, GraphDiagnostic, GraphDocument, GraphMetadata,
    NodeDetails, NodeKind, NodeRecord,
};
use compass_model::validate_code_graph;
use compass_store::{
    ImmutableWrite, Key, MAX_IMMUTABLE_BATCH_BYTES, MAX_IMMUTABLE_BATCH_ITEMS, MAX_KEY_SEGMENTS,
    MAX_SCAN_BYTES, MAX_SCAN_ITEMS, MAX_VALUE_BYTES, NamespaceId, PartitionKey, Store, StoreError,
    WriteCondition, decode_key_segments, encode_key_segments,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::ser::Formatter;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

pub const GRAPH_SNAPSHOT_LAYOUT_V1: &str = "compass.store.graph-index/1";
pub const GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1: &str = "compass.store.graph-selector/1";
pub const GRAPH_SNAPSHOT_CANONICAL_ENCODING_V1: &str = "canonical-json-v1";
pub const DISCOVERY_SCOPE_INDEX_CAPABILITY_V1: &str = "compass.discovery-scope-index/1";
pub const IDENTIFIER_SUBWORD_INDEX_CAPABILITY_V1: &str = "__compass_cap_identifier_subwords_v1__";
pub const OPERATION_ROLE_TERM_INDEX_CAPABILITY_V1: &str = "__compass_cap_operation_role_terms_v1__";
pub const DECLARATION_TERM_INDEX_CAPABILITY_V1: &str = "__compass_cap_declaration_terms_v1__";
pub const RELATIONSHIP_TERM_INDEX_CAPABILITY_V1: &str = "__compass_cap_relationship_terms_v1__";
pub const GRAPH_SNAPSHOT_OBJECT_PARTITION: &str = "graph-snapshot/objects";
pub const GRAPH_SNAPSHOT_CATALOG_PARTITION: &str = "graph-snapshot/catalog";
pub const GRAPH_SNAPSHOT_ACTIVE_KEY: &str = "active";
pub const GRAPH_SNAPSHOT_MAX_DEPTH: usize = 64;
pub const GRAPH_SNAPSHOT_MAX_OBJECTS: usize = 100_000;
/// Maximum records materialized by one snapshot read/export request.
///
/// This is deliberately not a limit on the logical graph stored in the
/// content-addressed tree. Point and range queries remain independently
/// bounded even when a snapshot contains more records than one materialized
/// response may return.
pub const GRAPH_SNAPSHOT_MAX_ITEMS: usize = 5_000_000;
pub const GRAPH_SNAPSHOT_MAX_FANOUT: usize = 32;
pub const GRAPH_SNAPSHOT_MAX_LEAF_ENTRIES: usize = 128;
/// Maximum previous JSON artifact retained while attempting a byte-preserving
/// fact-neutral publication. Larger artifacts use the bounded streaming
/// serializer instead of adding another resident graph-sized buffer.
pub const GRAPH_JSON_DELTA_MAX_SOURCE_BYTES: usize = 512 * 1024 * 1024;
const TREE_ZSTD_MAGIC: &[u8; 5] = b"CSTZ1";
const TREE_ZSTD_HEADER_BYTES: usize = TREE_ZSTD_MAGIC.len() + std::mem::size_of::<u32>();
const TREE_OBJECT_CACHE_MAX_BYTES: usize = 7 * 1024 * 1024;
const TREE_OBJECT_CACHE_MAX_OBJECTS: usize = 1_024;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("snapshot store operation failed: {0}")]
    Store(#[from] StoreError),
    #[error("snapshot encoding failed: {0}")]
    Encode(String),
    #[error("snapshot decoding failed: {0}")]
    Decode(String),
    #[error("snapshot is corrupt: {0}")]
    Corrupt(String),
    #[error("snapshot format is unsupported: {0}")]
    Unsupported(String),
    #[error("snapshot limit exceeded: {0}")]
    Limit(String),
    #[error("snapshot capability unavailable: {0}")]
    CapabilityUnavailable(String),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexKind {
    Metadata,
    Nodes,
    Edges,
    Outgoing,
    Incoming,
    Files,
    Names,
    Terms,
    Communities,
    Diagnostics,
}

impl IndexKind {
    pub const ALL: [Self; 10] = [
        Self::Metadata,
        Self::Nodes,
        Self::Edges,
        Self::Outgoing,
        Self::Incoming,
        Self::Files,
        Self::Names,
        Self::Terms,
        Self::Communities,
        Self::Diagnostics,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Nodes => "nodes",
            Self::Edges => "edges",
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
            Self::Files => "files",
            Self::Names => "names",
            Self::Terms => "terms",
            Self::Communities => "communities",
            Self::Diagnostics => "diagnostics",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRoot {
    pub index: IndexKind,
    pub digest: String,
    pub entry_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshotManifest {
    pub schema: String,
    pub canonical_encoding: String,
    pub snapshot_id: String,
    pub graph_schema: String,
    pub graph_digest: String,
    pub graph_bytes: u64,
    pub node_count: u64,
    pub edge_count: u64,
    pub roots: Vec<SnapshotRoot>,
}

impl GraphSnapshotManifest {
    pub fn validate(&self) -> Result<(), SnapshotError> {
        if self.schema != GRAPH_SNAPSHOT_LAYOUT_V1 {
            return Err(SnapshotError::Unsupported(format!(
                "expected {GRAPH_SNAPSHOT_LAYOUT_V1}, found {}",
                self.schema
            )));
        }
        if self.canonical_encoding != GRAPH_SNAPSHOT_CANONICAL_ENCODING_V1 {
            return Err(SnapshotError::Unsupported(format!(
                "expected {GRAPH_SNAPSHOT_CANONICAL_ENCODING_V1}, found {}",
                self.canonical_encoding
            )));
        }
        if self.graph_schema != CODE_GRAPH_SCHEMA_V1 {
            return Err(SnapshotError::Unsupported(format!(
                "expected {CODE_GRAPH_SCHEMA_V1}, found {}",
                self.graph_schema
            )));
        }
        for (name, digest) in [
            ("snapshot_id", self.snapshot_id.as_str()),
            ("graph_digest", self.graph_digest.as_str()),
        ] {
            parse_digest(digest).map_err(|error| {
                SnapshotError::Corrupt(format!("{name} is not a SHA-256 digest: {error}"))
            })?;
        }
        if self.graph_bytes == 0 {
            return Err(SnapshotError::Corrupt(
                "graph byte count must be nonzero".to_owned(),
            ));
        }
        if self.roots.len() != IndexKind::ALL.len() {
            return Err(SnapshotError::Corrupt(format!(
                "manifest has {} roots; expected {}",
                self.roots.len(),
                IndexKind::ALL.len()
            )));
        }
        let mut previous = None;
        for root in &self.roots {
            if previous.is_some_and(|value| value >= root.index) {
                return Err(SnapshotError::Corrupt(
                    "manifest roots are not in deterministic order".to_owned(),
                ));
            }
            previous = Some(root.index);
            parse_digest(&root.digest).map_err(|error| {
                SnapshotError::Corrupt(format!("{} root digest: {error}", root.index.as_str()))
            })?;
        }
        if self
            .roots
            .iter()
            .map(|root| root.index)
            .collect::<BTreeSet<_>>()
            .len()
            != IndexKind::ALL.len()
        {
            return Err(SnapshotError::Corrupt(
                "manifest contains duplicate or missing index roots".to_owned(),
            ));
        }
        for (index, expected) in [
            (IndexKind::Nodes, self.node_count),
            (IndexKind::Edges, self.edge_count),
            (IndexKind::Outgoing, self.edge_count),
            (IndexKind::Incoming, self.edge_count),
        ] {
            let actual = self
                .roots
                .iter()
                .find(|root| root.index == index)
                .map(|root| root.entry_count)
                .ok_or_else(|| {
                    SnapshotError::Corrupt(format!("{} root is missing", index.as_str()))
                })?;
            if actual != expected {
                return Err(SnapshotError::Corrupt(format!(
                    "{} root count {actual} does not match manifest count {expected}",
                    index.as_str()
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSelector {
    pub schema: String,
    pub snapshot_id: String,
    pub manifest_digest: String,
}

impl SnapshotSelector {
    pub fn validate(&self) -> Result<(), SnapshotError> {
        if self.schema != GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1 {
            return Err(SnapshotError::Unsupported(format!(
                "expected {GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1}, found {}",
                self.schema
            )));
        }
        parse_digest(&self.snapshot_id)?;
        parse_digest(&self.manifest_digest)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshotMetadata {
    pub directed: bool,
    pub multigraph: bool,
    pub graph: GraphMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotReadLimits {
    pub max_items: usize,
    pub max_bytes: usize,
    pub max_objects: usize,
    pub max_depth: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TermPostingWork {
    pub chunks_decoded: u64,
    pub node_ids_decoded: u64,
}

impl Default for SnapshotReadLimits {
    fn default() -> Self {
        Self {
            max_items: MAX_SCAN_ITEMS,
            max_bytes: MAX_SCAN_BYTES,
            max_objects: 4_096,
            max_depth: GRAPH_SNAPSHOT_MAX_DEPTH,
        }
    }
}

impl SnapshotReadLimits {
    fn validate(self) -> Result<Self, SnapshotError> {
        if self.max_items == 0 || self.max_items > GRAPH_SNAPSHOT_MAX_ITEMS {
            return Err(SnapshotError::Limit(format!(
                "max_items must be between 1 and {GRAPH_SNAPSHOT_MAX_ITEMS}"
            )));
        }
        if self.max_bytes == 0 || self.max_bytes > MAX_VALUE_BYTES.saturating_mul(4_096) {
            return Err(SnapshotError::Limit(
                "max_bytes is outside the supported bounded range".to_owned(),
            ));
        }
        if self.max_objects == 0 || self.max_objects > GRAPH_SNAPSHOT_MAX_OBJECTS {
            return Err(SnapshotError::Limit(format!(
                "max_objects must be between 1 and {GRAPH_SNAPSHOT_MAX_OBJECTS}"
            )));
        }
        if self.max_depth == 0 || self.max_depth > GRAPH_SNAPSHOT_MAX_DEPTH {
            return Err(SnapshotError::Limit(format!(
                "max_depth must be between 1 and {GRAPH_SNAPSHOT_MAX_DEPTH}"
            )));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub struct PreparedGraphSnapshot {
    pub manifest: GraphSnapshotManifest,
    pub manifest_digest: String,
    pub new_objects: u64,
    pub reused_objects: u64,
    pub write_transactions: u64,
    pub bytes_written: u64,
}

/// Immutable index roots prepared independently from the canonical graph JSON
/// publication. Keeping this intermediate typed allows callers to stream and
/// hash `graph.json` concurrently, then bind its exact digest into the final
/// snapshot manifest without serializing the complete graph a second time.
pub struct PreparedGraphSnapshotContent {
    snapshot_id: String,
    node_count: u64,
    edge_count: u64,
    roots: Vec<SnapshotRoot>,
    stats: ObjectStats,
}

/// Borrowed canonical serialization view shared by `graph.json` publication,
/// identity verification, and store manifests. Record order is imposed through
/// reference vectors, so producing the view does not clone the graph records.
#[derive(Serialize)]
pub struct CanonicalGraphDocument<'a> {
    directed: bool,
    multigraph: bool,
    graph: GraphMetadata,
    nodes: Vec<&'a NodeRecord>,
    links: Vec<&'a EdgeRecord>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GraphSnapshotGcStats {
    pub retained_manifests: u64,
    pub retained_objects: u64,
    pub deleted_entries: u64,
    pub delete_transactions: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ObjectStats {
    new_objects: u64,
    reused_objects: u64,
    write_transactions: u64,
    bytes_written: u64,
}

struct ObjectWriter<'a, S: Store + ?Sized> {
    store: &'a S,
    namespace: NamespaceId,
    partition: PartitionKey,
    pending: Vec<ImmutableWrite>,
    pending_bytes: usize,
    max_items: usize,
    max_bytes: usize,
    stats: ObjectStats,
}

impl<'a, S: Store + ?Sized> ObjectWriter<'a, S> {
    fn new(store: &'a S) -> Result<Self, SnapshotError> {
        let capabilities = store.capabilities();
        let max_items = capabilities
            .max_immutable_batch_items
            .min(MAX_IMMUTABLE_BATCH_ITEMS);
        let max_bytes = capabilities
            .max_immutable_batch_bytes
            .min(MAX_IMMUTABLE_BATCH_BYTES);
        if max_items == 0 || max_bytes == 0 {
            return Err(SnapshotError::Unsupported(
                "store does not advertise bounded immutable batches".to_owned(),
            ));
        }
        Ok(Self {
            store,
            namespace: NamespaceId::graph(),
            partition: object_partition()?,
            pending: Vec::with_capacity(max_items),
            pending_bytes: 0,
            max_items,
            max_bytes,
            stats: ObjectStats::default(),
        })
    }

    fn put(&mut self, key: Key, bytes: Vec<u8>) -> Result<(), SnapshotError> {
        if bytes.len() > MAX_VALUE_BYTES {
            return Err(SnapshotError::Limit(
                "immutable object exceeds the store value limit".to_owned(),
            ));
        }
        if !self.pending.is_empty()
            && (self.pending.len() == self.max_items
                || self.pending_bytes.saturating_add(bytes.len()) > self.max_bytes)
        {
            self.flush()?;
        }
        if bytes.len() > self.max_bytes {
            return Err(SnapshotError::Limit(
                "immutable object exceeds the adapter batch byte limit".to_owned(),
            ));
        }
        self.pending_bytes = self.pending_bytes.saturating_add(bytes.len());
        self.pending
            .push(ImmutableWrite::new(self.partition.clone(), key, bytes)?);
        if self.pending.len() == self.max_items || self.pending_bytes == self.max_bytes {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), SnapshotError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let outcome = self
            .store
            .put_immutable_batch(&self.namespace, &self.pending)?;
        if outcome.entries.len() != self.pending.len()
            || outcome.new_entries.saturating_add(outcome.reused_entries)
                != self.pending.len() as u64
        {
            return Err(SnapshotError::Corrupt(
                "store returned an incomplete immutable batch outcome".to_owned(),
            ));
        }
        self.stats.new_objects = self.stats.new_objects.saturating_add(outcome.new_entries);
        self.stats.reused_objects = self
            .stats
            .reused_objects
            .saturating_add(outcome.reused_entries);
        self.stats.write_transactions = self
            .stats
            .write_transactions
            .saturating_add(outcome.transactions);
        self.stats.bytes_written = self
            .stats
            .bytes_written
            .saturating_add(outcome.bytes_written);
        self.pending.clear();
        self.pending_bytes = 0;
        Ok(())
    }

    fn finish(mut self) -> Result<ObjectStats, SnapshotError> {
        self.flush()?;
        Ok(self.stats)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataRecord {
    directed: bool,
    multigraph: bool,
    graph: GraphMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TermPostingChunk {
    term: String,
    node_ids: Vec<String>,
}

pub const GRAPH_TERM_POSTING_CHUNK_ITEMS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeEntry {
    key: Vec<u8>,
    value: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeChild {
    first_key: Vec<u8>,
    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TreeObject {
    Leaf {
        schema: String,
        index: IndexKind,
        entries: Vec<TreeEntry>,
    },
    Branch {
        schema: String,
        index: IndexKind,
        children: Vec<TreeChild>,
    },
}

struct CachedTreeObject {
    object: Arc<TreeObject>,
    resident_bytes: usize,
    leaf_last_used: Option<u64>,
}

#[derive(Default)]
struct TreeObjectCache {
    entries: BTreeMap<IndexKind, BTreeMap<String, CachedTreeObject>>,
    object_count: usize,
    resident_bytes: usize,
    clock: u64,
}

impl TreeObjectCache {
    fn get(&mut self, index: IndexKind, digest: &str) -> Option<Arc<TreeObject>> {
        self.clock = self.clock.saturating_add(1);
        let entry = self.entries.get_mut(&index)?.get_mut(digest)?;
        if let Some(last_used) = &mut entry.leaf_last_used {
            *last_used = self.clock;
        }
        Some(Arc::clone(&entry.object))
    }

    fn insert_or_get(
        &mut self,
        index: IndexKind,
        digest: &str,
        object: TreeObject,
    ) -> Arc<TreeObject> {
        if let Some(cached) = self.get(index, digest) {
            return cached;
        }
        self.clock = self.clock.saturating_add(1);
        let object = Arc::new(object);
        let key = digest.to_owned();
        let resident_bytes = cached_tree_object_resident_bytes(&key, object.as_ref());
        if resident_bytes > TREE_OBJECT_CACHE_MAX_BYTES {
            return object;
        }
        while self.object_count >= TREE_OBJECT_CACHE_MAX_OBJECTS
            || self.resident_bytes.saturating_add(resident_bytes) > TREE_OBJECT_CACHE_MAX_BYTES
        {
            let Some(eviction_key) = self
                .entries
                .iter()
                .flat_map(|(entry_index, entries)| {
                    entries.iter().filter_map(move |(entry_digest, entry)| {
                        entry
                            .leaf_last_used
                            .map(|used| (used, *entry_index, entry_digest))
                    })
                })
                .min()
                .map(|(_, entry_index, entry_digest)| (entry_index, entry_digest.clone()))
            else {
                return object;
            };
            if let Some(entries) = self.entries.get_mut(&eviction_key.0)
                && let Some(evicted) = entries.remove(&eviction_key.1)
            {
                self.object_count = self.object_count.saturating_sub(1);
                self.resident_bytes = self.resident_bytes.saturating_sub(evicted.resident_bytes);
            }
        }
        let leaf_last_used =
            matches!(object.as_ref(), TreeObject::Leaf { .. }).then_some(self.clock);
        self.object_count = self.object_count.saturating_add(1);
        self.resident_bytes = self.resident_bytes.saturating_add(resident_bytes);
        self.entries.entry(index).or_default().insert(
            key,
            CachedTreeObject {
                object: Arc::clone(&object),
                resident_bytes,
                leaf_last_used,
            },
        );
        object
    }
}

fn cached_tree_object_resident_bytes(digest: &String, object: &TreeObject) -> usize {
    // Account owned allocation capacities, the Arc header, and a conservative
    // BTreeMap node allowance. This is intentionally stricter than serialized
    // bytes because decoded entry vectors and their nested buffers coexist.
    let allocation_overhead = size_of::<CachedTreeObject>()
        .saturating_add(size_of::<String>())
        .saturating_add(size_of::<TreeObject>())
        .saturating_add(size_of::<usize>().saturating_mul(8));
    let mut bytes = allocation_overhead.saturating_add(digest.capacity());
    match object {
        TreeObject::Leaf {
            schema, entries, ..
        } => {
            bytes = bytes
                .saturating_add(schema.capacity())
                .saturating_add(entries.capacity().saturating_mul(size_of::<TreeEntry>()));
            for entry in entries {
                bytes = bytes
                    .saturating_add(entry.key.capacity())
                    .saturating_add(entry.value.capacity());
            }
        }
        TreeObject::Branch {
            schema, children, ..
        } => {
            bytes = bytes
                .saturating_add(schema.capacity())
                .saturating_add(children.capacity().saturating_mul(size_of::<TreeChild>()));
            for child in children {
                bytes = bytes
                    .saturating_add(child.first_key.capacity())
                    .saturating_add(child.digest.capacity());
            }
        }
    }
    bytes
}

pub struct GraphSnapshotBuilder;

impl GraphSnapshotBuilder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn prepare<S: Store + ?Sized>(
        &self,
        store: &S,
        graph: &GraphDocument,
    ) -> Result<PreparedGraphSnapshot, SnapshotError> {
        self.prepare_canonical(store, graph)
    }

    /// Prepare a snapshot from an owned document without cloning the complete
    /// graph solely to establish deterministic record ordering.
    pub fn prepare_owned<S: Store + ?Sized>(
        &self,
        store: &S,
        graph: GraphDocument,
    ) -> Result<PreparedGraphSnapshot, SnapshotError> {
        self.prepare_canonical(store, &graph)
    }

    /// Prepare a canonical snapshot without cloning graph records or buffering
    /// its complete JSON representation. Ordering is imposed through bounded
    /// reference vectors while `graph.json` can be streamed in parallel.
    pub fn prepare_canonical<S: Store + ?Sized>(
        &self,
        store: &S,
        canonical: &GraphDocument,
    ) -> Result<PreparedGraphSnapshot, SnapshotError> {
        let (graph_digest, graph_bytes) = digest_canonical_graph(canonical, false)?;
        let content = self.prepare_content(store, canonical)?;
        self.finish_content(store, content, graph_digest, graph_bytes)
    }

    /// Prepare content-addressed logical indexes without constructing the
    /// manifest. This is the expensive backend-neutral phase and is safe to
    /// run concurrently with canonical `graph.json` publication.
    pub fn prepare_content<S: Store + ?Sized>(
        &self,
        store: &S,
        canonical: &GraphDocument,
    ) -> Result<PreparedGraphSnapshotContent, SnapshotError> {
        validate_code_graph(canonical)
            .map_err(|error| SnapshotError::Corrupt(format!("graph validation failed: {error}")))?;
        let snapshot_id = snapshot_identity(canonical)?;
        let mut writer = ObjectWriter::new(store)?;
        let mut roots = Vec::with_capacity(IndexKind::ALL.len());
        for index in IndexKind::ALL {
            // Keep only one encoded index in memory at a time. The previous
            // implementation retained all node, edge, and secondary-index
            // values until every tree had been written, making publication
            // peak several times larger than the canonical graph artifact.
            let term_postings = (index == IndexKind::Terms).then(|| build_term_postings(canonical));
            let entries = build_index(canonical, index, term_postings.as_ref())?;
            let entry_count = entries.len() as u64;
            let digest = build_index_tree(&mut writer, index, entries)?;
            roots.push(SnapshotRoot {
                index,
                digest,
                entry_count,
            });
        }
        let stats = writer.finish()?;
        Ok(PreparedGraphSnapshotContent {
            snapshot_id,
            node_count: canonical.nodes.len() as u64,
            edge_count: canonical.links.len() as u64,
            roots,
            stats,
        })
    }

    /// Prepare a snapshot for a source-only delta. This path reuses every
    /// immutable index tree except metadata and the changed file records. It
    /// is intentionally strict: callers may use it only when node and edge
    /// topology is unchanged and the changed nodes are file records whose
    /// content metadata moved with the edit.
    pub fn prepare_file_node_delta<S: Store + ?Sized>(
        &self,
        store: &S,
        previous: &GraphDocument,
        current: &GraphDocument,
    ) -> Result<PreparedGraphSnapshotContent, SnapshotError> {
        validate_code_graph(current)
            .map_err(|error| SnapshotError::Corrupt(format!("graph validation failed: {error}")))?;
        validate_file_node_delta(previous, current)?;
        let reader = GraphSnapshotReader::open_active(store)?.ok_or_else(|| {
            SnapshotError::Unsupported("file-node delta requires an active snapshot".to_owned())
        })?;
        let previous_snapshot_id = snapshot_identity(previous)?;
        if reader.selector().snapshot_id != previous_snapshot_id
            || reader.manifest().node_count != previous.nodes.len() as u64
            || reader.manifest().edge_count != previous.links.len() as u64
        {
            return Err(SnapshotError::Corrupt(
                "active snapshot does not match the previous graph".to_owned(),
            ));
        }

        let mut node_updates = BTreeMap::new();
        for (previous_node, node) in previous.nodes.iter().zip(&current.nodes) {
            if previous_node != node {
                node_updates.insert(
                    encode_graph_index_key(IndexKind::Nodes, &[node.id.as_bytes()])?,
                    Some(encode_json(node)?),
                );
            }
        }
        if node_updates.is_empty() {
            return Err(SnapshotError::Corrupt(
                "file-node delta contains no changed node records".to_owned(),
            ));
        }

        let mut writer = ObjectWriter::new(store)?;
        let metadata_entries = build_index(current, IndexKind::Metadata, None)?;
        let metadata_entry_count = metadata_entries.len() as u64;
        let metadata_digest = build_index_tree(&mut writer, IndexKind::Metadata, metadata_entries)?;
        let mut roots = reader.manifest().roots.clone();
        for root in &mut roots {
            if root.index == IndexKind::Metadata {
                root.entry_count = metadata_entry_count;
                root.digest = metadata_digest.clone();
            } else if root.index == IndexKind::Nodes {
                root.digest = update_index_tree(
                    store,
                    &mut writer,
                    root.index,
                    &root.digest,
                    &node_updates,
                    0,
                )?;
            }
        }
        let stats = writer.finish()?;
        Ok(PreparedGraphSnapshotContent {
            snapshot_id: snapshot_identity(current)?,
            node_count: current.nodes.len() as u64,
            edge_count: current.links.len() as u64,
            roots,
            stats,
        })
    }

    /// Prepare a point-update snapshot when graph metadata and node payloads
    /// change without changing any secondary-index projection. Callers must
    /// supply the exact changed-node set; the preflight proves that node IDs,
    /// relationships, file-path keys, names, terms, and communities remain
    /// unchanged before reusing their immutable roots.
    pub fn prepare_node_value_delta<S: Store + ?Sized>(
        &self,
        store: &S,
        previous: &GraphDocument,
        current: &GraphDocument,
        changed_node_ids: &BTreeSet<String>,
    ) -> Result<PreparedGraphSnapshotContent, SnapshotError> {
        validate_code_graph(current)
            .map_err(|error| SnapshotError::Corrupt(format!("graph validation failed: {error}")))?;
        validate_node_value_delta(previous, current, changed_node_ids)?;
        let reader = GraphSnapshotReader::open_active(store)?.ok_or_else(|| {
            SnapshotError::Unsupported("node-value delta requires an active snapshot".to_owned())
        })?;
        let previous_snapshot_id = snapshot_identity(previous)?;
        if reader.selector().snapshot_id != previous_snapshot_id
            || reader.manifest().node_count != previous.nodes.len() as u64
            || reader.manifest().edge_count != previous.links.len() as u64
        {
            return Err(SnapshotError::Corrupt(
                "active snapshot does not match the previous graph".to_owned(),
            ));
        }

        let current_nodes = current
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect::<BTreeMap<_, _>>();
        let node_updates = changed_node_ids
            .iter()
            .map(|id| {
                let node = current_nodes.get(id.as_str()).ok_or_else(|| {
                    SnapshotError::Corrupt(format!("node-value delta is missing changed node {id}"))
                })?;
                Ok((
                    encode_graph_index_key(IndexKind::Nodes, &[id.as_bytes()])?,
                    Some(encode_json(node)?),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SnapshotError>>()?;

        let mut writer = ObjectWriter::new(store)?;
        let metadata_entries = build_index(current, IndexKind::Metadata, None)?;
        let metadata_entry_count = metadata_entries.len() as u64;
        let metadata_digest = build_index_tree(&mut writer, IndexKind::Metadata, metadata_entries)?;
        let mut roots = reader.manifest().roots.clone();
        for root in &mut roots {
            if root.index == IndexKind::Metadata {
                root.entry_count = metadata_entry_count;
                root.digest = metadata_digest.clone();
            } else if root.index == IndexKind::Nodes {
                root.digest = update_index_tree(
                    store,
                    &mut writer,
                    root.index,
                    &root.digest,
                    &node_updates,
                    0,
                )?;
            }
        }
        let stats = writer.finish()?;
        Ok(PreparedGraphSnapshotContent {
            snapshot_id: snapshot_identity(current)?,
            node_count: current.nodes.len() as u64,
            edge_count: current.links.len() as u64,
            roots,
            stats,
        })
    }

    /// Prepare a bounded graph delta when an incremental edit changes graph
    /// records or relationships. Unchanged immutable index trees are reused;
    /// only indexes whose logical projection depends on changed records are
    /// rebuilt. This preserves the full graph contract while avoiding a full
    /// snapshot rewrite for small topology edits.
    pub fn prepare_graph_delta<S: Store + ?Sized>(
        &self,
        store: &S,
        previous: &GraphDocument,
        current: &GraphDocument,
    ) -> Result<PreparedGraphSnapshotContent, SnapshotError> {
        validate_code_graph(current)
            .map_err(|error| SnapshotError::Corrupt(format!("graph validation failed: {error}")))?;
        validate_graph_delta(previous, current)?;
        let reader = GraphSnapshotReader::open_active(store)?.ok_or_else(|| {
            SnapshotError::Unsupported("graph delta requires an active snapshot".to_owned())
        })?;
        let previous_snapshot_id = snapshot_identity(previous)?;
        if reader.selector().snapshot_id != previous_snapshot_id
            || reader.manifest().node_count != previous.nodes.len() as u64
            || reader.manifest().edge_count != previous.links.len() as u64
        {
            return Err(SnapshotError::Corrupt(
                "active snapshot does not match the previous graph".to_owned(),
            ));
        }

        let changed_indexes = graph_delta_indexes(previous, current);
        if changed_indexes.is_empty() {
            return Err(SnapshotError::Corrupt(
                "graph delta contains no changed index projections".to_owned(),
            ));
        }
        let mut writer = ObjectWriter::new(store)?;
        let mut roots = reader.manifest().roots.clone();
        for root in &mut roots {
            if !changed_indexes.contains(&root.index) {
                continue;
            }
            let previous_term_postings =
                (root.index == IndexKind::Terms).then(|| build_term_postings(previous));
            let current_term_postings =
                (root.index == IndexKind::Terms).then(|| build_term_postings(current));
            let previous_entries =
                build_index(previous, root.index, previous_term_postings.as_ref())?;
            let current_entries = build_index(current, root.index, current_term_postings.as_ref())?;
            if previous_entries == current_entries {
                continue;
            }
            root.entry_count = current_entries.len() as u64;
            root.digest = if previous_entries.keys().eq(current_entries.keys()) {
                let updates = current_entries
                    .iter()
                    .filter(|(key, value)| previous_entries.get(*key) != Some(*value))
                    .map(|(key, value)| (key.clone(), Some(value.clone())))
                    .collect::<BTreeMap<_, _>>();
                update_index_tree(store, &mut writer, root.index, &root.digest, &updates, 0)?
            } else {
                // Insertions and deletions can move persistent-tree separators.
                // Rebuild this index conservatively; point updates are safe only
                // when the complete ordered key set is unchanged.
                build_index_tree(&mut writer, root.index, current_entries)?
            };
        }
        let stats = writer.finish()?;
        Ok(PreparedGraphSnapshotContent {
            snapshot_id: snapshot_identity(current)?,
            node_count: current.nodes.len() as u64,
            edge_count: current.links.len() as u64,
            roots,
            stats,
        })
    }

    /// Bind prepared index content to the digest of the exact compact
    /// canonical JSON bytes and publish the immutable manifest.
    pub fn finish_content<S: Store + ?Sized>(
        &self,
        store: &S,
        content: PreparedGraphSnapshotContent,
        graph_digest: String,
        graph_bytes: u64,
    ) -> Result<PreparedGraphSnapshot, SnapshotError> {
        let manifest = GraphSnapshotManifest {
            schema: GRAPH_SNAPSHOT_LAYOUT_V1.to_owned(),
            canonical_encoding: GRAPH_SNAPSHOT_CANONICAL_ENCODING_V1.to_owned(),
            snapshot_id: content.snapshot_id,
            graph_schema: CODE_GRAPH_SCHEMA_V1.to_owned(),
            graph_digest,
            graph_bytes,
            node_count: content.node_count,
            edge_count: content.edge_count,
            roots: content.roots,
        };
        manifest.validate()?;
        let manifest_bytes = encode_json(&manifest)?;
        let manifest_digest = hex_digest(&manifest_bytes);
        let mut writer = ObjectWriter::new(store)?;
        put_immutable_object(&mut writer, manifest_key(&manifest_digest)?, manifest_bytes)?;
        let stats = writer.finish()?;
        Ok(PreparedGraphSnapshot {
            manifest,
            manifest_digest,
            new_objects: content.stats.new_objects.saturating_add(stats.new_objects),
            reused_objects: content
                .stats
                .reused_objects
                .saturating_add(stats.reused_objects),
            write_transactions: content
                .stats
                .write_transactions
                .saturating_add(stats.write_transactions),
            bytes_written: content
                .stats
                .bytes_written
                .saturating_add(stats.bytes_written),
        })
    }

    pub fn activate<S: Store + ?Sized>(
        &self,
        store: &S,
        prepared: &PreparedGraphSnapshot,
    ) -> Result<SnapshotSelector, SnapshotError> {
        activate_graph_snapshot(store, prepared)
    }
}

impl Default for GraphSnapshotBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Build all immutable content and the manifest, but do not change the active
/// selector.  A caller can safely abandon this result; unreachable content is
/// eligible for a future store garbage collector.
pub fn prepare_graph_snapshot<S: Store + ?Sized>(
    store: &S,
    graph: &GraphDocument,
) -> Result<PreparedGraphSnapshot, SnapshotError> {
    GraphSnapshotBuilder::new().prepare(store, graph)
}

/// Encode a graph using the same deterministic normalization used by a store
/// snapshot. Publication and cross-engine verification use this helper instead of
/// comparing source-order JSON bytes, so equivalent graph records have one
/// stable identity across adapters.
pub fn canonical_graph_json(graph: &GraphDocument) -> Result<Vec<u8>, SnapshotError> {
    encode_json(&canonical_graph_document(graph))
}

#[must_use]
pub fn canonical_graph_document(graph: &GraphDocument) -> CanonicalGraphDocument<'_> {
    canonical_graph_document_with_generation(graph, false)
}

/// Build the canonical serialization view for a document that already has
/// v1's node and link ordering. The v1 publication boundary guarantees node
/// order by ID and link order by the same tuple used by the canonical view;
/// retaining those references avoids sorting two large pointer vectors again
/// immediately before graph publication.
#[must_use]
pub fn canonical_graph_document_presorted(graph: &GraphDocument) -> CanonicalGraphDocument<'_> {
    let mut metadata = graph.graph.clone();
    metadata.files.sort_by(|left, right| left.id.cmp(&right.id));
    CanonicalGraphDocument {
        directed: graph.directed,
        multigraph: graph.multigraph,
        graph: metadata,
        nodes: graph.nodes.iter().collect(),
        links: graph.links.iter().collect(),
    }
}

// Keep enough independent work for modest real-repository graphs while
// bounding each temporary JSON buffer. At 16K records, a 10K-node/27K-edge
// graph exposed only three serialization tasks and retained multi-megabyte
// chunks; 8K preserves amortized encoding while allowing the in-flight bound
// to control both parallelism and peak memory.
const CANONICAL_RECORD_CHUNK: usize = 8_192;
const CANONICAL_RECORD_IN_FLIGHT: usize = 4;

/// Stream the canonical graph JSON while serializing record chunks in
/// parallel. The chunk boundary keeps temporary buffers bounded, and records
/// are still written in the exact v1 order used by the canonical view.
pub fn write_canonical_graph_json<W: Write + ?Sized>(
    graph: &GraphDocument,
    writer: &mut W,
) -> io::Result<()> {
    writer.write_all(b"{\"directed\":")?;
    serde_json::to_writer(&mut *writer, &graph.directed).map_err(io::Error::other)?;
    writer.write_all(b",\"multigraph\":")?;
    serde_json::to_writer(&mut *writer, &graph.multigraph).map_err(io::Error::other)?;
    writer.write_all(b",\"graph\":")?;
    if graph_metadata_files_are_canonical(graph) {
        // V1 publication already sorts the inventory. Borrowing the metadata
        // on that hot path avoids cloning every file record before the graph
        // stream starts; arbitrary caller-provided documents still take the
        // canonicalizing fallback below.
        serde_json::to_writer(&mut *writer, &graph.graph).map_err(io::Error::other)?;
    } else {
        let mut metadata = graph.graph.clone();
        metadata.files.sort_by(|left, right| left.id.cmp(&right.id));
        serde_json::to_writer(&mut *writer, &metadata).map_err(io::Error::other)?;
    }
    writer.write_all(b",\"nodes\":")?;
    write_canonical_record_array(writer, &graph.nodes)?;
    writer.write_all(b",\"links\":")?;
    write_canonical_record_array(writer, &graph.links)?;
    writer.write_all(b"}")
}

/// Publish a fact-neutral graph edit by reusing the previous canonical node
/// and link bytes. Fact-neutral edits only update file-node metadata and graph
/// inventory; the semantic node IDs and every relationship remain unchanged.
///
/// The function performs a complete structural preflight before writing any
/// bytes. It returns `Ok(false)` when the previous artifact is not the
/// canonical node-link shape or when the supplied changed-node set is not
/// compatible with a node-value-only delta, allowing the caller to use the normal
/// full serializer without risking a partial duplicate document.
pub fn write_fact_neutral_graph_json_delta<W: Write + ?Sized>(
    previous_bytes: &[u8],
    graph: &GraphDocument,
    changed_node_ids: &BTreeSet<String>,
    writer: &mut W,
) -> io::Result<bool> {
    write_fact_neutral_graph_json_delta_inner(previous_bytes, graph, changed_node_ids, true, writer)
}

/// Publish a fact-neutral edit after the caller has already validated the
/// previous graph document. The core pipeline uses this variant because it
/// loads `graph.json` as a bounded, typed `GraphDocument` before attempting the
/// byte-preserving publication. Skipping the redundant per-record payload
/// deserialization keeps large incremental edits bounded by one graph parse.
pub fn write_fact_neutral_graph_json_delta_prevalidated<W: Write + ?Sized>(
    previous_bytes: &[u8],
    graph: &GraphDocument,
    changed_node_ids: &BTreeSet<String>,
    writer: &mut W,
) -> io::Result<bool> {
    write_fact_neutral_graph_json_delta_inner(
        previous_bytes,
        graph,
        changed_node_ids,
        false,
        writer,
    )
}

fn write_fact_neutral_graph_json_delta_inner<W: Write + ?Sized>(
    previous_bytes: &[u8],
    graph: &GraphDocument,
    changed_node_ids: &BTreeSet<String>,
    validate_records: bool,
    writer: &mut W,
) -> io::Result<bool> {
    if previous_bytes.is_empty() || previous_bytes.len() > GRAPH_JSON_DELTA_MAX_SOURCE_BYTES {
        return Ok(false);
    }
    let Some(nodes_range) = top_level_member_range(previous_bytes, "nodes") else {
        return Ok(false);
    };
    // The canonical writer emits `links` immediately after `nodes`. Starting
    // from the end of the already located nodes value avoids rescanning the
    // complete node array just to find the next top-level member.
    let Some(links_range) = top_level_member_range_after(previous_bytes, nodes_range.end, "links")
    else {
        return Ok(false);
    };
    let Some(node_ranges) = json_array_element_ranges_matching(
        previous_bytes,
        nodes_range.clone(),
        graph.nodes.iter().map(|node| node.id.as_str()),
        validate_records,
    ) else {
        return Ok(false);
    };
    if node_ranges.len() != graph.nodes.len()
        || !json_array_identities_match(
            previous_bytes,
            links_range.clone(),
            graph.links.iter().map(|link| link.id.as_str()),
            validate_records,
        )
        .unwrap_or(false)
    {
        return Ok(false);
    }

    let mut changed_seen = BTreeSet::new();
    for (index, _) in node_ranges.iter().enumerate() {
        let current = &graph.nodes[index];
        if changed_node_ids.contains(&current.id) {
            changed_seen.insert(current.id.clone());
        }
    }
    if changed_seen.len() != changed_node_ids.len() {
        return Ok(false);
    }

    writer.write_all(b"{\"directed\":")?;
    serde_json::to_writer(&mut *writer, &graph.directed).map_err(io::Error::other)?;
    writer.write_all(b",\"multigraph\":")?;
    serde_json::to_writer(&mut *writer, &graph.multigraph).map_err(io::Error::other)?;
    writer.write_all(b",\"graph\":")?;
    if graph_metadata_files_are_canonical(graph) {
        serde_json::to_writer(&mut *writer, &graph.graph).map_err(io::Error::other)?;
    } else {
        let mut metadata = graph.graph.clone();
        metadata.files.sort_by(|left, right| left.id.cmp(&right.id));
        serde_json::to_writer(&mut *writer, &metadata).map_err(io::Error::other)?;
    }
    writer.write_all(b",\"nodes\":[")?;
    for (index, range) in node_ranges.iter().enumerate() {
        if index > 0 {
            writer.write_all(b",")?;
        }
        let node = &graph.nodes[index];
        if changed_node_ids.contains(&node.id) {
            serde_json::to_writer(&mut *writer, node).map_err(io::Error::other)?;
        } else {
            writer.write_all(&previous_bytes[range.clone()])?;
        }
    }
    writer.write_all(b"],\"links\":")?;
    writer.write_all(&previous_bytes[links_range])?;
    writer.write_all(b"}")?;
    Ok(true)
}

#[derive(Deserialize)]
struct JsonRecordIdentity<'a> {
    #[serde(borrow)]
    id: Cow<'a, str>,
}

fn json_record_identity(bytes: &[u8], validate_record: bool) -> Option<Cow<'_, str>> {
    if validate_record {
        return serde_json::from_slice::<JsonRecordIdentity<'_>>(bytes)
            .ok()
            .map(|record| record.id);
    }
    // Canonical node/link records serialize `id` first.  The caller has
    // already loaded the previous graph through `GraphDocument`, so the
    // complete record payload has been validated before this byte-preserving
    // preflight runs.  Avoid deserializing every large record a second time;
    // the array scanner above still validates the record boundaries and this
    // check only needs the leading identity used to prove ordering.
    let object_start = skip_json_whitespace(bytes, 0);
    if bytes.get(object_start) != Some(&b'{') {
        return None;
    }
    let key_start = skip_json_whitespace(bytes, object_start.saturating_add(1));
    let key_end = json_string_end(bytes, key_start)?;
    if &bytes[key_start..key_end] != b"\"id\"" {
        return None;
    }
    let colon = skip_json_whitespace(bytes, key_end);
    if bytes.get(colon) != Some(&b':') {
        return None;
    }
    let value_start = skip_json_whitespace(bytes, colon.saturating_add(1));
    let value_end = json_string_end(bytes, value_start)?;
    let raw = bytes.get(value_start.saturating_add(1)..value_end.saturating_sub(1))?;
    if raw.iter().all(|byte| *byte >= 0x20 && *byte != b'\\') {
        return std::str::from_utf8(raw).ok().map(Cow::Borrowed);
    }
    serde_json::from_slice::<Cow<'_, str>>(&bytes[value_start..value_end]).ok()
}

fn top_level_member_range(bytes: &[u8], wanted: &str) -> Option<Range<usize>> {
    let object_start = skip_json_whitespace(bytes, 0);
    if bytes.get(object_start) != Some(&b'{') {
        return None;
    }
    let mut index = skip_json_whitespace(bytes, object_start.saturating_add(1));
    if bytes.get(index) == Some(&b'}') {
        return None;
    }
    loop {
        let key_start = index;
        let key_end = json_string_end(bytes, key_start)?;
        index = skip_json_whitespace(bytes, key_end);
        if bytes.get(index) != Some(&b':') {
            return None;
        }
        let value_start = skip_json_whitespace(bytes, index.saturating_add(1));
        let value_end = json_value_end(bytes, value_start)?;
        if json_key_matches(bytes, key_start..key_end, wanted) {
            return Some(value_start..value_end);
        }
        index = skip_json_whitespace(bytes, value_end);
        match bytes.get(index) {
            Some(b',') => {
                index = skip_json_whitespace(bytes, index.saturating_add(1));
            }
            Some(b'}') => return None,
            _ => return None,
        }
    }
}

fn top_level_member_range_after(
    bytes: &[u8],
    previous_value_end: usize,
    wanted: &str,
) -> Option<Range<usize>> {
    let comma = skip_json_whitespace(bytes, previous_value_end);
    if bytes.get(comma) != Some(&b',') {
        return None;
    }
    let key_start = skip_json_whitespace(bytes, comma.saturating_add(1));
    let key_end = json_string_end(bytes, key_start)?;
    if !json_key_matches(bytes, key_start..key_end, wanted) {
        return None;
    }
    let colon = skip_json_whitespace(bytes, key_end);
    if bytes.get(colon) != Some(&b':') {
        return None;
    }
    let value_start = skip_json_whitespace(bytes, colon.saturating_add(1));
    let value_end = json_value_end(bytes, value_start)?;
    Some(value_start..value_end)
}

fn json_key_matches(bytes: &[u8], range: Range<usize>, wanted: &str) -> bool {
    let key = &bytes[range];
    key.len() == wanted.len().saturating_add(2)
        && key.first() == Some(&b'"')
        && key.last() == Some(&b'"')
        && &key[1..key.len().saturating_sub(1)] == wanted.as_bytes()
}

fn json_array_element_ranges_matching<'a>(
    bytes: &[u8],
    range: Range<usize>,
    expected_ids: impl Iterator<Item = &'a str>,
    validate_records: bool,
) -> Option<Vec<Range<usize>>> {
    if bytes.get(range.start) != Some(&b'[')
        || range.end <= range.start.saturating_add(1)
        || bytes.get(range.end.saturating_sub(1)) != Some(&b']')
    {
        return None;
    }
    let mut elements = Vec::new();
    let mut expected_ids = expected_ids;
    let mut index = skip_json_whitespace(bytes, range.start.saturating_add(1));
    if index == range.end.saturating_sub(1) {
        return expected_ids.next().is_none().then_some(elements);
    }
    loop {
        let value_start = index;
        let value_end = json_value_end(bytes, value_start)?;
        if value_end > range.end.saturating_sub(1) {
            return None;
        }
        if elements.len() >= GRAPH_SNAPSHOT_MAX_ITEMS {
            return None;
        }
        let expected_id = expected_ids.next()?;
        let identity = json_record_identity(&bytes[value_start..value_end], validate_records)?;
        if identity.as_ref() != expected_id {
            return None;
        }
        elements.push(value_start..value_end);
        index = skip_json_whitespace(bytes, value_end);
        match bytes.get(index) {
            Some(b',') => {
                index = skip_json_whitespace(bytes, index.saturating_add(1));
                if index >= range.end.saturating_sub(1) {
                    return None;
                }
            }
            Some(b']') if index == range.end.saturating_sub(1) => {
                return expected_ids.next().is_none().then_some(elements);
            }
            _ => return None,
        }
    }
}

fn json_array_identities_match<'a>(
    bytes: &[u8],
    range: Range<usize>,
    expected_ids: impl Iterator<Item = &'a str>,
    validate_records: bool,
) -> Option<bool> {
    if bytes.get(range.start) != Some(&b'[')
        || range.end <= range.start.saturating_add(1)
        || bytes.get(range.end.saturating_sub(1)) != Some(&b']')
    {
        return None;
    }
    let mut expected_ids = expected_ids;
    let mut element_count = 0_usize;
    let mut index = skip_json_whitespace(bytes, range.start.saturating_add(1));
    if index == range.end.saturating_sub(1) {
        return Some(expected_ids.next().is_none());
    }
    loop {
        let value_start = index;
        let value_end = json_value_end(bytes, value_start)?;
        if value_end > range.end.saturating_sub(1) {
            return None;
        }
        if element_count >= GRAPH_SNAPSHOT_MAX_ITEMS {
            return None;
        }
        element_count = element_count.saturating_add(1);
        let Some(expected_id) = expected_ids.next() else {
            return Some(false);
        };
        let identity = json_record_identity(&bytes[value_start..value_end], validate_records)?;
        if identity.as_ref() != expected_id {
            return Some(false);
        }
        index = skip_json_whitespace(bytes, value_end);
        match bytes.get(index) {
            Some(b',') => {
                index = skip_json_whitespace(bytes, index.saturating_add(1));
                if index >= range.end.saturating_sub(1) {
                    return None;
                }
            }
            Some(b']') if index == range.end.saturating_sub(1) => {
                return Some(expected_ids.next().is_none());
            }
            _ => return None,
        }
    }
}

fn skip_json_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
    {
        index = index.saturating_add(1);
    }
    index
}

fn json_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate().skip(start.saturating_add(1)) {
        let byte = *byte;
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Some(index.saturating_add(1));
        }
    }
    None
}

fn json_value_end(bytes: &[u8], start: usize) -> Option<usize> {
    let start = skip_json_whitespace(bytes, start);
    match bytes.get(start) {
        Some(b'"') => json_string_end(bytes, start),
        Some(b'{') | Some(b'[') => {
            let mut stack = vec![if bytes[start] == b'{' { b'}' } else { b']' }];
            let mut index = start.saturating_add(1);
            while index < bytes.len() {
                match bytes[index] {
                    b'"' => index = json_string_end(bytes, index)?,
                    b'{' => {
                        if stack.len() >= GRAPH_SNAPSHOT_MAX_DEPTH {
                            return None;
                        }
                        stack.push(b'}');
                        index = index.saturating_add(1);
                    }
                    b'[' => {
                        if stack.len() >= GRAPH_SNAPSHOT_MAX_DEPTH {
                            return None;
                        }
                        stack.push(b']');
                        index = index.saturating_add(1);
                    }
                    b'}' | b']' => {
                        if stack.pop() != Some(bytes[index]) {
                            return None;
                        }
                        index = index.saturating_add(1);
                        if stack.is_empty() {
                            return Some(index);
                        }
                    }
                    _ => index = index.saturating_add(1),
                }
            }
            None
        }
        Some(_) => {
            let mut index = start;
            while bytes.get(index).is_some_and(|byte| {
                !matches!(byte, b' ' | b'\n' | b'\r' | b'\t' | b',' | b']' | b'}')
            }) {
                index = index.saturating_add(1);
            }
            (index > start).then_some(index)
        }
        None => None,
    }
}

fn graph_metadata_files_are_canonical(graph: &GraphDocument) -> bool {
    graph
        .graph
        .files
        .windows(2)
        .all(|pair| pair[0].id.as_str() <= pair[1].id.as_str())
}

fn write_canonical_record_array<W, T>(writer: &mut W, records: &[T]) -> io::Result<()>
where
    W: Write + ?Sized,
    T: Serialize + Sync,
{
    // Most small and medium repositories fit in one bounded chunk. Avoid
    // dispatching a one-chunk array through Rayon: that path cannot overlap
    // useful work and otherwise pays the global-pool scheduling cost during
    // the latency-sensitive final publication step.
    if records.len() <= CANONICAL_RECORD_CHUNK {
        writer.write_all(b"[")?;
        if !records.is_empty() {
            writer.write_all(&encode_canonical_record_chunk(records)?)?;
        }
        return writer.write_all(b"]");
    }
    writer.write_all(b"[")?;
    let mut chunks = records.chunks(CANONICAL_RECORD_CHUNK);
    let mut first = true;
    loop {
        let batch = (0..CANONICAL_RECORD_IN_FLIGHT)
            .filter_map(|_| chunks.next())
            .collect::<Vec<_>>();
        if batch.is_empty() {
            break;
        }
        let encoded = batch
            .par_iter()
            .map(|records| encode_canonical_record_chunk(records))
            .collect::<Result<Vec<_>, _>>()?;
        for chunk in encoded {
            if !first {
                writer.write_all(b",")?;
            }
            writer.write_all(&chunk)?;
            first = false;
        }
    }
    writer.write_all(b"]")
}

fn encode_canonical_record_chunk<T: Serialize>(records: &[T]) -> io::Result<Vec<u8>> {
    let mut encoded = Vec::new();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut encoded, ArrayBodyFormatter::default());
    records
        .serialize(&mut serializer)
        .map_err(io::Error::other)?;
    Ok(encoded)
}

#[derive(Clone, Debug, Default)]
struct ArrayBodyFormatter {
    depth: usize,
}

impl Formatter for ArrayBodyFormatter {
    fn begin_array<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        let outer = self.depth == 0;
        self.depth = self.depth.saturating_add(1);
        if outer {
            Ok(())
        } else {
            writer.write_all(b"[")
        }
    }

    fn end_array<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        let outer = self.depth == 1;
        self.depth = self.depth.saturating_sub(1);
        if outer {
            Ok(())
        } else {
            writer.write_all(b"]")
        }
    }
}

pub fn activate_graph_snapshot<S: Store + ?Sized>(
    store: &S,
    prepared: &PreparedGraphSnapshot,
) -> Result<SnapshotSelector, SnapshotError> {
    prepared.manifest.validate()?;
    parse_digest(&prepared.manifest_digest)?;
    let namespace = NamespaceId::graph();
    let objects = object_partition()?;
    let manifest_key = manifest_key(&prepared.manifest_digest)?;
    let Some(manifest_entry) = store.get(&namespace, &objects, &manifest_key)? else {
        return Err(SnapshotError::Corrupt(
            "prepared snapshot manifest is missing".to_owned(),
        ));
    };
    verify_digest(&manifest_entry.value, &prepared.manifest_digest)?;
    let stored_manifest = decode_json::<GraphSnapshotManifest>(&manifest_entry.value)?;
    if stored_manifest != prepared.manifest {
        return Err(SnapshotError::Corrupt(
            "prepared snapshot manifest does not match immutable content".to_owned(),
        ));
    }
    let selector = SnapshotSelector {
        schema: GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1.to_owned(),
        snapshot_id: prepared.manifest.snapshot_id.clone(),
        manifest_digest: prepared.manifest_digest.clone(),
    };
    let selector_bytes = encode_json(&selector)?;
    let catalog = catalog_partition()?;
    let active = Key::new(GRAPH_SNAPSHOT_ACTIVE_KEY.as_bytes())?;
    let observed = store.get(&namespace, &catalog, &active)?;
    let condition = observed.as_ref().map_or(WriteCondition::Missing, |entry| {
        WriteCondition::Version(entry.version)
    });
    store.put(&namespace, &catalog, &active, &selector_bytes, condition)?;
    Ok(selector)
}

/// Mark immutable trees reachable from retained selectors and remove other
/// graph-snapshot objects. Work is bounded and fails before sweeping if the
/// mark set exceeds `max_objects`.
pub fn garbage_collect_graph_snapshots<S: Store + ?Sized>(
    store: &S,
    selectors: &[SnapshotSelector],
    max_objects: usize,
) -> Result<GraphSnapshotGcStats, SnapshotError> {
    if max_objects == 0 || max_objects > GRAPH_SNAPSHOT_MAX_OBJECTS.saturating_mul(8) {
        return Err(SnapshotError::Limit(
            "graph snapshot GC object bound is invalid".to_owned(),
        ));
    }
    let mut reachable = BTreeSet::<Vec<u8>>::new();
    let mut retained_manifests = 0_u64;
    for selector in selectors {
        let reader = GraphSnapshotReader::open_selector(store, selector.clone())?;
        reachable.insert(manifest_key(&selector.manifest_digest)?.as_bytes().to_vec());
        retained_manifests = retained_manifests.saturating_add(1);
        for root in &reader.manifest.roots {
            mark_tree_objects(
                store,
                root.index,
                &root.digest,
                &mut reachable,
                max_objects,
                0,
            )?;
        }
    }
    let retained_objects = reachable.len() as u64;
    let namespace = NamespaceId::graph();
    let partition = object_partition()?;
    let mut cursor = None;
    let mut scanned = 0_usize;
    let mut deleted_entries = 0_u64;
    let mut delete_transactions = 0_u64;
    loop {
        let page = store.scan_keys(
            &namespace,
            &partition,
            &compass_store::KeyRange::default(),
            compass_store::ScanLimits {
                max_items: MAX_SCAN_ITEMS,
                max_bytes: MAX_SCAN_BYTES,
            },
            cursor.as_ref(),
        )?;
        scanned = scanned.saturating_add(page.keys.len());
        if scanned > max_objects {
            return Err(SnapshotError::Limit(
                "graph snapshot GC scan exceeded its object bound".to_owned(),
            ));
        }
        let unreachable = page
            .keys
            .into_iter()
            .filter(|key| !reachable.contains(key))
            .map(|key| Key::new(key).map_err(SnapshotError::from))
            .collect::<Result<Vec<_>, _>>()?;
        for keys in unreachable.chunks(MAX_IMMUTABLE_BATCH_ITEMS) {
            deleted_entries =
                deleted_entries.saturating_add(store.delete_batch(&namespace, &partition, keys)?);
            if !keys.is_empty() {
                delete_transactions = delete_transactions.saturating_add(1);
            }
        }
        let Some(next) = page.next else {
            break;
        };
        cursor = Some(next);
    }
    Ok(GraphSnapshotGcStats {
        retained_manifests,
        retained_objects,
        deleted_entries,
        delete_transactions,
    })
}

/// Check whether immutable graph manifests exceed the retained-snapshot
/// budget using a key-only bounded projection.
pub fn graph_snapshot_needs_gc<S: Store + ?Sized>(
    store: &S,
    retained_manifests: usize,
) -> Result<bool, SnapshotError> {
    if retained_manifests == 0 || retained_manifests >= MAX_SCAN_ITEMS {
        return Err(SnapshotError::Limit(
            "retained manifest bound is invalid".to_owned(),
        ));
    }
    let namespace = NamespaceId::graph();
    let partition = object_partition()?;
    let page = store.scan_keys(
        &namespace,
        &partition,
        &compass_store::KeyRange {
            start_inclusive: Some(b"manifest/".to_vec()),
            end_exclusive: Some(b"manifest0".to_vec()),
        },
        compass_store::ScanLimits {
            max_items: retained_manifests.saturating_add(1),
            max_bytes: MAX_SCAN_BYTES,
        },
        None,
    )?;
    Ok(page.keys.len() > retained_manifests || page.next.is_some())
}

fn mark_tree_objects<S: Store + ?Sized>(
    store: &S,
    index: IndexKind,
    digest: &str,
    reachable: &mut BTreeSet<Vec<u8>>,
    max_objects: usize,
    depth: usize,
) -> Result<(), SnapshotError> {
    if depth >= GRAPH_SNAPSHOT_MAX_DEPTH {
        return Err(SnapshotError::Limit(
            "graph snapshot GC tree depth exceeded".to_owned(),
        ));
    }
    let key = object_key(digest)?;
    if !reachable.insert(key.as_bytes().to_vec()) {
        return Ok(());
    }
    if reachable.len() > max_objects {
        return Err(SnapshotError::Limit(
            "graph snapshot GC mark set exceeded its object bound".to_owned(),
        ));
    }
    if let TreeObject::Branch { children, .. } = load_tree_object(store, index, digest)? {
        for child in children {
            mark_tree_objects(
                store,
                index,
                &child.digest,
                reachable,
                max_objects,
                depth.saturating_add(1),
            )?;
        }
    }
    Ok(())
}

pub fn active_graph_snapshot<S: Store + ?Sized>(
    store: &S,
) -> Result<Option<SnapshotSelector>, SnapshotError> {
    let namespace = NamespaceId::graph();
    let catalog = catalog_partition()?;
    let active = Key::new(GRAPH_SNAPSHOT_ACTIVE_KEY.as_bytes())?;
    let Some(entry) = store.get(&namespace, &catalog, &active)? else {
        return Ok(None);
    };
    let selector = decode_json::<SnapshotSelector>(&entry.value)?;
    selector.validate()?;
    Ok(Some(selector))
}

pub struct GraphSnapshotReader<'a, S: Store + ?Sized> {
    store: &'a S,
    selector: SnapshotSelector,
    manifest: GraphSnapshotManifest,
    object_cache: Mutex<TreeObjectCache>,
}

impl<'a, S: Store + ?Sized> GraphSnapshotReader<'a, S> {
    pub fn open_active(store: &'a S) -> Result<Option<Self>, SnapshotError> {
        let Some(selector) = active_graph_snapshot(store)? else {
            return Ok(None);
        };
        Self::open_selector(store, selector).map(Some)
    }

    pub fn open_selector(store: &'a S, selector: SnapshotSelector) -> Result<Self, SnapshotError> {
        selector.validate()?;
        let namespace = NamespaceId::graph();
        let objects = object_partition()?;
        let key = manifest_key(&selector.manifest_digest)?;
        let Some(entry) = store.get(&namespace, &objects, &key)? else {
            return Err(SnapshotError::Corrupt(
                "selected snapshot manifest is missing".to_owned(),
            ));
        };
        verify_digest(&entry.value, &selector.manifest_digest)?;
        let manifest = decode_json::<GraphSnapshotManifest>(&entry.value)?;
        manifest.validate()?;
        if manifest.snapshot_id != selector.snapshot_id {
            return Err(SnapshotError::Corrupt(
                "selector snapshot ID does not match its manifest".to_owned(),
            ));
        }
        Ok(Self {
            store,
            selector,
            manifest,
            object_cache: Mutex::new(TreeObjectCache::default()),
        })
    }

    #[must_use]
    pub fn selector(&self) -> &SnapshotSelector {
        &self.selector
    }

    #[must_use]
    pub fn manifest(&self) -> &GraphSnapshotManifest {
        &self.manifest
    }

    pub fn metadata(&self) -> Result<GraphSnapshotMetadata, SnapshotError> {
        let entries = self.scan_entries(
            IndexKind::Metadata,
            None,
            SnapshotReadLimits {
                max_items: GRAPH_SNAPSHOT_MAX_OBJECTS,
                max_bytes: MAX_VALUE_BYTES.saturating_mul(4_096),
                max_objects: GRAPH_SNAPSHOT_MAX_OBJECTS,
                ..SnapshotReadLimits::default()
            },
        )?;
        let base_key = encode_graph_index_key(IndexKind::Metadata, &[])?;
        let base = entries
            .iter()
            .find(|entry| entry.key == base_key)
            .ok_or_else(|| SnapshotError::Corrupt("metadata index entry is missing".to_owned()))?;
        let record = decode_json::<MetadataRecord>(&base.value)?;
        let mut graph = record.graph;
        for entry in entries {
            let segments = decode_key_segments(&entry.key).map_err(SnapshotError::from)?;
            let Some(kind) = segments.get(1).map(Vec::as_slice) else {
                continue;
            };
            match kind {
                b"file" => graph.files.push(decode_json::<FileRecord>(&entry.value)?),
                b"coverage" => graph.coverage.push(decode_json::<
                    compass_model::code_graph::CoverageRecord,
                >(&entry.value)?),
                b"diagnostic" => graph
                    .diagnostics
                    .push(decode_json::<GraphDiagnostic>(&entry.value)?),
                b"diagnostic-code" | b"scope-capability" => {}
                _ => {
                    return Err(SnapshotError::Corrupt(
                        "metadata index contains an unknown supplement".to_owned(),
                    ));
                }
            }
        }
        graph.files.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(GraphSnapshotMetadata {
            directed: record.directed,
            multigraph: record.multigraph,
            graph,
        })
    }

    /// Verify every independently bounded immutable object reachable from the
    /// selected manifest without materializing the graph.
    ///
    /// The traversal validates content addresses, object schemas, index keys,
    /// branch separators, global key ordering, tree depth, and root entry
    /// counts. Memory remains bounded by the decoded-object cache and one
    /// branch path even when the logical graph exceeds whole-document reader
    /// budgets.
    pub fn validate_integrity(&self) -> Result<(), SnapshotError> {
        for root in &self.manifest.roots {
            let integrity = validate_tree_integrity(self, root.index, &root.digest, 0)?;
            if integrity.entries != root.entry_count {
                return Err(SnapshotError::Corrupt(format!(
                    "{} tree contains {} entries but its root declares {}",
                    root.index.as_str(),
                    integrity.entries,
                    root.entry_count
                )));
            }
        }
        Ok(())
    }

    /// Read graph-level metadata without materializing file, coverage, or
    /// diagnostic supplements.
    pub fn metadata_summary(&self) -> Result<GraphSnapshotMetadata, SnapshotError> {
        let key = encode_graph_index_key(IndexKind::Metadata, &[])?;
        let value = self
            .lookup(IndexKind::Metadata, &key)?
            .ok_or_else(|| SnapshotError::Corrupt("metadata index entry is missing".to_owned()))?;
        let record = decode_json::<MetadataRecord>(&value)?;
        Ok(GraphSnapshotMetadata {
            directed: record.directed,
            multigraph: record.multigraph,
            graph: record.graph,
        })
    }

    /// Read graph-level diagnostics without materializing files, coverage, or
    /// the node and edge collections.
    pub fn graph_diagnostics(
        &self,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<GraphDiagnostic>, bool), SnapshotError> {
        let prefix = encode_graph_index_key(IndexKind::Metadata, &[b"diagnostic"])?;
        let (values, truncated) =
            self.scan_values_bounded(IndexKind::Metadata, Some(&prefix), limits)?;
        let diagnostics = values
            .into_iter()
            .map(|value| decode_json::<GraphDiagnostic>(&value))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((diagnostics, truncated))
    }

    /// Read the first graph diagnostic for a stable code through a point
    /// projection. This avoids scanning large omission sets when a query only
    /// needs the publication summary.
    pub fn graph_diagnostic_by_code(
        &self,
        code: &str,
    ) -> Result<Option<GraphDiagnostic>, SnapshotError> {
        let key =
            encode_graph_index_key(IndexKind::Metadata, &[b"diagnostic-code", code.as_bytes()])?;
        self.lookup(IndexKind::Metadata, &key)?
            .map(|value| decode_json::<GraphDiagnostic>(&value))
            .transpose()
    }

    pub fn get_node(&self, id: &str) -> Result<Option<NodeRecord>, SnapshotError> {
        let key = encode_graph_index_key(IndexKind::Nodes, &[id.as_bytes()])?;
        self.lookup(IndexKind::Nodes, &key)?
            .map(|value| decode_json::<NodeRecord>(&value))
            .transpose()
    }

    pub fn get_edge(&self, id: &str) -> Result<Option<EdgeRecord>, SnapshotError> {
        let key = encode_graph_index_key(IndexKind::Edges, &[id.as_bytes()])?;
        self.lookup(IndexKind::Edges, &key)?
            .map(|value| decode_json::<EdgeRecord>(&value))
            .transpose()
    }

    /// Resolve a bounded sorted set of node IDs while sharing immutable tree
    /// branch and leaf reads across all requested keys.
    pub fn get_nodes_by_ids_bounded_work(
        &self,
        ids: &BTreeSet<String>,
        limits: SnapshotReadLimits,
    ) -> Result<Vec<NodeRecord>, SnapshotError> {
        let limits = limits.validate()?;
        if ids.len() > limits.max_items {
            return Err(SnapshotError::Limit(
                "node batch exceeds the snapshot item limit".to_owned(),
            ));
        }
        let keys = ids
            .iter()
            .map(|id| encode_graph_index_key(IndexKind::Nodes, &[id.as_bytes()]))
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = MultiLookupState {
            limits,
            objects: 0,
            bytes: 0,
            values: BTreeMap::new(),
        };
        let root = self.root(IndexKind::Nodes)?.digest.clone();
        lookup_many_tree(self, IndexKind::Nodes, &root, &keys, &mut state, 0)?;
        let mut nodes = Vec::with_capacity(ids.len());
        for key in keys {
            let value = state.values.remove(&key).ok_or_else(|| {
                SnapshotError::Corrupt("node batch references a missing node".to_owned())
            })?;
            nodes.push(decode_json::<NodeRecord>(&value)?);
        }
        Ok(nodes)
    }

    /// Resolve a bounded sorted set of edge IDs while sharing immutable tree
    /// branch and leaf reads across all requested keys.
    pub fn get_edges_by_ids_bounded_work(
        &self,
        ids: &BTreeSet<String>,
        limits: SnapshotReadLimits,
    ) -> Result<Vec<EdgeRecord>, SnapshotError> {
        let limits = limits.validate()?;
        if ids.len() > limits.max_items {
            return Err(SnapshotError::Limit(
                "edge batch exceeds the snapshot item limit".to_owned(),
            ));
        }
        let keys = ids
            .iter()
            .map(|id| encode_graph_index_key(IndexKind::Edges, &[id.as_bytes()]))
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = MultiLookupState {
            limits,
            objects: 0,
            bytes: 0,
            values: BTreeMap::new(),
        };
        let root = self.root(IndexKind::Edges)?.digest.clone();
        lookup_many_tree(self, IndexKind::Edges, &root, &keys, &mut state, 0)?;
        let mut edges = Vec::with_capacity(ids.len());
        for key in keys {
            let value = state.values.remove(&key).ok_or_else(|| {
                SnapshotError::Corrupt("edge batch references a missing edge".to_owned())
            })?;
            edges.push(decode_json::<EdgeRecord>(&value)?);
        }
        Ok(edges)
    }

    pub fn nodes(&self, limits: SnapshotReadLimits) -> Result<Vec<NodeRecord>, SnapshotError> {
        self.scan_values(IndexKind::Nodes, None, limits)?
            .into_iter()
            .map(|value| decode_json::<NodeRecord>(&value))
            .collect()
    }

    pub fn edges(&self, limits: SnapshotReadLimits) -> Result<Vec<EdgeRecord>, SnapshotError> {
        self.scan_values(IndexKind::Edges, None, limits)?
            .into_iter()
            .map(|value| decode_json::<EdgeRecord>(&value))
            .collect()
    }

    pub fn outgoing(
        &self,
        node_id: &str,
        limits: SnapshotReadLimits,
    ) -> Result<Vec<EdgeRecord>, SnapshotError> {
        self.adjacency(IndexKind::Outgoing, node_id, limits)
    }

    pub fn incoming(
        &self,
        node_id: &str,
        limits: SnapshotReadLimits,
    ) -> Result<Vec<EdgeRecord>, SnapshotError> {
        self.adjacency(IndexKind::Incoming, node_id, limits)
    }

    pub fn nodes_by_normalized_name(
        &self,
        normalized_name: &str,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<NodeRecord>, bool), SnapshotError> {
        let normalized = normalize_symbol(normalized_name);
        let prefix = encode_name_prefix(&normalized)?;
        let (entries, truncated) =
            self.scan_entries_bounded(IndexKind::Names, Some(&prefix), limits)?;
        let mut nodes = Vec::with_capacity(entries.len());
        for entry in entries {
            let segments = decode_key_segments(&entry.key).map_err(SnapshotError::from)?;
            let node_id = segments
                .last()
                .and_then(|segment| std::str::from_utf8(segment).ok())
                .ok_or_else(|| {
                    SnapshotError::Corrupt("name index node ID is invalid".to_owned())
                })?;
            let node = self.get_node(node_id)?.ok_or_else(|| {
                SnapshotError::Corrupt(format!("name index references missing node {node_id}"))
            })?;
            nodes.push(node);
        }
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        nodes.dedup_by(|left, right| left.id == right.id);
        Ok((nodes, truncated))
    }

    /// Resolve one exact canonical discovery scope through immutable postings.
    pub fn resolve_scope_values(
        &self,
        kind: &str,
        value: &str,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<String>, bool), SnapshotError> {
        let capability_key = encode_graph_index_key(IndexKind::Metadata, &[b"scope-capability"])?;
        let capability = self
            .lookup(IndexKind::Metadata, &capability_key)?
            .map(|value| decode_json::<String>(&value))
            .transpose()?;
        if capability.as_deref() != Some(DISCOVERY_SCOPE_INDEX_CAPABILITY_V1) {
            return Err(SnapshotError::CapabilityUnavailable(
                "scope_index_unavailable; rebuild the graph store with this Compass version"
                    .to_owned(),
            ));
        }
        let value_digest = hex_digest(value.as_bytes());
        let prefix = encode_graph_index_key(
            IndexKind::Terms,
            &[b"scope", kind.as_bytes(), value_digest.as_bytes()],
        )?;
        let (values, truncated) =
            self.scan_values_bounded(IndexKind::Terms, Some(&prefix), limits)?;
        let mut canonical = Vec::with_capacity(values.len());
        for encoded in values {
            let (stored_requested, stored_canonical) = decode_json::<(String, String)>(&encoded)?;
            if stored_requested == value {
                canonical.push(stored_canonical);
            }
        }
        canonical.sort();
        canonical.dedup();
        Ok((canonical, truncated))
    }

    /// Return node candidates present in every exact normalized term posting.
    pub fn nodes_for_terms(
        &self,
        terms: &[String],
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<NodeRecord>, bool), SnapshotError> {
        let mut intersection: Option<BTreeSet<String>> = None;
        let mut truncated = false;
        for term in terms {
            let normalized = normalize_search_term(term);
            if normalized.is_empty() {
                continue;
            }
            let prefix_length = normalized.len().min(3);
            let posting_prefix = normalized
                .get(..prefix_length)
                .unwrap_or(normalized.as_str());
            let prefix = encode_graph_index_key(
                IndexKind::Terms,
                &[posting_prefix.as_bytes(), b"node_prefix"],
            )?;
            let posting_limits = SnapshotReadLimits {
                max_items: GRAPH_SNAPSHOT_MAX_ITEMS,
                ..limits
            };
            let (values, mut posting_truncated) =
                self.scan_values_bounded(IndexKind::Terms, Some(&prefix), posting_limits)?;
            let mut ids = BTreeSet::new();
            for value in values {
                let posting = decode_json::<TermPostingChunk>(&value)?;
                if !normalize_search_term(&posting.term).starts_with(&normalized) {
                    continue;
                }
                for node_id in posting.node_ids {
                    ids.insert(node_id);
                    if ids.len() > limits.max_items {
                        ids.pop_last();
                        posting_truncated = true;
                    }
                }
            }
            truncated |= posting_truncated;
            intersection = Some(match intersection {
                Some(previous) => previous.intersection(&ids).cloned().collect(),
                None => ids,
            });
            if intersection.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        let ids = intersection.unwrap_or_default();
        let nodes =
            self.get_nodes_by_ids_bounded_work(&ids, point_lookup_batch_limits(ids.len()))?;
        Ok((nodes, truncated))
    }

    /// Return bounded discovery candidates and report the posting work decoded.
    pub fn nodes_for_terms_bounded_work(
        &self,
        terms: &[String],
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<NodeRecord>, bool, TermPostingWork), SnapshotError> {
        let searchable_terms = terms
            .iter()
            .filter(|term| !normalize_search_term(term).is_empty())
            .count()
            .max(1);
        let total_chunk_budget = limits.max_items / GRAPH_TERM_POSTING_CHUNK_ITEMS;
        if total_chunk_budget < searchable_terms {
            return Ok((Vec::new(), true, TermPostingWork::default()));
        }
        let per_term_chunk_limit = total_chunk_budget / searchable_terms;
        let per_term_item_limit = per_term_chunk_limit * GRAPH_TERM_POSTING_CHUNK_ITEMS;
        let mut intersection: Option<BTreeSet<String>> = None;
        let mut truncated = false;
        let mut work = TermPostingWork::default();
        for term in terms {
            let normalized = normalize_search_term(term);
            if normalized.is_empty() {
                continue;
            }
            let prefix_length = normalized.len().min(3);
            let posting_prefix = normalized
                .get(..prefix_length)
                .unwrap_or(normalized.as_str());
            let prefix = encode_graph_index_key(
                IndexKind::Terms,
                &[posting_prefix.as_bytes(), b"node_prefix"],
            )?;
            let posting_limits = SnapshotReadLimits {
                // Term values are fixed-size posting chunks. Divide the
                // caller's candidate ceiling across query terms so decoded
                // posting work remains independent of graph size. A prefix
                // collision can truncate recall, which is propagated rather
                // than hidden behind an unbounded scan.
                max_items: per_term_chunk_limit,
                ..limits
            };
            let (values, mut posting_truncated) =
                self.scan_values_bounded(IndexKind::Terms, Some(&prefix), posting_limits)?;
            let mut ids = BTreeSet::new();
            for value in values {
                let posting = decode_json::<TermPostingChunk>(&value)?;
                work.chunks_decoded = work.chunks_decoded.saturating_add(1);
                work.node_ids_decoded = work
                    .node_ids_decoded
                    .saturating_add(u64::try_from(posting.node_ids.len()).unwrap_or(u64::MAX));
                if !normalize_search_term(&posting.term).starts_with(&normalized) {
                    continue;
                }
                for node_id in posting.node_ids {
                    ids.insert(node_id);
                    if ids.len() > per_term_item_limit {
                        ids.pop_last();
                        posting_truncated = true;
                    }
                }
            }
            truncated |= posting_truncated;
            intersection = Some(match intersection {
                Some(previous) => previous.intersection(&ids).cloned().collect(),
                None => ids,
            });
            if intersection.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        let ids = intersection.unwrap_or_default();
        let nodes =
            self.get_nodes_by_ids_bounded_work(&ids, point_lookup_batch_limits(ids.len()))?;
        Ok((nodes, truncated, work))
    }

    /// Whether this snapshot includes raw identifier-subword term postings.
    ///
    /// The sentinel is an ordinary empty posting so readers predating this
    /// capability continue to accept the additive index entry.
    pub fn supports_identifier_subwords(&self) -> Result<bool, SnapshotError> {
        let capability = IDENTIFIER_SUBWORD_INDEX_CAPABILITY_V1;
        let posting_prefix = capability.get(..3).unwrap_or(capability);
        let key = encode_graph_index_key(
            IndexKind::Terms,
            &[
                posting_prefix.as_bytes(),
                b"node_prefix",
                capability.as_bytes(),
                b"00000000",
            ],
        )?;
        let Some(value) = self.lookup(IndexKind::Terms, &key)? else {
            return Ok(false);
        };
        let posting = decode_json::<TermPostingChunk>(&value)?;
        Ok(posting.term == capability && posting.node_ids.is_empty())
    }

    /// Whether this snapshot includes exact term postings restricted to
    /// source-backed operation-role declarations.
    pub fn supports_operation_role_terms(&self) -> Result<bool, SnapshotError> {
        let capability = OPERATION_ROLE_TERM_INDEX_CAPABILITY_V1;
        let key = encode_graph_index_key(
            IndexKind::Terms,
            &[b"operation_role", capability.as_bytes(), b"00000000"],
        )?;
        let Some(value) = self.lookup(IndexKind::Terms, &key)? else {
            return Ok(false);
        };
        let posting = decode_json::<TermPostingChunk>(&value)?;
        Ok(posting.term == capability && posting.node_ids.is_empty())
    }

    /// Whether this snapshot includes exact identifier terms restricted to
    /// source-backed type declarations.
    pub fn supports_declaration_terms(&self) -> Result<bool, SnapshotError> {
        let capability = DECLARATION_TERM_INDEX_CAPABILITY_V1;
        let key = encode_graph_index_key(
            IndexKind::Terms,
            &[b"declaration", capability.as_bytes(), b"00000000"],
        )?;
        let Some(value) = self.lookup(IndexKind::Terms, &key)? else {
            return Ok(false);
        };
        let posting = decode_json::<TermPostingChunk>(&value)?;
        Ok(posting.term == capability && posting.node_ids.is_empty())
    }

    /// Return the bounded union of operation-role declarations matching any
    /// exact normalized term, together with exact posting work.
    pub fn operation_role_nodes_for_terms_bounded_work(
        &self,
        terms: &[String],
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<NodeRecord>, bool, TermPostingWork), SnapshotError> {
        let normalized_terms = terms
            .iter()
            .map(|term| normalize_search_term(term))
            .filter(|term| !term.is_empty())
            .collect::<BTreeSet<_>>();
        if normalized_terms.is_empty() {
            return Ok((Vec::new(), false, TermPostingWork::default()));
        }
        let total_chunk_limit = limits.max_items / GRAPH_TERM_POSTING_CHUNK_ITEMS;
        if total_chunk_limit < normalized_terms.len() {
            return Ok((Vec::new(), true, TermPostingWork::default()));
        }
        let per_term_chunk_limit = total_chunk_limit / normalized_terms.len();
        let mut ids = BTreeSet::new();
        let mut truncated = false;
        let mut work = TermPostingWork::default();
        for term in normalized_terms {
            let prefix =
                encode_graph_index_key(IndexKind::Terms, &[b"operation_role", term.as_bytes()])?;
            let (values, posting_truncated) = self.scan_values_bounded(
                IndexKind::Terms,
                Some(&prefix),
                SnapshotReadLimits {
                    max_items: per_term_chunk_limit,
                    ..limits
                },
            )?;
            truncated |= posting_truncated;
            for value in values {
                let posting = decode_json::<TermPostingChunk>(&value)?;
                work.chunks_decoded = work.chunks_decoded.saturating_add(1);
                work.node_ids_decoded = work
                    .node_ids_decoded
                    .saturating_add(u64::try_from(posting.node_ids.len()).unwrap_or(u64::MAX));
                if normalize_search_term(&posting.term) != term {
                    continue;
                }
                for node_id in posting.node_ids {
                    ids.insert(node_id);
                    if ids.len() > limits.max_items {
                        ids.pop_last();
                        truncated = true;
                    }
                }
            }
        }
        let nodes =
            self.get_nodes_by_ids_bounded_work(&ids, point_lookup_batch_limits(ids.len()))?;
        Ok((nodes, truncated, work))
    }

    /// Return the bounded union of source-backed type declarations matching
    /// any exact normalized identifier term, together with exact posting work.
    pub fn declaration_nodes_for_terms_bounded_work(
        &self,
        terms: &[String],
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<NodeRecord>, bool, TermPostingWork), SnapshotError> {
        let normalized_terms = terms
            .iter()
            .map(|term| normalize_search_term(term))
            .filter(|term| !term.is_empty())
            .collect::<BTreeSet<_>>();
        if normalized_terms.is_empty() {
            return Ok((Vec::new(), false, TermPostingWork::default()));
        }
        let total_chunk_limit = limits.max_items / GRAPH_TERM_POSTING_CHUNK_ITEMS;
        if total_chunk_limit < normalized_terms.len() {
            return Ok((Vec::new(), true, TermPostingWork::default()));
        }
        let per_term_chunk_limit = total_chunk_limit / normalized_terms.len();
        let mut ids = BTreeSet::new();
        let mut truncated = false;
        let mut work = TermPostingWork::default();
        for term in normalized_terms {
            let prefix =
                encode_graph_index_key(IndexKind::Terms, &[b"declaration", term.as_bytes()])?;
            let (values, posting_truncated) = self.scan_values_bounded(
                IndexKind::Terms,
                Some(&prefix),
                SnapshotReadLimits {
                    max_items: per_term_chunk_limit,
                    ..limits
                },
            )?;
            truncated |= posting_truncated;
            for value in values {
                let posting = decode_json::<TermPostingChunk>(&value)?;
                work.chunks_decoded = work.chunks_decoded.saturating_add(1);
                work.node_ids_decoded = work
                    .node_ids_decoded
                    .saturating_add(u64::try_from(posting.node_ids.len()).unwrap_or(u64::MAX));
                if normalize_search_term(&posting.term) != term {
                    continue;
                }
                for node_id in posting.node_ids {
                    ids.insert(node_id);
                    if ids.len() > limits.max_items {
                        ids.pop_last();
                        truncated = true;
                    }
                }
            }
        }
        let nodes =
            self.get_nodes_by_ids_bounded_work(&ids, point_lookup_batch_limits(ids.len()))?;
        Ok((nodes, truncated, work))
    }

    /// Whether this snapshot includes exact direct-caller concept postings.
    pub fn supports_relationship_terms(&self) -> Result<bool, SnapshotError> {
        let capability = RELATIONSHIP_TERM_INDEX_CAPABILITY_V1;
        let posting_prefix = capability.get(..3).unwrap_or(capability);
        let key = encode_graph_index_key(
            IndexKind::Terms,
            &[
                b"call_source",
                posting_prefix.as_bytes(),
                capability.as_bytes(),
                b"00000000",
            ],
        )?;
        let Some(value) = self.lookup(IndexKind::Terms, &key)? else {
            return Ok(false);
        };
        let posting = decode_json::<TermPostingChunk>(&value)?;
        Ok(posting.term == capability && posting.node_ids.is_empty())
    }

    /// Return sorted source IDs from one exact direct-caller concept posting.
    pub fn source_ids_for_exact_relationship_term_bounded_work(
        &self,
        term: &str,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<String>, bool, TermPostingWork), SnapshotError> {
        let normalized = normalize_search_term(term);
        if normalized.is_empty() {
            return Ok((Vec::new(), false, TermPostingWork::default()));
        }
        let chunk_limit = limits.max_items / GRAPH_TERM_POSTING_CHUNK_ITEMS;
        if chunk_limit == 0 {
            return Ok((Vec::new(), true, TermPostingWork::default()));
        }
        let posting_prefix = normalized
            .get(..normalized.len().min(3))
            .unwrap_or(normalized.as_str());
        let prefix = encode_graph_index_key(
            IndexKind::Terms,
            &[
                b"call_source",
                posting_prefix.as_bytes(),
                normalized.as_bytes(),
            ],
        )?;
        let (values, mut truncated) = self.scan_values_bounded(
            IndexKind::Terms,
            Some(&prefix),
            SnapshotReadLimits {
                max_items: chunk_limit,
                ..limits
            },
        )?;
        let mut source_ids = BTreeSet::new();
        let mut work = TermPostingWork::default();
        for value in values {
            let posting = decode_json::<TermPostingChunk>(&value)?;
            work.chunks_decoded = work.chunks_decoded.saturating_add(1);
            work.node_ids_decoded = work
                .node_ids_decoded
                .saturating_add(u64::try_from(posting.node_ids.len()).unwrap_or(u64::MAX));
            if normalize_search_term(&posting.term) != normalized {
                continue;
            }
            for source_id in posting.node_ids {
                source_ids.insert(source_id);
                if source_ids.len() > limits.max_items {
                    source_ids.pop_last();
                    truncated = true;
                }
            }
        }
        Ok((source_ids.into_iter().collect(), truncated, work))
    }

    /// Test one exact direct-caller concept membership.
    pub fn relationship_source_matches_term(
        &self,
        source_id: &str,
        term: &str,
    ) -> Result<bool, SnapshotError> {
        let normalized = normalize_search_term(term);
        if normalized.is_empty() {
            return Ok(false);
        }
        let key = encode_graph_index_key(
            IndexKind::Terms,
            &[
                b"call_source_member",
                source_id.as_bytes(),
                normalized.as_bytes(),
            ],
        )?;
        self.lookup(IndexKind::Terms, &key)
            .map(|value| value.is_some())
    }

    /// Return sorted target IDs supporting one exact caller concept.
    pub fn relationship_target_ids_for_source_terms_bounded_work(
        &self,
        source_id: &str,
        terms: &BTreeSet<String>,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<String>, bool, TermPostingWork), SnapshotError> {
        let normalized = terms
            .iter()
            .map(|term| normalize_search_term(term))
            .filter(|term| !term.is_empty())
            .collect::<BTreeSet<_>>();
        if normalized.is_empty() {
            return Ok((Vec::new(), false, TermPostingWork::default()));
        }
        let mut target_ids = BTreeSet::new();
        let term_count = normalized.len();
        let per_term_items = limits.max_items.div_ceil(term_count);
        let mut entries_decoded = 0_usize;
        let mut truncated = false;
        for term in &normalized {
            let items_remaining = limits.max_items.saturating_sub(entries_decoded);
            if items_remaining == 0 {
                truncated = true;
                break;
            }
            let prefix = encode_graph_index_key(
                IndexKind::Terms,
                &[b"call_source_target", source_id.as_bytes(), term.as_bytes()],
            )?;
            let (entries, term_truncated) = self.scan_entries_bounded(
                IndexKind::Terms,
                Some(&prefix),
                SnapshotReadLimits {
                    max_items: items_remaining.min(per_term_items),
                    max_bytes: (limits.max_bytes / term_count).max(1),
                    max_objects: (limits.max_objects / term_count).max(1),
                    max_depth: limits.max_depth,
                },
            )?;
            truncated |= term_truncated;
            entries_decoded = entries_decoded.saturating_add(entries.len());
            for entry in entries {
                let segments = decode_key_segments(&entry.key).map_err(SnapshotError::from)?;
                let target_id = segments
                    .get(4)
                    .and_then(|segment| std::str::from_utf8(segment).ok())
                    .ok_or_else(|| {
                        SnapshotError::Corrupt(
                            "relationship target index key has an invalid target ID".to_owned(),
                        )
                    })?;
                target_ids.insert(target_id.to_owned());
            }
        }
        Ok((
            target_ids.into_iter().collect(),
            truncated,
            TermPostingWork {
                chunks_decoded: 0,
                node_ids_decoded: u64::try_from(entries_decoded).unwrap_or(u64::MAX),
            },
        ))
    }

    /// Return node IDs for one exact normalized term posting without hydrating
    /// node records. Multi-term discovery intersects these compact IDs first
    /// so common postings do not force full-record reads for candidates that a
    /// later term will reject.
    pub fn node_ids_for_exact_term_bounded_work(
        &self,
        term: &str,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<String>, bool, TermPostingWork), SnapshotError> {
        let normalized = normalize_search_term(term);
        if normalized.is_empty() {
            return Ok((Vec::new(), false, TermPostingWork::default()));
        }
        let chunk_limit = limits.max_items / GRAPH_TERM_POSTING_CHUNK_ITEMS;
        if chunk_limit == 0 {
            return Ok((Vec::new(), true, TermPostingWork::default()));
        }
        let prefix_length = normalized.len().min(3);
        let posting_prefix = normalized
            .get(..prefix_length)
            .unwrap_or(normalized.as_str());
        let prefix = encode_graph_index_key(
            IndexKind::Terms,
            &[
                posting_prefix.as_bytes(),
                b"node_prefix",
                normalized.as_bytes(),
            ],
        )?;
        let (values, mut truncated) = self.scan_values_bounded(
            IndexKind::Terms,
            Some(&prefix),
            SnapshotReadLimits {
                max_items: chunk_limit,
                ..limits
            },
        )?;
        let mut ids = BTreeSet::new();
        let mut work = TermPostingWork::default();
        for value in values {
            let posting = decode_json::<TermPostingChunk>(&value)?;
            work.chunks_decoded = work.chunks_decoded.saturating_add(1);
            work.node_ids_decoded = work
                .node_ids_decoded
                .saturating_add(u64::try_from(posting.node_ids.len()).unwrap_or(u64::MAX));
            if normalize_search_term(&posting.term) != normalized {
                continue;
            }
            for node_id in posting.node_ids {
                ids.insert(node_id);
                if ids.len() > limits.max_items {
                    ids.pop_last();
                    truncated = true;
                }
            }
        }
        Ok((ids.into_iter().collect(), truncated, work))
    }

    /// Return candidates for one exact normalized term posting. Discovery uses
    /// exact token matches before the broader bounded prefix channel so dense
    /// shared prefixes cannot hide a present identifier token.
    pub fn nodes_for_exact_term_bounded_work(
        &self,
        term: &str,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<NodeRecord>, bool, TermPostingWork), SnapshotError> {
        let (ids, truncated, work) = self.node_ids_for_exact_term_bounded_work(term, limits)?;
        let ids = ids.into_iter().collect::<BTreeSet<_>>();
        let nodes =
            self.get_nodes_by_ids_bounded_work(&ids, point_lookup_batch_limits(ids.len()))?;
        Ok((nodes, truncated, work))
    }

    pub fn file_by_path(&self, path: &str) -> Result<Option<FileRecord>, SnapshotError> {
        let key = encode_file_path_key(path)?;
        let Some(value) = self.lookup(IndexKind::Files, &key)? else {
            return Ok(None);
        };
        let file_id = decode_json::<String>(&value)?;
        let id_key = encode_graph_index_key(IndexKind::Metadata, &[b"file", file_id.as_bytes()])?;
        self.lookup(IndexKind::Metadata, &id_key)?
            .map(|value| decode_json::<FileRecord>(&value))
            .transpose()
    }

    /// Read only requested directional kind buckets and merge by edge ID.
    pub fn adjacency_by_kinds(
        &self,
        node_id: &str,
        incoming: bool,
        kinds: &[compass_model::code_graph::EdgeKind],
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<EdgeRecord>, bool), SnapshotError> {
        let index = if incoming {
            IndexKind::Incoming
        } else {
            IndexKind::Outgoing
        };
        let mut edge_ids = BTreeSet::new();
        let mut truncated = false;
        for kind in kinds {
            let prefix =
                encode_graph_index_key(index, &[node_id.as_bytes(), kind.as_str().as_bytes()])?;
            let (entries, bucket_truncated) =
                self.scan_entries_bounded(index, Some(&prefix), limits)?;
            truncated |= bucket_truncated;
            for entry in entries {
                let edge_id = index_entry_id(&entry, "directional adjacency")?;
                edge_ids.insert(edge_id);
            }
        }
        let edges = self
            .get_edges_by_ids_bounded_work(&edge_ids, point_lookup_batch_limits(edge_ids.len()))?;
        Ok((edges, truncated))
    }

    /// Read one globally bounded directional adjacency prefix. Callers may
    /// filter kinds after this read without multiplying the limit per kind.
    pub fn directional_adjacency(
        &self,
        node_id: &str,
        incoming: bool,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<EdgeRecord>, bool), SnapshotError> {
        let index = if incoming {
            IndexKind::Incoming
        } else {
            IndexKind::Outgoing
        };
        let prefix = encode_graph_index_key(index, &[node_id.as_bytes()])?;
        let (entries, truncated) = self.scan_entries_bounded(index, Some(&prefix), limits)?;
        let mut edge_ids = BTreeSet::new();
        for entry in entries {
            let edge_id = index_entry_id(&entry, "directional adjacency")?;
            edge_ids.insert(edge_id);
        }
        let edges = self
            .get_edges_by_ids_bounded_work(&edge_ids, point_lookup_batch_limits(edge_ids.len()))?;
        Ok((edges, truncated))
    }

    /// Read outgoing occurrences whose target is already in a bounded selected
    /// node set. The outgoing index key carries the target and edge ID, so
    /// external edges are rejected before their full records are hydrated.
    pub fn outgoing_edge_ids_within_nodes_bounded_work(
        &self,
        source_id: &str,
        selected_node_ids: &BTreeSet<String>,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<String>, bool, usize), SnapshotError> {
        let prefix = encode_graph_index_key(IndexKind::Outgoing, &[source_id.as_bytes()])?;
        let (entries, truncated) =
            self.scan_entries_bounded(IndexKind::Outgoing, Some(&prefix), limits)?;
        let entries_examined = entries.len();
        let mut edge_ids = Vec::new();
        for entry in entries {
            let segments = decode_key_segments(&entry.key).map_err(SnapshotError::from)?;
            let target_id = segments
                .get(3)
                .and_then(|segment| std::str::from_utf8(segment).ok())
                .ok_or_else(|| {
                    SnapshotError::Corrupt("outgoing index key has an invalid target ID".to_owned())
                })?;
            if !selected_node_ids.contains(target_id) {
                continue;
            }
            let edge_id = segments
                .get(4)
                .and_then(|segment| std::str::from_utf8(segment).ok())
                .ok_or_else(|| {
                    SnapshotError::Corrupt("outgoing index key has an invalid edge ID".to_owned())
                })?;
            edge_ids.push(edge_id.to_owned());
        }
        Ok((edge_ids, truncated, entries_examined))
    }

    pub fn incident(
        &self,
        node_id: &str,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<EdgeRecord>, bool), SnapshotError> {
        let incoming_prefix = encode_graph_index_key(IndexKind::Incoming, &[node_id.as_bytes()])?;
        let outgoing_prefix = encode_graph_index_key(IndexKind::Outgoing, &[node_id.as_bytes()])?;
        let (incoming, incoming_truncated) =
            self.scan_entries_bounded(IndexKind::Incoming, Some(&incoming_prefix), limits)?;
        let (outgoing, outgoing_truncated) =
            self.scan_entries_bounded(IndexKind::Outgoing, Some(&outgoing_prefix), limits)?;
        let mut edge_ids = BTreeSet::new();
        for entry in incoming.into_iter().chain(outgoing) {
            let edge_id = index_entry_id(&entry, "incident adjacency")?;
            edge_ids.insert(edge_id);
        }
        let edges = self
            .get_edges_by_ids_bounded_work(&edge_ids, point_lookup_batch_limits(edge_ids.len()))?;
        Ok((edges, incoming_truncated || outgoing_truncated))
    }

    pub fn export_graph(&self) -> Result<GraphDocument, SnapshotError> {
        let metadata = self.metadata()?;
        let limits = SnapshotReadLimits {
            max_items: bounded_count(self.manifest.node_count)?,
            max_bytes: MAX_VALUE_BYTES.saturating_mul(4_096),
            ..SnapshotReadLimits::default()
        };
        let mut graph = GraphDocument {
            directed: metadata.directed,
            multigraph: metadata.multigraph,
            graph: metadata.graph,
            nodes: self.nodes(limits)?,
            links: self.edges(SnapshotReadLimits {
                max_items: bounded_count(self.manifest.edge_count)?,
                ..limits
            })?,
        };
        graph.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        graph.links.sort_by(|left, right| left.id.cmp(&right.id));
        validate_code_graph(&graph)
            .map_err(|error| SnapshotError::Corrupt(format!("exported graph invalid: {error}")))?;
        let bytes = encode_json(&graph)?;
        if bytes.len() as u64 != self.manifest.graph_bytes {
            return Err(SnapshotError::Corrupt(
                "exported graph byte count does not match the manifest".to_owned(),
            ));
        }
        if hex_digest(&bytes) != self.manifest.graph_digest {
            return Err(SnapshotError::Corrupt(
                "exported graph digest does not match the manifest".to_owned(),
            ));
        }
        Ok(graph)
    }

    pub fn export_json_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        encode_json(&self.export_graph()?)
    }

    fn adjacency(
        &self,
        index: IndexKind,
        node_id: &str,
        limits: SnapshotReadLimits,
    ) -> Result<Vec<EdgeRecord>, SnapshotError> {
        let prefix = encode_graph_index_key(index, &[node_id.as_bytes()])?;
        let entries = self.scan_entries(index, Some(&prefix), limits)?;
        let edge_ids = entries
            .iter()
            .map(|entry| index_entry_id(entry, "adjacency"))
            .collect::<Result<Vec<_>, _>>()?;
        let requested = edge_ids.iter().cloned().collect::<BTreeSet<_>>();
        let edges = self.get_edges_by_ids_bounded_work(
            &requested,
            point_lookup_batch_limits(requested.len()),
        )?;
        let by_id = edges
            .into_iter()
            .map(|edge| (edge.id.clone(), edge))
            .collect::<BTreeMap<_, _>>();
        let mut ordered = Vec::with_capacity(edge_ids.len());
        for edge_id in edge_ids {
            let edge = by_id.get(&edge_id).cloned().ok_or_else(|| {
                SnapshotError::Corrupt(format!("{index:?} index references missing edge {edge_id}"))
            })?;
            ordered.push(edge);
        }
        Ok(ordered)
    }

    fn root(&self, index: IndexKind) -> Result<&SnapshotRoot, SnapshotError> {
        self.manifest
            .roots
            .iter()
            .find(|root| root.index == index)
            .ok_or_else(|| SnapshotError::Corrupt(format!("{} root is missing", index.as_str())))
    }

    fn load_tree_object_cached(
        &self,
        index: IndexKind,
        digest: &str,
    ) -> Result<Arc<TreeObject>, SnapshotError> {
        {
            let mut cache = self.object_cache.lock().map_err(|_| {
                SnapshotError::Corrupt("decoded tree cache lock was poisoned".to_owned())
            })?;
            if let Some(object) = cache.get(index, digest) {
                return Ok(object);
            }
        }
        let object = load_tree_object(self.store, index, digest)?;
        let mut cache = self.object_cache.lock().map_err(|_| {
            SnapshotError::Corrupt("decoded tree cache lock was poisoned".to_owned())
        })?;
        Ok(cache.insert_or_get(index, digest, object))
    }

    fn lookup(&self, index: IndexKind, key: &[u8]) -> Result<Option<Vec<u8>>, SnapshotError> {
        let root = self.root(index)?.digest.clone();
        let limits = SnapshotReadLimits {
            max_items: 1,
            max_bytes: MAX_VALUE_BYTES,
            max_objects: 1_024,
            max_depth: GRAPH_SNAPSHOT_MAX_DEPTH,
        };
        lookup_tree(self, index, &root, key, limits, 0)
    }

    fn scan_values(
        &self,
        index: IndexKind,
        prefix: Option<&[u8]>,
        limits: SnapshotReadLimits,
    ) -> Result<Vec<Vec<u8>>, SnapshotError> {
        let limits = limits.validate()?;
        let root = self.root(index)?.digest.clone();
        let mut state = ScanState {
            limits,
            objects: 0,
            bytes: 0,
            entries: Vec::new(),
            truncate_on_limit: false,
            truncated: false,
        };
        scan_tree(self, index, &root, prefix, &mut state, 0)?;
        Ok(state.entries.into_iter().map(|entry| entry.value).collect())
    }

    fn scan_values_bounded(
        &self,
        index: IndexKind,
        prefix: Option<&[u8]>,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<Vec<u8>>, bool), SnapshotError> {
        let limits = limits.validate()?;
        let root = self.root(index)?.digest.clone();
        let mut state = ScanState {
            limits,
            objects: 0,
            bytes: 0,
            entries: Vec::new(),
            truncate_on_limit: true,
            truncated: false,
        };
        scan_tree(self, index, &root, prefix, &mut state, 0)?;
        Ok((
            state.entries.into_iter().map(|entry| entry.value).collect(),
            state.truncated,
        ))
    }

    fn scan_entries_bounded(
        &self,
        index: IndexKind,
        prefix: Option<&[u8]>,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<TreeEntry>, bool), SnapshotError> {
        let limits = limits.validate()?;
        let root = self.root(index)?.digest.clone();
        let mut state = ScanState {
            limits,
            objects: 0,
            bytes: 0,
            entries: Vec::new(),
            truncate_on_limit: true,
            truncated: false,
        };
        scan_tree(self, index, &root, prefix, &mut state, 0)?;
        Ok((state.entries, state.truncated))
    }

    fn scan_entries(
        &self,
        index: IndexKind,
        prefix: Option<&[u8]>,
        limits: SnapshotReadLimits,
    ) -> Result<Vec<TreeEntry>, SnapshotError> {
        let limits = limits.validate()?;
        let root = self.root(index)?.digest.clone();
        let mut state = ScanState {
            limits,
            objects: 0,
            bytes: 0,
            entries: Vec::new(),
            truncate_on_limit: false,
            truncated: false,
        };
        scan_tree(self, index, &root, prefix, &mut state, 0)?;
        Ok(state.entries)
    }
}

struct TreeIntegrity {
    entries: u64,
    first_key: Option<Vec<u8>>,
    last_key: Option<Vec<u8>>,
}

fn validate_tree_integrity<S: Store + ?Sized>(
    reader: &GraphSnapshotReader<'_, S>,
    index: IndexKind,
    digest: &str,
    depth: usize,
) -> Result<TreeIntegrity, SnapshotError> {
    if depth >= GRAPH_SNAPSHOT_MAX_DEPTH {
        return Err(SnapshotError::Limit(
            "tree integrity validation exceeded the depth limit".to_owned(),
        ));
    }
    let object = reader.load_tree_object_cached(index, digest)?;
    match object.as_ref() {
        TreeObject::Leaf { entries, .. } => Ok(TreeIntegrity {
            entries: u64::try_from(entries.len()).map_err(|_| {
                SnapshotError::Limit("tree leaf entry count does not fit u64".to_owned())
            })?,
            first_key: entries.first().map(|entry| entry.key.clone()),
            last_key: entries.last().map(|entry| entry.key.clone()),
        }),
        TreeObject::Branch { children, .. } => {
            let mut entry_count = 0_u64;
            let mut first_key = None;
            let mut last_key: Option<Vec<u8>> = None;
            for child in children {
                let child_integrity =
                    validate_tree_integrity(reader, index, &child.digest, depth.saturating_add(1))?;
                let child_first = child_integrity.first_key.ok_or_else(|| {
                    SnapshotError::Corrupt("tree branch references an empty child".to_owned())
                })?;
                if child.first_key != child_first {
                    return Err(SnapshotError::Corrupt(format!(
                        "{} tree branch separator does not match its child",
                        index.as_str()
                    )));
                }
                if last_key
                    .as_ref()
                    .is_some_and(|previous| previous >= &child_first)
                {
                    return Err(SnapshotError::Corrupt(format!(
                        "{} tree child ranges are not strictly ordered",
                        index.as_str()
                    )));
                }
                first_key.get_or_insert_with(|| child_first.clone());
                last_key = child_integrity.last_key;
                entry_count = entry_count
                    .checked_add(child_integrity.entries)
                    .ok_or_else(|| {
                        SnapshotError::Limit("tree entry count exceeds u64".to_owned())
                    })?;
            }
            Ok(TreeIntegrity {
                entries: entry_count,
                first_key,
                last_key,
            })
        }
    }
}

fn index_entry_id(entry: &TreeEntry, label: &str) -> Result<String, SnapshotError> {
    let segments = decode_key_segments(&entry.key).map_err(SnapshotError::from)?;
    let id = segments
        .last()
        .and_then(|segment| std::str::from_utf8(segment).ok())
        .ok_or_else(|| SnapshotError::Corrupt(format!("{label} ID is invalid")))?;
    Ok(id.to_owned())
}

/// Encode an index key using the portable, length-prefixed store encoding.
pub fn encode_graph_index_key(
    index: IndexKind,
    segments: &[&[u8]],
) -> Result<Vec<u8>, SnapshotError> {
    let mut all = Vec::with_capacity(segments.len() + 1);
    all.push(index.as_str().as_bytes());
    all.extend_from_slice(segments);
    encode_key_segments(&all).map_err(SnapshotError::from)
}

/// Keep normal names ordered and readable while giving graph-valid, deeply
/// qualified names a deterministic portable representation. The extra marker
/// segment prevents a digest key from colliding with the raw two-segment form.
fn encode_name_index_key(name: &str, node_id: &str) -> Result<Vec<u8>, SnapshotError> {
    let name = normalize_symbol(name);
    if name.is_empty() {
        return encode_graph_index_key(IndexKind::Names, &[b"empty", node_id.as_bytes()]);
    }
    match encode_graph_index_key(
        IndexKind::Names,
        &[b"value", name.as_bytes(), node_id.as_bytes()],
    ) {
        Ok(key) => Ok(key),
        Err(SnapshotError::Store(StoreError::ComponentTooLarge { .. })) => {
            let digest = hex_digest(name.as_bytes());
            encode_graph_index_key(
                IndexKind::Names,
                &[b"sha256", digest.as_bytes(), node_id.as_bytes()],
            )
        }
        Err(error) => Err(error),
    }
}

fn encode_name_prefix(name: &str) -> Result<Vec<u8>, SnapshotError> {
    if name.is_empty() {
        return encode_graph_index_key(IndexKind::Names, &[b"empty"]);
    }
    match encode_graph_index_key(IndexKind::Names, &[b"value", name.as_bytes()]) {
        Ok(key) => Ok(key),
        Err(SnapshotError::Store(StoreError::ComponentTooLarge { .. })) => {
            let digest = hex_digest(name.as_bytes());
            encode_graph_index_key(IndexKind::Names, &[b"sha256", digest.as_bytes()])
        }
        Err(error) => Err(error),
    }
}

fn encode_file_path_key(path: &str) -> Result<Vec<u8>, SnapshotError> {
    match encode_graph_index_key(IndexKind::Files, &[b"path", path.as_bytes()]) {
        Ok(key) => Ok(key),
        Err(SnapshotError::Store(StoreError::ComponentTooLarge { .. })) => {
            let digest = hex_digest(path.as_bytes());
            encode_graph_index_key(IndexKind::Files, &[b"path-sha256", digest.as_bytes()])
        }
        Err(error) => Err(error),
    }
}

fn normalize_symbol(value: &str) -> String {
    value
        .trim()
        .trim_end_matches("()")
        .trim_start_matches('.')
        .to_lowercase()
}

fn snapshot_identity(graph: &GraphDocument) -> Result<String, SnapshotError> {
    digest_canonical_graph(graph, true).map(|(digest, _)| digest)
}

fn digest_canonical_graph(
    graph: &GraphDocument,
    clear_generation: bool,
) -> Result<(String, u64), SnapshotError> {
    digest_json(&canonical_graph_document_with_generation(
        graph,
        clear_generation,
    ))
}

fn canonical_graph_document_with_generation(
    graph: &GraphDocument,
    clear_generation: bool,
) -> CanonicalGraphDocument<'_> {
    let mut metadata = graph.graph.clone();
    metadata.files.sort_by(|left, right| left.id.cmp(&right.id));
    if clear_generation {
        metadata.build.generation_id.clear();
    }
    let mut nodes = graph.nodes.iter().collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let mut links = graph.links.iter().collect::<Vec<_>>();
    links.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
    });
    CanonicalGraphDocument {
        directed: graph.directed,
        multigraph: graph.multigraph,
        graph: metadata,
        nodes,
        links,
    }
}

fn build_term_postings(graph: &GraphDocument) -> BTreeMap<String, Vec<String>> {
    let node_by_id = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut aliases_by_target = BTreeMap::<&str, BTreeSet<&str>>::new();
    for edge in &graph.links {
        if edge.kind == compass_model::code_graph::EdgeKind::Aliases
            && let Some(alias) = node_by_id.get(edge.source.as_str())
        {
            aliases_by_target
                .entry(edge.target.as_str())
                .or_default()
                .insert(alias.name.as_str());
        }
    }

    let mut term_postings = BTreeMap::<String, Vec<String>>::new();
    for node in &graph.nodes {
        let mut terms = searchable_node_terms(node);
        for alias in aliases_by_target
            .get(node.id.as_str())
            .into_iter()
            .flat_map(|aliases| aliases.iter())
        {
            terms.extend(search_terms(alias));
            terms.extend(compass_model::search::identifier_search_terms(alias));
        }
        for term in terms {
            term_postings.entry(term).or_default().push(node.id.clone());
        }
    }
    for node_ids in term_postings.values_mut() {
        node_ids.sort();
        node_ids.dedup();
    }
    term_postings
}

fn searchable_node_terms(node: &NodeRecord) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    terms.extend(search_terms(&node.name));
    terms.extend(search_terms(&node.qualified_name));
    terms.extend(compass_model::search::identifier_search_terms(&node.name));
    terms.extend(compass_model::search::identifier_search_terms(
        &node.qualified_name,
    ));
    terms.extend(search_terms(node.kind.as_str()));
    for role in &node.roles {
        let role = format!("{role:?}");
        terms.extend(search_terms(&role));
    }
    if let Some(language) = &node.language {
        terms.extend(search_terms(language));
    }
    if let Some(framework) = &node.framework {
        terms.extend(search_terms(framework));
    }
    if let Some(source) = &node.source {
        terms.extend(search_terms(&source.file));
    }
    if let Some(community) = &node.community {
        terms.extend(search_terms(&community.id.to_string()));
        if let Some(label) = &community.label {
            terms.extend(search_terms(label));
        }
    }
    if let Some(path) = node
        .details
        .as_ref()
        .and_then(|details| serde_json::to_value(details).ok())
        .and_then(|value| {
            value
                .get("data")
                .and_then(|data| data.get("path"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
    {
        terms.extend(search_terms(&path));
    }
    terms
}

fn build_index(
    graph: &GraphDocument,
    index: IndexKind,
    term_postings: Option<&BTreeMap<String, Vec<String>>>,
) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, SnapshotError> {
    let mut entries = BTreeMap::new();
    match index {
        IndexKind::Metadata => {
            let mut metadata_graph = graph.graph.clone();
            metadata_graph.files.clear();
            metadata_graph.coverage.clear();
            metadata_graph.diagnostics.clear();
            let metadata = MetadataRecord {
                directed: graph.directed,
                multigraph: graph.multigraph,
                graph: metadata_graph,
            };
            insert_json(
                &mut entries,
                encode_graph_index_key(IndexKind::Metadata, &[])?,
                &metadata,
            )?;
            for file in &graph.graph.files {
                insert_json(
                    &mut entries,
                    encode_graph_index_key(IndexKind::Metadata, &[b"file", file.id.as_bytes()])?,
                    file,
                )?;
            }
            for (ordinal, coverage) in graph.graph.coverage.iter().enumerate() {
                let ordinal = format!("{ordinal:08}");
                insert_json(
                    &mut entries,
                    encode_graph_index_key(
                        IndexKind::Metadata,
                        &[b"coverage", ordinal.as_bytes()],
                    )?,
                    coverage,
                )?;
            }
            for (ordinal, diagnostic) in graph.graph.diagnostics.iter().enumerate() {
                let ordinal = format!("{ordinal:08}");
                insert_json(
                    &mut entries,
                    encode_graph_index_key(
                        IndexKind::Metadata,
                        &[
                            b"diagnostic",
                            ordinal.as_bytes(),
                            diagnostic.code.as_bytes(),
                        ],
                    )?,
                    diagnostic,
                )?;
                if diagnostic.code == "publication_omission_summary" {
                    let key = encode_graph_index_key(
                        IndexKind::Metadata,
                        &[b"diagnostic-code", diagnostic.code.as_bytes()],
                    )?;
                    let value = encode_json(diagnostic)?;
                    entries.entry(key).or_insert(value);
                }
            }
            insert_json(
                &mut entries,
                encode_graph_index_key(IndexKind::Metadata, &[b"scope-capability"])?,
                &DISCOVERY_SCOPE_INDEX_CAPABILITY_V1,
            )?;
        }
        IndexKind::Nodes => {
            for node in &graph.nodes {
                insert_json(
                    &mut entries,
                    encode_graph_index_key(IndexKind::Nodes, &[node.id.as_bytes()])?,
                    node,
                )?;
            }
        }
        IndexKind::Names => {
            for node in &graph.nodes {
                insert_json(
                    &mut entries,
                    encode_name_index_key(&node.name, &node.id)?,
                    &(),
                )?;
                if node.qualified_name != node.name {
                    insert_json(
                        &mut entries,
                        encode_name_index_key(&node.qualified_name, &node.id)?,
                        &(),
                    )?;
                }
            }
        }
        IndexKind::Terms => {
            let term_postings = term_postings
                .ok_or_else(|| SnapshotError::Corrupt("term postings are missing".to_owned()))?;
            for (term, node_ids) in term_postings {
                let prefix_length = term.len().min(3);
                let prefix = term.get(..prefix_length).unwrap_or(term.as_str());
                for (chunk_index, chunk) in
                    node_ids.chunks(GRAPH_TERM_POSTING_CHUNK_ITEMS).enumerate()
                {
                    let chunk_index = format!("{chunk_index:08}");
                    insert_json(
                        &mut entries,
                        encode_graph_index_key(
                            IndexKind::Terms,
                            &[
                                prefix.as_bytes(),
                                b"node_prefix",
                                term.as_bytes(),
                                chunk_index.as_bytes(),
                            ],
                        )?,
                        &TermPostingChunk {
                            term: term.clone(),
                            node_ids: chunk.to_vec(),
                        },
                    )?;
                }
            }
            let capability = IDENTIFIER_SUBWORD_INDEX_CAPABILITY_V1;
            let prefix = capability.get(..3).unwrap_or(capability);
            insert_json(
                &mut entries,
                encode_graph_index_key(
                    IndexKind::Terms,
                    &[
                        prefix.as_bytes(),
                        b"node_prefix",
                        capability.as_bytes(),
                        b"00000000",
                    ],
                )?,
                &TermPostingChunk {
                    term: capability.to_owned(),
                    node_ids: Vec::new(),
                },
            )?;
            for (term, node_ids) in operation_role_term_postings(graph) {
                for (chunk_index, chunk) in
                    node_ids.chunks(GRAPH_TERM_POSTING_CHUNK_ITEMS).enumerate()
                {
                    let chunk_index = format!("{chunk_index:08}");
                    insert_json(
                        &mut entries,
                        encode_graph_index_key(
                            IndexKind::Terms,
                            &[b"operation_role", term.as_bytes(), chunk_index.as_bytes()],
                        )?,
                        &TermPostingChunk {
                            term: term.clone(),
                            node_ids: chunk.to_vec(),
                        },
                    )?;
                }
            }
            let operation_capability = OPERATION_ROLE_TERM_INDEX_CAPABILITY_V1;
            insert_json(
                &mut entries,
                encode_graph_index_key(
                    IndexKind::Terms,
                    &[
                        b"operation_role",
                        operation_capability.as_bytes(),
                        b"00000000",
                    ],
                )?,
                &TermPostingChunk {
                    term: operation_capability.to_owned(),
                    node_ids: Vec::new(),
                },
            )?;
            for (term, node_ids) in declaration_term_postings(graph, term_postings) {
                for (chunk_index, chunk) in
                    node_ids.chunks(GRAPH_TERM_POSTING_CHUNK_ITEMS).enumerate()
                {
                    let chunk_index = format!("{chunk_index:08}");
                    insert_json(
                        &mut entries,
                        encode_graph_index_key(
                            IndexKind::Terms,
                            &[b"declaration", term.as_bytes(), chunk_index.as_bytes()],
                        )?,
                        &TermPostingChunk {
                            term: term.clone(),
                            node_ids: chunk.to_vec(),
                        },
                    )?;
                }
            }
            let declaration_capability = DECLARATION_TERM_INDEX_CAPABILITY_V1;
            insert_json(
                &mut entries,
                encode_graph_index_key(
                    IndexKind::Terms,
                    &[
                        b"declaration",
                        declaration_capability.as_bytes(),
                        b"00000000",
                    ],
                )?,
                &TermPostingChunk {
                    term: declaration_capability.to_owned(),
                    node_ids: Vec::new(),
                },
            )?;
            let relationship_postings =
                compass_model::search::direct_call_source_identifier_postings(graph);
            for (term, source_ids) in &relationship_postings {
                for source_id in source_ids {
                    insert_json(
                        &mut entries,
                        encode_graph_index_key(
                            IndexKind::Terms,
                            &[b"call_source_member", source_id.as_bytes(), term.as_bytes()],
                        )?,
                        &(),
                    )?;
                }
                let prefix = term.get(..term.len().min(3)).unwrap_or(term.as_str());
                for (chunk_index, chunk) in source_ids
                    .chunks(GRAPH_TERM_POSTING_CHUNK_ITEMS)
                    .enumerate()
                {
                    let chunk_index = format!("{chunk_index:08}");
                    insert_json(
                        &mut entries,
                        encode_graph_index_key(
                            IndexKind::Terms,
                            &[
                                b"call_source",
                                prefix.as_bytes(),
                                term.as_bytes(),
                                chunk_index.as_bytes(),
                            ],
                        )?,
                        &TermPostingChunk {
                            term: term.clone(),
                            node_ids: chunk.to_vec(),
                        },
                    )?;
                }
            }
            for (term, source_id, target_id) in
                compass_model::search::direct_call_source_identifier_targets(graph)
            {
                insert_json(
                    &mut entries,
                    encode_graph_index_key(
                        IndexKind::Terms,
                        &[
                            b"call_source_target",
                            source_id.as_bytes(),
                            term.as_bytes(),
                            target_id.as_bytes(),
                        ],
                    )?,
                    &(),
                )?;
            }
            let relationship_capability = RELATIONSHIP_TERM_INDEX_CAPABILITY_V1;
            let relationship_prefix = relationship_capability
                .get(..3)
                .unwrap_or(relationship_capability);
            insert_json(
                &mut entries,
                encode_graph_index_key(
                    IndexKind::Terms,
                    &[
                        b"call_source",
                        relationship_prefix.as_bytes(),
                        relationship_capability.as_bytes(),
                        b"00000000",
                    ],
                )?,
                &TermPostingChunk {
                    term: relationship_capability.to_owned(),
                    node_ids: Vec::new(),
                },
            )?;
            for node in &graph.nodes {
                for (kind, value, canonical) in
                    compass_model::query_contract::discovery_scope_postings(node)
                {
                    let value_digest = hex_digest(value.as_bytes());
                    let canonical_digest = hex_digest(canonical.as_bytes());
                    let key = encode_graph_index_key(
                        IndexKind::Terms,
                        &[
                            b"scope",
                            kind.as_bytes(),
                            value_digest.as_bytes(),
                            canonical_digest.as_bytes(),
                        ],
                    )?;
                    entries
                        .entry(key)
                        .or_insert(encode_json(&(value, canonical))?);
                }
            }
        }
        IndexKind::Communities => {
            for node in &graph.nodes {
                if let Some(community) = &node.community {
                    let community_id = community.id.to_string();
                    insert_json(
                        &mut entries,
                        encode_graph_index_key(
                            IndexKind::Communities,
                            &[community_id.as_bytes(), node.id.as_bytes()],
                        )?,
                        &node.id,
                    )?;
                }
            }
        }
        IndexKind::Files => {
            for file in &graph.graph.files {
                insert_json(&mut entries, encode_file_path_key(&file.path)?, &file.id)?;
            }
        }
        IndexKind::Edges => {
            for edge in &graph.links {
                insert_json(
                    &mut entries,
                    encode_graph_index_key(IndexKind::Edges, &[edge.id.as_bytes()])?,
                    edge,
                )?;
            }
        }
        IndexKind::Outgoing => {
            for edge in &graph.links {
                let kind = edge.kind.as_str().as_bytes();
                insert_json(
                    &mut entries,
                    encode_graph_index_key(
                        IndexKind::Outgoing,
                        &[
                            edge.source.as_bytes(),
                            kind,
                            edge.target.as_bytes(),
                            edge.id.as_bytes(),
                        ],
                    )?,
                    &(),
                )?;
            }
        }
        IndexKind::Incoming => {
            for edge in &graph.links {
                let kind = edge.kind.as_str().as_bytes();
                insert_json(
                    &mut entries,
                    encode_graph_index_key(
                        IndexKind::Incoming,
                        &[
                            edge.target.as_bytes(),
                            kind,
                            edge.source.as_bytes(),
                            edge.id.as_bytes(),
                        ],
                    )?,
                    &(),
                )?;
            }
        }
        IndexKind::Diagnostics => {}
    }
    Ok(entries)
}

fn is_operation_role_declaration(node: &NodeRecord) -> bool {
    is_source_backed_type_declaration(node)
        && compass_model::search::identifier_search_terms(&node.name)
            .iter()
            .any(|term| compass_model::search::OPERATION_ROLE_TOKENS.contains(&term.as_str()))
}

fn is_source_backed_type_declaration(node: &NodeRecord) -> bool {
    matches!(
        node.kind,
        NodeKind::Class
            | NodeKind::Struct
            | NodeKind::Interface
            | NodeKind::Trait
            | NodeKind::Protocol
            | NodeKind::Enum
            | NodeKind::TypeAlias
    ) && node.source_file().is_some_and(|file| !file.is_empty())
}

fn operation_role_term_postings(graph: &GraphDocument) -> BTreeMap<String, Vec<String>> {
    let mut postings = BTreeMap::<String, Vec<String>>::new();
    for node in graph
        .nodes
        .iter()
        .filter(|node| is_operation_role_declaration(node))
    {
        let mut terms = compass_model::search::identifier_search_terms(&node.name);
        terms.extend(compass_model::search::identifier_search_terms(
            &node.qualified_name,
        ));
        for term in terms {
            postings.entry(term).or_default().push(node.id.clone());
        }
    }
    for node_ids in postings.values_mut() {
        node_ids.sort();
        node_ids.dedup();
    }
    postings
}

fn declaration_term_postings(
    graph: &GraphDocument,
    term_postings: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<String>> {
    let declaration_ids = graph
        .nodes
        .iter()
        .filter(|node| is_source_backed_type_declaration(node))
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    term_postings
        .iter()
        .filter_map(|(term, node_ids)| {
            let declarations = node_ids
                .iter()
                .filter(|node_id| declaration_ids.contains(node_id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            (!declarations.is_empty()).then(|| (term.clone(), declarations))
        })
        .collect()
}

fn validate_file_node_delta(
    previous: &GraphDocument,
    current: &GraphDocument,
) -> Result<(), SnapshotError> {
    if previous.directed != current.directed || previous.multigraph != current.multigraph {
        return Err(SnapshotError::Unsupported(
            "file-node delta changed graph directionality".to_owned(),
        ));
    }
    if previous.nodes.len() != current.nodes.len()
        || previous
            .nodes
            .iter()
            .zip(&current.nodes)
            .any(|(previous, current)| previous.id != current.id)
    {
        return Err(SnapshotError::Unsupported(
            "file-node delta changed the node set".to_owned(),
        ));
    }
    let mut changed_node = false;
    // V1 graph publication orders nodes, links, and file records by their
    // stable identities. The identity walk above makes that ordering an
    // explicit precondition, so validation stays linear and allocation-free
    // for large fact-neutral edits.
    for (previous_node, node) in previous.nodes.iter().zip(&current.nodes) {
        let changed = previous_node != node;
        if changed
            && (node.kind != NodeKind::File
                || !file_node_index_projection_equal(previous_node, node))
        {
            return Err(SnapshotError::Unsupported(
                "file-node delta changed a non-file node".to_owned(),
            ));
        }
        changed_node |= changed;
    }
    if previous.links.len() != current.links.len()
        || previous
            .links
            .iter()
            .zip(&current.links)
            .any(|(previous, current)| previous != current)
    {
        return Err(SnapshotError::Unsupported(
            "file-node delta changed graph relationships".to_owned(),
        ));
    }
    if previous.graph.files.len() != current.graph.files.len()
        || previous
            .graph
            .files
            .iter()
            .zip(&current.graph.files)
            .any(|(previous, current)| previous.path != current.path || previous.id != current.id)
    {
        return Err(SnapshotError::Unsupported(
            "file-node delta changed the file path index".to_owned(),
        ));
    }
    if !changed_node {
        return Err(SnapshotError::Corrupt(
            "file-node delta contains no changed node records".to_owned(),
        ));
    }
    Ok(())
}

fn validate_graph_delta(
    previous: &GraphDocument,
    current: &GraphDocument,
) -> Result<(), SnapshotError> {
    if previous.directed != current.directed || previous.multigraph != current.multigraph {
        return Err(SnapshotError::Unsupported(
            "graph delta changed graph directionality".to_owned(),
        ));
    }
    Ok(())
}

fn validate_node_value_delta(
    previous: &GraphDocument,
    current: &GraphDocument,
    changed_node_ids: &BTreeSet<String>,
) -> Result<(), SnapshotError> {
    if previous.directed != current.directed
        || previous.multigraph != current.multigraph
        || previous.links != current.links
    {
        return Err(SnapshotError::Unsupported(
            "node-value delta changed graph topology".to_owned(),
        ));
    }
    let file_keys = |graph: &GraphDocument| {
        graph
            .graph
            .files
            .iter()
            .map(|file| (file.path.clone(), file.id.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    if file_keys(previous) != file_keys(current) {
        return Err(SnapshotError::Unsupported(
            "node-value delta changed the file path index".to_owned(),
        ));
    }
    if previous.nodes.len() != current.nodes.len() {
        return Err(SnapshotError::Unsupported(
            "node-value delta changed the node set".to_owned(),
        ));
    }
    let mut changed_seen = BTreeSet::new();
    for (before, after) in previous.nodes.iter().zip(&current.nodes) {
        if before.id != after.id {
            return Err(SnapshotError::Unsupported(
                "node-value delta changed node identity or ordering".to_owned(),
            ));
        }
        if before == after {
            continue;
        }
        if !changed_node_ids.contains(&after.id)
            || before.name != after.name
            || before.qualified_name != after.qualified_name
            || before.community != after.community
            || searchable_node_terms(before) != searchable_node_terms(after)
        {
            return Err(SnapshotError::Unsupported(
                "node-value delta changed a secondary index projection".to_owned(),
            ));
        }
        changed_seen.insert(after.id.clone());
    }
    if changed_seen != *changed_node_ids {
        return Err(SnapshotError::Corrupt(
            "node-value delta changed-node set is not exact".to_owned(),
        ));
    }
    if changed_node_ids.is_empty() && previous.graph == current.graph {
        return Err(SnapshotError::Corrupt(
            "node-value delta contains no changed graph values".to_owned(),
        ));
    }
    Ok(())
}

fn graph_delta_indexes(previous: &GraphDocument, current: &GraphDocument) -> BTreeSet<IndexKind> {
    let mut changed = BTreeSet::new();
    if previous.graph != current.graph {
        changed.insert(IndexKind::Metadata);
    }
    if previous.graph.files != current.graph.files {
        changed.insert(IndexKind::Files);
    }

    let nodes_changed = previous.nodes != current.nodes;
    if nodes_changed {
        changed.insert(IndexKind::Nodes);
        changed.insert(IndexKind::Names);
        changed.insert(IndexKind::Communities);
    }

    let links_changed = previous.links != current.links;
    if links_changed {
        changed.insert(IndexKind::Edges);
        changed.insert(IndexKind::Outgoing);
        changed.insert(IndexKind::Incoming);
    }
    if nodes_changed || links_changed {
        // Alias edges contribute searchable terms for their target nodes.
        changed.insert(IndexKind::Terms);
    }
    changed
}

fn file_node_index_projection_equal(previous: &NodeRecord, current: &NodeRecord) -> bool {
    if previous.kind != current.kind
        || previous.roles != current.roles
        || previous.name != current.name
        || previous.qualified_name != current.qualified_name
        || previous.language != current.language
        || previous.framework != current.framework
        || previous.community != current.community
    {
        return false;
    }
    match (&previous.details, &current.details) {
        (Some(NodeDetails::File(previous)), Some(NodeDetails::File(current))) => {
            previous.generated == current.generated
        }
        _ => previous.details == current.details,
    }
}

fn update_index_tree<S: Store + ?Sized>(
    store: &S,
    writer: &mut ObjectWriter<'_, S>,
    index: IndexKind,
    digest: &str,
    updates: &BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    depth: usize,
) -> Result<String, SnapshotError> {
    if updates.is_empty() {
        return Ok(digest.to_owned());
    }
    if depth >= GRAPH_SNAPSHOT_MAX_DEPTH {
        return Err(SnapshotError::Limit(
            "delta tree depth limit exceeded".to_owned(),
        ));
    }
    match load_tree_object(store, index, digest)? {
        TreeObject::Leaf { entries, .. } => {
            let mut values = entries
                .into_iter()
                .map(|entry| (entry.key, entry.value))
                .collect::<BTreeMap<_, _>>();
            for (key, value) in updates {
                match value {
                    Some(value) => {
                        values.insert(key.clone(), value.clone());
                    }
                    None => {
                        values.remove(key);
                    }
                }
            }
            build_index_tree(writer, index, values)
        }
        TreeObject::Branch { children, .. } => {
            let mut changed = false;
            let mut updated_children = children.clone();
            for child_index in 0..children.len() {
                let Some(child) = children.get(child_index) else {
                    continue;
                };
                let next_first_key = children
                    .get(child_index.saturating_add(1))
                    .map(|next| next.first_key.as_slice());
                let child_updates = updates
                    .iter()
                    .filter(|(key, _)| {
                        child.first_key.as_slice() <= key.as_slice()
                            && next_first_key.is_none_or(|next| key.as_slice() < next)
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<BTreeMap<_, _>>();
                if child_updates.is_empty() {
                    continue;
                }
                let child_digest = update_index_tree(
                    store,
                    writer,
                    index,
                    &child.digest,
                    &child_updates,
                    depth.saturating_add(1),
                )?;
                if child_digest != child.digest {
                    changed = true;
                    if let Some(updated) = updated_children.get_mut(child_index) {
                        updated.digest = child_digest;
                    }
                }
            }
            if !changed {
                return Ok(digest.to_owned());
            }
            put_tree_object(
                writer,
                &TreeObject::Branch {
                    schema: GRAPH_SNAPSHOT_LAYOUT_V1.to_owned(),
                    index,
                    children: updated_children,
                },
            )
        }
    }
}

fn insert_json<T: Serialize>(
    entries: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    key: Vec<u8>,
    value: &T,
) -> Result<(), SnapshotError> {
    let value = encode_json(value)?;
    if let Some(previous) = entries.get(&key) {
        if previous != &value {
            return Err(SnapshotError::Corrupt(
                "duplicate index key with different values".to_owned(),
            ));
        }
        return Ok(());
    }
    entries.insert(key, value);
    Ok(())
}

fn build_index_tree<S: Store + ?Sized>(
    writer: &mut ObjectWriter<'_, S>,
    index: IndexKind,
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
) -> Result<String, SnapshotError> {
    let mut leaves = Vec::new();
    let mut current = Vec::with_capacity(GRAPH_SNAPSHOT_MAX_LEAF_ENTRIES);
    for (key, value) in entries {
        current.push(TreeEntry { key, value });
        if current.len() == GRAPH_SNAPSHOT_MAX_LEAF_ENTRIES {
            put_leaf_entries(writer, index, std::mem::take(&mut current), &mut leaves)?;
        }
    }
    if current.is_empty() && leaves.is_empty() {
        let object = TreeObject::Leaf {
            schema: GRAPH_SNAPSHOT_LAYOUT_V1.to_owned(),
            index,
            entries: Vec::new(),
        };
        leaves.push(TreeChild {
            first_key: Vec::new(),
            digest: put_tree_object(writer, &object)?,
        });
    } else if !current.is_empty() {
        put_leaf_entries(writer, index, current, &mut leaves)?;
    }
    build_branch_levels(writer, index, leaves)
}

fn put_leaf_entries<S: Store + ?Sized>(
    writer: &mut ObjectWriter<'_, S>,
    index: IndexKind,
    mut entries: Vec<TreeEntry>,
    leaves: &mut Vec<TreeChild>,
) -> Result<(), SnapshotError> {
    let first_key = entries
        .first()
        .map(|entry| entry.key.clone())
        .ok_or_else(|| SnapshotError::Corrupt("empty leaf".to_owned()))?;
    let object = TreeObject::Leaf {
        schema: GRAPH_SNAPSHOT_LAYOUT_V1.to_owned(),
        index,
        entries,
    };
    let raw = encode_tree_object_raw(&object)?;
    if raw.len() <= MAX_VALUE_BYTES {
        leaves.push(TreeChild {
            first_key,
            digest: put_encoded_tree_object(writer, encode_tree_object_raw_bytes(raw)?)?,
        });
        return Ok(());
    }
    let TreeObject::Leaf {
        entries: oversized, ..
    } = object
    else {
        return Err(SnapshotError::Corrupt("expected a leaf object".to_owned()));
    };
    entries = oversized;
    if entries.len() == 1 {
        return Err(SnapshotError::Limit(format!(
            "{} index entry exceeds the maximum immutable object size",
            index.as_str()
        )));
    }
    let right = entries.split_off(entries.len() / 2);
    put_leaf_entries(writer, index, entries, leaves)?;
    put_leaf_entries(writer, index, right, leaves)
}

fn build_branch_levels<S: Store + ?Sized>(
    writer: &mut ObjectWriter<'_, S>,
    index: IndexKind,
    children: Vec<TreeChild>,
) -> Result<String, SnapshotError> {
    if children.is_empty() {
        return Err(SnapshotError::Corrupt(
            "tree cannot have an empty branch".to_owned(),
        ));
    }
    if children.len() == 1 {
        return children
            .first()
            .map(|child| child.digest.clone())
            .ok_or_else(|| SnapshotError::Corrupt("tree root is missing".to_owned()));
    }
    let mut parents = Vec::new();
    for chunk in children.chunks(GRAPH_SNAPSHOT_MAX_FANOUT) {
        let chunk = chunk.to_vec();
        let first_key = chunk
            .first()
            .map(|child| child.first_key.clone())
            .ok_or_else(|| SnapshotError::Corrupt("empty branch group".to_owned()))?;
        let object = TreeObject::Branch {
            schema: GRAPH_SNAPSHOT_LAYOUT_V1.to_owned(),
            index,
            children: chunk,
        };
        let digest = put_tree_object(writer, &object)?;
        parents.push(TreeChild { first_key, digest });
    }
    build_branch_levels(writer, index, parents)
}

fn put_tree_object<S: Store + ?Sized>(
    writer: &mut ObjectWriter<'_, S>,
    object: &TreeObject,
) -> Result<String, SnapshotError> {
    let bytes = encode_tree_object(object)?;
    if bytes.len() > MAX_VALUE_BYTES {
        return Err(SnapshotError::Limit(
            "immutable tree object exceeds the store value limit".to_owned(),
        ));
    }
    put_encoded_tree_object(writer, bytes)
}

fn put_encoded_tree_object<S: Store + ?Sized>(
    writer: &mut ObjectWriter<'_, S>,
    bytes: Vec<u8>,
) -> Result<String, SnapshotError> {
    let digest = hex_digest(&bytes);
    writer.put(object_key(&digest)?, bytes)?;
    Ok(digest)
}

fn put_immutable_object<S: Store + ?Sized>(
    writer: &mut ObjectWriter<'_, S>,
    key: Key,
    bytes: Vec<u8>,
) -> Result<(), SnapshotError> {
    if bytes.len() > MAX_VALUE_BYTES {
        return Err(SnapshotError::Limit(
            "snapshot manifest exceeds the store value limit".to_owned(),
        ));
    }
    writer.put(key, bytes)
}

fn lookup_tree<S: Store + ?Sized>(
    reader: &GraphSnapshotReader<'_, S>,
    index: IndexKind,
    digest: &str,
    key: &[u8],
    limits: SnapshotReadLimits,
    depth: usize,
) -> Result<Option<Vec<u8>>, SnapshotError> {
    if depth >= limits.max_depth {
        return Err(SnapshotError::Limit("tree depth limit exceeded".to_owned()));
    }
    let object = reader.load_tree_object_cached(index, digest)?;
    match object.as_ref() {
        TreeObject::Leaf { entries, .. } => Ok(entries
            .binary_search_by(|entry| entry.key.as_slice().cmp(key))
            .ok()
            .and_then(|position| entries.get(position).map(|entry| entry.value.clone()))),
        TreeObject::Branch { children, .. } => {
            let child = children
                .iter()
                .take_while(|child| child.first_key.as_slice() <= key)
                .last();
            child.map_or(Ok(None), |child| {
                lookup_tree(reader, index, &child.digest, key, limits, depth + 1)
            })
        }
    }
}

struct MultiLookupState {
    limits: SnapshotReadLimits,
    objects: usize,
    bytes: usize,
    values: BTreeMap<Vec<u8>, Vec<u8>>,
}

fn lookup_many_tree<S: Store + ?Sized>(
    reader: &GraphSnapshotReader<'_, S>,
    index: IndexKind,
    digest: &str,
    keys: &[Vec<u8>],
    state: &mut MultiLookupState,
    depth: usize,
) -> Result<(), SnapshotError> {
    if keys.is_empty() {
        return Ok(());
    }
    if depth >= state.limits.max_depth {
        return Err(SnapshotError::Limit("tree depth limit exceeded".to_owned()));
    }
    state.objects = state.objects.saturating_add(1);
    if state.objects > state.limits.max_objects {
        return Err(SnapshotError::Limit(
            "tree object read limit exceeded".to_owned(),
        ));
    }
    let object = reader.load_tree_object_cached(index, digest)?;
    match object.as_ref() {
        TreeObject::Leaf { entries, .. } => {
            for key in keys {
                let Ok(position) = entries.binary_search_by(|entry| entry.key.cmp(key)) else {
                    continue;
                };
                let Some(entry) = entries.get(position) else {
                    continue;
                };
                state.bytes = state
                    .bytes
                    .saturating_add(entry.key.len())
                    .saturating_add(entry.value.len());
                if state.bytes > state.limits.max_bytes {
                    return Err(SnapshotError::Limit(
                        "snapshot byte limit exceeded".to_owned(),
                    ));
                }
                state.values.insert(entry.key.clone(), entry.value.clone());
            }
        }
        TreeObject::Branch { children, .. } => {
            let mut grouped = BTreeMap::<usize, Vec<Vec<u8>>>::new();
            for key in keys {
                let position =
                    children.partition_point(|child| child.first_key.as_slice() <= key.as_slice());
                if let Some(child_index) = position.checked_sub(1) {
                    grouped.entry(child_index).or_default().push(key.clone());
                }
            }
            for (child_index, child_keys) in grouped {
                let child = children.get(child_index).ok_or_else(|| {
                    SnapshotError::Corrupt("tree child index is missing".to_owned())
                })?;
                lookup_many_tree(
                    reader,
                    index,
                    &child.digest,
                    &child_keys,
                    state,
                    depth.saturating_add(1),
                )?;
            }
        }
    }
    Ok(())
}

struct ScanState {
    limits: SnapshotReadLimits,
    objects: usize,
    bytes: usize,
    entries: Vec<TreeEntry>,
    truncate_on_limit: bool,
    truncated: bool,
}

fn scan_tree<S: Store + ?Sized>(
    reader: &GraphSnapshotReader<'_, S>,
    index: IndexKind,
    digest: &str,
    prefix: Option<&[u8]>,
    state: &mut ScanState,
    depth: usize,
) -> Result<(), SnapshotError> {
    if state.truncated {
        return Ok(());
    }
    if depth >= state.limits.max_depth {
        return Err(SnapshotError::Limit("tree depth limit exceeded".to_owned()));
    }
    state.objects = state.objects.saturating_add(1);
    if state.objects > state.limits.max_objects {
        return Err(SnapshotError::Limit(
            "tree object read limit exceeded".to_owned(),
        ));
    }
    let object = reader.load_tree_object_cached(index, digest)?;
    match object.as_ref() {
        TreeObject::Leaf { entries, .. } => {
            for entry in entries {
                if let Some(prefix) = prefix
                    && !key_has_segment_prefix(&entry.key, prefix)?
                {
                    continue;
                }
                if state.entries.len() >= state.limits.max_items {
                    if state.truncate_on_limit {
                        state.truncated = true;
                        return Ok(());
                    }
                    return Err(SnapshotError::Limit(
                        "snapshot item limit exceeded".to_owned(),
                    ));
                }
                state.bytes = state
                    .bytes
                    .saturating_add(entry.key.len())
                    .saturating_add(entry.value.len());
                if state.bytes > state.limits.max_bytes {
                    if state.truncate_on_limit {
                        state.truncated = true;
                        return Ok(());
                    }
                    return Err(SnapshotError::Limit(
                        "snapshot byte limit exceeded".to_owned(),
                    ));
                }
                state.entries.push(entry.clone());
            }
        }
        TreeObject::Branch { children, .. } => {
            for (child_index, child) in children.iter().enumerate() {
                if let Some(prefix) = prefix
                    && !child_may_match_prefix(children, child_index, prefix)?
                {
                    continue;
                }
                scan_tree(reader, index, &child.digest, prefix, state, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn child_may_match_prefix(
    children: &[TreeChild],
    index: usize,
    prefix: &[u8],
) -> Result<bool, SnapshotError> {
    let first = children
        .get(index)
        .ok_or_else(|| SnapshotError::Corrupt("tree child index is missing".to_owned()))?;
    let next = children
        .get(index.saturating_add(1))
        .map(|child| child.first_key.as_slice());
    let segments = decode_key_segments(prefix).map_err(SnapshotError::from)?;
    let prefix_count = segments.len();
    for total_count in prefix_count..=MAX_KEY_SEGMENTS {
        let total_count = u8::try_from(total_count).map_err(|_| {
            SnapshotError::Corrupt("key segment count does not fit the v1 encoding".to_owned())
        })?;
        let mut lower = prefix.to_vec();
        let Some(encoded_count) = lower.get_mut(1) else {
            return Err(SnapshotError::Corrupt(
                "encoded key prefix is truncated".to_owned(),
            ));
        };
        *encoded_count = total_count;
        let upper = lexicographic_successor(&lower);
        let starts_before_upper = upper
            .as_ref()
            .is_none_or(|upper| first.first_key.as_slice() < upper.as_slice());
        let ends_after_lower = next.is_none_or(|next| next > lower.as_slice());
        if starts_before_upper && ends_after_lower {
            return Ok(true);
        }
    }
    Ok(false)
}

fn lexicographic_successor(value: &[u8]) -> Option<Vec<u8>> {
    let mut successor = value.to_vec();
    for index in (0..successor.len()).rev() {
        if successor[index] != u8::MAX {
            successor[index] = successor[index].saturating_add(1);
            successor.truncate(index.saturating_add(1));
            return Some(successor);
        }
    }
    None
}

fn key_has_segment_prefix(key: &[u8], prefix: &[u8]) -> Result<bool, SnapshotError> {
    let key_segments = decode_key_segments(key).map_err(SnapshotError::from)?;
    let prefix_segments = decode_key_segments(prefix).map_err(SnapshotError::from)?;
    Ok(key_segments.len() >= prefix_segments.len()
        && key_segments
            .iter()
            .zip(prefix_segments.iter())
            .all(|(key, prefix)| key == prefix))
}

fn load_tree_object<S: Store + ?Sized>(
    store: &S,
    index: IndexKind,
    digest: &str,
) -> Result<TreeObject, SnapshotError> {
    let namespace = NamespaceId::graph();
    let partition = object_partition()?;
    let key = object_key(digest)?;
    let Some(entry) = store.get(&namespace, &partition, &key)? else {
        return Err(SnapshotError::Corrupt(format!(
            "{} tree object {digest} is missing",
            index.as_str()
        )));
    };
    verify_digest(&entry.value, digest)?;
    let object = decode_tree_object(&entry.value)?;
    validate_tree_object(&object, index)?;
    Ok(object)
}

fn validate_tree_object(object: &TreeObject, expected: IndexKind) -> Result<(), SnapshotError> {
    match object {
        TreeObject::Leaf {
            schema,
            index,
            entries,
        } => {
            validate_tree_header(schema, *index, expected)?;
            if entries.len() > GRAPH_SNAPSHOT_MAX_LEAF_ENTRIES {
                return Err(SnapshotError::Corrupt(
                    "tree leaf exceeds the configured fanout".to_owned(),
                ));
            }
            for pair in entries.windows(2) {
                if pair[0].key >= pair[1].key {
                    return Err(SnapshotError::Corrupt(
                        "tree leaf keys are not strictly ordered".to_owned(),
                    ));
                }
            }
            for entry in entries {
                validate_index_key(expected, &entry.key)?;
            }
        }
        TreeObject::Branch {
            schema,
            index,
            children,
        } => {
            validate_tree_header(schema, *index, expected)?;
            if children.is_empty() || children.len() > GRAPH_SNAPSHOT_MAX_FANOUT {
                return Err(SnapshotError::Corrupt(
                    "tree branch has an invalid child count".to_owned(),
                ));
            }
            for (position, pair) in children.windows(2).enumerate() {
                if pair[0].first_key >= pair[1].first_key {
                    return Err(SnapshotError::Corrupt(format!(
                        "{} tree branch separators are not strictly ordered at children {} and {} (lengths {} and {}, digests {} and {})",
                        expected.as_str(),
                        position,
                        position.saturating_add(1),
                        pair[0].first_key.len(),
                        pair[1].first_key.len(),
                        hex_digest(&pair[0].first_key),
                        hex_digest(&pair[1].first_key),
                    )));
                }
            }
            for child in children {
                validate_index_key(expected, &child.first_key)?;
                parse_digest(&child.digest)?;
            }
        }
    }
    Ok(())
}

fn validate_tree_header(
    schema: &str,
    index: IndexKind,
    expected: IndexKind,
) -> Result<(), SnapshotError> {
    if schema != GRAPH_SNAPSHOT_LAYOUT_V1 {
        return Err(SnapshotError::Unsupported(format!(
            "tree object schema {schema} is not supported"
        )));
    }
    if index != expected {
        return Err(SnapshotError::Corrupt(format!(
            "tree object index is {}; expected {}",
            index.as_str(),
            expected.as_str()
        )));
    }
    Ok(())
}

fn validate_index_key(index: IndexKind, key: &[u8]) -> Result<(), SnapshotError> {
    let segments = decode_key_segments(key).map_err(SnapshotError::from)?;
    if segments.first().map(Vec::as_slice) != Some(index.as_str().as_bytes()) {
        return Err(SnapshotError::Corrupt(format!(
            "{} tree contains a key for another index",
            index.as_str()
        )));
    }
    Ok(())
}

fn search_terms(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|term| !term.is_empty())
        .filter_map(|term| {
            let term = normalize_search_term(term);
            (!term.is_empty()).then_some(term)
        })
}

fn normalize_search_term(value: &str) -> String {
    value
        .trim()
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .collect::<String>()
        .to_lowercase()
}

fn point_lookup_batch_limits(item_count: usize) -> SnapshotReadLimits {
    SnapshotReadLimits {
        max_items: item_count.max(1),
        max_bytes: MAX_VALUE_BYTES.saturating_mul(4_096),
        max_objects: GRAPH_SNAPSHOT_MAX_OBJECTS,
        max_depth: GRAPH_SNAPSHOT_MAX_DEPTH,
    }
}

fn bounded_count(count: u64) -> Result<usize, SnapshotError> {
    let count = usize::try_from(count).map_err(|_| {
        SnapshotError::Limit("snapshot count does not fit this platform".to_owned())
    })?;
    if count > GRAPH_SNAPSHOT_MAX_ITEMS {
        return Err(SnapshotError::Limit(format!(
            "snapshot count exceeds the {GRAPH_SNAPSHOT_MAX_ITEMS}-item limit"
        )));
    }
    Ok(count.max(1))
}

fn encode_json<T: Serialize>(value: &T) -> Result<Vec<u8>, SnapshotError> {
    serde_json::to_vec(value).map_err(|error| SnapshotError::Encode(error.to_string()))
}

struct DigestWriter {
    hasher: Sha256,
    bytes: u64,
    overflowed: bool,
}

impl DigestWriter {
    fn new() -> Self {
        Self {
            hasher: Sha256::new(),
            bytes: 0,
            overflowed: false,
        }
    }
}

impl Write for DigestWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let buffer_len = u64::try_from(buffer.len())
            .map_err(|_| std::io::Error::other("serialized byte count does not fit u64"))?;
        let Some(next) = self.bytes.checked_add(buffer_len) else {
            self.overflowed = true;
            return Err(std::io::Error::other("serialized byte count exceeds u64"));
        };
        self.hasher.update(buffer);
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn digest_json<T: Serialize>(value: &T) -> Result<(String, u64), SnapshotError> {
    let mut writer = DigestWriter::new();
    if let Err(error) = serde_json::to_writer(&mut writer, value) {
        if writer.overflowed {
            return Err(SnapshotError::Limit(
                "canonical graph byte count exceeds u64".to_owned(),
            ));
        }
        return Err(SnapshotError::Encode(error.to_string()));
    }
    if writer.bytes == 0 {
        return Err(SnapshotError::Limit(
            "canonical graph serialization is empty".to_owned(),
        ));
    }
    Ok((format!("{:x}", writer.hasher.finalize()), writer.bytes))
}

fn encode_tree_object(value: &TreeObject) -> Result<Vec<u8>, SnapshotError> {
    let raw = encode_tree_object_raw(value)?;
    if raw.len() > MAX_VALUE_BYTES {
        return Err(SnapshotError::Limit(
            "uncompressed tree object exceeds the store value limit".to_owned(),
        ));
    }
    encode_tree_object_raw_bytes(raw)
}

fn encode_tree_object_raw(value: &TreeObject) -> Result<Vec<u8>, SnapshotError> {
    rmp_serde::to_vec_named(value).map_err(|error| SnapshotError::Encode(error.to_string()))
}

fn encode_tree_object_raw_bytes(raw: Vec<u8>) -> Result<Vec<u8>, SnapshotError> {
    let compressed = zstd::stream::encode_all(raw.as_slice(), 1)
        .map_err(|error| SnapshotError::Encode(format!("tree compression failed: {error}")))?;
    if compressed.len().saturating_add(TREE_ZSTD_HEADER_BYTES) >= raw.len() {
        return Ok(raw);
    }
    let raw_len = u32::try_from(raw.len())
        .map_err(|_| SnapshotError::Limit("tree object length does not fit u32".to_owned()))?;
    let mut encoded = Vec::with_capacity(TREE_ZSTD_HEADER_BYTES.saturating_add(compressed.len()));
    encoded.extend_from_slice(TREE_ZSTD_MAGIC);
    encoded.extend_from_slice(&raw_len.to_be_bytes());
    encoded.extend_from_slice(&compressed);
    Ok(encoded)
}

fn decode_json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, SnapshotError> {
    serde_json::from_slice(bytes).map_err(|error| SnapshotError::Decode(error.to_string()))
}

fn decode_tree_object(bytes: &[u8]) -> Result<TreeObject, SnapshotError> {
    let mut decoded;
    let bytes = if bytes.starts_with(TREE_ZSTD_MAGIC) {
        let length_bytes = bytes
            .get(TREE_ZSTD_MAGIC.len()..TREE_ZSTD_HEADER_BYTES)
            .and_then(|value| <[u8; 4]>::try_from(value).ok())
            .ok_or_else(|| {
                SnapshotError::Decode("compressed tree header is truncated".to_owned())
            })?;
        let expected = usize::try_from(u32::from_be_bytes(length_bytes))
            .map_err(|_| SnapshotError::Decode("compressed tree length is invalid".to_owned()))?;
        if expected == 0 || expected > MAX_VALUE_BYTES {
            return Err(SnapshotError::Decode(
                "compressed tree length exceeds the bounded object limit".to_owned(),
            ));
        }
        let payload = bytes.get(TREE_ZSTD_HEADER_BYTES..).ok_or_else(|| {
            SnapshotError::Decode("compressed tree payload is missing".to_owned())
        })?;
        let decoder = zstd::stream::read::Decoder::new(payload).map_err(|error| {
            SnapshotError::Decode(format!("tree decompression failed: {error}"))
        })?;
        let mut limited = decoder.take(expected.saturating_add(1) as u64);
        decoded = Vec::with_capacity(expected);
        limited.read_to_end(&mut decoded).map_err(|error| {
            SnapshotError::Decode(format!("tree decompression failed: {error}"))
        })?;
        if decoded.len() != expected {
            return Err(SnapshotError::Decode(
                "compressed tree length does not match its header".to_owned(),
            ));
        }
        decoded.as_slice()
    } else {
        bytes
    };
    rmp_serde::from_slice(bytes).or_else(|message_pack_error| {
        serde_json::from_slice(bytes).map_err(|json_error| {
            SnapshotError::Decode(format!(
                "tree object is neither MessagePack ({message_pack_error}) nor legacy JSON ({json_error})"
            ))
        })
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_digest(value: &str) -> Result<[u8; 32], SnapshotError> {
    if value.len() != 64 {
        return Err(SnapshotError::Corrupt(
            "digest must contain 64 hexadecimal characters".to_owned(),
        ));
    }
    let mut output = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_digit(chunk[0]).ok_or_else(|| {
            SnapshotError::Corrupt("digest contains a non-hexadecimal character".to_owned())
        })?;
        let low = hex_digit(chunk[1]).ok_or_else(|| {
            SnapshotError::Corrupt("digest contains a non-hexadecimal character".to_owned())
        })?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn verify_digest(bytes: &[u8], expected: &str) -> Result<(), SnapshotError> {
    let expected = parse_digest(expected)?;
    if digest_bytes(bytes) != expected {
        return Err(SnapshotError::Corrupt(
            "content digest does not match its address".to_owned(),
        ));
    }
    Ok(())
}

fn object_partition() -> Result<PartitionKey, SnapshotError> {
    PartitionKey::new(GRAPH_SNAPSHOT_OBJECT_PARTITION.as_bytes()).map_err(SnapshotError::from)
}

fn catalog_partition() -> Result<PartitionKey, SnapshotError> {
    PartitionKey::new(GRAPH_SNAPSHOT_CATALOG_PARTITION.as_bytes()).map_err(SnapshotError::from)
}

fn object_key(digest: &str) -> Result<Key, SnapshotError> {
    parse_digest(digest)?;
    Key::new(format!("object/{digest}").as_bytes()).map_err(SnapshotError::from)
}

fn manifest_key(digest: &str) -> Result<Key, SnapshotError> {
    parse_digest(digest)?;
    Key::new(format!("manifest/{digest}").as_bytes()).map_err(SnapshotError::from)
}

fn digest_bytes(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::code_graph::{
        BuildMetadata, EdgeKind, EdgeRecord, ExtractionStatus, FileNodeDetails, FileRecord,
        NodeDetails, NodeKind,
    };
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
    use compass_store::{
        Entry, KeyRange, MemoryStore, ScanCursor, ScanLimits, ScanPage, StoreCapabilities,
    };
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn search_terms_omit_unicode_marks_removed_by_normalization() {
        assert_eq!(
            search_terms("handler \u{0947}\u{0902}").collect::<Vec<_>>(),
            vec!["handler".to_owned()]
        );
        assert!(search_terms("\u{093e}\u{0940}\u{0947}").next().is_none());
        assert_eq!(
            search_terms("café").collect::<Vec<_>>(),
            vec!["cafe".to_owned()]
        );
    }

    #[test]
    fn canonical_digest_counter_has_no_two_gibibyte_cutoff() -> Result<(), std::io::Error> {
        let mut writer = DigestWriter::new();
        writer.bytes = (2_u64 * 1024 * 1024 * 1024) + 1;

        writer.write_all(b"x")?;

        assert_eq!(writer.bytes, (2_u64 * 1024 * 1024 * 1024) + 2);
        writer.bytes = u64::MAX;
        assert!(writer.write_all(b"x").is_err());
        assert!(writer.overflowed);
        Ok(())
    }

    #[derive(Default)]
    struct CountingStore {
        inner: MemoryStore,
        object_gets: AtomicUsize,
        object_barrier: Mutex<Option<Arc<Barrier>>>,
    }

    impl CountingStore {
        fn reset_object_gets(&self) {
            self.object_gets.store(0, Ordering::SeqCst);
        }

        fn object_gets(&self) -> usize {
            self.object_gets.load(Ordering::SeqCst)
        }

        fn set_object_barrier(&self, barrier: Option<Arc<Barrier>>) -> Result<(), SnapshotError> {
            *self.object_barrier.lock().map_err(|_| {
                SnapshotError::Corrupt("test object barrier lock was poisoned".to_owned())
            })? = barrier;
            Ok(())
        }
    }

    impl Store for CountingStore {
        fn capabilities(&self) -> StoreCapabilities {
            self.inner.capabilities()
        }

        fn get(
            &self,
            namespace: &NamespaceId,
            partition: &PartitionKey,
            key: &Key,
        ) -> Result<Option<Entry>, StoreError> {
            if partition.as_bytes() == GRAPH_SNAPSHOT_OBJECT_PARTITION.as_bytes()
                && key.as_bytes().starts_with(b"object/")
            {
                self.object_gets.fetch_add(1, Ordering::SeqCst);
                let barrier = self
                    .object_barrier
                    .lock()
                    .map_err(|_| StoreError::Corrupt("object barrier lock poisoned".to_owned()))?
                    .clone();
                if let Some(barrier) = barrier {
                    barrier.wait();
                }
            }
            self.inner.get(namespace, partition, key)
        }

        fn scan(
            &self,
            namespace: &NamespaceId,
            partition: &PartitionKey,
            range: &KeyRange,
            limits: ScanLimits,
            cursor: Option<&ScanCursor>,
        ) -> Result<ScanPage, StoreError> {
            self.inner.scan(namespace, partition, range, limits, cursor)
        }

        fn put(
            &self,
            namespace: &NamespaceId,
            partition: &PartitionKey,
            key: &Key,
            value: &[u8],
            condition: WriteCondition,
        ) -> Result<Entry, StoreError> {
            self.inner.put(namespace, partition, key, value, condition)
        }

        fn delete(
            &self,
            namespace: &NamespaceId,
            partition: &PartitionKey,
            key: &Key,
            condition: WriteCondition,
        ) -> Result<bool, StoreError> {
            self.inner.delete(namespace, partition, key, condition)
        }
    }

    fn cache_fixture_graph() -> GraphDocument {
        let mut graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        let source = SourceAnchor {
            file: "src/lib.rs".to_owned(),
            start_byte: 0,
            end_byte: 1,
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 1,
        };
        graph.graph.files.push(FileRecord {
            id: compass_model::identity::file_id("src/lib.rs"),
            path: "src/lib.rs".to_owned(),
            language: Some("rust".to_owned()),
            content_digest: "sha256:test".to_owned(),
            byte_size: 1,
            generated: false,
            extraction_status: ExtractionStatus::Extracted,
            extractor_versions: vec!["test".to_owned()],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        });
        graph.nodes.push(NodeRecord {
            id: "a".to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: "a".to_owned(),
            qualified_name: "crate::a".to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            source: Some(source.clone()),
            details: None,
            evidence: vec![Provenance {
                origin: EvidenceOrigin::Ast,
                extractor: "test".to_owned(),
                confidence: EvidenceConfidence::Exact,
                rule: None,
                anchors: vec![source],
                wiring_site: None,
                score: None,
                candidates: Vec::new(),
            }],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        });
        graph
    }

    #[test]
    fn decoded_tree_cache_is_bounded_and_retains_branches_over_leaf_lru() {
        let mut cache = TreeObjectCache::default();
        cache.insert_or_get(
            IndexKind::Nodes,
            "branch",
            TreeObject::Branch {
                schema: GRAPH_SNAPSHOT_LAYOUT_V1.to_owned(),
                index: IndexKind::Nodes,
                children: vec![TreeChild {
                    first_key: vec![0],
                    digest: "child".to_owned(),
                }],
            },
        );
        for index in 0..16 {
            cache.insert_or_get(
                IndexKind::Nodes,
                &format!("leaf-{index:02}"),
                TreeObject::Leaf {
                    schema: GRAPH_SNAPSHOT_LAYOUT_V1.to_owned(),
                    index: IndexKind::Nodes,
                    entries: vec![TreeEntry {
                        key: vec![u8::try_from(index).unwrap_or_default()],
                        value: vec![0; 1024 * 1024],
                    }],
                },
            );
        }

        assert!(cache.object_count <= TREE_OBJECT_CACHE_MAX_OBJECTS);
        assert!(cache.resident_bytes <= TREE_OBJECT_CACHE_MAX_BYTES);
        assert!(cache.get(IndexKind::Nodes, "branch").is_some());
        assert!(cache.get(IndexKind::Nodes, "leaf-00").is_none());
        assert!(cache.get(IndexKind::Nodes, "leaf-15").is_some());
    }

    #[test]
    fn graph_snapshot_reader_remains_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GraphSnapshotReader<'static, MemoryStore>>();
    }

    #[test]
    fn repeated_reader_lookup_reuses_verified_tree_objects() -> Result<(), SnapshotError> {
        let store = CountingStore::default();
        let builder = GraphSnapshotBuilder::new();
        let prepared = builder.prepare(&store, &cache_fixture_graph())?;
        builder.activate(&store, &prepared)?;
        let reader = GraphSnapshotReader::open_active(&store)?
            .ok_or_else(|| SnapshotError::Corrupt("active snapshot missing".to_owned()))?;
        store.reset_object_gets();

        let first = reader.get_node("a")?;
        let first_reads = store.object_gets();
        let second = reader.get_node("a")?;

        assert!(first_reads > 0);
        assert_eq!(first, second);
        assert_eq!(store.object_gets(), first_reads);
        Ok(())
    }

    #[test]
    fn corrupt_tree_objects_are_rejected_without_caching() -> Result<(), SnapshotError> {
        let store = CountingStore::default();
        let builder = GraphSnapshotBuilder::new();
        let prepared = builder.prepare(&store, &cache_fixture_graph())?;
        builder.activate(&store, &prepared)?;
        let reader = GraphSnapshotReader::open_active(&store)?
            .ok_or_else(|| SnapshotError::Corrupt("active snapshot missing".to_owned()))?;
        let root = reader
            .manifest()
            .roots
            .iter()
            .find(|root| root.index == IndexKind::Nodes)
            .ok_or_else(|| SnapshotError::Corrupt("nodes root missing".to_owned()))?;
        store.inner.put(
            &NamespaceId::graph(),
            &object_partition()?,
            &object_key(&root.digest)?,
            b"corrupt",
            WriteCondition::Any,
        )?;
        store.reset_object_gets();

        assert!(matches!(
            reader.get_node("a"),
            Err(SnapshotError::Corrupt(_))
        ));
        assert!(matches!(
            reader.get_node("a"),
            Err(SnapshotError::Corrupt(_))
        ));
        assert_eq!(store.object_gets(), 2);
        assert_eq!(
            reader
                .object_cache
                .lock()
                .map_err(|_| SnapshotError::Corrupt("cache lock poisoned".to_owned()))?
                .object_count,
            0
        );
        Ok(())
    }

    #[test]
    fn concurrent_same_key_misses_charge_one_cache_entry() -> Result<(), SnapshotError> {
        let store = CountingStore::default();
        let builder = GraphSnapshotBuilder::new();
        let prepared = builder.prepare(&store, &cache_fixture_graph())?;
        builder.activate(&store, &prepared)?;
        let reader = GraphSnapshotReader::open_active(&store)?
            .ok_or_else(|| SnapshotError::Corrupt("active snapshot missing".to_owned()))?;
        store.reset_object_gets();
        store.set_object_barrier(Some(Arc::new(Barrier::new(2))))?;

        let (left, right) = std::thread::scope(|scope| {
            let left = scope.spawn(|| reader.get_node("a"));
            let right = scope.spawn(|| reader.get_node("a"));
            (left.join(), right.join())
        });
        let left =
            left.map_err(|_| SnapshotError::Corrupt("left lookup thread panicked".to_owned()))??;
        let right = right
            .map_err(|_| SnapshotError::Corrupt("right lookup thread panicked".to_owned()))??;
        store.set_object_barrier(None)?;

        assert_eq!(left, right);
        assert_eq!(store.object_gets(), 2);
        let cache = reader
            .object_cache
            .lock()
            .map_err(|_| SnapshotError::Corrupt("cache lock poisoned".to_owned()))?;
        assert_eq!(cache.object_count, 1);
        assert_eq!(
            cache.resident_bytes,
            cache
                .entries
                .values()
                .flat_map(BTreeMap::values)
                .map(|entry| entry.resident_bytes)
                .sum::<usize>()
        );
        Ok(())
    }

    #[test]
    fn streamed_canonical_graph_json_matches_serde_encoding() -> Result<(), SnapshotError> {
        let graph = GraphDocument {
            directed: true,
            multigraph: true,
            graph: GraphMetadata::v1(BuildMetadata {
                builder_version: "test".to_owned(),
                schema_fingerprint: "schema".to_owned(),
                source_tree_digest: "tree".to_owned(),
                configuration_digest: "config".to_owned(),
                generation_id: "generation".to_owned(),
                source_commit: None,
            }),
            nodes: vec![
                NodeRecord {
                    id: "b".to_owned(),
                    kind: NodeKind::Function,
                    roles: Vec::new(),
                    name: "B".to_owned(),
                    qualified_name: "b".to_owned(),
                    language: Some("rust".to_owned()),
                    framework: None,
                    source: None,
                    details: None,
                    evidence: Vec::new(),
                    coverage: Vec::new(),
                    diagnostics: Vec::new(),
                    community: None,
                },
                NodeRecord {
                    id: "a".to_owned(),
                    kind: NodeKind::Function,
                    roles: Vec::new(),
                    name: "A".to_owned(),
                    qualified_name: "a".to_owned(),
                    language: Some("rust".to_owned()),
                    framework: None,
                    source: None,
                    details: None,
                    evidence: Vec::new(),
                    coverage: Vec::new(),
                    diagnostics: Vec::new(),
                    community: None,
                },
            ],
            links: vec![EdgeRecord {
                id: "edge".to_owned(),
                key: "edge".to_owned(),
                source: "file".to_owned(),
                target: "symbol".to_owned(),
                kind: EdgeKind::Contains,
                occurrence_rule: None,
                relationship_site: None,
                details: None,
                evidence: Vec::new(),
                weight: None,
                context: None,
                deferred: false,
                diagnostics: Vec::new(),
            }],
        };
        let expected = serde_json::to_vec(&canonical_graph_document_presorted(&graph))
            .map_err(|error| SnapshotError::Encode(error.to_string()))?;
        let mut actual = Vec::new();
        write_canonical_graph_json(&graph, &mut actual)
            .map_err(|error| SnapshotError::Encode(error.to_string()))?;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn streamed_canonical_graph_json_sorts_unsorted_file_inventory() -> Result<(), SnapshotError> {
        let mut graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        let file = |id: &str| FileRecord {
            id: id.to_owned(),
            path: format!("{id}.rs"),
            language: Some("rust".to_owned()),
            content_digest: "sha256:test".to_owned(),
            byte_size: 1,
            generated: false,
            extraction_status: ExtractionStatus::Extracted,
            extractor_versions: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        };
        graph.graph.files = vec![file("z"), file("a")];

        let expected = serde_json::to_vec(&canonical_graph_document_presorted(&graph))
            .map_err(|error| SnapshotError::Encode(error.to_string()))?;
        let mut actual = Vec::new();
        write_canonical_graph_json(&graph, &mut actual)
            .map_err(|error| SnapshotError::Encode(error.to_string()))?;

        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn fact_neutral_delta_reuses_unchanged_record_bytes() -> Result<(), SnapshotError> {
        let build = BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        };
        let file_node = NodeRecord {
            id: "file".to_owned(),
            kind: NodeKind::File,
            roles: Vec::new(),
            name: "main.rs".to_owned(),
            qualified_name: "main.rs".to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            source: None,
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        };
        let symbol_node = NodeRecord {
            id: "symbol".to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: "main".to_owned(),
            qualified_name: "main".to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            source: None,
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        };
        let previous = GraphDocument {
            directed: true,
            multigraph: true,
            graph: GraphMetadata::v1(build),
            nodes: vec![file_node, symbol_node],
            links: Vec::new(),
        };
        let mut source_changed = previous.clone();
        source_changed.nodes[0].source = Some(SourceAnchor {
            file: "src/main.rs".to_owned(),
            start_byte: 0,
            end_byte: 1,
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 1,
        });
        assert!(validate_file_node_delta(&previous, &source_changed).is_ok());
        let mut previous_bytes = Vec::new();
        write_canonical_graph_json(&previous, &mut previous_bytes)
            .map_err(|error| SnapshotError::Encode(error.to_string()))?;

        let mut current = previous.clone();
        current.nodes[0].details = Some(NodeDetails::File(FileNodeDetails {
            content_digest: "sha256:changed".to_owned(),
            byte_size: 42,
            generated: false,
        }));
        let expected = {
            let mut bytes = Vec::new();
            write_canonical_graph_json(&current, &mut bytes)
                .map_err(|error| SnapshotError::Encode(error.to_string()))?;
            bytes
        };
        let changed = BTreeSet::from(["file".to_owned()]);
        let mut actual = Vec::new();
        assert!(
            write_fact_neutral_graph_json_delta(&previous_bytes, &current, &changed, &mut actual,)
                .map_err(|error| SnapshotError::Encode(error.to_string()))?
        );
        assert_eq!(actual, expected);

        let mut prevalidated = Vec::new();
        assert!(
            write_fact_neutral_graph_json_delta_prevalidated(
                &previous_bytes,
                &current,
                &changed,
                &mut prevalidated,
            )
            .map_err(|error| SnapshotError::Encode(error.to_string()))?
        );
        assert_eq!(prevalidated, expected);

        let mut node_value_changed = current.clone();
        node_value_changed.nodes[1].source = Some(SourceAnchor {
            file: "src/main.rs".to_owned(),
            start_byte: 2,
            end_byte: 6,
            start_line: 2,
            start_column: 0,
            end_line: 2,
            end_column: 4,
        });
        let node_value_expected = {
            let mut bytes = Vec::new();
            write_canonical_graph_json(&node_value_changed, &mut bytes)
                .map_err(|error| SnapshotError::Encode(error.to_string()))?;
            bytes
        };
        let mut node_value_actual = Vec::new();
        assert!(
            write_fact_neutral_graph_json_delta(
                &previous_bytes,
                &node_value_changed,
                &BTreeSet::from(["file".to_owned(), "symbol".to_owned()]),
                &mut node_value_actual,
            )
            .map_err(|error| SnapshotError::Encode(error.to_string()))?
        );
        assert_eq!(node_value_actual, node_value_expected);

        let invalid = BTreeSet::from(["missing".to_owned()]);
        let mut no_output = Vec::new();
        assert!(
            !write_fact_neutral_graph_json_delta(
                &previous_bytes,
                &current,
                &invalid,
                &mut no_output,
            )
            .map_err(|error| SnapshotError::Encode(error.to_string()))?
        );
        assert!(no_output.is_empty());
        Ok(())
    }

    #[test]
    fn snapshot_without_scope_capability_remains_readable_but_rejects_scopes()
    -> Result<(), SnapshotError> {
        let graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "legacy-test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        let store = compass_store::MemoryStore::default();
        let builder = GraphSnapshotBuilder::new();
        let mut content = builder.prepare_content(&store, &graph)?;
        let mut metadata_entries = build_index(&graph, IndexKind::Metadata, None)?;
        metadata_entries.remove(&encode_graph_index_key(
            IndexKind::Metadata,
            &[b"scope-capability"],
        )?);
        let metadata_entry_count = metadata_entries.len() as u64;
        let mut writer = ObjectWriter::new(&store)?;
        let metadata_digest = build_index_tree(&mut writer, IndexKind::Metadata, metadata_entries)?;
        let _ = writer.finish()?;
        let metadata_root = content
            .roots
            .iter_mut()
            .find(|root| root.index == IndexKind::Metadata)
            .ok_or_else(|| SnapshotError::Corrupt("metadata root is missing".to_owned()))?;
        metadata_root.digest = metadata_digest;
        metadata_root.entry_count = metadata_entry_count;
        let (graph_digest, graph_bytes) = digest_canonical_graph(&graph, false)?;
        let prepared = builder.finish_content(&store, content, graph_digest, graph_bytes)?;
        builder.activate(&store, &prepared)?;

        let reader = GraphSnapshotReader::open_active(&store)?
            .ok_or_else(|| SnapshotError::Corrupt("active snapshot is missing".to_owned()))?;
        assert!(reader.nodes(SnapshotReadLimits::default())?.is_empty());
        assert_eq!(reader.metadata()?.graph.build, graph.graph.build);
        assert!(matches!(
            reader.resolve_scope_values("node-id", "missing", SnapshotReadLimits::default()),
            Err(SnapshotError::CapabilityUnavailable(message))
                if message.contains("scope_index_unavailable")
        ));
        Ok(())
    }

    #[test]
    fn legacy_snapshot_without_relationship_terms_remains_readable_and_degraded()
    -> Result<(), SnapshotError> {
        let graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "legacy-relationship-test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        let store = compass_store::MemoryStore::default();
        let builder = GraphSnapshotBuilder::new();
        let mut content = builder.prepare_content(&store, &graph)?;
        let term_postings = build_term_postings(&graph);
        let mut term_entries = build_index(&graph, IndexKind::Terms, Some(&term_postings))?;
        let capability = RELATIONSHIP_TERM_INDEX_CAPABILITY_V1;
        let prefix = capability.get(..3).unwrap_or(capability);
        term_entries.remove(&encode_graph_index_key(
            IndexKind::Terms,
            &[
                b"call_source",
                prefix.as_bytes(),
                capability.as_bytes(),
                b"00000000",
            ],
        )?);
        term_entries.remove(&encode_graph_index_key(
            IndexKind::Terms,
            &[
                b"declaration",
                DECLARATION_TERM_INDEX_CAPABILITY_V1.as_bytes(),
                b"00000000",
            ],
        )?);
        let term_entry_count = term_entries.len() as u64;
        let mut writer = ObjectWriter::new(&store)?;
        let term_digest = build_index_tree(&mut writer, IndexKind::Terms, term_entries)?;
        let _ = writer.finish()?;
        let term_root = content
            .roots
            .iter_mut()
            .find(|root| root.index == IndexKind::Terms)
            .ok_or_else(|| SnapshotError::Corrupt("terms root is missing".to_owned()))?;
        term_root.digest = term_digest;
        term_root.entry_count = term_entry_count;
        let (graph_digest, graph_bytes) = digest_canonical_graph(&graph, false)?;
        let prepared = builder.finish_content(&store, content, graph_digest, graph_bytes)?;
        builder.activate(&store, &prepared)?;

        let reader = GraphSnapshotReader::open_active(&store)?
            .ok_or_else(|| SnapshotError::Corrupt("active snapshot is missing".to_owned()))?;
        assert!(reader.nodes(SnapshotReadLimits::default())?.is_empty());
        assert!(reader.supports_identifier_subwords()?);
        assert!(!reader.supports_declaration_terms()?);
        assert!(!reader.supports_relationship_terms()?);
        assert_eq!(
            reader
                .source_ids_for_exact_relationship_term_bounded_work(
                    "checkpoint",
                    SnapshotReadLimits::default(),
                )?
                .0,
            Vec::<String>::new()
        );
        Ok(())
    }

    #[test]
    fn operation_role_term_index_is_complete_and_excludes_data_types() -> Result<(), SnapshotError>
    {
        let mut graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "operation-role-test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        graph.graph.files.push(FileRecord {
            id: compass_model::identity::file_id("src/table.rs"),
            path: "src/table.rs".to_owned(),
            language: Some("rust".to_owned()),
            content_digest: "sha256:test".to_owned(),
            byte_size: 1,
            generated: false,
            extraction_status: ExtractionStatus::Extracted,
            extractor_versions: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        });
        let node = |id: &str, name: &str| NodeRecord {
            id: id.to_owned(),
            kind: NodeKind::Struct,
            roles: Vec::new(),
            name: name.to_owned(),
            qualified_name: format!("crate::{name}"),
            language: Some("rust".to_owned()),
            framework: None,
            source: Some(SourceAnchor {
                file: "src/table.rs".to_owned(),
                start_byte: 0,
                end_byte: 1,
                start_line: 1,
                start_column: 0,
                end_line: 1,
                end_column: 1,
            }),
            details: None,
            evidence: vec![Provenance {
                origin: EvidenceOrigin::Ast,
                extractor: "test".to_owned(),
                confidence: EvidenceConfidence::Exact,
                rule: None,
                anchors: vec![SourceAnchor {
                    file: "src/table.rs".to_owned(),
                    start_byte: 0,
                    end_byte: 1,
                    start_line: 1,
                    start_column: 0,
                    end_line: 1,
                    end_column: 1,
                }],
                wiring_site: None,
                score: None,
                candidates: Vec::new(),
            }],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        };
        graph.nodes = vec![
            node("builder", "DeltaTableBuilder"),
            node("table", "DeltaTable"),
        ];
        let store = compass_store::MemoryStore::default();
        let builder = GraphSnapshotBuilder::new();
        let prepared = builder.prepare(&store, &graph)?;
        builder.activate(&store, &prepared)?;
        let reader = GraphSnapshotReader::open_active(&store)?
            .ok_or_else(|| SnapshotError::Corrupt("active snapshot is missing".to_owned()))?;

        assert!(reader.supports_operation_role_terms()?);
        let (nodes, truncated, work) = reader.operation_role_nodes_for_terms_bounded_work(
            &["delta".to_owned(), "open".to_owned()],
            SnapshotReadLimits::default(),
        )?;
        assert!(!truncated);
        assert_eq!(
            nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["builder"]
        );
        assert_eq!(work.node_ids_decoded, 1);

        assert!(reader.supports_declaration_terms()?);
        let (declarations, truncated, work) = reader.declaration_nodes_for_terms_bounded_work(
            &["delta".to_owned(), "table".to_owned()],
            SnapshotReadLimits::default(),
        )?;
        assert!(!truncated);
        assert_eq!(
            declarations
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["builder", "table"]
        );
        assert_eq!(work.node_ids_decoded, 4);
        Ok(())
    }

    #[test]
    fn raw_prefix_child_routing_matches_a_full_scan_across_key_shapes() -> Result<(), SnapshotError>
    {
        let mut state = 0x9e37_79b9_u64;
        let mut encoded = BTreeSet::new();
        for _ in 0..600 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let count = usize::try_from(state % 6).unwrap_or(0).saturating_add(1);
            let mut segments = Vec::new();
            for _ in 0..count {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let length = usize::try_from(state % 12).unwrap_or(0).saturating_add(1);
                let mut segment = Vec::with_capacity(length);
                for _ in 0..length {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1);
                    segment.push(b'a'.saturating_add(u8::try_from(state % 26).unwrap_or(0)));
                }
                segments.push(segment);
            }
            encoded.insert(
                encode_key_segments(&segments.iter().map(Vec::as_slice).collect::<Vec<_>>())
                    .map_err(SnapshotError::from)?,
            );
        }
        let keys = encoded.into_iter().collect::<Vec<_>>();
        let chunks = keys.chunks(11).collect::<Vec<_>>();
        let children = chunks
            .iter()
            .filter_map(|chunk| chunk.first())
            .map(|first_key| TreeChild {
                first_key: first_key.clone(),
                digest: "test".to_owned(),
            })
            .collect::<Vec<_>>();
        let mut prefixes = Vec::new();
        for key in keys.iter().step_by(7) {
            let segments = decode_key_segments(key).map_err(SnapshotError::from)?;
            for count in 1..=segments.len() {
                prefixes.push(
                    encode_key_segments(
                        &segments[..count]
                            .iter()
                            .map(Vec::as_slice)
                            .collect::<Vec<_>>(),
                    )
                    .map_err(SnapshotError::from)?,
                );
            }
        }
        prefixes.push(
            encode_key_segments(&[b"not-present", b"different-length"])
                .map_err(SnapshotError::from)?,
        );

        for prefix in prefixes {
            let expected = keys
                .iter()
                .filter(|key| key_has_segment_prefix(key, &prefix).unwrap_or(false))
                .cloned()
                .collect::<Vec<_>>();
            let mut actual = Vec::new();
            for (index, chunk) in chunks.iter().enumerate() {
                if child_may_match_prefix(&children, index, &prefix)? {
                    actual.extend(
                        chunk
                            .iter()
                            .filter(|key| key_has_segment_prefix(key, &prefix).unwrap_or(false))
                            .cloned(),
                    );
                }
            }
            assert_eq!(actual, expected);
        }
        Ok(())
    }

    #[test]
    fn json_record_identity_uses_the_canonical_leading_id() {
        assert_eq!(
            json_record_identity(br#"{"id":"plain","name":"ignored"}"#, false).as_deref(),
            Some("plain")
        );
        assert_eq!(
            json_record_identity(br#"{"id":"escaped\"id","name":"ignored"}"#, false).as_deref(),
            Some("escaped\"id")
        );
        let malformed = br#"{"id":"plain","value":01}"#;
        assert!(json_record_identity(malformed, true).is_none());
        assert_eq!(
            json_record_identity(malformed, false).as_deref(),
            Some("plain")
        );
        assert!(json_record_identity(br#"{"name":"plain","id":"not-leading"}"#, false).is_none());
    }

    #[test]
    fn tree_decoder_accepts_compact_and_legacy_encodings() -> Result<(), SnapshotError> {
        let object = TreeObject::Leaf {
            schema: GRAPH_SNAPSHOT_LAYOUT_V1.to_owned(),
            index: IndexKind::Nodes,
            entries: vec![TreeEntry {
                key: encode_graph_index_key(IndexKind::Nodes, &[b"node-id"])?,
                value: br#"{"id":"node-id"}"#.to_vec(),
            }],
        };

        assert_eq!(decode_tree_object(&encode_tree_object(&object)?)?, object);
        assert_eq!(decode_tree_object(&encode_json(&object)?)?, object);

        let compressible = TreeObject::Leaf {
            schema: GRAPH_SNAPSHOT_LAYOUT_V1.to_owned(),
            index: IndexKind::Nodes,
            entries: vec![TreeEntry {
                key: encode_graph_index_key(IndexKind::Nodes, &[b"compressible"])?,
                value: vec![b'x'; 32 * 1024],
            }],
        };
        let encoded = encode_tree_object(&compressible)?;
        assert!(encoded.starts_with(TREE_ZSTD_MAGIC));
        assert_eq!(decode_tree_object(&encoded)?, compressible);
        Ok(())
    }
}
