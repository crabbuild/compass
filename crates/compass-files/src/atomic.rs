use std::fs::{self, File, OpenOptions};
#[cfg(target_vendor = "apple")]
use std::io::Read;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{FileError, io_error};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const ATOMIC_WRITE_BUFFER_BYTES: usize = 1024 * 1024;
#[cfg(windows)]
const WINDOWS_REPLACE_ATTEMPTS: u32 = 8;
#[cfg(windows)]
const WINDOWS_REPLACE_RETRY_MILLIS: u64 = 10;

fn resolved_destination(path: &Path) -> PathBuf {
    if path.is_symlink() {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("compass");
    destination.with_file_name(format!(".{name}.{pid}.{sequence}.tmp"))
}

#[cfg(target_vendor = "apple")]
fn unpredictable_temporary_path(destination: &Path) -> Result<PathBuf, FileError> {
    let mut nonce = [0_u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut nonce))
        .map_err(|error| io_error(destination, error))?;
    let pid = std::process::id();
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("compass");
    Ok(destination.with_file_name(format!(
        ".{name}.{pid}.{:032x}.tmp",
        u128::from_ne_bytes(nonce)
    )))
}

fn atomic_replace<F>(path: &Path, write: F) -> Result<(), FileError>
where
    F: FnOnce(&mut BufWriter<File>) -> Result<(), FileError>,
{
    let destination = resolved_destination(path);
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    let temporary = temporary_path(&destination);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|source| io_error(&temporary, source))?;

    let result = (|| {
        let mut writer = BufWriter::with_capacity(ATOMIC_WRITE_BUFFER_BYTES, file);
        write(&mut writer)?;
        writer
            .flush()
            .map_err(|source| io_error(&temporary, source))?;
        let file = writer
            .into_inner()
            .map_err(|error| io_error(&temporary, error.into_error()))?;

        if let Ok(metadata) = fs::metadata(&destination) {
            fs::set_permissions(&temporary, metadata.permissions())
                .map_err(|source| io_error(&temporary, source))?;
        }
        file.sync_all()
            .map_err(|source| io_error(&temporary, source))?;
        drop(file);

        #[cfg(windows)]
        {
            replace_atomic_windows(&temporary, &destination)
                .map_err(|source| io_error(&destination, source))?;
        }

        #[cfg(not(windows))]
        fs::rename(&temporary, &destination).map_err(|source| io_error(&destination, source))?;

        sync_directory(parent)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Atomically copy an existing file using the platform's native copy path.
///
/// Native copies can use copy-on-write clones or in-kernel copying while the
/// temporary-file, sync, and rename sequence preserves the same publication
/// guarantees as the streaming atomic writers.
pub(crate) fn copy_file_atomic(source: &Path, path: &Path) -> Result<(), FileError> {
    let destination = resolved_destination(path);
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    #[cfg(target_vendor = "apple")]
    let temporary = {
        // Keep the destination absent so `fs::copy` can use clonefile on
        // APFS. An OS-random name prevents another process from predicting
        // and replacing the path before clonefile creates it.
        unpredictable_temporary_path(&destination)?
    };
    #[cfg(not(target_vendor = "apple"))]
    let temporary = {
        let temporary = temporary_path(&destination);
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| io_error(&temporary, error))?;
        temporary
    };

    let result = (|| {
        let expected_bytes = fs::metadata(source)
            .map_err(|error| io_error(source, error))?
            .len();
        let copied_bytes =
            fs::copy(source, &temporary).map_err(|error| io_error(&temporary, error))?;
        if copied_bytes != expected_bytes {
            return Err(io_error(
                &temporary,
                std::io::Error::other(format!(
                    "native copy wrote {copied_bytes} bytes; expected {expected_bytes}"
                )),
            ));
        }
        if let Ok(metadata) = fs::metadata(&destination) {
            fs::set_permissions(&temporary, metadata.permissions())
                .map_err(|error| io_error(&temporary, error))?;
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&temporary)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_error(&temporary, error))?;

        #[cfg(windows)]
        {
            replace_atomic_windows(&temporary, &destination)
                .map_err(|error| io_error(&destination, error))?;
        }

        #[cfg(not(windows))]
        fs::rename(&temporary, &destination).map_err(|error| io_error(&destination, error))?;

        sync_directory(parent)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn replace_atomic_windows(source: &Path, destination: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match atomicwrites::replace_atomic(source, destination) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < WINDOWS_REPLACE_ATTEMPTS
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::PermissionDenied
                            | std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::Interrupted
                    ) =>
            {
                attempt += 1;
                thread::sleep(Duration::from_millis(
                    WINDOWS_REPLACE_RETRY_MILLIS * u64::from(attempt),
                ));
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), FileError> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| io_error(path, source))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

pub fn write_text_atomic(path: impl AsRef<Path>, text: &str) -> Result<(), FileError> {
    atomic_replace(path.as_ref(), |writer| {
        writer
            .write_all(text.as_bytes())
            .map_err(|source| io_error(path.as_ref(), source))
    })
}

pub fn write_bytes_atomic(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), FileError> {
    atomic_replace(path.as_ref(), |writer| {
        writer
            .write_all(bytes)
            .map_err(|source| io_error(path.as_ref(), source))
    })
}

/// Atomically publish bytes produced by a bounded streaming callback.
///
/// The callback receives the same buffered writer and durability guarantees as
/// the typed JSON helpers, while callers can choose a specialized serializer
/// without first materializing the complete artifact.
pub fn write_atomic_with<F, E>(path: impl AsRef<Path>, write: F) -> Result<(), E>
where
    F: FnOnce(&mut dyn Write) -> Result<(), E>,
    E: From<FileError>,
{
    let mut callback_error = None;
    let atomic_result = atomic_replace(path.as_ref(), |writer| {
        if let Err(error) = write(writer) {
            callback_error = Some(error);
            return Err(io_error(
                path.as_ref(),
                std::io::Error::other("atomic byte stream callback failed"),
            ));
        }
        Ok(())
    });
    if let Some(error) = callback_error {
        return Err(error);
    }
    atomic_result.map_err(E::from)
}

pub fn write_json_atomic<T: Serialize>(
    path: impl AsRef<Path>,
    value: &T,
    pretty: bool,
) -> Result<(), FileError> {
    atomic_replace(path.as_ref(), |writer| {
        if pretty {
            serde_json::to_writer_pretty(writer, value)
        } else {
            serde_json::to_writer(writer, value)
        }
        .map_err(|source| FileError::Json {
            path: path.as_ref().to_path_buf(),
            source,
        })
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AtomicJsonDigest {
    pub sha256: String,
    pub bytes: u64,
}

/// Atomically serialize compact JSON and return the digest of the exact bytes
/// that reached the staging file. The digest is collected in the same bounded
/// streaming pass, avoiding a second serialization or file read.
pub fn write_json_atomic_with_digest<T: Serialize>(
    path: impl AsRef<Path>,
    value: &T,
) -> Result<AtomicJsonDigest, FileError> {
    let mut receipt = None;
    atomic_replace(path.as_ref(), |writer| {
        let mut hashing = HashingWriter::new(writer);
        serde_json::to_writer(&mut hashing, value).map_err(|source| FileError::Json {
            path: path.as_ref().to_path_buf(),
            source,
        })?;
        receipt = Some(hashing.finish());
        Ok(())
    })?;
    receipt.ok_or_else(|| {
        io_error(
            path.as_ref(),
            std::io::Error::other("JSON digest receipt was not produced"),
        )
    })
}

/// Atomically produce a bounded stream of bytes while returning the digest of
/// the exact bytes that reached the staging file.
pub fn write_atomic_with_digest<F, E>(
    path: impl AsRef<Path>,
    write: F,
) -> Result<AtomicJsonDigest, E>
where
    F: FnOnce(&mut dyn Write) -> Result<(), E>,
    E: From<FileError>,
{
    let mut receipt = None;
    let mut callback_error = None;
    let atomic_result = atomic_replace(path.as_ref(), |writer| {
        let mut hashing = HashingWriter::new(writer);
        if let Err(error) = write(&mut hashing) {
            callback_error = Some(error);
            return Err(io_error(
                path.as_ref(),
                std::io::Error::other("atomic byte stream callback failed"),
            ));
        }
        receipt = Some(hashing.finish());
        Ok(())
    });
    if let Some(error) = callback_error {
        return Err(error);
    }
    atomic_result.map_err(E::from)?;
    receipt.ok_or_else(|| {
        E::from(io_error(
            path.as_ref(),
            std::io::Error::other("atomic byte stream did not produce a digest receipt"),
        ))
    })
}

struct HashingWriter<'a, W> {
    inner: &'a mut W,
    hasher: Sha256,
    bytes: u64,
}

impl<'a, W> HashingWriter<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes: 0,
        }
    }

    fn finish(self) -> AtomicJsonDigest {
        AtomicJsonDigest {
            sha256: format!("{:x}", self.hasher.finalize()),
            bytes: self.bytes,
        }
    }
}

impl<W: Write> Write for HashingWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.inner.write_all(bytes)?;
        self.hasher.update(bytes);
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("JSON byte count overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Atomically serialize JSON while escaping every non-ASCII scalar exactly as
/// Python's default `json.dump(..., ensure_ascii=True)` does.
///
/// Unlike serializing to a `String` and escaping it afterward, this adapter
/// keeps memory proportional to the buffered writer rather than the document.
pub fn write_json_ascii_atomic<T: Serialize>(
    path: impl AsRef<Path>,
    value: &T,
    pretty: bool,
    trailing_newline: bool,
) -> Result<(), FileError> {
    atomic_replace(path.as_ref(), |writer| {
        {
            let mut ascii = AsciiJsonWriter { inner: writer };
            let result = if pretty {
                serde_json::to_writer_pretty(&mut ascii, value)
            } else {
                serde_json::to_writer(&mut ascii, value)
            };
            result.map_err(|source| FileError::Json {
                path: path.as_ref().to_path_buf(),
                source,
            })?;
        }
        if trailing_newline {
            writer
                .write_all(b"\n")
                .map_err(|source| io_error(path.as_ref(), source))?;
        }
        Ok(())
    })
}

struct AsciiJsonWriter<'a, W> {
    inner: &'a mut W,
}

impl<W: Write> Write for AsciiJsonWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.is_ascii() {
            self.inner.write_all(bytes)?;
            return Ok(bytes.len());
        }
        let text = std::str::from_utf8(bytes).map_err(|error| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
        })?;
        let mut start = 0;
        for (offset, character) in text.char_indices() {
            if character.is_ascii() {
                continue;
            }
            self.inner.write_all(&bytes[start..offset])?;
            let code = character as u32;
            if code <= 0xffff {
                write!(self.inner, "\\u{code:04x}")?;
            } else {
                let scalar = code - 0x1_0000;
                write!(
                    self.inner,
                    "\\u{:04x}\\u{:04x}",
                    0xd800 + (scalar >> 10),
                    0xdc00 + (scalar & 0x3ff)
                )?;
            }
            start = offset + character.len_utf8();
        }
        self.inner.write_all(&bytes[start..])?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use sha2::{Digest, Sha256};

    use super::{write_atomic_with_digest, write_json_ascii_atomic, write_json_atomic_with_digest};

    #[test]
    fn streams_python_compatible_ascii_json_with_optional_newline() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = directory.path().join("ascii.json");
        write_json_ascii_atomic(&path, &json!({"text": "café 🦀"}), false, true)
            .unwrap_or_else(|_| std::process::abort());
        let encoded = fs::read_to_string(path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(encoded, "{\"text\":\"caf\\u00e9 \\ud83e\\udd80\"}\n");
    }

    #[test]
    fn compact_json_digest_matches_the_published_bytes() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = directory.path().join("graph.json");
        let receipt = write_json_atomic_with_digest(&path, &json!({"value": [1, 2, 3]}))
            .unwrap_or_else(|_| std::process::abort());
        let bytes = fs::read(path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(receipt.bytes, bytes.len() as u64);
        assert_eq!(receipt.sha256, format!("{:x}", Sha256::digest(&bytes)));
    }

    #[test]
    fn streamed_digest_matches_callback_bytes() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = directory.path().join("streamed.bin");
        let receipt = write_atomic_with_digest(&path, |writer| {
            writer
                .write_all(b"streamed bytes")
                .map_err(|source| super::FileError::Io {
                    path: path.clone(),
                    source,
                })
        })
        .unwrap_or_else(|_| std::process::abort());
        let bytes = fs::read(path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(receipt.bytes, bytes.len() as u64);
        assert_eq!(receipt.sha256, format!("{:x}", Sha256::digest(&bytes)));
    }
}
