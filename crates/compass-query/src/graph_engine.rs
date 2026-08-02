//! Immutable graph-engine boundary shared by JSON and store-backed readers.
//!
//! The first local store implementation still materializes the validated typed
//! document so existing query algorithms remain byte-for-byte compatible. The
//! boundary is deliberately independent of SQLite and is the seam for the
//! planned projection/streaming implementation.

use std::fs;
use std::path::{Path, PathBuf};

use compass_graph::{GraphSnapshotManifest, GraphSnapshotReader, canonical_graph_json};
use compass_model::code_graph::{CODE_GRAPH_SCHEMA_V1, GraphDocument};
use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore, Store, StoreRef};

use crate::cql::{QueryError, QueryErrorKind};
use crate::index::{EngineSelection, QueryEngineKind};

const MAX_STORE_REF_BYTES: u64 = 16 * 1024;

/// Read-only graph source used by query planners and public commands.
pub trait GraphEngine: Send + Sync {
    fn kind(&self) -> QueryEngineKind;
    fn graph(&self) -> &GraphDocument;
    fn graph_bytes(&self) -> &[u8];
}

/// Permanent compatible engine for a validated `graph.json` artifact.
pub struct JsonGraphEngine {
    graph: GraphDocument,
    graph_bytes: Vec<u8>,
}

impl JsonGraphEngine {
    pub fn open(path: &Path) -> Result<Self, QueryError> {
        let graph = GraphDocument::load(path).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "graph_load_failed",
                error.to_string(),
            )
        })?;
        validate_graph_schema(&graph)?;
        let graph_bytes = canonical_graph_json(&graph).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "graph_canonicalization_failed",
                error.to_string(),
            )
        })?;
        Ok(Self { graph, graph_bytes })
    }
}

impl GraphEngine for JsonGraphEngine {
    fn kind(&self) -> QueryEngineKind {
        QueryEngineKind::Json
    }

    fn graph(&self) -> &GraphDocument {
        &self.graph
    }

    fn graph_bytes(&self) -> &[u8] {
        &self.graph_bytes
    }
}

/// Store-backed engine for one immutable, validated SQLite snapshot.
pub struct StoreGraphEngine {
    graph: GraphDocument,
    graph_bytes: Vec<u8>,
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
        )
    }

    pub fn open(graph_path: &Path) -> Result<Self, QueryError> {
        let store_path = adjacent_store_path(graph_path);
        let store = SqliteStore::open_read_only(&store_path).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_open_failed",
                error.to_string(),
            )
        })?;
        let active = GraphSnapshotReader::open_active(&store).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_snapshot_failed",
                error.to_string(),
            )
        })?;
        let (graph_schema, node_count, edge_count, graph_bytes) = if let Some(reader) = active {
            let manifest = reader.manifest();
            let graph_bytes = reader.export_json_bytes().map_err(|error| {
                QueryError::new(
                    QueryErrorKind::CorruptArtifact,
                    "store_graph_export_failed",
                    error.to_string(),
                )
            })?;
            validate_store_ref(graph_path, &store, Some(manifest))?;
            (
                manifest.graph_schema.clone(),
                manifest.node_count,
                manifest.edge_count,
                graph_bytes,
            )
        } else {
            let (manifest, graph_bytes) = store.read_snapshot().map_err(|error| {
                QueryError::new(
                    QueryErrorKind::CorruptArtifact,
                    "store_snapshot_failed",
                    error.to_string(),
                )
            })?;
            validate_store_ref(graph_path, &store, None)?;
            (
                manifest.graph_schema,
                manifest.node_count,
                manifest.edge_count,
                graph_bytes,
            )
        };
        Self::from_parts(graph_schema, node_count, edge_count, graph_bytes)
    }

    fn from_parts(
        graph_schema: String,
        node_count: u64,
        edge_count: u64,
        graph_bytes: Vec<u8>,
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
        let graph_bytes = canonical_graph_json(&graph).map_err(|error| {
            QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_graph_canonicalization_failed",
                error.to_string(),
            )
        })?;
        Ok(Self { graph, graph_bytes })
    }
}

impl GraphEngine for StoreGraphEngine {
    fn kind(&self) -> QueryEngineKind {
        QueryEngineKind::Store
    }

    fn graph(&self) -> &GraphDocument {
        &self.graph
    }

    fn graph_bytes(&self) -> &[u8] {
        &self.graph_bytes
    }
}

/// Open the selected graph engine. Default selection prefers the co-published
/// store and uses JSON only when no store sidecar exists, preserving direct
/// compatibility for raw graph files.
pub fn open_graph_engine(
    graph_path: &Path,
    selection: EngineSelection,
) -> Result<Box<dyn GraphEngine>, QueryError> {
    let store_exists = adjacent_store_path(graph_path).is_file();
    match selection {
        EngineSelection::Json => Ok(Box::new(JsonGraphEngine::open(graph_path)?)),
        EngineSelection::Store => Ok(Box::new(StoreGraphEngine::open(graph_path)?)),
        EngineSelection::Default if store_exists => {
            Ok(Box::new(StoreGraphEngine::open(graph_path)?))
        }
        EngineSelection::Default => Ok(Box::new(JsonGraphEngine::open(graph_path)?)),
    }
}

fn adjacent_store_path(graph_path: &Path) -> PathBuf {
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(STORE_FILE_NAME)
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

fn validate_store_ref(
    graph_path: &Path,
    store: &SqliteStore,
    graph_snapshot: Option<&GraphSnapshotManifest>,
) -> Result<(), QueryError> {
    let reference_path = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(STORE_REF_FILE_NAME);
    if !reference_path.is_file() {
        if graph_snapshot.is_some() {
            return Err(QueryError::new(
                QueryErrorKind::CorruptArtifact,
                "store_ref_missing",
                "store.ref is required for an immutable graph snapshot",
            ));
        }
        return Ok(());
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
    reference.validate().map_err(|error| {
        QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_ref_invalid",
            error.to_string(),
        )
    })?;
    let actual = store.snapshot_reference().map_err(|error| {
        QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_ref_store_read_failed",
            error.to_string(),
        )
    })?;
    if actual != reference {
        return Err(QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_ref_mismatch",
            "store.ref does not describe the selected store snapshot",
        ));
    }
    if let Some(manifest) = graph_snapshot
        && (reference.snapshot_id != manifest.snapshot_id
            || reference.graph_digest != manifest.graph_digest)
    {
        return Err(QueryError::new(
            QueryErrorKind::CorruptArtifact,
            "store_ref_snapshot_mismatch",
            "store.ref does not describe the active immutable graph snapshot",
        ));
    }
    Ok(())
}

fn io_error(code: &'static str, error: std::io::Error) -> QueryError {
    QueryError::new(QueryErrorKind::Internal, code, error.to_string())
}
