use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use crate::{FileError, io_error};

const READ_PREALLOC_MAX_BYTES: u64 = 1024 * 1024;

/// Read a regular file without ever consuming more than `max_bytes + 1` bytes.
///
/// The second, stream-level bound is intentional: metadata is only a snapshot,
/// and a file may grow or be replaced between inspection and the final read.
pub fn read_bytes_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>, FileError> {
    let path_metadata = fs::metadata(path).map_err(|source| io_error(path, source))?;
    if !path_metadata.is_file() {
        return Err(FileError::NotAFile(path.to_path_buf()));
    }
    if path_metadata.len() > max_bytes {
        return Err(FileError::TooLarge {
            path: path.to_path_buf(),
            limit: max_bytes,
        });
    }

    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let opened_metadata = file.metadata().map_err(|source| io_error(path, source))?;
    if !opened_metadata.is_file() {
        return Err(FileError::NotAFile(path.to_path_buf()));
    }
    if opened_metadata.len() > max_bytes {
        return Err(FileError::TooLarge {
            path: path.to_path_buf(),
            limit: max_bytes,
        });
    }

    let capacity = opened_metadata
        .len()
        .min(max_bytes)
        .min(READ_PREALLOC_MAX_BYTES) as usize;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    if bytes.len() as u64 > max_bytes {
        return Err(FileError::TooLarge {
            path: path.to_path_buf(),
            limit: max_bytes,
        });
    }
    Ok(bytes)
}

/// Read source bytes with Python's `errors="replace"` UTF-8 behavior.
pub fn read_source_lossy(path: &Path, max_bytes: u64) -> Result<String, FileError> {
    let bytes = read_bytes_bounded(path, max_bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn bounded_reader_accepts_exact_limit_and_rejects_one_over_and_non_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("source.bin");
        fs::write(&path, b"1234")?;
        assert_eq!(read_bytes_bounded(&path, 4)?, b"1234");
        assert!(matches!(
            read_bytes_bounded(&path, 3),
            Err(FileError::TooLarge { limit: 3, .. })
        ));
        assert!(matches!(
            read_bytes_bounded(directory.path(), 4),
            Err(FileError::NotAFile(_))
        ));
        Ok(())
    }

    #[test]
    fn lossy_source_read_uses_the_same_stream_bound() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("source.txt");
        fs::write(&path, b"a\xffb")?;
        assert_eq!(read_source_lossy(&path, 3)?, "a\u{fffd}b");
        assert!(matches!(
            read_source_lossy(&path, 2),
            Err(FileError::TooLarge { .. })
        ));
        Ok(())
    }
}
