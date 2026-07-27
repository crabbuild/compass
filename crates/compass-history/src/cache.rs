use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::store::{create_owner_dir, reject_directory, reject_symlink};
use crate::{HistoryError, canonical_json_bytes};

pub const HISTORY_CACHE_VERSION: u32 = 1;
const CACHE_ENTRY_SCHEMA: &str = "compass.history.cache_entry/1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DerivedCacheNamespace {
    SemanticDiff,
    Viewer,
}

impl DerivedCacheNamespace {
    fn as_str(self) -> &'static str {
        match self {
            Self::SemanticDiff => "semantic-diff",
            Self::Viewer => "viewer",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HistoryCache {
    root: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CacheNamespaceStatus {
    pub files: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CacheStatus {
    pub files: u64,
    pub bytes: u64,
    pub namespaces: BTreeMap<String, CacheNamespaceStatus>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CacheGcPlan {
    pub files: u64,
    pub bytes: u64,
    pub paths: Vec<PathBuf>,
}

#[derive(Deserialize, Serialize)]
struct CacheEnvelope {
    schema: String,
    namespace: String,
    key_sha256: String,
    payload_sha256: String,
    payload: Value,
}

impl HistoryCache {
    pub(crate) fn open(history_root: &Path) -> Result<Self, HistoryError> {
        reject_directory(history_root)?;
        let cache = history_root.join("cache");
        create_owner_dir(&cache)?;
        let root = cache.join(format!("v{HISTORY_CACHE_VERSION}"));
        create_owner_dir(&root)?;
        Ok(Self { root })
    }

    #[must_use]
    pub fn extraction_root(&self) -> &Path {
        &self.root
    }

    pub fn read(
        &self,
        namespace: DerivedCacheNamespace,
        key_material: &Value,
        max_payload_bytes: u64,
    ) -> Result<Option<Vec<u8>>, HistoryError> {
        let key = digest(&canonical_json_bytes(key_material)?);
        let path = self.entry_path(namespace, &key)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(crate::error::io_error(path, source)),
        };
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_payload_bytes {
            return Ok(None);
        }
        let envelope: CacheEnvelope = match serde_json::from_slice(&bytes) {
            Ok(envelope) => envelope,
            Err(_) => return Ok(None),
        };
        if envelope.schema != CACHE_ENTRY_SCHEMA
            || envelope.namespace != namespace.as_str()
            || envelope.key_sha256 != key
        {
            return Ok(None);
        }
        let payload = canonical_json_bytes(&envelope.payload)?;
        if digest(&payload) != envelope.payload_sha256 {
            return Ok(None);
        }
        Ok(Some(payload))
    }

    pub fn write(
        &self,
        namespace: DerivedCacheNamespace,
        key_material: &Value,
        payload: &[u8],
    ) -> Result<(), HistoryError> {
        let payload: Value = serde_json::from_slice(payload)?;
        let payload_bytes = canonical_json_bytes(&payload)?;
        let key = digest(&canonical_json_bytes(key_material)?);
        let path = self.entry_path(namespace, &key)?;
        let envelope = CacheEnvelope {
            schema: CACHE_ENTRY_SCHEMA.to_owned(),
            namespace: namespace.as_str().to_owned(),
            key_sha256: key,
            payload_sha256: digest(&payload_bytes),
            payload,
        };
        compass_files::write_json_atomic(path, &envelope, false)?;
        Ok(())
    }

    pub fn status(&self) -> Result<CacheStatus, HistoryError> {
        let mut status = CacheStatus::default();
        for file in self.files()? {
            let namespace = file
                .path
                .strip_prefix(&self.root)
                .ok()
                .and_then(|path| path.components().next())
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".to_owned());
            status.files = status.files.saturating_add(1);
            status.bytes = status.bytes.saturating_add(file.bytes);
            let entry = status.namespaces.entry(namespace).or_default();
            entry.files = entry.files.saturating_add(1);
            entry.bytes = entry.bytes.saturating_add(file.bytes);
        }
        Ok(status)
    }

    pub fn plan_gc(
        &self,
        max_bytes: Option<u64>,
        max_age: Option<Duration>,
    ) -> Result<CacheGcPlan, HistoryError> {
        if max_bytes.is_none() && max_age.is_none() {
            return Err(HistoryError::OperationalState(
                "cache GC requires a byte or age limit".to_owned(),
            ));
        }
        let now = SystemTime::now();
        let mut files = self.files()?;
        files.sort_by(|left, right| {
            left.modified
                .cmp(&right.modified)
                .then_with(|| left.path.cmp(&right.path))
        });
        let mut selected = BTreeMap::<PathBuf, u64>::new();
        if let Some(max_age) = max_age {
            for file in &files {
                if now
                    .duration_since(file.modified)
                    .is_ok_and(|age| age > max_age)
                {
                    selected.insert(file.path.clone(), file.bytes);
                }
            }
        }
        if let Some(max_bytes) = max_bytes {
            let mut retained = files
                .iter()
                .filter(|file| !selected.contains_key(&file.path))
                .map(|file| file.bytes)
                .sum::<u64>();
            for file in &files {
                if retained <= max_bytes {
                    break;
                }
                if selected.contains_key(&file.path) {
                    continue;
                }
                retained = retained.saturating_sub(file.bytes);
                selected.insert(file.path.clone(), file.bytes);
            }
        }
        Ok(CacheGcPlan {
            files: selected.len() as u64,
            bytes: selected.values().sum(),
            paths: selected.into_keys().collect(),
        })
    }

    pub fn sweep(&self, plan: &CacheGcPlan) -> Result<(), HistoryError> {
        for path in &plan.paths {
            let parent = path.parent().ok_or_else(|| {
                HistoryError::OperationalState("cache GC path has no parent".to_owned())
            })?;
            let parent = fs::canonicalize(parent)
                .map_err(|source| crate::error::io_error(parent, source))?;
            if !parent.starts_with(&self.root) {
                return Err(HistoryError::OperationalState(format!(
                    "cache GC path escapes cache root: {}",
                    path.display()
                )));
            }
            let metadata = fs::symlink_metadata(path)
                .map_err(|source| crate::error::io_error(path, source))?;
            if !metadata.file_type().is_file() {
                return Err(HistoryError::OperationalState(format!(
                    "cache GC target is not a regular file: {}",
                    path.display()
                )));
            }
            fs::remove_file(path).map_err(|source| crate::error::io_error(path, source))?;
        }
        Ok(())
    }

    fn entry_path(
        &self,
        namespace: DerivedCacheNamespace,
        key: &str,
    ) -> Result<PathBuf, HistoryError> {
        let directory = self.root.join(namespace.as_str());
        create_owner_dir(&directory)?;
        let path = directory.join(format!("{key}.json"));
        reject_symlink(&path, true)?;
        Ok(path)
    }

    fn files(&self) -> Result<Vec<CacheFile>, HistoryError> {
        let mut pending = vec![self.root.clone()];
        let mut files = Vec::new();
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory)
                .map_err(|source| crate::error::io_error(&directory, source))?
            {
                let entry = entry.map_err(|source| crate::error::io_error(&directory, source))?;
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|source| crate::error::io_error(&path, source))?;
                if metadata.file_type().is_symlink() {
                    return Err(HistoryError::OperationalState(format!(
                        "cache contains a symlink: {}",
                        path.display()
                    )));
                }
                if metadata.is_dir() {
                    pending.push(path);
                } else if metadata.is_file() {
                    files.push(CacheFile {
                        path,
                        bytes: metadata.len(),
                        modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    });
                }
            }
        }
        Ok(files)
    }
}

struct CacheFile {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
