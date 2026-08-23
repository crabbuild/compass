use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rayon::prelude::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{FileError, StatHashIndex, file_hash, io_error, write_bytes_atomic, write_json_atomic};

/// Changes whenever cached extraction semantics change, even if the wire encoding does not.
pub const AST_CACHE_VERSION: &str = "2";
/// Portable cache encoding version used in the on-disk namespace.
pub const CACHE_ENCODING_VERSION: u32 = 1;
const MESSAGEPACK_EXTENSION: &str = "msgpack";
const COMPRESSED_MESSAGEPACK_MAGIC: &[u8; 5] = b"CMPZ1";
const COMPRESSED_MESSAGEPACK_HEADER_BYTES: usize = COMPRESSED_MESSAGEPACK_MAGIC.len() + 8;
const MAX_DECOMPRESSED_CACHE_ENTRY_BYTES: usize = 256 * 1024 * 1024;
const CACHE_COMPRESSION_LEVEL: i32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheLayout {
    OutputDirectory,
    SharedHistory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheHashPolicy {
    StatIndexed,
    VerifiedContent,
}

#[derive(Clone, Copy, Debug)]
pub struct CacheOptions<'a> {
    pub storage_root: Option<&'a Path>,
    pub layout: CacheLayout,
    pub hash_policy: CacheHashPolicy,
}

impl<'a> CacheOptions<'a> {
    #[must_use]
    pub const fn output_directory(storage_root: Option<&'a Path>) -> Self {
        Self {
            storage_root,
            layout: CacheLayout::OutputDirectory,
            hash_policy: CacheHashPolicy::StatIndexed,
        }
    }

