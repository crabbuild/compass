#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use compass_cypher::{CompileLimits, CompileRequest, ParameterTypes, Parameters, compile};
use compass_graph::{
    GRAPH_SNAPSHOT_MAX_OBJECTS, GraphSnapshotBuilder, GraphSnapshotReader, canonical_graph_json,
    garbage_collect_graph_snapshots, graph_snapshot_needs_gc,
};
use compass_model::code_graph::{CODE_GRAPH_SCHEMA_V1, EdgeKind, GraphDocument};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
use compass_model::query_contract::{CodeQueryLimits, SearchRequest};
use compass_model::{
    EdgeRecord as LegacyEdgeRecord, Graph, GraphDocument as LegacyGraphDocument,
    NodeRecord as LegacyNodeRecord,
};
use compass_query::{
    EngineSelection, QueryLimits as CompassQlLimits, QueryRequest as CompassQlRequest, execute,
    open_with_engine, open_with_store,
};
use compass_store::{
    Entry, ImmutableBatchOutcome, ImmutableWrite, Key, KeyRange, NamespaceId, PartitionKey,
    ScanCursor, ScanLimits, ScanPage, Store, StoreCapabilities, StoreError, WriteCondition,
};
use compass_store_redb::RedbStore;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const REPORT_SCHEMA: &str = "compass.store.release-qualification/1";
const MAX_NODES: usize = 100_000;

#[derive(Default)]
struct Counters {
    get_requests: AtomicU64,
    scan_requests: AtomicU64,
    put_requests: AtomicU64,
    batch_requests: AtomicU64,
    write_transactions: AtomicU64,
    delete_requests: AtomicU64,
    bytes_read: AtomicU64,
    bytes_written: AtomicU64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CounterSnapshot {
    get_requests: u64,
    scan_requests: u64,
    put_requests: u64,
    batch_requests: u64,
    write_transactions: u64,
    delete_requests: u64,
    bytes_read: u64,
    bytes_written: u64,
}

impl Counters {
    fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            get_requests: self.get_requests.load(Ordering::Relaxed),
            scan_requests: self.scan_requests.load(Ordering::Relaxed),
            put_requests: self.put_requests.load(Ordering::Relaxed),
            batch_requests: self.batch_requests.load(Ordering::Relaxed),
            write_transactions: self.write_transactions.load(Ordering::Relaxed),
            delete_requests: self.delete_requests.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
        }
    }
}

struct CountingStore<S> {
    inner: S,
    counters: Counters,
}

impl<S> CountingStore<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            counters: Counters::default(),
        }
    }
}

impl<S: Store> Store for CountingStore<S> {
    fn capabilities(&self) -> StoreCapabilities {
        self.inner.capabilities()
    }

    fn get(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
    ) -> Result<Option<Entry>, StoreError> {
        self.counters.get_requests.fetch_add(1, Ordering::Relaxed);
        let entry = self.inner.get(namespace, partition, key)?;
        if let Some(entry) = &entry {
            self.counters
                .bytes_read
                .fetch_add(entry.value.len() as u64, Ordering::Relaxed);
        }
        Ok(entry)
    }

    fn scan(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        range: &KeyRange,
        limits: ScanLimits,
        cursor: Option<&ScanCursor>,
    ) -> Result<ScanPage, StoreError> {
        self.counters.scan_requests.fetch_add(1, Ordering::Relaxed);
        let page = self
            .inner
            .scan(namespace, partition, range, limits, cursor)?;
        self.counters
            .bytes_read
            .fetch_add(page.bytes_read as u64, Ordering::Relaxed);
        Ok(page)
    }

    fn scan_keys(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        range: &KeyRange,
        limits: ScanLimits,
        cursor: Option<&ScanCursor>,
    ) -> Result<compass_store::KeyPage, StoreError> {
        self.counters.scan_requests.fetch_add(1, Ordering::Relaxed);
        let page = self
            .inner
            .scan_keys(namespace, partition, range, limits, cursor)?;
        self.counters
            .bytes_read
            .fetch_add(page.bytes_read as u64, Ordering::Relaxed);
        Ok(page)
    }

