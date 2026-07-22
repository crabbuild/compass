use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::{FileError, io_error};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

fn atomic_replace<F>(path: &Path, write: F) -> Result<(), FileError>
where
    F: FnOnce(&mut BufWriter<File>) -> Result<(), FileError>,
{
    let destination = resolved_destination(path);
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    let temporary = temporary_path(&destination);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|source| io_error(&temporary, source))?;

    let result = (|| {
        let mut writer = BufWriter::new(file);
        write(&mut writer)?;
        writer
            .flush()
            .map_err(|source| io_error(&temporary, source))?;
        drop(writer);

        if let Ok(metadata) = fs::metadata(&destination) {
            fs::set_permissions(&temporary, metadata.permissions())
                .map_err(|source| io_error(&temporary, source))?;
        }

        #[cfg(windows)]
        if destination.exists() {
            match fs::rename(&temporary, &destination) {
                Ok(()) => return Ok(()),
                Err(_) => {
                    fs::copy(&temporary, &destination)
                        .map_err(|source| io_error(&destination, source))?;
                    fs::remove_file(&temporary).map_err(|source| io_error(&temporary, source))?;
                    return Ok(());
                }
            }
        }

        fs::rename(&temporary, &destination).map_err(|source| io_error(&destination, source))
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
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

    use super::write_json_ascii_atomic;

    #[test]
    fn streams_python_compatible_ascii_json_with_optional_newline() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = directory.path().join("ascii.json");
        write_json_ascii_atomic(&path, &json!({"text": "café 🦀"}), false, true)
            .unwrap_or_else(|_| std::process::abort());
        let encoded = fs::read_to_string(path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(encoded, "{\"text\":\"caf\\u00e9 \\ud83e\\udd80\"}\n");
    }
}
