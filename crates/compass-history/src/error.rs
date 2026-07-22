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
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> HistoryError {
    HistoryError::Io {
        path: path.into(),
        source,
    }
}