    fn put(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        value: &[u8],
        condition: WriteCondition,
    ) -> Result<Entry, StoreError> {
        self.counters.put_requests.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes_written
            .fetch_add(value.len() as u64, Ordering::Relaxed);
        let entry = self
            .inner
            .put(namespace, partition, key, value, condition)?;
        self.counters
            .write_transactions
            .fetch_add(1, Ordering::Relaxed);
        Ok(entry)
    }

    fn delete(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        key: &Key,
        condition: WriteCondition,
    ) -> Result<bool, StoreError> {
        self.counters
            .delete_requests
            .fetch_add(1, Ordering::Relaxed);
        self.inner.delete(namespace, partition, key, condition)
    }

    fn delete_batch(
        &self,
        namespace: &NamespaceId,
        partition: &PartitionKey,
        keys: &[Key],
    ) -> Result<u64, StoreError> {
        self.counters
            .delete_requests
            .fetch_add(keys.len() as u64, Ordering::Relaxed);
        let deleted = self.inner.delete_batch(namespace, partition, keys)?;
        if !keys.is_empty() {
            self.counters
                .write_transactions
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(deleted)
    }

    fn put_immutable_batch(
        &self,
        namespace: &NamespaceId,
        writes: &[ImmutableWrite],
    ) -> Result<ImmutableBatchOutcome, StoreError> {
        self.counters.batch_requests.fetch_add(1, Ordering::Relaxed);
        self.counters
            .put_requests
            .fetch_add(writes.len() as u64, Ordering::Relaxed);
        let outcome = self.inner.put_immutable_batch(namespace, writes)?;
        self.counters
            .write_transactions
            .fetch_add(outcome.transactions, Ordering::Relaxed);
        self.counters
            .bytes_written
            .fetch_add(outcome.bytes_written, Ordering::Relaxed);
        Ok(outcome)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseReport {
    schema: &'static str,
    adapter: &'static str,
    nodes: usize,
    edges: usize,
    graph_bytes: usize,
    graph_digest: String,
    snapshot_id: String,
    manifest_digest: String,
    canonical_json_equal: bool,
    compassql_equal: bool,
    build_seconds: f64,
    query_seconds: f64,
    database_bytes: u64,
    write_amplification: f64,
    build_requests: CounterSnapshot,
    query_requests: CounterSnapshot,
    query_results: usize,
    new_objects: u64,
    reused_objects: u64,
    gc: Value,
}

fn main() -> Result<(), String> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let adapter = option(&arguments, "--adapter").unwrap_or("sqlite");
    let nodes = option(&arguments, "--nodes")
        .unwrap_or("128")
        .parse::<usize>()
        .map_err(|_| "--nodes must be a positive integer".to_owned())?;
    if nodes == 0 || nodes > MAX_NODES {
        return Err(format!("--nodes must be between 1 and {MAX_NODES}"));
    }
    let output = option(&arguments, "--output").map(PathBuf::from);
    let report = match adapter {
        "sqlite" => run_sqlite(nodes)?,
        "redb" => run_redb(nodes)?,
        value => return Err(format!("--adapter must be sqlite or redb (found {value})")),
    };
    let encoded = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(output, encoded).map_err(|error| error.to_string())?;
    } else {
        println!("{}", String::from_utf8_lossy(&encoded));
    }
    Ok(())
}

fn run_sqlite(nodes: usize) -> Result<ReleaseReport, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let path = directory.path().join("compass-store.sqlite3");
    let store = compass_store::SqliteStore::open(&path).map_err(|error| error.to_string())?;
    run_with_store("sqlite", nodes, directory, path, store)
}

