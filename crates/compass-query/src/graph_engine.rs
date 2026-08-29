//! Immutable graph-engine boundary shared by JSON and store-backed readers.
//!
//! Local SQLite queries pin an immutable selector and read projected snapshot
//! indexes directly. Generic store adapters can still use the materializing
//! engine for compatibility and differential validation.

use std::fs;
use std::path::{Path, PathBuf};

use compass_graph::{
    GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1, GraphSnapshotReader, SnapshotSelector, canonical_graph_json,
};
use compass_model::code_graph::{CODE_GRAPH_SCHEMA_V1, GraphDocument};
use compass_store::{STORE_REF_FILE_NAME, SqliteStore, Store, StoreRef, local_sqlite_store_path};
use sha2::{Digest, Sha256};

use crate::cql::{QueryError, QueryErrorKind};
use crate::index::{EngineSelection, QueryEngineKind};

const MAX_STORE_REF_BYTES: u64 = 16 * 1024;

/// Read-only graph source used by query planners and public commands.
pub trait GraphEngine: Send + Sync {
    fn kind(&self) -> QueryEngineKind;
    fn graph(&self) -> &GraphDocument;
    /// Exact content identity used to address the materialized query index.
    ///
    /// JSON engines hash the authoritative artifact bytes without building a
    /// second canonical graph-sized buffer. Store engines use the immutable
    /// snapshot's already-verified canonical digest.
    fn graph_identity(&self) -> &str;
}

/// Permanent compatible engine for a validated `graph.json` artifact.
pub struct JsonGraphEngine {
    graph: GraphDocument,
    graph_identity: String,
}

/// Validated in-memory graph source with a canonical content identity.
pub struct DirectGraphEngine {
    graph: GraphDocument,
    graph_identity: String,
}

impl DirectGraphEngine {
    pub fn from_document(graph: GraphDocument) -> Result<Self, QueryError> {
        validate_graph_schema(&graph)?;
        compass_model::validate_code_graph(&graph).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "direct_graph_validation_failed",
                error.to_string(),
            )
        })?;
        let bytes = canonical_graph_json(&graph).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "direct_graph_identity_failed",
                error.to_string(),
            )
        })?;
        let graph_identity = format!("{:x}", Sha256::digest(&bytes));
        Ok(Self {
            graph,
            graph_identity,
        })
    }

    /// Use an identity already verified by an immutable source such as a
    /// history realization, avoiding another graph-sized canonical buffer.
    pub fn from_verified_document(
        graph: GraphDocument,
        graph_identity: String,
    ) -> Result<Self, QueryError> {
        validate_graph_schema(&graph)?;
        compass_model::validate_code_graph(&graph).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "direct_graph_validation_failed",
                error.to_string(),
            )
        })?;
        if graph_identity.len() != 64
            || !graph_identity.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "direct_graph_identity_invalid",
                "verified graph identity must be a 64-character hexadecimal digest",
            ));
        }
        Ok(Self {
            graph,
            graph_identity: graph_identity.to_ascii_lowercase(),
        })
    }
}

impl GraphEngine for DirectGraphEngine {
    fn kind(&self) -> QueryEngineKind {
        QueryEngineKind::Memory
    }

    fn graph(&self) -> &GraphDocument {
        &self.graph
    }

    fn graph_identity(&self) -> &str {
        &self.graph_identity
    }
}

impl JsonGraphEngine {
    pub fn open(path: &Path) -> Result<Self, QueryError> {
        let (graph, graph_identity) =
            GraphDocument::load_with_artifact_digest(path).map_err(|error| {
                QueryError::new(
                    QueryErrorKind::CorruptArtifact,
                    "graph_load_failed",
                    error.to_string(),
                )
            })?;
        validate_graph_schema(&graph)?;
        Ok(Self {
            graph,
            graph_identity,
        })
    }
}

impl GraphEngine for JsonGraphEngine {
    fn kind(&self) -> QueryEngineKind {
        QueryEngineKind::Json
    }

    fn graph(&self) -> &GraphDocument {
        &self.graph
    }

    fn graph_identity(&self) -> &str {
        &self.graph_identity
    }
}

/// Store-backed engine for one immutable, validated SQLite snapshot.
pub struct StoreGraphEngine {
    graph: GraphDocument,
    graph_identity: String,
}

