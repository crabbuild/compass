use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use compass_graph::SnapshotSelector;
use compass_ir::{PROGRAM_SCHEMA, ProgramBundle};
use compass_model::code_graph::GraphDocument;
use compass_model::query_contract::CODE_QUERY_SCHEMA_V1;
use compass_model::search::{
    direct_call_source_identifier_postings, direct_call_source_identifier_targets,
    identifier_search_terms,
};
use compass_store::Store;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::CodeQueryEngine;
use crate::code_query::CodeGraphBackend;
use crate::cql::{QueryError, QueryErrorKind};
use crate::graph_engine::{
    DirectGraphEngine, StoreGraphEngine, open_graph_engine, open_local_store_snapshot,
    read_store_ref,
};

const INDEX_FORMAT_VERSION: &str = "compass-code-index/1";

/// Selects the source used to hydrate the typed query engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineSelection {
    /// Read the compatible `graph.json` artifact. This is the public default.
    Default,
    /// Read and validate the compatible `graph.json` artifact directly.
    Json,
    /// Require the store sidecar and fail if it is unavailable or corrupt.
    Store,
}

/// Identifies the engine that supplied a query engine's graph snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryEngineKind {
    Json,
    Store,
    Memory,
}

pub const DEFAULT_QUERY_ENGINE_CACHE_CAPACITY: usize = 8;
pub const MAX_QUERY_ENGINE_CACHE_CAPACITY: usize = 32;
pub type CachedQueryEngine = Arc<Mutex<CodeQueryEngine>>;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum QueryEngineIdentity {
    PublishedStore {
        store_id: String,
        snapshot_id: String,
        manifest_digest: String,
        graph_digest: String,
    },
    VerifiedDocument {
        graph_identity: String,
    },
}

struct QueryEngineCacheEntry {
    identity: QueryEngineIdentity,
    engine: CachedQueryEngine,
}

/// Bounded LRU of long-lived query engines keyed by exact graph identity.
pub struct QueryEngineCache {
    capacity: usize,
    state: Mutex<QueryEngineCacheState>,
}

#[derive(Default)]
struct QueryEngineCacheState {
    entries: BTreeMap<PathBuf, QueryEngineCacheEntry>,
    order: VecDeque<PathBuf>,
}

impl Default for QueryEngineCache {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_QUERY_ENGINE_CACHE_CAPACITY,
            state: Mutex::new(QueryEngineCacheState::default()),
        }
    }
}

impl QueryEngineCache {
    pub fn new(capacity: usize) -> Result<Self, QueryError> {
        if capacity == 0 || capacity > MAX_QUERY_ENGINE_CACHE_CAPACITY {
            return Err(QueryError::new(
                QueryErrorKind::InvalidParameter,
                "invalid_query_engine_cache_capacity",
                format!(
                    "query engine cache capacity must be between 1 and {MAX_QUERY_ENGINE_CACHE_CAPACITY}"
                ),
            ));
        }
        Ok(Self {
            capacity,
            state: Mutex::new(QueryEngineCacheState::default()),
        })
    }