fn run_redb(nodes: usize) -> Result<ReleaseReport, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let path = directory.path().join(compass_store_redb::REDB_FILE_NAME);
    let store = RedbStore::open(&path).map_err(|error| error.to_string())?;
    run_with_store("redb", nodes, directory, path, store)
}

fn run_with_store<S: Store>(
    adapter: &'static str,
    nodes: usize,
    directory: TempDir,
    store_path: PathBuf,
    store: S,
) -> Result<ReleaseReport, String> {
    let graph = graph(nodes)?;
    let canonical = canonical_graph_json(&graph).map_err(|error| error.to_string())?;
    let graph_path = directory.path().join("graph.json");
    fs::write(&graph_path, &canonical).map_err(|error| error.to_string())?;
    let counted = CountingStore::new(store);
    let build_started = Instant::now();
    let prepared = GraphSnapshotBuilder::new()
        .prepare(&counted, &graph)
        .map_err(|error| format!("prepare snapshot: {error}"))?;
    let active_selector = GraphSnapshotBuilder::new()
        .activate(&counted, &prepared)
        .map_err(|error| format!("activate snapshot: {error}"))?;
    let build_seconds = build_started.elapsed().as_secs_f64();
    let build_requests = counted.counters.snapshot();
    let reader = GraphSnapshotReader::open_active(&counted)
        .map_err(|error| format!("open active snapshot: {error}"))?
        .ok_or_else(|| "active snapshot is missing".to_owned())?;
    let exported = reader
        .export_json_bytes()
        .map_err(|error| format!("export active snapshot: {error}"))?;
    if exported != canonical {
        return Err("store export differs from canonical graph JSON".to_owned());
    }

    let query_started = Instant::now();
    let store_engine = open_with_store(
        &counted,
        &graph_path,
        None,
        &directory.path().join("store-query-cache"),
    )
    .map_err(|error| format!("open store query engine: {error}"))?;
    let json_engine = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("json-query-cache"),
        EngineSelection::Json,
    )
    .map_err(|error| format!("open JSON query engine: {error}"))?;
    let request = SearchRequest {
        query: "symbol".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    let store_response = store_engine
        .search(request.clone())
        .map_err(|error| format!("execute store search: {error}"))?;
    let json_response = json_engine
        .search(request)
        .map_err(|error| format!("execute JSON search: {error}"))?;
    let canonical_results = serde_json::to_value(&store_response)
        .map_err(|error| error.to_string())?
        == serde_json::to_value(&json_response).map_err(|error| error.to_string())?;
    let store_document =
        serde_json::from_slice::<GraphDocument>(&exported).map_err(|error| error.to_string())?;
    let store_graph = compassql_graph(&store_document)?;
    let json_graph = compassql_graph(&graph)?;
    let compassql_equal = run_compassql(&store_graph, &json_graph)?;
    let query_seconds = query_started.elapsed().as_secs_f64();
    let query_requests = subtract(build_requests.clone(), counted.counters.snapshot());
    drop(store_engine);
    drop(json_engine);

    let mut orphan_graph = graph.clone();
    orphan_graph
        .graph
        .build
        .generation_id
        .push_str("-unselected");
    GraphSnapshotBuilder::new()
        .prepare(&counted, &orphan_graph)
        .map_err(|error| format!("prepare unselected snapshot for GC: {error}"))?;
    if !graph_snapshot_needs_gc(&counted, 1)
        .map_err(|error| format!("discover snapshots before GC: {error}"))?
    {
        return Err("unselected snapshot was not discoverable before GC".to_owned());
    }
    let gc = garbage_collect_graph_snapshots(
        &counted,
        std::slice::from_ref(&active_selector),
        GRAPH_SNAPSHOT_MAX_OBJECTS,
    )
    .map_err(|error| format!("collect unselected snapshot: {error}"))?;
    if gc.deleted_entries == 0
        || graph_snapshot_needs_gc(&counted, 1)
            .map_err(|error| format!("discover snapshots after GC: {error}"))?
    {
        return Err("snapshot GC did not remove the unselected realization".to_owned());
    }
    drop(counted);
    let database_bytes = fs::metadata(&store_path)
        .map_err(|error| error.to_string())?
        .len();
    let write_amplification = database_bytes as f64 / canonical.len() as f64;
    Ok(ReleaseReport {
        schema: REPORT_SCHEMA,
        adapter,
        nodes: graph.nodes.len(),
        edges: graph.links.len(),
        graph_bytes: canonical.len(),
        graph_digest: digest(&canonical),
        snapshot_id: prepared.manifest.snapshot_id,
        manifest_digest: prepared.manifest_digest,
        canonical_json_equal: canonical_results,
        compassql_equal,
        build_seconds,
        query_seconds,
        database_bytes,
        write_amplification,
        build_requests,
        query_requests,
        query_results: store_response.results.len(),
        new_objects: prepared.new_objects,
        reused_objects: prepared.reused_objects,
        gc: json!({
            "mode": "bounded-mark-sweep",
            "executed": true,
            "supported": true,
            "retainedManifests": gc.retained_manifests,
            "retainedObjects": gc.retained_objects,
            "deletedEntries": gc.deleted_entries,
            "deleteTransactions": gc.delete_transactions,
        }),
    })
}

