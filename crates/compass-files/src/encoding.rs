use std::fs;
use std::path::Path;

use crate::{FileError, io_error};

/// Read source bytes with Python's `errors="replace"` UTF-8 behavior.
pub fn read_source_lossy(path: &Path, max_bytes: u64) -> Result<String, FileError> {
    let metadata = fs::metadata(path).map_err(|source| io_error(path, source))?;
    if metadata.len() > max_bytes {
        return Err(FileError::TooLarge {
            path: path.to_path_buf(),
            limit: max_bytes,
        });
    }
    let bytes = fs::read(path).map_err(|source| io_error(path, source))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