    pub fn open_published_store(&self, graph_path: &Path) -> Result<CachedQueryEngine, QueryError> {
        let path = fs::canonicalize(graph_path).map_err(|error| {
            QueryError::new(
                QueryErrorKind::Internal,
                "canonicalize_store_graph_failed",
                error.to_string(),
            )
        })?;
        let reference = read_store_ref(&path)?;
        let identity = QueryEngineIdentity::PublishedStore {
            store_id: reference.store_id,
            snapshot_id: reference.snapshot_id,
            manifest_digest: reference.manifest_digest,
            graph_digest: reference.graph_digest,
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(engine) = state.get(&path, &identity) {
            return Ok(engine);
        }

        let cache_root = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("cache");
        let engine = Arc::new(Mutex::new(open_with_engine(
            &path,
            None,
            &cache_root,
            EngineSelection::Store,
        )?));
        Ok(state.insert(self.capacity, path, identity, engine))
    }

    pub fn open_verified_document(
        &self,
        document: &GraphDocument,
        graph_identity: &str,
        graph_path: &Path,
        cache_root: &Path,
    ) -> Result<CachedQueryEngine, QueryError> {
        let path = fs::canonicalize(graph_path).map_err(|error| {
            QueryError::new(
                QueryErrorKind::Internal,
                "canonicalize_verified_graph_failed",
                error.to_string(),
            )
        })?;
        let identity = QueryEngineIdentity::VerifiedDocument {
            graph_identity: graph_identity.to_owned(),
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(engine) = state.get(&path, &identity) {
            return Ok(engine);
        }
        let engine = Arc::new(Mutex::new(open_with_verified_document(
            document.clone(),
            graph_identity.to_owned(),
            &path,
            None,
            cache_root,
        )?));
        Ok(state.insert(self.capacity, path, identity, engine))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl QueryEngineCacheState {
    fn get(&mut self, path: &Path, identity: &QueryEngineIdentity) -> Option<CachedQueryEngine> {
        let engine = self
            .entries
            .get(path)
            .filter(|entry| &entry.identity == identity)
            .map(|entry| Arc::clone(&entry.engine));
        if engine.is_some() {
            self.order.retain(|cached| cached != path);
            self.order.push_back(path.to_path_buf());
        }
        engine
    }

    fn insert(
        &mut self,
        capacity: usize,
        path: PathBuf,
        identity: QueryEngineIdentity,
        engine: CachedQueryEngine,
    ) -> CachedQueryEngine {
        self.order.retain(|cached| cached != &path);
        while self.entries.len() >= capacity && !self.entries.contains_key(&path) {
            let Some(expired) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&expired);
        }
        self.order.push_back(path.clone());
        self.entries.insert(
            path,
            QueryEngineCacheEntry {
                identity,
                engine: Arc::clone(&engine),
            },
        );
        engine
    }
}

#[must_use]
pub fn has_published_store(graph_path: &Path) -> bool {
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(compass_store::STORE_REF_FILE_NAME)
        .is_file()
}

pub fn open(
    graph_path: &Path,
    program_path: Option<&Path>,
    cache_root: &Path,
) -> Result<CodeQueryEngine, QueryError> {
    open_with_engine(
        graph_path,
        program_path,
        cache_root,
        EngineSelection::Default,
    )
}

pub fn open_with_engine(
    graph_path: &Path,
    program_path: Option<&Path>,
    cache_root: &Path,
    selection: EngineSelection,
) -> Result<CodeQueryEngine, QueryError> {
    // A published store is the bounded default for large graphs.  Keep the
    // fallback to JSON for older/output-only builds, but never fall back after
    // a store reference is present: a corrupt or mismatched sidecar must fail
    // closed instead of silently querying a different realization.
    if selection == EngineSelection::Default && has_published_store(graph_path) {
        return open_from_local_store(graph_path, program_path);
    }
    if selection == EngineSelection::Store {
        return open_from_local_store(graph_path, program_path);
    }
    let graph_engine = open_graph_engine(graph_path, selection)?;
    open_from_graph_engine(graph_path, program_path, cache_root, graph_engine)
}

/// Hydrate the typed query engine from any common-contract store adapter.
///
/// This is the backend-neutral hook used by adapter conformance and future
/// service integrations. It does not add a backend to the CLI binary.
pub fn open_with_store<S: Store + ?Sized>(
    store: &S,
    graph_path: &Path,
    program_path: Option<&Path>,
    cache_root: &Path,
) -> Result<CodeQueryEngine, QueryError> {
    let graph_engine = Box::new(crate::graph_engine::StoreGraphEngine::from_store(store)?);
    open_from_graph_engine(graph_path, program_path, cache_root, graph_engine)
}

/// Hydrate the typed query engine from one exact immutable store selector.
/// The active selector and JSON artifacts are never consulted.
pub fn open_with_store_selector<S: Store + ?Sized>(
    store: &S,
    selector: SnapshotSelector,
    graph_path: &Path,
    program_path: Option<&Path>,
    cache_root: &Path,
) -> Result<CodeQueryEngine, QueryError> {
    let graph_engine = Box::new(StoreGraphEngine::from_store_selector(store, selector)?);
    open_from_graph_engine(graph_path, program_path, cache_root, graph_engine)
}

/// Hydrate a long-lived typed query engine from a validated in-memory graph.
/// Its materialized index and caches are keyed by the canonical graph identity.
pub fn open_with_document(
    graph: GraphDocument,
    graph_path: &Path,
    program_path: Option<&Path>,
    cache_root: &Path,
) -> Result<CodeQueryEngine, QueryError> {
    let graph_engine = Box::new(DirectGraphEngine::from_document(graph)?);
    open_from_graph_engine(graph_path, program_path, cache_root, graph_engine)
}

/// Hydrate a typed engine from a graph whose immutable source has already
/// verified its content identity.
pub fn open_with_verified_document(
    graph: GraphDocument,
    graph_identity: String,
    graph_path: &Path,
    program_path: Option<&Path>,
    cache_root: &Path,
) -> Result<CodeQueryEngine, QueryError> {
    let graph_engine = Box::new(DirectGraphEngine::from_verified_document(
        graph,
        graph_identity,
    )?);
    open_from_graph_engine(graph_path, program_path, cache_root, graph_engine)
}

fn open_from_graph_engine(
    graph_path: &Path,
    program_path: Option<&Path>,
    cache_root: &Path,
    graph_engine: Box<dyn crate::graph_engine::GraphEngine>,
) -> Result<CodeQueryEngine, QueryError> {
    let graph = graph_engine.graph().clone();
    let graph_identity = graph_engine.graph_identity().to_owned();
    let build_generation_identity = graph.graph.build.generation_id.clone();
    let engine_kind = graph_engine.kind();
    let (program, program_digest) = load_program(program_path)?;
    let key = index_key(
        &graph_identity,
        program_digest.as_deref(),
        &graph.graph.build.schema_fingerprint,
    );
    let index_dir = cache_root.join("code-query").join(&key);
    fs::create_dir_all(&index_dir).map_err(|error| io_error("create_index_dir", error))?;
    let index_path = index_dir.join("index.sqlite3");
    if index_path.exists() && !valid_index(&index_path, &key) {
        fs::remove_file(&index_path).map_err(|error| io_error("remove_invalid_index", error))?;
    }
    if !valid_index(&index_path, &key) {
        build_with_lock(&index_path, &key, &graph, program.as_ref())?;
    }
    let connection = Connection::open_with_flags(&index_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(sql_error)?;
    if !valid_connection(&connection, &key) {
        drop(connection);
        if index_path.exists() {
            fs::remove_file(&index_path)
                .map_err(|error| io_error("remove_corrupt_index", error))?;
        }
        build_with_lock(&index_path, &key, &graph, program.as_ref())?;
    }
    let connection = Connection::open_with_flags(&index_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(sql_error)?;
    let adjacency = crate::code_query::CodeAdjacencyIndex::build(&graph);
    let lookup = crate::code_query::CodeLookupIndex::build(&graph);
    let partial_graph_message = graph
        .graph
        .diagnostics
        .iter()
        .rfind(|diagnostic| diagnostic.code == "publication_omission_summary")
        .map(|diagnostic| {
            format!(
                "Published graph coverage is incomplete: {}",
                diagnostic.message
            )
        });
    Ok(CodeQueryEngine {
        backend: CodeGraphBackend::Materialized {
            graph: Box::new(graph),
            adjacency: Box::new(adjacency),
            lookup: Box::new(lookup),
        },
        program,
        connection: Some(connection),
        graph_path: graph_path.to_path_buf(),
        index_path,
        partial_graph_message,
        engine_kind,
        graph_identity,
        build_generation_identity,
        search_query_cache: std::sync::Mutex::new(Default::default()),
        fuzzy_lookup_cache: std::sync::Mutex::new(Default::default()),
    })
}

fn open_from_local_store(
    graph_path: &Path,
    program_path: Option<&Path>,
) -> Result<CodeQueryEngine, QueryError> {
    let snapshot = open_local_store_snapshot(graph_path)?;
    let graph_identity = snapshot.graph_identity.clone();
    let build_generation_identity = snapshot.build_generation_identity.clone();
    let partial_graph_message = snapshot.partial_graph_message.clone();
    let (program, _) = load_program(program_path)?;
    let index_path = snapshot.store_path.clone();
    Ok(CodeQueryEngine {
        backend: CodeGraphBackend::Store(Box::new(snapshot)),
        program,
        connection: None,
        graph_path: graph_path.to_path_buf(),
        index_path,
        partial_graph_message,
        engine_kind: QueryEngineKind::Store,
        graph_identity,
        build_generation_identity,
        search_query_cache: std::sync::Mutex::new(Default::default()),
        fuzzy_lookup_cache: std::sync::Mutex::new(Default::default()),
    })
}

fn load_program(
    path: Option<&Path>,
) -> Result<(Option<ProgramBundle>, Option<String>), QueryError> {
    let Some(path) = path else {
        return Ok((None, None));
    };
    let bytes = fs::read(path).map_err(|error| io_error("read_program", error))?;
    let program = serde_json::from_slice::<ProgramBundle>(&bytes).map_err(|error| {
        QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "program_load_failed",
            error.to_string(),
        )
    })?;
    if program.schema != PROGRAM_SCHEMA {
        return Err(QueryError::new(
            QueryErrorKind::UnsupportedSchema,
            "unsupported_program_schema",
            format!("expected {PROGRAM_SCHEMA}, found {}", program.schema),
        ));
    }
    Ok((Some(program), Some(hex_digest(&bytes))))
}

fn index_key(graph_identity: &str, program_digest: Option<&str>, graph_schema: &str) -> String {
    let mut digest = Sha256::new();
    for value in [
        graph_identity.to_owned(),
        program_digest.unwrap_or("none").to_owned(),
        graph_schema.to_owned(),
        CODE_QUERY_SCHEMA_V1.to_owned(),
        INDEX_FORMAT_VERSION.to_owned(),
    ] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn build_with_lock(
    index_path: &Path,
    key: &str,
    graph: &GraphDocument,
    program: Option<&ProgramBundle>,
) -> Result<(), QueryError> {
    let lock_path = index_path.with_extension("lock");
    let mut lock = None;
    for _ in 0..100 {
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(file) => {
                lock = Some(file);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if valid_index(index_path, key) {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(io_error("create_index_lock", error)),
        }
    }
    let Some(mut lock) = lock else {
        return Err(QueryError::new(
            QueryErrorKind::Internal,
            "query_index_lock_timeout",
            "timed out waiting for query index builder",
        ));
    };
    let result = build_index(index_path, key, graph, program);
    let _ = lock.flush();
    drop(lock);
    let _ = fs::remove_file(lock_path);
    result
}

fn build_index(
    index_path: &Path,
    key: &str,
    graph: &GraphDocument,
    program: Option<&ProgramBundle>,
) -> Result<(), QueryError> {
    let sequence = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = index_path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let mut connection = Connection::open(&temporary).map_err(sql_error)?;
    connection
        .execute_batch(
            r#"PRAGMA journal_mode=DELETE;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE nodes(
               id TEXT PRIMARY KEY, name TEXT NOT NULL, qualified_name TEXT NOT NULL,
               kind TEXT NOT NULL, roles TEXT NOT NULL, language TEXT NOT NULL,
               framework TEXT NOT NULL, normalized_path TEXT NOT NULL,
               source_file TEXT NOT NULL, community_id TEXT NOT NULL,
               community_label TEXT NOT NULL, json TEXT NOT NULL
             );
             CREATE TABLE edges(id TEXT PRIMARY KEY, source TEXT NOT NULL, target TEXT NOT NULL, kind TEXT NOT NULL, json TEXT NOT NULL);
             CREATE TABLE files(path TEXT PRIMARY KEY, digest TEXT NOT NULL, json TEXT NOT NULL);
             CREATE TABLE evidence(owner_type TEXT NOT NULL, owner_id TEXT NOT NULL, position INTEGER NOT NULL, json TEXT NOT NULL);
             CREATE TABLE aliases(node_id TEXT NOT NULL, alias TEXT NOT NULL);
             CREATE TABLE relationship_terms(
               term TEXT NOT NULL, source_id TEXT NOT NULL,
               PRIMARY KEY(term, source_id)
             ) WITHOUT ROWID;
             CREATE TABLE relationship_term_targets(
               term TEXT NOT NULL, source_id TEXT NOT NULL, target_id TEXT NOT NULL,
               PRIMARY KEY(source_id, term, target_id)
             ) WITHOUT ROWID;
             CREATE TABLE program_joins(graph_node_id TEXT NOT NULL, symbol_id TEXT NOT NULL, json TEXT NOT NULL);
             CREATE VIRTUAL TABLE node_fts USING fts5(
               node_id UNINDEXED, name, qualified_name, aliases, kind, roles,
               language, framework, normalized_path, source_file, community_id,
               community_label, identifier_terms,
               tokenize="unicode61 remove_diacritics 2 tokenchars '_'"
             );"#,
        )
        .map_err(sql_error)?;
    {
        let transaction = connection.transaction().map_err(sql_error)?;
        transaction
            .execute(
                "INSERT INTO metadata(key,value) VALUES('format',?1),('key',?2),('complete','0')",
                params![INDEX_FORMAT_VERSION, key],
            )
            .map_err(sql_error)?;
        let node_by_id = graph
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect::<HashMap<_, _>>();
        let mut aliases_by_target = HashMap::<&str, Vec<&str>>::new();
        for edge in &graph.links {
            if edge.kind == compass_model::code_graph::EdgeKind::Aliases
                && let Some(alias) = node_by_id.get(edge.source.as_str())
            {
                aliases_by_target
                    .entry(edge.target.as_str())
                    .or_default()
                    .push(alias.name.as_str());
            }
        }
        for node in &graph.nodes {
            let roles = node
                .roles
                .iter()
                .map(|role| format!("{role:?}").to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let normalized_path = node
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
                .unwrap_or_default();
            let aliases = [node.name.as_str(), node.qualified_name.as_str()]
                .into_iter()
                .chain(
                    aliases_by_target
                        .get(node.id.as_str())
                        .into_iter()
                        .flatten()
                        .copied(),
                )
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let identifier_terms = [node.name.as_str(), node.qualified_name.as_str()]
                .into_iter()
                .chain(
                    aliases_by_target
                        .get(node.id.as_str())
                        .into_iter()
                        .flatten()
                        .copied(),
                )
                .flat_map(identifier_search_terms)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" ");
            for alias in aliases_by_target
                .get(node.id.as_str())
                .into_iter()
                .flatten()
            {
                transaction
                    .execute("INSERT INTO aliases VALUES(?1,?2)", params![node.id, alias])
                    .map_err(sql_error)?;
            }
            transaction
                .execute(
                    "INSERT INTO nodes VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![
                        node.id,
                        node.name,
                        node.qualified_name,
                        node.kind.as_str(),
                        roles,
                        node.language.as_deref().unwrap_or_default(),
                        node.framework.as_deref().unwrap_or_default(),
                        normalized_path,
                        node.source
                            .as_ref()
                            .map_or("", |source| source.file.as_str()),
                        node.community
                            .as_ref()
                            .map_or_else(String::new, |community| community.id.to_string()),
                        node.community
                            .as_ref()
                            .and_then(|community| community.label.as_deref())
                            .unwrap_or_default(),
                        serde_json::to_string(node).map_err(json_error)?,
                    ],
                )
                .map_err(sql_error)?;
            transaction
                .execute(
                    "INSERT INTO node_fts VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                    params![
                        node.id,
                        node.name,
                        node.qualified_name,
                        aliases,
                        node.kind.as_str(),
                        roles,
                        node.language.as_deref().unwrap_or_default(),
                        node.framework.as_deref().unwrap_or_default(),
                        normalized_path,
                        node.source
                            .as_ref()
                            .map_or("", |source| source.file.as_str()),
                        node.community
                            .as_ref()
                            .map_or_else(String::new, |community| community.id.to_string()),
                        node.community
                            .as_ref()
                            .and_then(|community| community.label.as_deref())
                            .unwrap_or_default(),
                        identifier_terms,
                    ],
                )
                .map_err(sql_error)?;
            for (position, evidence) in node.evidence.iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO evidence VALUES('node',?1,?2,?3)",
                        params![
                            node.id,
                            position as i64,
                            serde_json::to_string(evidence).map_err(json_error)?
                        ],
                    )
                    .map_err(sql_error)?;
            }
        }
        for (term, source_ids) in direct_call_source_identifier_postings(graph) {
            for source_id in source_ids {
                transaction
                    .execute(
                        "INSERT INTO relationship_terms VALUES(?1,?2)",
                        params![term, source_id],
                    )
                    .map_err(sql_error)?;
            }
        }
        for (term, source_id, target_id) in direct_call_source_identifier_targets(graph) {
            transaction
                .execute(
                    "INSERT INTO relationship_term_targets VALUES(?1,?2,?3)",
                    params![term, source_id, target_id],
                )
                .map_err(sql_error)?;
        }
        for edge in &graph.links {
            transaction
                .execute(
                    "INSERT INTO edges VALUES(?1,?2,?3,?4,?5)",
                    params![
                        edge.id,
                        edge.source,
                        edge.target,
                        edge.kind.as_str(),
                        serde_json::to_string(edge).map_err(json_error)?
                    ],
                )
                .map_err(sql_error)?;
            for (position, evidence) in edge.evidence.iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO evidence VALUES('edge',?1,?2,?3)",
                        params![
                            edge.id,
                            position as i64,
                            serde_json::to_string(evidence).map_err(json_error)?
                        ],
                    )
                    .map_err(sql_error)?;
            }
        }
        for file in &graph.graph.files {
            transaction
                .execute(
                    "INSERT INTO files VALUES(?1,?2,?3)",
                    params![
                        file.path,
                        file.content_digest,
                        serde_json::to_string(file).map_err(json_error)?
                    ],
                )
                .map_err(sql_error)?;
        }
        if let Some(program) = program {
            for function in program.modules.iter().flat_map(|module| &module.functions) {
                if let Some(graph_node_id) = &function.graph_node_id {
                    transaction
                        .execute(
                            "INSERT INTO program_joins VALUES(?1,?2,?3)",
                            params![
                                graph_node_id,
                                function.symbol_id,
                                serde_json::to_string(function).map_err(json_error)?
                            ],
                        )
                        .map_err(sql_error)?;
                }
            }
        }
        transaction
            .execute("UPDATE metadata SET value='1' WHERE key='complete'", [])
            .map_err(sql_error)?;
        transaction.commit().map_err(sql_error)?;
    }
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(sql_error)?;
    if integrity != "ok" {
        return Err(QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "query_index_integrity",
            integrity,
        ));
    }
    drop(connection);
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error("sync_index", error))?;
    fs::rename(&temporary, index_path).map_err(|error| io_error("publish_index", error))?;
    if let Some(parent) = index_path.parent() {
        let _ = File::open(parent).and_then(|file| file.sync_all());
    }
    Ok(())
}

fn valid_index(path: &Path, key: &str) -> bool {
    if !path.is_file() {
        return false;
    }
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .ok()
        .is_some_and(|connection| valid_connection(&connection, key))
}

fn valid_connection(connection: &Connection, key: &str) -> bool {
    let metadata = connection
        .query_row(
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
        .optional()
        .ok()
        .flatten();
    let integrity = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .ok();
    metadata.is_some_and(|(format, found_key, complete)| {
        format == INDEX_FORMAT_VERSION && found_key == key && complete == "1"
    }) && integrity.as_deref() == Some("ok")
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn io_error(code: &'static str, error: std::io::Error) -> QueryError {
    QueryError::new(QueryErrorKind::Internal, code, error.to_string())
}

fn sql_error(error: rusqlite::Error) -> QueryError {
    QueryError::new(
        QueryErrorKind::CorruptArtifact,
        "query_index_error",
        error.to_string(),
    )
}

fn json_error(error: serde_json::Error) -> QueryError {
    QueryError::new(
        QueryErrorKind::Internal,
        "query_index_serialization",
        error.to_string(),
    )
}