    #[must_use]
    pub const fn shared_history(storage_root: &'a Path) -> Self {
        Self {
            storage_root: Some(storage_root),
            layout: CacheLayout::SharedHistory,
            hash_policy: CacheHashPolicy::VerifiedContent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheKind {
    Ast,
    Semantic,
    SemanticMode(String),
    ProgramSyntax {
        ir_schema: u32,
        provider_version: String,
    },
    ProgramArtifact {
        ir_schema: u32,
        decoder_version: String,
    },
    ProgramMerge {
        ir_schema: u32,
        merger_version: u32,
        analyzer_version: u32,
    },
}

impl CacheKind {
    fn directory_name(&self) -> String {
        match self {
            Self::Ast => "ast".to_owned(),
            Self::Semantic => "semantic".to_owned(),
            Self::SemanticMode(mode) => format!("semantic-{mode}"),
            Self::ProgramSyntax {
                ir_schema,
                provider_version,
            } => format!(
                "program-syntax/ir{ir_schema}/p{}",
                logical_key_hash(provider_version)
            ),
            Self::ProgramArtifact {
                ir_schema,
                decoder_version,
            } => format!(
                "program-artifact/ir{ir_schema}/d{}",
                logical_key_hash(decoder_version)
            ),
            Self::ProgramMerge {
                ir_schema,
                merger_version,
                analyzer_version,
            } => format!("program-merge/ir{ir_schema}/m{merger_version}/a{analyzer_version}"),
        }
    }
}

/// Reader/writer for Compass's content-addressed extraction cache.
#[derive(Debug)]
pub struct Cache {
    root: PathBuf,
    logical_root: PathBuf,
    cache_base: PathBuf,
    ast_cache_version: String,
    hashes: StatHashIndex,
    hash_policy: CacheHashPolicy,
    session_hashes: HashMap<PathBuf, SessionHash>,
    flush_hashes_on_drop: bool,
}

#[derive(Debug, Clone)]
struct SessionHash {
    size: u64,
    modified: Option<SystemTime>,
    value: String,
}

/// Opaque, compressed cache payload ready for atomic publication.
pub struct EncodedCacheWrite {
    destination: PathBuf,
    bytes: Vec<u8>,
}

impl Cache {
    pub fn open(root: impl AsRef<Path>, options: CacheOptions<'_>) -> Result<Self, FileError> {
        let requested_root = root.as_ref();
        let logical_root = if requested_root.is_absolute() {
            requested_root.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|source| io_error(requested_root, source))?
                .join(requested_root)
        };
        let root =
            fs::canonicalize(requested_root).map_err(|source| io_error(requested_root, source))?;
        let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
        let storage_root = options
            .storage_root
            .map_or_else(|| root.clone(), Path::to_path_buf);
        if options.layout == CacheLayout::SharedHistory && !storage_root.is_absolute() {
            return Err(FileError::OutsideRoot(storage_root));
        }
        let cache_base = match options.layout {
            CacheLayout::OutputDirectory => storage_root.join(&output_name).join("cache"),
            CacheLayout::SharedHistory => storage_root.clone(),
        };
        let hashes = StatHashIndex::load(&storage_root, &output_name);
        let cache = Self {
            root,
            logical_root,
            cache_base,
            ast_cache_version: AST_CACHE_VERSION.to_owned(),
            hashes,
            hash_policy: options.hash_policy,
            session_hashes: HashMap::new(),
            flush_hashes_on_drop: options.hash_policy == CacheHashPolicy::StatIndexed,
        };
        cache.cleanup_stale_ast();
        Ok(cache)
    }

    /// Program-only cache users do not consult the path stat index. Disabling
    /// its drop-time flush prevents a parallel Program worker from overwriting
    /// hashes recorded by the graph extraction worker.
    pub fn without_hash_flush(mut self) -> Self {
        self.flush_hashes_on_drop = false;
        self
    }

    pub fn directory(&self, kind: &CacheKind, prompt_fingerprint: Option<&str>) -> PathBuf {
        let mut directory = self.cache_base.join(kind.directory_name());
        if matches!(kind, CacheKind::Ast) {
            directory = directory.join(format!("v{}", self.ast_cache_version));
        } else if let Some(fingerprint) = prompt_fingerprint {
            directory = directory.join(format!("p{fingerprint}"));
        }
        if deterministic_binary_kind(kind) {
            directory = directory.join(format!("e{CACHE_ENCODING_VERSION}"));
        }
        directory
    }

    pub fn load(
        &mut self,
        path: &Path,
        kind: &CacheKind,
        prompt_fingerprint: Option<&str>,
        allow_partial: bool,
    ) -> Result<Option<Value>, FileError> {
        self.load_with_source_paths(path, kind, prompt_fingerprint, allow_partial, true)
    }

    /// Load a pipeline-owned AST cache entry without expanding its portable
    /// source paths. The extraction pipeline keeps these values portable until
    /// publication, so expanding and then re-normalizing every cached node and
    /// edge would only add work to incremental builds.
    pub fn load_portable_ast(
        &mut self,
        path: &Path,
        allow_partial: bool,
    ) -> Result<Option<Value>, FileError> {
        self.load_with_source_paths(path, &CacheKind::Ast, None, allow_partial, false)
    }

    /// Load a portable AST cache entry directly into its typed representation.
    ///
    /// The compatibility [`Self::load_portable_ast`] API intentionally returns
    /// a JSON value, but the extraction pipeline immediately deserializes that
    /// value into its typed extraction. Keeping this typed path beside the
    /// compatibility API avoids materializing a second full tree of JSON
    /// values on every warm build. `is_partial` is supplied by the caller so
    /// this filesystem crate does not need to depend on a language crate.
    pub fn load_portable_ast_typed<T, F>(
        &mut self,
        path: &Path,
        allow_partial: bool,
        is_partial: F,
    ) -> Result<Option<T>, FileError>
    where
        T: DeserializeOwned,
        F: Fn(&T) -> bool,
    {
        let hash = self.content_hash(path)?;
        let key = self.source_cache_key(path, &hash);
        let entry = self
            .directory(&CacheKind::Ast, None)
            .join(format!("{key}.{MESSAGEPACK_EXTENSION}"));
        let Ok(bytes) = fs::read(entry) else {
            return Ok(None);
        };
        let Some(value) = decode_messagepack::<T>(&bytes) else {
            return Ok(None);
        };
        if !allow_partial && is_partial(&value) {
            return Ok(None);
        }
        Ok(Some(value))
    }

    fn load_with_source_paths(
        &mut self,
        path: &Path,
        kind: &CacheKind,
        prompt_fingerprint: Option<&str>,
        allow_partial: bool,
        absolutize_paths: bool,
    ) -> Result<Option<Value>, FileError> {
        let hash = self.content_hash(path)?;
        let key = self.source_cache_key(path, &hash);
        if deterministic_binary_kind(kind) {
            let entry = self
                .directory(kind, prompt_fingerprint)
                .join(format!("{key}.{MESSAGEPACK_EXTENSION}"));
            if let Ok(bytes) = fs::read(entry)
                && let Some(mut value) = decode_messagepack::<Value>(&bytes)
            {
                if !allow_partial && value.get("partial").and_then(Value::as_bool) == Some(true) {
                    return Ok(None);
                }
                if absolutize_paths {
                    absolutize_source_files(&mut value, &self.root);
                }
                return Ok(Some(value));
            }
            return Ok(None);
        }
        let entry = self
            .directory(kind, prompt_fingerprint)
            .join(format!("{key}.json"));
        if !entry.exists() {
            return Ok(None);
        }
        load_json_value(&entry, allow_partial, &self.root, absolutize_paths)
    }

    pub fn save(
        &mut self,
        path: &Path,
        value: &Value,
        kind: &CacheKind,
        prompt_fingerprint: Option<&str>,
    ) -> Result<(), FileError> {
        if !path.is_file() {
            return Ok(());
        }
        let mut on_disk = value.clone();
        relativize_source_files(&mut on_disk, &self.root);
        let hash = self.content_hash(path)?;
        let key = self.source_cache_key(path, &hash);
        let directory = self.directory(kind, prompt_fingerprint);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        if deterministic_binary_kind(kind) {
            let destination = directory.join(format!("{key}.{MESSAGEPACK_EXTENSION}"));
            let bytes = encode_messagepack(&on_disk, &destination)?;
            write_cache_bytes(&destination, &bytes)
        } else {
            write_json_atomic(directory.join(format!("{key}.json")), &on_disk, false)
        }
    }

    /// Persist a group of independent content-addressed cache entries
    /// concurrently.
    pub fn save_batch(
        &mut self,
        entries: &[(PathBuf, Value)],
        kind: &CacheKind,
        prompt_fingerprint: Option<&str>,
    ) -> Result<(), FileError> {
        let directory = self.directory(kind, prompt_fingerprint);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        let mut jobs = Vec::with_capacity(entries.len());
        for (path, value) in entries {
            if !path.is_file() {
                continue;
            }
            let hash = self.content_hash(path)?;
            let key = self.source_cache_key(path, &hash);
            let extension = if deterministic_binary_kind(kind) {
                MESSAGEPACK_EXTENSION
            } else {
                "json"
            };
            jobs.push((directory.join(format!("{key}.{extension}")), value));
        }
        let root = &self.root;
        let write_job = |(destination, value): (PathBuf, &Value)| {
            let mut on_disk = value.clone();
            relativize_source_files(&mut on_disk, root);
            if deterministic_binary_kind(kind) {
                let bytes = encode_messagepack(&on_disk, &destination)?;
                write_cache_bytes(&destination, &bytes)
            } else {
                write_json_atomic(destination, &on_disk, false)
            }
        };
        if jobs.len() < 256 {
            jobs.into_iter().try_for_each(write_job)
        } else {
            jobs.into_par_iter().try_for_each(write_job)
        }
    }

    /// Persist AST values whose source paths are already repository-relative.
    ///
    /// The extraction pipeline produces portable paths directly. Encoding the
    /// typed values avoids an intermediate JSON tree plus a second full clone
    /// and traversal of every node and edge.
    pub fn save_portable_ast_batch<T: Serialize + Sync>(
        &mut self,
        entries: &[(PathBuf, T)],
    ) -> Result<(), FileError> {
        let directory = self.directory(&CacheKind::Ast, None);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        let mut jobs = Vec::with_capacity(entries.len());
        for (path, value) in entries {
            if !path.is_file() {
                continue;
            }
            let hash = self.content_hash(path)?;
            let key = self.source_cache_key(path, &hash);
            jobs.push((
                directory.join(format!("{key}.{MESSAGEPACK_EXTENSION}")),
                value,
            ));
        }
        let write_job = |(destination, value): (PathBuf, &T)| {
            let bytes = encode_messagepack(value, &destination)?;
            write_cache_bytes(&destination, &bytes)
        };
        if jobs.len() < 256 {
            jobs.into_iter().try_for_each(write_job)
        } else {
            jobs.into_par_iter().try_for_each(write_job)
        }
    }

    /// Hash source bytes that the caller has already read for extraction.
    ///
    /// The cache key uses the same path salt as [`Self::content_hash`], but
    /// this method does not touch the filesystem. Callers can therefore avoid
    /// rereading a cold source file merely to prepare its cache destination.
    #[must_use]
    pub fn content_hash_from_bytes(&self, path: &Path, bytes: &[u8]) -> String {
        let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let salt = resolved
            .strip_prefix(&self.root)
            .unwrap_or(&resolved)
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        let mut digest = Sha256::new();
        digest.update(bytes);
        digest.update([0]);
        digest.update(salt.as_bytes());
        format!("{:x}", digest.finalize())
    }

    /// Seed a session hash when the source bytes were read by the extractor.
    ///
    /// Current size and modification-time checks prevent a stale seed from
    /// being used when a file changed while the build was running. A changed
    /// file simply falls back to the ordinary bounded read in
    /// [`Self::content_hash`].
    pub fn seed_content_hash(
        &mut self,
        path: &Path,
        hash: String,
        size: u64,
        modified: Option<SystemTime>,
    ) -> Result<(), FileError> {
        let metadata = fs::metadata(path).map_err(|source| io_error(path, source))?;
        if metadata.is_file() && metadata.len() == size && metadata.modified().ok() == modified {
            self.session_hashes.insert(
                path.to_path_buf(),
                SessionHash {
                    size,
                    modified,
                    value: hash,
                },
            );
        }
        Ok(())
    }

    /// Prepare portable AST entries in parallel without retaining cloned typed
    /// values after each entry has been compressed.
    pub fn encode_portable_ast_batch<T, F>(
        &mut self,
        entries: &[(&PathBuf, &T)],
        prepare: F,
    ) -> Result<Vec<EncodedCacheWrite>, FileError>
    where
        T: Serialize + Sync,
        F: Fn(&Path, &T) -> T + Sync,
    {
        let directory = self.directory(&CacheKind::Ast, None);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        let mut jobs = Vec::with_capacity(entries.len());
        for &(path, value) in entries {
            let hash = self.content_hash(path)?;
            let key = self.source_cache_key(path, &hash);
            jobs.push((
                directory.join(format!("{key}.{MESSAGEPACK_EXTENSION}")),
                path,
                value,
            ));
        }
        jobs.into_par_iter()
            .map(|(destination, path, value)| {
                let prepared = prepare(path, value);
                let bytes = encode_messagepack(&prepared, &destination)?;
                Ok(EncodedCacheWrite { destination, bytes })
            })
            .collect()
    }

    /// Encode already-normalized portable AST values without cloning them.
    ///
    /// Callers that need a project-specific normalization pass can mutate
    /// their owned values before invoking this method. Keeping the encoder
    /// borrow-only avoids a second full extraction clone on cold builds while
    /// preserving the same compressed cache payload and atomic publication
    /// boundary as encode_portable_ast_batch.
    pub fn encode_portable_ast_batch_ref<T: Serialize + Sync>(
        &mut self,
        entries: &[(&PathBuf, &T)],
    ) -> Result<Vec<EncodedCacheWrite>, FileError> {
        let directory = self.directory(&CacheKind::Ast, None);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        let mut jobs = Vec::with_capacity(entries.len());
        for &(path, value) in entries {
            let hash = self.content_hash(path)?;
            let key = self.source_cache_key(path, &hash);
            jobs.push((
                directory.join(format!("{key}.{MESSAGEPACK_EXTENSION}")),
                value,
            ));
        }
        jobs.into_par_iter()
            .map(|(destination, value)| {
                let bytes = encode_messagepack(value, &destination)?;
                Ok(EncodedCacheWrite { destination, bytes })
            })
            .collect()
    }

    /// Encode and publish already-normalized portable AST values one entry at
    /// a time. Unlike [`Self::encode_portable_ast_batch_ref`], this does not
    /// retain every compressed payload until the whole batch is ready, which
    /// keeps peak memory proportional to the largest in-flight file.
    pub fn write_portable_ast_batch_ref<T: Serialize + Sync>(
        &mut self,
        entries: &[(&PathBuf, &T)],
    ) -> Result<(), FileError> {
        let directory = self.directory(&CacheKind::Ast, None);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        let mut jobs = Vec::with_capacity(entries.len());
        for &(path, value) in entries {
            let hash = self.content_hash(path)?;
            let key = self.source_cache_key(path, &hash);
            jobs.push((
                directory.join(format!("{key}.{MESSAGEPACK_EXTENSION}")),
                value,
            ));
        }
        let write_job = |(destination, value): &(PathBuf, &T)| {
            let bytes = encode_messagepack(*value, destination)?;
            write_cache_bytes(destination, &bytes)
        };
        // This API is the bounded-residency alternative to the parallel batch
        // encoder above. Encoding a large batch in parallel retains one zstd
        // workspace and MessagePack buffer per worker, contradicting the
        // one-entry-at-a-time contract and raising cold-build peak RSS.
        jobs.iter().try_for_each(write_job)
    }

    /// Atomically publish cache payloads prepared by
    /// encode_portable_ast_batch or encode_portable_ast_batch_ref.
    pub fn write_encoded_batch(entries: &[EncodedCacheWrite]) -> Result<(), FileError> {
        entries
            .par_iter()
            .try_for_each(|entry| write_cache_bytes(&entry.destination, &entry.bytes))
    }

    /// Load a Program IR cache value by a caller-owned logical input key.
    ///
    /// Program values remain repository-relative and are never rewritten with
    /// the checkout root.
    pub fn load_program<T: DeserializeOwned>(
        &self,
        kind: &CacheKind,
        logical_key: &str,
    ) -> Result<Option<T>, FileError> {
        ensure_program_kind(kind)?;
        let key = logical_key_hash(logical_key);
        let entry = self
            .directory(kind, None)
            .join(format!("{key}.{MESSAGEPACK_EXTENSION}"));
        if let Ok(bytes) = fs::read(entry)
            && let Some(value) = decode_messagepack(&bytes)
        {
            return Ok(Some(value));
        }
        Ok(None)
    }

    /// Safely save a repository-relative Program IR cache value.
    pub fn save_program<T: Serialize>(
        &self,
        kind: &CacheKind,
        logical_key: &str,
        value: &T,
    ) -> Result<(), FileError> {
        ensure_program_kind(kind)?;
        let directory = self.directory(kind, None);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        let destination = directory.join(format!(
            "{}.{MESSAGEPACK_EXTENSION}",
            logical_key_hash(logical_key)
        ));
        let bytes = encode_messagepack(value, &destination)?;
        write_cache_bytes(&destination, &bytes)
    }

    /// Safely persist a group of repository-relative Program cache values in
    /// parallel. Program syntax extraction commonly produces thousands of
    /// independent entries, so serial encoding and file publication otherwise
    /// becomes a cold-build bottleneck.
    pub fn save_program_batch<T: Serialize + Sync>(
        &self,
        kind: &CacheKind,
        entries: &[(String, T)],
    ) -> Result<(), FileError> {
        ensure_program_kind(kind)?;
        let directory = self.directory(kind, None);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        entries.par_iter().try_for_each(|(logical_key, value)| {
            let destination = directory.join(format!(
                "{}.{MESSAGEPACK_EXTENSION}",
                logical_key_hash(logical_key)
            ));
            let bytes = encode_messagepack(value, &destination)?;
            write_cache_bytes(&destination, &bytes)
        })
    }

    /// Remove entries outside a successfully completed provider's live set.
    pub fn prune_program(
        &self,
        kind: &CacheKind,
        live_logical_keys: &BTreeSet<String>,
    ) -> Result<usize, FileError> {
        ensure_program_kind(kind)?;
        let hashes = live_logical_keys
            .iter()
            .map(|key| logical_key_hash(key))
            .collect::<BTreeSet<_>>();
        Ok(prune_cache_entries(&self.directory(kind, None), &hashes))
    }

    pub fn flush(&mut self) -> Result<(), FileError> {
        self.hashes.flush()
    }

    pub fn cached_files(&self) -> BTreeSet<String> {
        let mut hashes = BTreeSet::new();
        collect_cache_stems(&self.cache_base, &mut hashes);
        hashes
    }

    pub fn clear(&self) {
        clear_cache_entries(&self.cache_base);
    }

    pub fn prune_semantic(&self, live_hashes: &BTreeSet<String>) -> usize {
        let mut removed = 0;
        for kind in ["semantic", "semantic-deep"] {
            removed += prune_cache_entries(&self.cache_base.join(kind), live_hashes);
        }
        removed
    }

    /// Derive the complete path-sensitive entry keys used to retain live
    /// source-backed cache records during pruning.
    pub fn source_cache_keys(&mut self, paths: &[PathBuf]) -> Result<BTreeSet<String>, FileError> {
        paths
            .iter()
            .map(|path| {
                let hash = self.content_hash(path)?;
                Ok(self.source_cache_key(path, &hash))
            })
            .collect()
    }

    fn cleanup_stale_ast(&self) {
        let base = self.cache_base.join("ast");
        let current = format!("v{}", self.ast_cache_version);
        let Ok(entries) = fs::read_dir(&base) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() && name.to_string_lossy().starts_with('v') && name != current.as_str()
            {
                let _ = fs::remove_dir_all(path);
            } else if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn content_hash(&mut self, path: &Path) -> Result<String, FileError> {
        let metadata = fs::metadata(path).map_err(|source| io_error(path, source))?;
        let modified = metadata.modified().ok();
        if let Some(cached) = self.session_hashes.get(path)
            && cached.size == metadata.len()
            && cached.modified == modified
        {
            return Ok(cached.value.clone());
        }
        let value = match self.hash_policy {
            CacheHashPolicy::StatIndexed => self.hashes.hash(path, &self.root)?,
            CacheHashPolicy::VerifiedContent => file_hash(path, &self.root)?,
        };
        self.session_hashes.insert(
            path.to_path_buf(),
            SessionHash {
                size: metadata.len(),
                modified,
                value: value.clone(),
            },
        );
        Ok(value)
    }

    fn source_cache_key(&self, path: &Path, content_hash: &str) -> String {
        let logical_path = self.logical_source_path(path);
        format!("{content_hash}-{}", logical_key_hash(&logical_path))
    }

    fn logical_source_path<'a>(&'a self, path: &'a Path) -> Cow<'a, str> {
        path.strip_prefix(&self.logical_root)
            .or_else(|_| path.strip_prefix(&self.root))
            .unwrap_or(path)
            .to_string_lossy()
    }
}

fn write_cache_bytes(destination: &Path, bytes: &[u8]) -> Result<(), FileError> {
    let file = match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
    {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            return write_bytes_atomic(destination, bytes);
        }
        Err(source) => return Err(io_error(destination, source)),
    };
    // Content-addressed cache entries are rebuildable. A reader that observes
    // an interrupted create treats the undecodable entry as a miss, and the
    // next writer atomically replaces it through the existing-entry branch.
    let mut writer = BufWriter::new(file);
    writer
        .write_all(bytes)
        .and_then(|()| writer.flush())
        .map_err(|source| io_error(destination, source))
}

impl Drop for Cache {
    fn drop(&mut self) {
        if self.flush_hashes_on_drop {
            let _ = self.flush();
        }
    }
}

fn ensure_program_kind(kind: &CacheKind) -> Result<(), FileError> {
    if matches!(
        kind,
        CacheKind::ProgramSyntax { .. }
            | CacheKind::ProgramArtifact { .. }
            | CacheKind::ProgramMerge { .. }
    ) {
        Ok(())
    } else {
        Err(FileError::InvalidCacheKind(format!("{kind:?}")))
    }
}

fn deterministic_binary_kind(kind: &CacheKind) -> bool {
    matches!(
        kind,
        CacheKind::Ast
            | CacheKind::ProgramSyntax { .. }
            | CacheKind::ProgramArtifact { .. }
            | CacheKind::ProgramMerge { .. }
    )
}

fn load_json_value(
    path: &Path,
    allow_partial: bool,
    root: &Path,
    absolutize_paths: bool,
) -> Result<Option<Value>, FileError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let mut value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if !allow_partial && value.get("partial").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    if absolutize_paths {
        absolutize_source_files(&mut value, root);
    }
    Ok(Some(value))
}

fn encode_messagepack<T: Serialize>(value: &T, path: &Path) -> Result<Vec<u8>, FileError> {
    let messagepack =
        rmp_serde::to_vec_named(value).map_err(|source| FileError::MessagePackEncode {
            path: path.to_path_buf(),
            source,
        })?;
    if messagepack.len() > MAX_DECOMPRESSED_CACHE_ENTRY_BYTES {
        return Err(FileError::CacheEntryTooLarge {
            path: path.to_path_buf(),
            size: messagepack.len(),
            limit: MAX_DECOMPRESSED_CACHE_ENTRY_BYTES,
        });
    }
    let compressed =
        zstd::bulk::compress(&messagepack, CACHE_COMPRESSION_LEVEL).map_err(|source| {
            FileError::CacheCompression {
                path: path.to_path_buf(),
                source,
            }
        })?;
    let mut encoded =
        Vec::with_capacity(COMPRESSED_MESSAGEPACK_HEADER_BYTES.saturating_add(compressed.len()));
    encoded.extend_from_slice(COMPRESSED_MESSAGEPACK_MAGIC);
    encoded.extend_from_slice(&(messagepack.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&compressed);
    Ok(encoded)
}

fn decode_messagepack<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    if bytes.len() < COMPRESSED_MESSAGEPACK_HEADER_BYTES
        || &bytes[..COMPRESSED_MESSAGEPACK_MAGIC.len()] != COMPRESSED_MESSAGEPACK_MAGIC
    {
        return None;
    }
    let size_bytes: [u8; 8] = bytes
        [COMPRESSED_MESSAGEPACK_MAGIC.len()..COMPRESSED_MESSAGEPACK_HEADER_BYTES]
        .try_into()
        .ok()?;
    let size = usize::try_from(u64::from_le_bytes(size_bytes)).ok()?;
    if size > MAX_DECOMPRESSED_CACHE_ENTRY_BYTES {
        return None;
    }
    let messagepack =
        zstd::bulk::decompress(&bytes[COMPRESSED_MESSAGEPACK_HEADER_BYTES..], size).ok()?;
    if messagepack.len() != size {
        return None;
    }
    let mut deserializer = rmp_serde::Deserializer::new(std::io::Cursor::new(&messagepack));
    let value = serde::Deserialize::deserialize(&mut deserializer).ok()?;
    (deserializer.position() == u64::try_from(messagepack.len()).unwrap_or(u64::MAX))
        .then_some(value)
}

fn logical_key_hash(value: &str) -> String {
    use std::fmt::Write;

    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn cache_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "json" || extension == MESSAGEPACK_EXTENSION)
}

