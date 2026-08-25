//! Deterministic source discovery and Python-compatible cache artifacts.

mod atomic;
mod build_guard;
mod cache;
mod detect;
mod encoding;
mod file_set;
mod generated;
mod hash;
mod manifest;
mod project_config;
mod scope;
mod slice;

pub use atomic::{
    AtomicJsonDigest, write_atomic_with, write_atomic_with_digest, write_bytes_atomic,
    write_json_ascii_atomic, write_json_atomic, write_json_atomic_new,
    write_json_atomic_with_digest, write_text_atomic,
};
pub use build_guard::BuildGuard;
pub use cache::{
    AST_CACHE_VERSION, CACHE_ENCODING_VERSION, Cache, CacheHashPolicy, CacheKind, CacheLayout,
    CacheOptions,
};
pub use detect::{
    DetectOptions, Detection, FileType, IgnorePolicy, WatchPathFilter, classify_file, detect,
};
pub use encoding::{read_bytes_bounded, read_source_lossy};
pub use file_set::FileSetMatcher;
pub use generated::source_is_generated;
pub use hash::{StatHashIndex, body_content, file_hash, md5_file, prompt_fingerprint};
pub use manifest::{IncrementalDetection, Manifest, ManifestEntry, ManifestKind};
pub use project_config::{PROJECT_CONFIG_RELATIVE_PATH, ProjectConfig};
pub use scope::{BuildScope, ScopeMatcher};
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
    #[error("could not encode MessagePack cache at {path}: {source}")]
    MessagePackEncode {
        path: PathBuf,
        #[source]
        source: rmp_serde::encode::Error,
    },
    #[error("could not compress deterministic cache entry at {path}: {source}")]
    CacheCompression {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("deterministic cache entry at {path} is {size} bytes, exceeding limit {limit}")]
    CacheEntryTooLarge {
        path: PathBuf,
        size: usize,
        limit: usize,
    },
    #[error("file hash requires a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("path is outside the scan root: {0}")]
    OutsideRoot(PathBuf),
    #[error("source file exceeds the {limit}-byte limit: {path}")]
    TooLarge { path: PathBuf, limit: u64 },
    #[error("an interrupted graph build is recorded at {0}")]
    IncompleteBuild(PathBuf),
    #[error("snapshot artifact is missing or unsafe: {0}")]
    InvalidSnapshotArtifact(PathBuf),
    #[error("invalid cache kind for operation: {0}")]
    InvalidCacheKind(String),
    #[error("invalid Compass project config at {path}: {source}")]
    ProjectConfigToml {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("could not encode Compass project config at {path}: {source}")]
    ProjectConfigEncode {
        path: PathBuf,
        #[source]
        source: Box<toml::ser::Error>,
    },
    #[error("unsupported Compass config version {version} at {path}")]
    UnsupportedProjectConfig { path: PathBuf, version: u32 },
    #[error("Compass project config path {path} resolves outside project root {root}")]
    ProjectConfigOutsideRoot { path: PathBuf, root: PathBuf },
    #[error("invalid build scope entry '{entry}': {reason}")]
    InvalidScope { entry: String, reason: String },
    #[error("invalid framework file-set pattern '{pattern}': {reason}")]
    InvalidFileSet { pattern: String, reason: String },
    #[error("framework file-set {kind} limit exceeded: observed {observed}, maximum {maximum}")]
    FileSetLimit {
        kind: &'static str,
        observed: usize,
        maximum: usize,
    },
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> FileError {
    FileError::Io {
        path: path.into(),
        source,
    }
}
