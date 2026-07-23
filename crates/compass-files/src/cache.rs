use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rayon::prelude::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{FileError, StatHashIndex, io_error, write_json_atomic};

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
            } => format!(
                "program-merge/ir{ir_schema}/m{merger_version}/a{analyzer_version}"
            ),
        }
    }
}

/// Reader/writer for Graphify's content-addressed extraction cache.
#[derive(Debug)]
pub struct Cache {
    root: PathBuf,
    cache_root: PathBuf,
    output_name: String,
    extractor_version: String,
    hashes: StatHashIndex,
    session_hashes: HashMap<PathBuf, SessionHash>,
}

#[derive(Debug, Clone)]
struct SessionHash {
    size: u64,
    modified: Option<SystemTime>,
    value: String,
}

impl Cache {
    pub fn new(root: impl AsRef<Path>, cache_root: Option<&Path>) -> Result<Self, FileError> {
        let root =
            fs::canonicalize(root.as_ref()).map_err(|source| io_error(root.as_ref(), source))?;
        let cache_root = cache_root.map_or_else(|| root.clone(), Path::to_path_buf);
        let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
        let hashes = StatHashIndex::load(&cache_root, &output_name);
        let cache = Self {
            root,
            cache_root,
            output_name,
            extractor_version: "0.9.20".to_owned(),
            hashes,
            session_hashes: HashMap::new(),
        };
        cache.cleanup_stale_ast();
        Ok(cache)
    }

    pub fn with_extractor_version(mut self, version: impl Into<String>) -> Self {
        self.extractor_version = version.into();
        self
    }

    pub fn directory(&self, kind: &CacheKind, prompt_fingerprint: Option<&str>) -> PathBuf {
        let mut directory = self
            .cache_root
            .join(&self.output_name)
            .join("cache")
            .join(kind.directory_name());
        if matches!(kind, CacheKind::Ast) {
            directory = directory.join(format!("v{}", self.extractor_version));
        } else if let Some(fingerprint) = prompt_fingerprint {
            directory = directory.join(format!("p{fingerprint}"));
        }
        directory
    }

    pub fn load(
        &mut self,
        path: &Path,
        kind: &CacheKind,
        prompt_fingerprint: Option<&str>,
        allow_legacy: bool,
        allow_partial: bool,
    ) -> Result<Option<Value>, FileError> {
        let hash = self.content_hash(path)?;
        let mut entry = self
            .directory(kind, prompt_fingerprint)
            .join(format!("{hash}.json"));
        if !entry.exists() && prompt_fingerprint.is_some() && allow_legacy {
            let legacy = self.directory(kind, None).join(format!("{hash}.json"));
            if legacy.exists() {
                entry = legacy;
            }
        }
        if !entry.exists() {
            return Ok(None);
        }
        let bytes = match fs::read(&entry) {
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
        absolutize_source_files(&mut value, &self.root);
        Ok(Some(value))
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
        let directory = self.directory(kind, prompt_fingerprint);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        write_json_atomic(directory.join(format!("{hash}.json")), &on_disk, false)
    }

    /// Persist a group of cache entries concurrently while retaining the same
    /// content-addressed, atomic on-disk format as [`Self::save`].
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
            jobs.push((directory.join(format!("{hash}.json")), value));
        }
        let root = &self.root;
        jobs.into_par_iter().try_for_each(|(destination, value)| {
            let mut on_disk = value.clone();
            relativize_source_files(&mut on_disk, root);
            write_json_atomic(destination, &on_disk, false)
        })
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
        let entry = self
            .directory(kind, None)
            .join(format!("{}.json", logical_key_hash(logical_key)));
        let bytes = match fs::read(entry) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Ok(None),
        };
        Ok(serde_json::from_slice(&bytes).ok())
    }

    /// Atomically save a repository-relative Program IR cache value.
    pub fn save_program<T: Serialize>(
        &self,
        kind: &CacheKind,
        logical_key: &str,
        value: &T,
    ) -> Result<(), FileError> {
        ensure_program_kind(kind)?;
        let directory = self.directory(kind, None);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        write_json_atomic(
            directory.join(format!("{}.json", logical_key_hash(logical_key))),
            value,
            false,
        )
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
        Ok(prune_json(&self.directory(kind, None), &hashes))
    }

    pub fn flush(&mut self) -> Result<(), FileError> {
        self.hashes.flush()
    }

    pub fn cached_files(&self) -> BTreeSet<String> {
        let base = self.cache_root.join(&self.output_name).join("cache");
        let mut hashes = BTreeSet::new();
        collect_json_stems(&base, &mut hashes);
        hashes
    }

    pub fn clear(&self) {
        let base = self.cache_root.join(&self.output_name).join("cache");
        clear_json(&base);
    }

    pub fn prune_semantic(&self, live_hashes: &BTreeSet<String>) -> usize {
        let base = self.cache_root.join(&self.output_name).join("cache");
        let mut removed = 0;
        for kind in ["semantic", "semantic-deep"] {
            removed += prune_json(&base.join(kind), live_hashes);
        }
        removed
    }

    fn cleanup_stale_ast(&self) {
        let base = self.cache_root.join(&self.output_name).join("cache/ast");
        let current = format!("v{}", self.extractor_version);
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
        let value = self.hashes.hash(path, &self.root)?;
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
}

impl Drop for Cache {
    fn drop(&mut self) {
        let _ = self.flush();
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

fn logical_key_hash(value: &str) -> String {
    use std::fmt::Write;

    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn collect_json_stems(directory: &Path, output: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_stems(&path, output);
        } else if path.extension().is_some_and(|ext| ext == "json")
            && let Some(stem) = path.file_stem().and_then(|value| value.to_str())
        {
            output.insert(stem.to_owned());
        }
    }
}

fn clear_json(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            clear_json(&path);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let _ = fs::remove_file(path);
        }
    }
}

fn prune_json(directory: &Path, live_hashes: &BTreeSet<String>) -> usize {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            removed += prune_json(&path, live_hashes);
        } else if path.extension().is_some_and(|ext| ext == "json")
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
}

fn relativize_source_files(value: &mut Value, root: &Path) {
    source_items_mut(value, |item| {
        let Some(source) = item.get("source_file").and_then(Value::as_str) else {
            return;
        };
        if source.is_empty() {
            return;
        }
        let path = Path::new(source);
        if !path.is_absolute() {
            return;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            return;
        };
        item.insert(
            "source_file".to_owned(),
            Value::String(relative.to_string_lossy().replace('\\', "/")),
        );
    });
}

fn absolutize_source_files(value: &mut Value, root: &Path) {
    source_items_mut(value, |item| {
        let Some(source) = item.get("source_file").and_then(Value::as_str) else {
            return;
        };
        if source.is_empty() {
            return;
        }
        if Path::new(source).is_absolute() {
            return;
        }
        item.insert(
            "source_file".to_owned(),
            Value::String(root.join(source).to_string_lossy().into_owned()),
        );
    });
}