fn collect_cache_stems(directory: &Path, output: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cache_stems(&path, output);
        } else if cache_extension(&path)
            && let Some(stem) = path.file_stem().and_then(|value| value.to_str())
        {
            output.insert(stem.to_owned());
        }
    }
}

fn clear_cache_entries(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            clear_cache_entries(&path);
        } else if cache_extension(&path) {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, path::Path};

    use serde_json::json;

    use super::{
        COMPRESSED_MESSAGEPACK_HEADER_BYTES, COMPRESSED_MESSAGEPACK_MAGIC, Cache, CacheOptions,
        MAX_DECOMPRESSED_CACHE_ENTRY_BYTES, decode_messagepack, encode_messagepack,
    };
    use crate::file_hash;

    #[test]
    fn compressed_messagepack_round_trips_and_rejects_invalid_envelopes() {
        let value = json!({
            "source_file": "src/example.py",
            "facts": vec!["repeated deterministic evidence"; 128],
        });
        let encoded = encode_messagepack(&value, Path::new("cache.msgpack"))
            .unwrap_or_else(|_| std::process::abort());
        assert!(encoded.starts_with(COMPRESSED_MESSAGEPACK_MAGIC));
        assert_eq!(decode_messagepack(&encoded), Some(value));
        assert_eq!(
            decode_messagepack::<serde_json::Value>(b"not-a-cache"),
            None
        );

        let mut oversized = Vec::with_capacity(COMPRESSED_MESSAGEPACK_HEADER_BYTES);
        oversized.extend_from_slice(COMPRESSED_MESSAGEPACK_MAGIC);
        oversized.extend_from_slice(
            &(u64::try_from(MAX_DECOMPRESSED_CACHE_ENTRY_BYTES)
                .unwrap_or(u64::MAX)
                .saturating_add(1))
            .to_le_bytes(),
        );
        assert_eq!(decode_messagepack::<serde_json::Value>(&oversized), None);
    }

    #[test]
    fn seeded_hash_reuses_the_extracted_bytes_for_the_same_file_stat() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source.rs");
        let bytes = b"fn source() {}\n";
        fs::write(&source, bytes)?;
        let modified = fs::metadata(&source)?.modified().ok();
        let expected = file_hash(&source, directory.path())?;
        let mut cache = Cache::open(&directory, CacheOptions::output_directory(None))?;
        let extracted_hash = cache.content_hash_from_bytes(&source, bytes);

        assert_eq!(extracted_hash, expected);
        cache.seed_content_hash(
            &source,
            extracted_hash.clone(),
            bytes.len() as u64,
            modified,
        )?;
        assert_eq!(cache.content_hash(&source)?, extracted_hash);
        Ok(())
    }
}

