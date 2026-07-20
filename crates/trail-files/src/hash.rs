use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use md5::Md5;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{FileError, io_error, write_json_atomic};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StatEntry {
    size: u64,
    mtime_ns: u128,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    hashes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    word_count: Option<u64>,
}

/// Persisted `(size, mtime_ns)` fast path compatible with Python's stat index.
#[derive(Debug, Default)]
pub struct StatHashIndex {
    path: PathBuf,
    entries: BTreeMap<String, StatEntry>,
    dirty: bool,
}

impl StatHashIndex {
    pub fn load(cache_root: impl AsRef<Path>, output_name: &str) -> Self {
        let path = cache_root
            .as_ref()
            .join(output_name)
            .join("cache/stat-index.json");
        let entries = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            entries,
            dirty: false,
        }
    }

    pub fn hash(&mut self, path: &Path, root: &Path) -> Result<String, FileError> {
        let resolved = fs::canonicalize(path).map_err(|source| io_error(path, source))?;
        if !resolved.is_file() {
            return Err(FileError::NotAFile(path.to_path_buf()));
        }
        let root = fs::canonicalize(root).map_err(|source| io_error(root, source))?;
        let salt = resolved
            .strip_prefix(&root)
            .unwrap_or(&resolved)
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        let metadata = fs::metadata(path).map_err(|source| io_error(path, source))?;
        let mtime_ns = modified_ns(&metadata);
        let key = resolved.to_string_lossy().into_owned();
        if let Some(entry) = self.entries.get(&key)
            && entry.size == metadata.len()
            && entry.mtime_ns == mtime_ns
            && let Some(hash) = entry.hashes.get(&salt)
        {
            return Ok(hash.clone());
        }

        let digest = file_hash_bytes(path, &salt)?;
        let entry = self.entries.entry(key).or_default();
        if entry.size != metadata.len() || entry.mtime_ns != mtime_ns {
            *entry = StatEntry {
                size: metadata.len(),
                mtime_ns,
                ..StatEntry::default()
            };
        }
        entry.hashes.insert(salt, digest.clone());
        self.dirty = true;
        Ok(digest)
    }

    pub fn word_count<F>(&mut self, path: &Path, compute: F) -> u64
    where
        F: FnOnce(&Path) -> u64,
    {
        let Ok(resolved) = fs::canonicalize(path) else {
            return compute(path);
        };
        let Ok(metadata) = fs::metadata(path) else {
            return compute(path);
        };
        let mtime_ns = modified_ns(&metadata);
        let key = resolved.to_string_lossy().into_owned();
        if let Some(entry) = self.entries.get(&key)
            && entry.size == metadata.len()
            && entry.mtime_ns == mtime_ns
            && let Some(count) = entry.word_count
        {
            return count;
        }
        let count = compute(path);
        let entry = self.entries.entry(key).or_default();
        if entry.size != metadata.len() || entry.mtime_ns != mtime_ns {
            *entry = StatEntry {
                size: metadata.len(),
                mtime_ns,
                ..StatEntry::default()
            };
        }
        entry.word_count = Some(count);
        self.dirty = true;
        count
    }

    pub fn flush(&mut self) -> Result<(), FileError> {
        if self.dirty {
            write_json_atomic(&self.path, &self.entries, false)?;
            self.dirty = false;
        }
        Ok(())
    }
}

fn modified_ns(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos())
}

pub fn body_content(content: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(content);
    let mut offset = 0;
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return content.to_vec();
    };
    if first
        .trim_end_matches(['\r', '\n'])
        .trim_end_matches([' ', '\t'])
        != "---"
    {
        return content.to_vec();
    }
    offset += first.len();
    for line in lines {
        let without_newline = line.trim_end_matches(['\r', '\n']);
        if without_newline.trim_end_matches([' ', '\t']) == "---" {
            let delimiter_start = offset;
            return text[delimiter_start + 3..].as_bytes().to_vec();
        }
        offset += line.len();
    }
    content.to_vec()
}

fn file_hash_bytes(path: &Path, salt: &str) -> Result<String, FileError> {
    let raw = fs::read(path).map_err(|source| io_error(path, source))?;
    let content = if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    {
        body_content(&raw)
    } else {
        raw
    };
    let mut hash = Sha256::new();
    hash.update(&content);
    hash.update([0]);
    hash.update(salt.as_bytes());
    Ok(format!("{:x}", hash.finalize()))
}

pub fn file_hash(path: &Path, root: &Path) -> Result<String, FileError> {
    let resolved = fs::canonicalize(path).map_err(|source| io_error(path, source))?;
    if !resolved.is_file() {
        return Err(FileError::NotAFile(path.to_path_buf()));
    }
    let root = fs::canonicalize(root).map_err(|source| io_error(root, source))?;
    let salt = resolved
        .strip_prefix(&root)
        .unwrap_or(&resolved)
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    file_hash_bytes(path, &salt)
}

pub fn md5_file(path: &Path) -> Result<String, FileError> {
    let file = fs::File::open(path).map_err(|source| io_error(path, source))?;
    let mut reader = BufReader::new(file);
    let mut hash = Md5::new();
    let mut buffer = [0_u8; 65_536];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|source| io_error(path, source))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn prompt_fingerprint(prompt: &str) -> String {
    let normalized = prompt
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();
    let mut hash = Sha256::new();
    hash.update(normalized.as_bytes());
    format!("{:x}", hash.finalize())[..12].to_owned()
}