pub(crate) struct LocalStoreSnapshot {
    pub(crate) store: SqliteStore,
    pub(crate) selector: SnapshotSelector,
    pub(crate) store_path: PathBuf,
    pub(crate) graph_identity: String,
    pub(crate) build_generation_identity: String,
    pub(crate) partial_graph_message: Option<String>,
}

impl LocalStoreSnapshot {
    pub(crate) fn reader(&self) -> Result<GraphSnapshotReader<'_, SqliteStore>, QueryError> {
        GraphSnapshotReader::open_selector(&self.store, self.selector.clone()).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_snapshot_failed",
                error.to_string(),
            )
        })
    }
}

impl StoreGraphEngine {
    /// Open a store-backed engine from any common-contract adapter.
    ///
    /// The caller owns backend selection and lifecycle. This constructor only
    /// consumes the active immutable graph snapshot, so redb and future remote
    /// adapters can use the same query path without exposing backend types.
    pub fn from_store<S: Store + ?Sized>(store: &S) -> Result<Self, QueryError> {
        let reader = GraphSnapshotReader::open_active(store).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_snapshot_failed",
                error.to_string(),
            )
        })?;
        let Some(reader) = reader else {
            return Err(QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_snapshot_missing",
                "store has no active immutable graph snapshot",
            ));
        };
        let manifest = reader.manifest();
        let graph_bytes = reader.export_json_bytes().map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_export_failed",
                error.to_string(),
            )
        })?;
        Self::from_parts(
            manifest.graph_schema.clone(),
            manifest.node_count,
            manifest.edge_count,
            graph_bytes,
            manifest.graph_digest.clone(),
        )
    }

    /// Open one exact immutable selector without consulting the active ref.
    pub fn from_store_selector<S: Store + ?Sized>(
        store: &S,
        selector: SnapshotSelector,
    ) -> Result<Self, QueryError> {
        let reader = GraphSnapshotReader::open_selector(store, selector).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_snapshot_failed",
                error.to_string(),
            )
        })?;
        let manifest = reader.manifest();
        let graph_bytes = reader.export_json_bytes().map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_export_failed",
                error.to_string(),
            )
        })?;
        Self::from_parts(
            manifest.graph_schema.clone(),
            manifest.node_count,
            manifest.edge_count,
            graph_bytes,
            manifest.graph_digest.clone(),
        )
    }

    pub fn open(graph_path: &Path) -> Result<Self, QueryError> {
        let snapshot = open_local_store_snapshot(graph_path)?;
        let reader = snapshot.reader()?;
        let manifest = reader.manifest();
        let graph_schema = manifest.graph_schema.clone();
        let node_count = manifest.node_count;
        let edge_count = manifest.edge_count;
        let graph_identity = manifest.graph_digest.clone();
        let graph_bytes = reader.export_json_bytes().map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_export_failed",
                error.to_string(),
            )
        })?;
        Self::from_parts(
            graph_schema,
            node_count,
            edge_count,
            graph_bytes,
            graph_identity,
        )
    }

    fn from_parts(
        graph_schema: String,
        node_count: u64,
        edge_count: u64,
        graph_bytes: Vec<u8>,
        graph_identity: String,
    ) -> Result<Self, QueryError> {
        if graph_schema != CODE_GRAPH_SCHEMA_V1 {
            return Err(QueryError::new(
                QueryErrorKind::UnsupportedSchema,
                "unsupported_graph_schema",
                format!("expected {CODE_GRAPH_SCHEMA_V1}, found {}", graph_schema),
            ));
        }
        let graph = serde_json::from_slice::<GraphDocument>(&graph_bytes).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_decode_failed",
                error.to_string(),
            )
        })?;
        compass_model::validate_code_graph(&graph).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_validation_failed",
                error.to_string(),
            )
        })?;
        if node_count != graph.nodes.len() as u64 || edge_count != graph.links.len() as u64 {
            return Err(QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_manifest_counts_mismatch",
                "store manifest counts do not match the decoded graph",
            ));
        }
        Ok(Self {
            graph,
            graph_identity,
        })
    }
}

