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
        .unwrap_or("trail");
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
