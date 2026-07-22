use std::fs;
use std::path::{Path, PathBuf};

use crate::{FileError, io_error, write_text_atomic};

/// Crash marker preventing an interrupted build from being mistaken for complete output.
#[derive(Debug)]
pub struct BuildGuard {
    marker: PathBuf,
}

impl BuildGuard {
    pub fn begin(output_directory: &Path) -> Result<Self, FileError> {
        let marker = output_directory.join(".compass-build-incomplete");
        write_text_atomic(&marker, "1")?;
        Ok(Self { marker })
    }

    pub fn ensure_complete(output_directory: &Path) -> Result<(), FileError> {
        let marker = output_directory.join(".compass-build-incomplete");
        if marker.exists() {
            Err(FileError::IncompleteBuild(marker))
        } else {
            Ok(())
        }
    }

    pub fn commit(self) -> Result<(), FileError> {
        if self.marker.exists() {
            fs::remove_file(&self.marker).map_err(|source| io_error(&self.marker, source))?;
        }
        Ok(())
    }
}
