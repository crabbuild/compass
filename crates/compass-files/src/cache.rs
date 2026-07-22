use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{FileError, StatHashIndex, io_error, write_json_atomic};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheKind {
    Ast,
    Semantic,
    SemanticMode(String),
}

impl CacheKind {
    fn directory_name(&self) -> String {
        match self {
            Self::Ast => "ast".to_owned(),
            Self::Semantic => "semantic".to_owned(),
            Self::SemanticMode(mode) => format!("semantic-{mode}"),
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
        let hash = self.hashes.hash(path, &self.root)?;
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
        let hash = self.hashes.hash(path, &self.root)?;
        let directory = self.directory(kind, prompt_fingerprint);
        fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
        write_json_atomic(directory.join(format!("{hash}.json")), &on_disk, false)
    }

    pub fn flush(&mut self) -> Result<(), FileError> {
        self.hashes.flush()
    }

    pub fn cached_files(&self) -> BTreeSet<String> {
        let base = self.cache_root.join(&self.output_name).join("cache");
        let mut hashes = BTreeSet::new();
        if let Ok(entries) = fs::read_dir(&base) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && path.extension().is_some_and(|ext| ext == "json")
                    && let Some(stem) = path.file_stem().and_then(|value| value.to_str())
                {
                    hashes.insert(stem.to_owned());
                }
            }
        }
        for kind in ["ast", "semantic", "semantic-deep"] {
            collect_json_stems(&base.join(kind), &mut hashes);
        }
        hashes
    }

    pub fn clear(&self) {
        let base = self.cache_root.join(&self.output_name).join("cache");
        if let Ok(entries) = fs::read_dir(&base) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
                    let _ = fs::remove_file(path);
                }
            }
        }
        for kind in ["ast", "semantic", "semantic-deep"] {
            clear_json(&base.join(kind));
        }
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
}

impl Drop for Cache {
    fn drop(&mut self) {
        let _ = self.flush();
    }
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
