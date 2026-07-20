use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DetectOptions, Detection, FileError, detect, md5_file, write_json_atomic};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    Ast,
    Semantic,
    Both,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub mtime: f64,
    #[serde(default)]
    pub ast_hash: String,
    #[serde(default)]
    pub semantic_hash: String,
    #[serde(skip)]
    legacy_mtime_only: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Manifest {
    entries: BTreeMap<String, ManifestEntry>,
}

#[derive(Debug, Clone)]
pub struct IncrementalDetection {
    pub detection: Detection,
    pub new_files: BTreeMap<String, Vec<String>>,
    pub unchanged_files: BTreeMap<String, Vec<String>>,
    pub new_total: usize,
    pub deleted_files: Vec<String>,
    pub excluded_files: Vec<String>,
}

impl Manifest {
    pub fn load(path: &Path, root: Option<&Path>) -> Self {
        let value = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let Some(Value::Object(object)) = value else {
            return Self::default();
        };
        let mut entries = BTreeMap::new();
        for (key, value) in object {
            let key = root.map_or(key.clone(), |root| absolute_from_storage(&key, root));
            if let Some(entry) = normalize_entry(&value) {
                entries.insert(key, entry);
            }
        }
        Self { entries }
    }

    pub fn entries(&self) -> &BTreeMap<String, ManifestEntry> {
        &self.entries
    }

    /// Return true when the detected corpus is exactly the manifest corpus and
    /// every relevant content stamp is still valid.
    #[must_use]
    pub fn is_unchanged(&self, files: &BTreeMap<String, Vec<String>>, kind: ManifestKind) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let current = files.values().flatten().cloned().collect::<BTreeSet<_>>();
        let stored = self.entries.keys().cloned().collect::<BTreeSet<_>>();
        current == stored
            && current
                .into_iter()
                .all(|file| !changed(Path::new(&file), self.entries.get(&file), kind))
    }

    pub fn save(
        &mut self,
        files: &BTreeMap<String, Vec<String>>,
        path: &Path,
        kind: ManifestKind,
        root: Option<&Path>,
        scan_corpus: Option<&BTreeSet<String>>,
        clear_semantic: Option<&BTreeSet<String>>,
    ) -> Result<(), FileError> {
        let root_resolved = root.and_then(|value| fs::canonicalize(value).ok());
        self.entries.retain(|file, _| {
            let path = Path::new(file);
            if !path.exists() {
                return false;
            }
            if let (Some(scan), Some(root)) = (scan_corpus, root_resolved.as_deref())
                && under_root(path, root)
                && !set_contains_path(scan, path)
            {
                return false;
            }
            true
        });
        if let Some(clear) = clear_semantic {
            for (file, entry) in &mut self.entries {
                if set_contains_path(clear, Path::new(file)) {
                    entry.semantic_hash.clear();
                }
            }
        }

        let all_files = files.values().flatten().cloned().collect::<Vec<_>>();
        let hashed = all_files
            .par_iter()
            .filter_map(|file| {
                let path = Path::new(file);
                let metadata = fs::metadata(path).ok()?;
                let hash = md5_file(path).ok()?;
                Some((file.clone(), modified_seconds(&metadata), hash))
            })
            .collect::<Vec<_>>();
        for (file, mtime, hash) in hashed {
            let previous = self.entries.get(&file).cloned().unwrap_or_default();
            let ast_hash = if matches!(kind, ManifestKind::Ast | ManifestKind::Both) {
                hash.clone()
            } else {
                previous.ast_hash.clone()
            };
            let semantic_hash = if matches!(kind, ManifestKind::Semantic | ManifestKind::Both) {
                hash.clone()
            } else if hash == previous.ast_hash {
                previous.semantic_hash
            } else {
                String::new()
            };
            self.entries.insert(
                file,
                ManifestEntry {
                    mtime,
                    ast_hash,
                    semantic_hash,
                    legacy_mtime_only: false,
                },
            );
        }

        let on_disk = self
            .entries
            .iter()
            .map(|(key, value)| {
                let key = root.map_or_else(|| key.clone(), |root| relative_for_storage(key, root));
                (key, value)
            })
            .collect::<BTreeMap<_, _>>();
        write_json_atomic(path, &on_disk, true)
    }

    pub fn incremental(
        root: &Path,
        manifest_path: &Path,
        options: &DetectOptions,
        kind: ManifestKind,
    ) -> Result<IncrementalDetection, FileError> {
        let detection = detect(root, options)?;
        let manifest = Self::load(manifest_path, Some(root));
        let empty_buckets = detection
            .files
            .keys()
            .map(|key| (key.clone(), Vec::new()))
            .collect::<BTreeMap<_, _>>();
        if manifest.entries.is_empty() {
            return Ok(IncrementalDetection {
                new_files: detection.files.clone(),
                unchanged_files: empty_buckets,
                new_total: detection.total_files,
                deleted_files: Vec::new(),
                excluded_files: Vec::new(),
                detection,
            });
        }

        let mut new_files = empty_buckets.clone();
        let mut unchanged_files = empty_buckets;
        for (file_type, files) in &detection.files {
            for file in files {
                let changed = changed(Path::new(file), manifest.entries.get(file), kind);
                let target = if changed {
                    &mut new_files
                } else {
                    &mut unchanged_files
                };
                if let Some(bucket) = target.get_mut(file_type) {
                    bucket.push(file.clone());
                }
            }
        }
        let current = detection
            .files
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut deleted_files = Vec::new();
        let mut excluded_files = Vec::new();
        for file in manifest.entries.keys() {
            if current.contains(file) {
                continue;
            }
            if Path::new(file).exists() {
                excluded_files.push(file.clone());
            } else {
                deleted_files.push(file.clone());
            }
        }
        let new_total = new_files.values().map(Vec::len).sum();
        Ok(IncrementalDetection {
            detection,
            new_files,
            unchanged_files,
            new_total,
            deleted_files,
            excluded_files,
        })
    }
}