fn compassql_graph(document: &GraphDocument) -> Result<Graph, String> {
    let nodes = document
        .nodes
        .iter()
        .map(|node| {
            let mut attributes = serde_json::Map::new();
            attributes.insert("label".to_owned(), Value::String(node.name.clone()));
            attributes.insert(
                "kind".to_owned(),
                Value::String(node.kind.as_str().to_owned()),
            );
            attributes.insert(
                "qualified_name".to_owned(),
                Value::String(node.qualified_name.clone()),
            );
            attributes.insert("file_type".to_owned(), Value::String("function".to_owned()));
            LegacyNodeRecord {
                id: node.id.clone(),
                attributes,
            }
        })
        .collect();
    let links = document
        .links
        .iter()
        .map(|edge| {
            let mut attributes = serde_json::Map::new();
            attributes.insert(
                "relation".to_owned(),
                Value::String(edge.kind.as_str().to_owned()),
            );
            attributes.insert(
                "kind".to_owned(),
                Value::String(edge.kind.as_str().to_owned()),
            );
            attributes.insert(
                "confidence".to_owned(),
                Value::String("EXTRACTED".to_owned()),
            );
            LegacyEdgeRecord {
                source: edge.source.clone(),
                target: edge.target.clone(),
                attributes,
            }
        })
        .collect();
    Graph::from_document(LegacyGraphDocument {
        directed: document.directed,
        multigraph: document.multigraph,
        graph: serde_json::Map::new(),
        nodes,
        links,
        extras: BTreeMap::new(),
    })
    .map_err(|error| error.to_string())
}

fn run_compassql(store_graph: &Graph, json_graph: &Graph) -> Result<bool, String> {
    let source = "MATCH (n:Function) RETURN n.id AS id ORDER BY id";
    let parameter_types = ParameterTypes::new();
    let store_schema = store_graph.schema_fingerprint();
    let compiled = compile(CompileRequest {
        source_name: "store-release-qualification.cypher",
        source,
        parameter_types: &parameter_types,
        schema: &store_schema,
        limits: CompileLimits::default(),
    })
    .map_err(|error| error.to_string())?;
    let parameters = Parameters::new();
    let store_result = execute(CompassQlRequest {
        compiled: &compiled,
        graph: store_graph,
        parameters: &parameters,
        limits: CompassQlLimits::interactive(),
        cancellation: &AtomicBool::new(false),
    })
    .map_err(|error| error.to_string())?;
    let json_result = execute(CompassQlRequest {
        compiled: &compiled,
        graph: json_graph,
        parameters: &parameters,
        limits: CompassQlLimits::interactive(),
        cancellation: &AtomicBool::new(false),
    })
    .map_err(|error| error.to_string())?;
    Ok(
        serde_json::to_value(store_result).map_err(|error| error.to_string())?
            == serde_json::to_value(json_result).map_err(|error| error.to_string())?,
    )
}