pub(crate) fn open_local_store_snapshot(
    graph_path: &Path,
) -> Result<LocalStoreSnapshot, QueryError> {
    let reference = read_store_ref(graph_path)?;
    let store_path = local_sqlite_store_path(graph_path);
    let store = SqliteStore::open_read_only(&store_path).map_err(|error| {
        QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_open_failed",
            error.to_string(),
        )
    })?;
    store.begin_read_snapshot().map_err(|error| {
        QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_read_snapshot_failed",
            error.to_string(),
        )
    })?;
    let selector = SnapshotSelector {
        schema: GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1.to_owned(),
        snapshot_id: reference.snapshot_id.clone(),
        manifest_digest: reference.manifest_digest.clone(),
    };
    let reader = GraphSnapshotReader::open_selector(&store, selector.clone()).map_err(|error| {
        QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_graph_snapshot_failed",
            error.to_string(),
        )
    })?;
    if reader.manifest().graph_digest != reference.graph_digest {
        return Err(QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_ref_mismatch",
            "store.ref does not describe the selected immutable graph snapshot",
        ));
    }
    let graph_identity = reader.manifest().graph_digest.clone();
    let build_generation_identity = reader
        .metadata_summary()
        .map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_snapshot_failed",
                error.to_string(),
            )
        })?
        .graph
        .build
        .generation_id;
    let partial_graph_message = reader
        .graph_diagnostic_by_code("publication_omission_summary")
        .map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_snapshot_failed",
                error.to_string(),
            )
        })?
        .map(|diagnostic| {
            format!(
                "Published graph coverage is incomplete: {}",
                diagnostic.message
            )
        });
    drop(reader);
    Ok(LocalStoreSnapshot {
        store,
        selector,
        store_path,
        graph_identity,
        build_generation_identity,
        partial_graph_message,
    })
}

impl GraphEngine for StoreGraphEngine {
    fn kind(&self) -> QueryEngineKind {
        QueryEngineKind::Store
    }

    fn graph(&self) -> &GraphDocument {
        &self.graph
    }

    fn graph_identity(&self) -> &str {
        &self.graph_identity
    }
}

/// Open the selected materialized graph engine. The bounded local-store path
/// used by the public default lives in `index::open_with_engine`; callers that
/// need this lower-level adapter can still select JSON or an explicit store.
pub fn open_graph_engine(
    graph_path: &Path,
    selection: EngineSelection,
) -> Result<Box<dyn GraphEngine>, QueryError> {
    match selection {
        EngineSelection::Default | EngineSelection::Json => {
            Ok(Box::new(JsonGraphEngine::open(graph_path)?))
        }
        EngineSelection::Store => Ok(Box::new(StoreGraphEngine::open(graph_path)?)),
    }
}

fn validate_graph_schema(graph: &GraphDocument) -> Result<(), QueryError> {
    if graph.graph.schema != CODE_GRAPH_SCHEMA_V1 {
        return Err(QueryError::new(
            QueryErrorKind::UnsupportedSchema,
            "unsupported_graph_schema",
            format!(
                "expected {CODE_GRAPH_SCHEMA_V1}, found {}",
                graph.graph.schema
            ),
        ));
    }
    Ok(())
}

pub(crate) fn read_store_ref(graph_path: &Path) -> Result<StoreRef, QueryError> {
    let reference_path = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(STORE_REF_FILE_NAME);
    if !reference_path.is_file() {
        return Err(QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_ref_missing",
            "store.ref is required for an immutable graph snapshot",
        ));
    }
    let size = fs::metadata(&reference_path)
        .map_err(|error| io_error("stat_store_ref", error))?
        .len();
    if size > MAX_STORE_REF_BYTES {
        return Err(QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_ref_too_large",
            format!("store.ref is {size} bytes; maximum is {MAX_STORE_REF_BYTES}"),
        ));
    }
    let bytes = fs::read(&reference_path).map_err(|error| io_error("read_store_ref", error))?;
    let reference = serde_json::from_slice::<StoreRef>(&bytes).map_err(|error| {
        QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_ref_decode_failed",
            error.to_string(),
        )
    })?;
    reference.validate_local_sqlite_graph().map_err(|error| {
        QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_ref_invalid",
            error.to_string(),
        )
    })?;
    Ok(reference)
}

fn io_error(code: &'static str, error: std::io::Error) -> QueryError {
    QueryError::new(QueryErrorKind::Internal, code, error.to_string())
}
