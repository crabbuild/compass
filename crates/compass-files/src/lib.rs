//! Deterministic source discovery and Python-compatible cache artifacts.

mod atomic;
mod build_guard;
mod cache;
mod detect;
mod encoding;
mod hash;
mod manifest;
mod slice;

pub use atomic::{
    write_bytes_atomic, write_json_ascii_atomic, write_json_atomic, write_text_atomic,
};
pub use build_guard::BuildGuard;
pub use cache::{Cache, CacheKind};
pub use detect::{
    DetectOptions, Detection, FileType, IgnorePolicy, WatchPathFilter, classify_file, detect,
};
pub use encoding::read_source_lossy;
pub use hash::{StatHashIndex, body_content, file_hash, md5_file, prompt_fingerprint};
pub use manifest::{IncrementalDetection, Manifest, ManifestEntry, ManifestKind};
pub use slice::{FileSlice, bisect_slice, read_slice_text, slice_boundaries, split_file};

use std::path::PathBuf;

/// Errors shared by the deterministic filesystem layer.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("file hash requires a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("path is outside the scan root: {0}")]
    OutsideRoot(PathBuf),
    #[error("source file exceeds the {limit}-byte limit: {path}")]
    TooLarge { path: PathBuf, limit: u64 },
    #[error("an interrupted graph build is recorded at {0}")]
    IncompleteBuild(PathBuf),
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> FileError {
    FileError::Io {
        path: path.into(),
        source,
    }
}