fn prune_cache_entries(directory: &Path, live_hashes: &BTreeSet<String>) -> usize {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            removed += prune_cache_entries(&path, live_hashes);
        } else if cache_extension(&path)
            && path
                .file_stem()
                .and_then(|value| value.to_str())
                .is_some_and(|stem| !live_hashes.contains(stem))
            && fs::remove_file(path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

fn source_items_mut(value: &mut Value, mut visit: impl FnMut(&mut serde_json::Map<String, Value>)) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for bucket in ["nodes", "edges", "hyperedges", "raw_calls"] {
        let Some(items) = object.get_mut(bucket).and_then(Value::as_array_mut) else {
            continue;
        };
        for item in items {
            if let Some(item) = item.as_object_mut() {
                visit(item);
            }
        }
    }
    let Some(facts) = object
        .get_mut("framework_facts")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for fact in facts {
        let Some(anchor) = fact
            .get_mut("fact")
            .and_then(Value::as_object_mut)
            .and_then(|fact| fact.get_mut("anchor"))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        visit(anchor);
    }
}

fn relativize_source_files(value: &mut Value, root: &Path) {
    source_items_mut(value, |item| {
        for key in ["source_file", "sourceFile"] {
            let Some(source) = item.get(key).and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            if source.is_empty() {
                continue;
            }
            let path = Path::new(&source);
            if !path.is_absolute() {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            item.insert(
                key.to_owned(),
                Value::String(relative.to_string_lossy().replace('\\', "/")),
            );
        }
    });
}

fn absolutize_source_files(value: &mut Value, root: &Path) {
    source_items_mut(value, |item| {
        for key in ["source_file", "sourceFile"] {
            let Some(source) = item.get(key).and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            if source.is_empty() || Path::new(&source).is_absolute() {
                continue;
            }
            item.insert(
                key.to_owned(),
                Value::String(root.join(source).to_string_lossy().into_owned()),
            );
        }
    });
}