fn subtract(before: CounterSnapshot, after: CounterSnapshot) -> CounterSnapshot {
    CounterSnapshot {
        get_requests: after.get_requests.saturating_sub(before.get_requests),
        scan_requests: after.scan_requests.saturating_sub(before.scan_requests),
        put_requests: after.put_requests.saturating_sub(before.put_requests),
        batch_requests: after.batch_requests.saturating_sub(before.batch_requests),
        write_transactions: after
            .write_transactions
            .saturating_sub(before.write_transactions),
        delete_requests: after.delete_requests.saturating_sub(before.delete_requests),
        bytes_read: after.bytes_read.saturating_sub(before.bytes_read),
        bytes_written: after.bytes_written.saturating_sub(before.bytes_written),
    }
}

fn graph(nodes: usize) -> Result<GraphDocument, String> {
    let anchor = SourceAnchor {
        file: "src/release_qualification.rs".to_owned(),
        start_byte: 0,
        end_byte: 1,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 1,
    };
    let evidence = Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "compass.store.release-qualification".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: None,
        anchors: vec![anchor.clone()],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    };
    let evidence = serde_json::to_value(&evidence).map_err(|error| error.to_string())?;
    let anchor_value = serde_json::to_value(&anchor).map_err(|error| error.to_string())?;
    let mut node_values = Vec::with_capacity(nodes);
    for index in 0..nodes {
        node_values.push(json!({
            "id": format!("node-{index:08}"),
            "kind": "function",
            "name": format!("symbol_{index}"),
            "qualifiedName": format!("symbol::{index}"),
            "source": anchor_value.clone(),
            "evidence": [evidence.clone()],
            "coverage": [],
            "diagnostics": [],
        }));
    }
    let mut edge_values = Vec::with_capacity(nodes.saturating_sub(1));
    for index in 1..nodes {
        let source = format!("node-{index_minus_one:08}", index_minus_one = index - 1);
        let target = format!("node-{index:08}");
        let id = edge_id(&source, EdgeKind::Calls, &target, Some(&anchor), None);
        edge_values.push(json!({
            "id": id.clone(),
            "key": id,
            "source": source,
            "target": target,
            "kind": "calls",
            "relationshipSite": anchor_value.clone(),
            "evidence": [evidence.clone()],
            "diagnostics": [],
        }));
    }
    let value = json!({
        "directed": true,
        "multigraph": true,
        "graph": {
            "schema": CODE_GRAPH_SCHEMA_V1,
            "build": {
                "builderVersion": "release-qualification",
                "schemaFingerprint": "release-qualification",
                "sourceTreeDigest": "release-qualification",
                "configurationDigest": "release-qualification",
                "generationId": "release-qualification",
            },
            "files": [{
                "id": file_id("src/release_qualification.rs"),
                "path": "src/release_qualification.rs",
                "language": "rust",
                "contentDigest": "sha256:release-qualification",
                "byteSize": 1,
                "generated": false,
                "extractionStatus": "extracted",
                "extractorVersions": ["compass.store.release-qualification"]
            }],
            "coverage": [],
            "diagnostics": [],
        },
        "nodes": node_values,
        "links": edge_values,
    });
    serde_json::from_value(value).map_err(|error| format!("qualification graph: {error}"))
}

fn option<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments.iter().enumerate().find_map(|(index, argument)| {
        if argument == name {
            arguments.get(index + 1).map(String::as_str)
        } else {
            argument
                .strip_prefix(name)
                .and_then(|value| value.strip_prefix('='))
        }
    })
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
