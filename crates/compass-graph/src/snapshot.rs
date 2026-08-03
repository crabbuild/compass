//! Backend-neutral immutable graph snapshots.
//!
//! The snapshot layer deliberately knows only the [`compass_store::Store`]
//! contract.  It stores canonical records in content-addressed ordered trees;
//! SQLite, redb, PostgreSQL, and a remote adapter therefore observe the same
//! logical layout.  The JSON artifact remains the compatibility engine and is
//! reconstructed from these records for export and differential testing.

use std::collections::{BTreeMap, BTreeSet};

use compass_model::code_graph::{
    CODE_GRAPH_SCHEMA_V1, EdgeRecord, FileRecord, GraphDiagnostic, GraphDocument, GraphMetadata,
    NodeRecord,
};
use compass_model::validate_code_graph;
use compass_store::{
    ImmutableWrite, Key, MAX_GRAPH_BYTES, MAX_IMMUTABLE_BATCH_BYTES, MAX_IMMUTABLE_BATCH_ITEMS,
    MAX_SCAN_BYTES, MAX_SCAN_ITEMS, MAX_VALUE_BYTES, NamespaceId, PartitionKey, Store, StoreError,
    WriteCondition, decode_key_segments, encode_key_segments,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GRAPH_SNAPSHOT_LAYOUT_V2: &str = "compass.store.graph-index/2";
pub const GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1: &str = "compass.store.graph-selector/1";
pub const GRAPH_SNAPSHOT_CANONICAL_ENCODING_V1: &str = "canonical-json-v1";
pub const GRAPH_SNAPSHOT_OBJECT_PARTITION: &str = "graph-snapshot/objects";
pub const GRAPH_SNAPSHOT_CATALOG_PARTITION: &str = "graph-snapshot/catalog";
pub const GRAPH_SNAPSHOT_ACTIVE_KEY: &str = "active";
pub const GRAPH_SNAPSHOT_MAX_DEPTH: usize = 64;
pub const GRAPH_SNAPSHOT_MAX_OBJECTS: usize = 100_000;
pub const GRAPH_SNAPSHOT_MAX_ITEMS: usize = 5_000_000;
pub const GRAPH_SNAPSHOT_MAX_FANOUT: usize = 32;
pub const GRAPH_SNAPSHOT_MAX_LEAF_ENTRIES: usize = 128;
const GRAPH_SNAPSHOT_LEAF_OVERHEAD_BYTES: usize = 512;

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
        if self.schema != GRAPH_SNAPSHOT_LAYOUT_V2 {
            return Err(SnapshotError::Unsupported(format!(
                "expected {GRAPH_SNAPSHOT_LAYOUT_V2}, found {}",
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
        if self.graph_bytes == 0 || self.graph_bytes > MAX_GRAPH_BYTES as u64 {
            return Err(SnapshotError::Corrupt(format!(
                "graph byte count exceeds the {MAX_GRAPH_BYTES}-byte limit"
            )));
        }
        if self.node_count > GRAPH_SNAPSHOT_MAX_ITEMS as u64
            || self.edge_count > GRAPH_SNAPSHOT_MAX_ITEMS as u64
        {
            return Err(SnapshotError::Limit(format!(
                "graph record count exceeds the {GRAPH_SNAPSHOT_MAX_ITEMS}-item snapshot limit"
            )));
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

type SnapshotIndexes = BTreeMap<IndexKind, BTreeMap<Vec<u8>, Vec<u8>>>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataRecord {
    directed: bool,
    multigraph: bool,
    graph: GraphMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticRecord {
    owner: String,
    diagnostic: GraphDiagnostic,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TermPosting {
    term: String,
    node_id: String,
}

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
        self.prepare_owned(store, graph.clone())
    }

    /// Prepare a snapshot from an owned document without cloning the complete
    /// graph solely to establish deterministic record ordering.
    pub fn prepare_owned<S: Store + ?Sized>(
        &self,
        store: &S,
        graph: GraphDocument,
    ) -> Result<PreparedGraphSnapshot, SnapshotError> {
        let mut canonical = canonical_document_owned(graph)?;
        validate_code_graph(&canonical)
            .map_err(|error| SnapshotError::Corrupt(format!("graph validation failed: {error}")))?;
        let graph_bytes = encode_json(&canonical)?;
        if graph_bytes.is_empty() || graph_bytes.len() > MAX_GRAPH_BYTES {
            return Err(SnapshotError::Limit(format!(
                "canonical graph exceeds the {MAX_GRAPH_BYTES}-byte limit"
            )));
        }
        let graph_digest = hex_digest(&graph_bytes);
        let snapshot_id = snapshot_identity(&mut canonical)?;
        let indexes = build_indexes(&canonical)?;
        let mut writer = ObjectWriter::new(store)?;
        let mut roots = Vec::with_capacity(IndexKind::ALL.len());
        for index in IndexKind::ALL {
            let entries = indexes.get(&index).ok_or_else(|| {
                SnapshotError::Corrupt(format!("missing {} index", index.as_str()))
            })?;
            let digest = build_index_tree(&mut writer, index, entries)?;
            roots.push(SnapshotRoot {
                index,
                digest,
                entry_count: entries.len() as u64,
            });
        }
        let manifest = GraphSnapshotManifest {
            schema: GRAPH_SNAPSHOT_LAYOUT_V2.to_owned(),
            canonical_encoding: GRAPH_SNAPSHOT_CANONICAL_ENCODING_V1.to_owned(),
            snapshot_id,
            graph_schema: CODE_GRAPH_SCHEMA_V1.to_owned(),
            graph_digest,
            graph_bytes: graph_bytes.len() as u64,
            node_count: canonical.nodes.len() as u64,
            edge_count: canonical.links.len() as u64,
            roots,
        };
        manifest.validate()?;
        let manifest_bytes = encode_json(&manifest)?;
        let manifest_digest = hex_digest(&manifest_bytes);
        put_immutable_object(&mut writer, manifest_key(&manifest_digest)?, manifest_bytes)?;
        let stats = writer.finish()?;
        Ok(PreparedGraphSnapshot {
            manifest,
            manifest_digest,
            new_objects: stats.new_objects,
            reused_objects: stats.reused_objects,
            write_transactions: stats.write_transactions,
            bytes_written: stats.bytes_written,
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
    encode_json(&canonical_document(graph)?)
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

/// Check whether immutable graph manifests exceed the retained-generation
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
        let (values, truncated) =
            self.scan_values_bounded(IndexKind::Names, Some(&prefix), limits)?;
        let mut nodes = Vec::with_capacity(values.len());
        for value in values {
            let node_id = decode_json::<String>(&value)?;
            let node = self.get_node(&node_id)?.ok_or_else(|| {
                SnapshotError::Corrupt(format!("name index references missing node {node_id}"))
            })?;
            nodes.push(node);
        }
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        nodes.dedup_by(|left, right| left.id == right.id);
        Ok((nodes, truncated))
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
            let normalized = term.to_lowercase();
            let prefix_length = normalized.len().min(3);
            let posting_prefix = normalized
                .get(..prefix_length)
                .unwrap_or(normalized.as_str());
            let prefix = encode_graph_index_key(
                IndexKind::Terms,
                &[posting_prefix.as_bytes(), b"node_prefix"],
            )?;
            let (values, posting_truncated) =
                self.scan_values_bounded(IndexKind::Terms, Some(&prefix), limits)?;
            truncated |= posting_truncated;
            let ids = values
                .into_iter()
                .map(|value| decode_json::<TermPosting>(&value))
                .filter_map(|posting| match posting {
                    Ok(posting) if posting.term.starts_with(&normalized) => {
                        Some(Ok(posting.node_id))
                    }
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            intersection = Some(match intersection {
                Some(previous) => previous.intersection(&ids).cloned().collect(),
                None => ids,
            });
            if intersection.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        let mut nodes = Vec::new();
        for node_id in intersection.unwrap_or_default() {
            if let Some(node) = self.get_node(&node_id)? {
                nodes.push(node);
            }
        }
        Ok((nodes, truncated))
    }

    pub fn file_by_path(&self, path: &str) -> Result<Option<FileRecord>, SnapshotError> {
        let key = encode_file_path_key(path)?;
        self.lookup(IndexKind::Files, &key)?
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
        let mut edges = BTreeMap::new();
        let mut truncated = false;
        for kind in kinds {
            let prefix =
                encode_graph_index_key(index, &[node_id.as_bytes(), kind.as_str().as_bytes()])?;
            let (values, bucket_truncated) =
                self.scan_values_bounded(index, Some(&prefix), limits)?;
            truncated |= bucket_truncated;
            for value in values {
                let edge_id = decode_json::<String>(&value)?;
                let edge = self.get_edge(&edge_id)?.ok_or_else(|| {
                    SnapshotError::Corrupt(format!(
                        "{index:?} index references missing edge {edge_id}"
                    ))
                })?;
                edges.insert(edge.id.clone(), edge);
            }
        }
        Ok((edges.into_values().collect(), truncated))
    }

    pub fn incident(
        &self,
        node_id: &str,
        limits: SnapshotReadLimits,
    ) -> Result<(Vec<EdgeRecord>, bool), SnapshotError> {
        let incoming_prefix = encode_graph_index_key(IndexKind::Incoming, &[node_id.as_bytes()])?;
        let outgoing_prefix = encode_graph_index_key(IndexKind::Outgoing, &[node_id.as_bytes()])?;
        let (incoming, incoming_truncated) =
            self.scan_values_bounded(IndexKind::Incoming, Some(&incoming_prefix), limits)?;
        let (outgoing, outgoing_truncated) =
            self.scan_values_bounded(IndexKind::Outgoing, Some(&outgoing_prefix), limits)?;
        let mut edges = BTreeMap::new();
        for value in incoming.into_iter().chain(outgoing) {
            let edge_id = decode_json::<String>(&value)?;
            let edge = self.get_edge(&edge_id)?.ok_or_else(|| {
                SnapshotError::Corrupt(format!("incident index references missing edge {edge_id}"))
            })?;
            edges.insert(edge.id.clone(), edge);
        }
        Ok((
            edges.into_values().collect(),
            incoming_truncated || outgoing_truncated,
        ))
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
        let values = self.scan_values(index, Some(&prefix), limits)?;
        let mut edges = Vec::with_capacity(values.len());
        for value in values {
            let edge_id = decode_json::<String>(&value)?;
            let edge = self.get_edge(&edge_id)?.ok_or_else(|| {
                SnapshotError::Corrupt(format!("{index:?} index references missing edge {edge_id}"))
            })?;
            edges.push(edge);
        }
        Ok(edges)
    }

    fn root(&self, index: IndexKind) -> Result<&SnapshotRoot, SnapshotError> {
        self.manifest
            .roots
            .iter()
            .find(|root| root.index == index)
            .ok_or_else(|| SnapshotError::Corrupt(format!("{} root is missing", index.as_str())))
    }

    fn lookup(&self, index: IndexKind, key: &[u8]) -> Result<Option<Vec<u8>>, SnapshotError> {
        let root = self.root(index)?.digest.clone();
        let limits = SnapshotReadLimits {
            max_items: 1,
            max_bytes: MAX_VALUE_BYTES,
            max_objects: 1_024,
            max_depth: GRAPH_SNAPSHOT_MAX_DEPTH,
        };
        lookup_tree(self.store, index, &root, key, limits, 0)
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
        scan_tree(self.store, index, &root, prefix, &mut state, 0)?;
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
        scan_tree(self.store, index, &root, prefix, &mut state, 0)?;
        Ok((
            state.entries.into_iter().map(|entry| entry.value).collect(),
            state.truncated,
        ))
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
        scan_tree(self.store, index, &root, prefix, &mut state, 0)?;
        Ok(state.entries)
    }
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

fn canonical_document(graph: &GraphDocument) -> Result<GraphDocument, SnapshotError> {
    canonical_document_owned(graph.clone())
}

fn canonical_document_owned(mut canonical: GraphDocument) -> Result<GraphDocument, SnapshotError> {
    canonical
        .nodes
        .sort_by(|left, right| left.id.cmp(&right.id));
    canonical.links.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
    });
    canonical
        .graph
        .files
        .sort_by(|left, right| left.id.cmp(&right.id));
    let _ = encode_json(&canonical)?;
    Ok(canonical)
}

fn snapshot_identity(graph: &mut GraphDocument) -> Result<String, SnapshotError> {
    let generation_id = std::mem::take(&mut graph.graph.build.generation_id);
    let identity = encode_json(graph).map(|bytes| hex_digest(&bytes));
    graph.graph.build.generation_id = generation_id;
    identity
}

fn build_indexes(graph: &GraphDocument) -> Result<SnapshotIndexes, SnapshotError> {
    let mut indexes = IndexKind::ALL
        .into_iter()
        .map(|index| (index, BTreeMap::new()))
        .collect::<BTreeMap<_, _>>();
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
        &mut indexes,
        IndexKind::Metadata,
        encode_graph_index_key(IndexKind::Metadata, &[])?,
        &metadata,
    )?;
    for file in &graph.graph.files {
        insert_json(
            &mut indexes,
            IndexKind::Metadata,
            encode_graph_index_key(IndexKind::Metadata, &[b"file", file.id.as_bytes()])?,
            file,
        )?;
    }
    for (ordinal, coverage) in graph.graph.coverage.iter().enumerate() {
        let ordinal = format!("{ordinal:08}");
        insert_json(
            &mut indexes,
            IndexKind::Metadata,
            encode_graph_index_key(IndexKind::Metadata, &[b"coverage", ordinal.as_bytes()])?,
            coverage,
        )?;
    }
    for (ordinal, diagnostic) in graph.graph.diagnostics.iter().enumerate() {
        let ordinal = format!("{ordinal:08}");
        insert_json(
            &mut indexes,
            IndexKind::Metadata,
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
    }

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

    for node in &graph.nodes {
        insert_json(
            &mut indexes,
            IndexKind::Nodes,
            encode_graph_index_key(IndexKind::Nodes, &[node.id.as_bytes()])?,
            node,
        )?;
        insert_json(
            &mut indexes,
            IndexKind::Names,
            encode_name_index_key(&node.name, &node.id)?,
            &node.id,
        )?;
        if let Some(anchor) = &node.source {
            insert_anchor_entry(&mut indexes, "node", &node.id, anchor)?;
        }
        for anchor in node.evidence.iter().flat_map(|item| item.anchors.iter()) {
            insert_anchor_entry(&mut indexes, "node", &node.id, anchor)?;
        }
        if node.qualified_name != node.name {
            insert_json(
                &mut indexes,
                IndexKind::Names,
                encode_name_index_key(&node.qualified_name, &node.id)?,
                &node.id,
            )?;
        }
        let mut terms = BTreeSet::new();
        terms.extend(search_terms(&node.name));
        terms.extend(search_terms(&node.qualified_name));
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
        for alias in aliases_by_target
            .get(node.id.as_str())
            .into_iter()
            .flat_map(|aliases| aliases.iter())
        {
            terms.extend(search_terms(alias));
        }
        for term in terms {
            insert_json(
                &mut indexes,
                IndexKind::Terms,
                encode_graph_index_key(
                    IndexKind::Terms,
                    &[term.as_bytes(), b"node", node.id.as_bytes()],
                )?,
                &node.id,
            )?;
            let prefix_length = term.len().min(3);
            let prefix = term.get(..prefix_length).unwrap_or(term.as_str());
            insert_json(
                &mut indexes,
                IndexKind::Terms,
                encode_graph_index_key(
                    IndexKind::Terms,
                    &[
                        prefix.as_bytes(),
                        b"node_prefix",
                        term.as_bytes(),
                        node.id.as_bytes(),
                    ],
                )?,
                &TermPosting {
                    term: term.clone(),
                    node_id: node.id.clone(),
                },
            )?;
        }
        if let Some(community) = &node.community {
            let community_id = community.id.to_string();
            insert_json(
                &mut indexes,
                IndexKind::Communities,
                encode_graph_index_key(
                    IndexKind::Communities,
                    &[community_id.as_bytes(), node.id.as_bytes()],
                )?,
                &node.id,
            )?;
        }
        insert_diagnostics(&mut indexes, &node.id, &node.diagnostics)?;
    }
    insert_diagnostics(&mut indexes, "graph", &graph.graph.diagnostics)?;
    for file in &graph.graph.files {
        insert_json(
            &mut indexes,
            IndexKind::Files,
            encode_graph_index_key(IndexKind::Files, &[b"id", file.id.as_bytes()])?,
            file,
        )?;
        insert_json(
            &mut indexes,
            IndexKind::Files,
            encode_file_path_key(&file.path)?,
            file,
        )?;
    }
    for edge in &graph.links {
        insert_json(
            &mut indexes,
            IndexKind::Edges,
            encode_graph_index_key(IndexKind::Edges, &[edge.id.as_bytes()])?,
            edge,
        )?;
        let kind = edge.kind.as_str().as_bytes();
        insert_json(
            &mut indexes,
            IndexKind::Outgoing,
            encode_graph_index_key(
                IndexKind::Outgoing,
                &[
                    edge.source.as_bytes(),
                    kind,
                    edge.target.as_bytes(),
                    edge.id.as_bytes(),
                ],
            )?,
            &edge.id,
        )?;
        insert_json(
            &mut indexes,
            IndexKind::Incoming,
            encode_graph_index_key(
                IndexKind::Incoming,
                &[
                    edge.target.as_bytes(),
                    kind,
                    edge.source.as_bytes(),
                    edge.id.as_bytes(),
                ],
            )?,
            &edge.id,
        )?;
        if let Some(anchor) = &edge.relationship_site {
            insert_anchor_entry(&mut indexes, "edge", &edge.id, anchor)?;
        }
        for anchor in edge.evidence.iter().flat_map(|item| item.anchors.iter()) {
            insert_anchor_entry(&mut indexes, "edge", &edge.id, anchor)?;
        }
        let mut terms = BTreeSet::new();
        if let Some(context) = &edge.context {
            terms.extend(search_terms(context));
        }
        for term in terms {
            insert_json(
                &mut indexes,
                IndexKind::Terms,
                encode_graph_index_key(IndexKind::Terms, &[term.as_bytes(), edge.id.as_bytes()])?,
                &edge.id,
            )?;
        }
        insert_diagnostics(&mut indexes, &edge.id, &edge.diagnostics)?;
    }
    Ok(indexes)
}

fn insert_anchor_entry(
    indexes: &mut SnapshotIndexes,
    record_kind: &str,
    record_id: &str,
    anchor: &compass_model::provenance::SourceAnchor,
) -> Result<(), SnapshotError> {
    let start_byte = anchor.start_byte.to_be_bytes();
    let end_byte = anchor.end_byte.to_be_bytes();
    let key = encode_graph_index_key(
        IndexKind::Files,
        &[
            b"anchor",
            anchor.file.as_bytes(),
            &start_byte,
            &end_byte,
            record_kind.as_bytes(),
            record_id.as_bytes(),
        ],
    )?;
    insert_json(indexes, IndexKind::Files, key, &record_id.to_owned())
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

fn insert_diagnostics(
    indexes: &mut SnapshotIndexes,
    owner: &str,
    diagnostics: &[GraphDiagnostic],
) -> Result<(), SnapshotError> {
    for (ordinal, diagnostic) in diagnostics.iter().enumerate() {
        let record = DiagnosticRecord {
            owner: owner.to_owned(),
            diagnostic: diagnostic.clone(),
        };
        let ordinal = ordinal.to_string();
        let key = encode_graph_index_key(
            IndexKind::Diagnostics,
            &[
                owner.as_bytes(),
                diagnostic.code.as_bytes(),
                ordinal.as_bytes(),
            ],
        )?;
        insert_json(indexes, IndexKind::Diagnostics, key, &record)?;
    }
    Ok(())
}

fn insert_json<T: Serialize>(
    indexes: &mut SnapshotIndexes,
    index: IndexKind,
    key: Vec<u8>,
    value: &T,
) -> Result<(), SnapshotError> {
    let value = encode_json(value)?;
    let entries = indexes
        .get_mut(&index)
        .ok_or_else(|| SnapshotError::Corrupt(format!("{} index is missing", index.as_str())))?;
    if let Some(previous) = entries.get(&key) {
        if previous != &value {
            return Err(SnapshotError::Corrupt(format!(
                "duplicate {} index key with different values",
                index.as_str()
            )));
        }
        return Ok(());
    }
    entries.insert(key, value);
    Ok(())
}

fn build_index_tree<S: Store + ?Sized>(
    writer: &mut ObjectWriter<'_, S>,
    index: IndexKind,
    entries: &BTreeMap<Vec<u8>, Vec<u8>>,
) -> Result<String, SnapshotError> {
    let mut leaves = Vec::new();
    let mut current = Vec::new();
    let mut current_encoded_bytes = 0_usize;
    for (key, value) in entries {
        let entry = TreeEntry {
            key: key.clone(),
            value: value.clone(),
        };
        let encoded_entry_bytes = rmp_serde::to_vec_named(&entry)
            .map_err(|error| SnapshotError::Encode(error.to_string()))?
            .len();
        if !current.is_empty()
            && (current.len() == GRAPH_SNAPSHOT_MAX_LEAF_ENTRIES
                || current_encoded_bytes
                    .saturating_add(encoded_entry_bytes)
                    .saturating_add(GRAPH_SNAPSHOT_LEAF_OVERHEAD_BYTES)
                    > MAX_VALUE_BYTES)
        {
            let first_key = current
                .first()
                .map(|entry: &TreeEntry| entry.key.clone())
                .ok_or_else(|| SnapshotError::Corrupt("empty leaf".to_owned()))?;
            let object = TreeObject::Leaf {
                schema: GRAPH_SNAPSHOT_LAYOUT_V2.to_owned(),
                index,
                entries: std::mem::take(&mut current),
            };
            let digest = put_tree_object(writer, &object)?;
            leaves.push(TreeChild { first_key, digest });
            current_encoded_bytes = 0;
        }
        current_encoded_bytes = current_encoded_bytes.saturating_add(encoded_entry_bytes);
        current.push(entry);
        if current.len() == 1
            && encode_tree_object(&TreeObject::Leaf {
                schema: GRAPH_SNAPSHOT_LAYOUT_V2.to_owned(),
                index,
                entries: current.clone(),
            })?
            .len()
                > MAX_VALUE_BYTES
        {
            return Err(SnapshotError::Limit(format!(
                "{} index entry exceeds the maximum immutable object size",
                index.as_str()
            )));
        }
    }
    if current.is_empty() {
        let object = TreeObject::Leaf {
            schema: GRAPH_SNAPSHOT_LAYOUT_V2.to_owned(),
            index,
            entries: Vec::new(),
        };
        leaves.push(TreeChild {
            first_key: Vec::new(),
            digest: put_tree_object(writer, &object)?,
        });
    } else {
        let first_key = current
            .first()
            .map(|entry| entry.key.clone())
            .ok_or_else(|| SnapshotError::Corrupt("empty leaf".to_owned()))?;
        let object = TreeObject::Leaf {
            schema: GRAPH_SNAPSHOT_LAYOUT_V2.to_owned(),
            index,
            entries: current,
        };
        leaves.push(TreeChild {
            first_key,
            digest: put_tree_object(writer, &object)?,
        });
    }
    build_branch_levels(writer, index, leaves)
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
            schema: GRAPH_SNAPSHOT_LAYOUT_V2.to_owned(),
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
    let digest = hex_digest(&bytes);
    let key = object_key(&digest)?;
    writer.put(key, bytes)?;
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
    store: &S,
    index: IndexKind,
    digest: &str,
    key: &[u8],
    limits: SnapshotReadLimits,
    depth: usize,
) -> Result<Option<Vec<u8>>, SnapshotError> {
    if depth >= limits.max_depth {
        return Err(SnapshotError::Limit("tree depth limit exceeded".to_owned()));
    }
    let object = load_tree_object(store, index, digest)?;
    match object {
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
                lookup_tree(store, index, &child.digest, key, limits, depth + 1)
            })
        }
    }
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
    store: &S,
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
    match load_tree_object(store, index, digest)? {
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
                state.entries.push(entry);
            }
        }
        TreeObject::Branch { children, .. } => {
            for (child_index, child) in children.iter().enumerate() {
                if let Some(prefix) = prefix
                    && !child_may_match_prefix(&children, child_index, prefix)?
                {
                    continue;
                }
                scan_tree(store, index, &child.digest, prefix, state, depth + 1)?;
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
    let prefix_segments = decode_key_segments(prefix).map_err(SnapshotError::from)?;
    let first = children
        .get(index)
        .ok_or_else(|| SnapshotError::Corrupt("tree child index is missing".to_owned()))?;
    let first_segments = decode_key_segments(&first.first_key).map_err(SnapshotError::from)?;
    let first_cmp = compare_key_prefix(&first_segments, &prefix_segments);
    if first_cmp.is_gt() {
        return Ok(false);
    }
    if first_cmp.is_eq() {
        return Ok(true);
    }
    let Some(next) = children.get(index.saturating_add(1)) else {
        return Ok(true);
    };
    let next_segments = decode_key_segments(&next.first_key).map_err(SnapshotError::from)?;
    Ok(!compare_key_prefix(&next_segments, &prefix_segments).is_lt())
}

fn compare_key_prefix(left: &[Vec<u8>], prefix: &[Vec<u8>]) -> std::cmp::Ordering {
    left.iter()
        .take(prefix.len())
        .map(Vec::as_slice)
        .cmp(prefix.iter().map(Vec::as_slice))
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
            for pair in children.windows(2) {
                if pair[0].first_key >= pair[1].first_key {
                    return Err(SnapshotError::Corrupt(
                        "tree branch separators are not strictly ordered".to_owned(),
                    ));
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
    if schema != GRAPH_SNAPSHOT_LAYOUT_V2 {
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
        .map(str::to_lowercase)
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

fn encode_tree_object(value: &TreeObject) -> Result<Vec<u8>, SnapshotError> {
    rmp_serde::to_vec_named(value).map_err(|error| SnapshotError::Encode(error.to_string()))
}

fn decode_json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, SnapshotError> {
    serde_json::from_slice(bytes).map_err(|error| SnapshotError::Decode(error.to_string()))
}

fn decode_tree_object(bytes: &[u8]) -> Result<TreeObject, SnapshotError> {
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

    #[test]
    fn tree_decoder_accepts_compact_and_legacy_encodings() -> Result<(), SnapshotError> {
        let object = TreeObject::Leaf {
            schema: GRAPH_SNAPSHOT_LAYOUT_V2.to_owned(),
            index: IndexKind::Nodes,
            entries: vec![TreeEntry {
                key: encode_graph_index_key(IndexKind::Nodes, &[b"node-id"])?,
                value: br#"{"id":"node-id"}"#.to_vec(),
            }],
        };

        assert_eq!(decode_tree_object(&encode_tree_object(&object)?)?, object);
        assert_eq!(decode_tree_object(&encode_json(&object)?)?, object);
        Ok(())
    }
}
