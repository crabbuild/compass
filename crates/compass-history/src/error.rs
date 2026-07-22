use std::path::PathBuf;

/// Errors produced by graph-history storage and repository discovery.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// A SQLite-backed Prolly operation failed.
    #[error("could not open history store: {0}")]
    Store(#[from] prolly_store_sqlite::SqliteStoreError),
    /// A Prolly tree operation failed.
    #[error("prolly operation failed: {0}")]
    Prolly(#[from] prolly::Error),
    /// A Compass graph document could not be read or reconstructed.
    #[error("graph document failed: {0}")]
    Graph(#[from] compass_model::GraphError),
    /// A compatible artifact file could not be written.
    #[error("artifact file failed: {0}")]
    Files(#[from] compass_files::FileError),
    /// JSON artifact data was malformed.
    #[error("artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// A filesystem operation failed.
    #[error("history I/O failed at {path}: {source}")]
    Io {
        /// Path involved in the operation.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A Git command failed or returned malformed output.
    #[error("Git repository discovery failed: {0}")]
    Git(String),
    /// A history path was unsafe or had an unexpected type.
    #[error("unsafe history path {path}: {reason}")]
    UnsafePath {
        /// Rejected path.
        path: PathBuf,
        /// Rejection reason.
        reason: String,
    },
    /// The maintenance lock could not be acquired before its deadline.
    #[error("timed out acquiring {kind} history lock at {path}")]
    LockTimeout {
        /// Requested lock kind.
        kind: &'static str,
        /// Lock path.
        path: PathBuf,
    },
    /// The history database has no supported store-format marker.
    #[error("history store format is missing or incompatible")]
    IncompatibleStoreFormat,
    /// A value could not be represented by the canonical encoding.
    #[error("canonical encoding failed: {0}")]
    Canonical(String),
    /// A typed key was malformed.
    #[error("invalid typed key: {0}")]
    InvalidKey(String),
    /// A fingerprint field name appears to contain secret material.
    #[error("secret-bearing field cannot enter extraction identity: {0}")]
    FingerprintSecretKey(String),
    /// A fingerprint digest was not strict lowercase SHA-256 text.
    #[error("invalid extraction fingerprint: {0}")]
    InvalidFingerprint(String),
    /// A commit was not a full SHA-1 or SHA-256 object ID.
    #[error("invalid Git commit ID: {0}")]
    InvalidCommit(String),
    /// Graph artifacts violated the immutable history schema.
    #[error("invalid graph artifacts: {0}")]
    InvalidArtifacts(String),
    /// Durable catalog state conflicts with immutable realization content.
    #[error("corrupt graph history: {0}")]
    CorruptHistory(String),
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> HistoryError {
    HistoryError::Io {
        path: path.into(),
        source,
    }
}