fn normalize_entry(value: &Value) -> Option<ManifestEntry> {
    if let Some(mtime) = value.as_f64() {
        return Some(ManifestEntry {
            mtime,
            legacy_mtime_only: true,
            ..ManifestEntry::default()
        });
    }
    let object = value.as_object()?;
    let mtime = object
        .get("mtime")
        .and_then(Value::as_f64)
        .unwrap_or_default();
    let ast_hash = object
        .get("ast_hash")
        .or_else(|| object.get("hash"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let semantic_hash = object
        .get("semantic_hash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Some(ManifestEntry {
        mtime,
        ast_hash,
        semantic_hash,
        legacy_mtime_only: false,
    })
}

fn changed(path: &Path, stored: Option<&ManifestEntry>, kind: ManifestKind) -> bool {
    let Some(stored) = stored else {
        return true;
    };
    let current_mtime = fs::metadata(path).map_or(0.0, |metadata| modified_seconds(&metadata));
    if stored.legacy_mtime_only {
        return current_mtime != stored.mtime;
    }
    let hash = if matches!(kind, ManifestKind::Semantic) {
        &stored.semantic_hash
    } else {
        &stored.ast_hash
    };
    if hash.is_empty() {
        return true;
    }
    current_mtime != stored.mtime && md5_file(path).map_or(true, |current| current != *hash)
}

fn modified_seconds(metadata: &fs::Metadata) -> f64 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |value| value.as_secs_f64())
}

fn absolute_from_storage(key: &str, root: &Path) -> String {
    let path = Path::new(key);
    if path.is_absolute() {
        key.to_owned()
    } else {
        fs::canonicalize(root)
            .unwrap_or_else(|_| root.to_path_buf())
            .join(path)
            .to_string_lossy()
            .into_owned()
    }
}

fn relative_for_storage(key: &str, root: &Path) -> String {
    let path = Path::new(key);
    if !path.is_absolute() {
        return key.to_owned();
    }
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(root).map_or_else(
        |_| key.to_owned(),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    )
}

fn under_root(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).is_ok()
        || fs::canonicalize(path).is_ok_and(|path| path.strip_prefix(root).is_ok())
}

fn set_contains_path(paths: &BTreeSet<String>, path: &Path) -> bool {
    let direct = path.to_string_lossy();
    paths.contains(direct.as_ref())
        || fs::canonicalize(path)
            .ok()
            .is_some_and(|path| paths.contains(path.to_string_lossy().as_ref()))
}
