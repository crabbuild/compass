use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use crate::store::{create_owner_dir, reject_symlink, set_owner_file};
use crate::{HistoryError, MAX_JOB_BYTES};

/// Atomically persist owner-only operational state in one directory.
pub(crate) fn write_json_atomic<T: serde::Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), HistoryError> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_JOB_BYTES {
        return Err(HistoryError::OperationalState(format!(
            "{} exceeds the {}-byte durable-state limit",
            path.display(),
            MAX_JOB_BYTES
        )));
    }
    write_bytes_atomic(path, &bytes)
}

pub(crate) fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), HistoryError> {
    write_bytes_atomic_with(path, bytes, |_| Ok(()))
}

#[derive(Clone, Copy, Debug)]
enum DurableBoundary {
    BeforeFileSync,
    AfterFileSync,
    BeforeReplace,
    AfterReplace,
    BeforeDirectorySync,
    AfterDirectorySync,
}

fn write_bytes_atomic_with(
    path: &Path,
    bytes: &[u8],
    mut boundary: impl FnMut(DurableBoundary) -> Result<(), HistoryError>,
) -> Result<(), HistoryError> {
    let parent = path.parent().ok_or_else(|| HistoryError::UnsafePath {
        path: path.to_path_buf(),
        reason: "durable state has no parent directory".to_owned(),
    })?;
    create_owner_dir(parent)?;
    reject_symlink(path, true)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".compass-state-")
        .tempfile_in(parent)
        .map_err(|source| crate::error::io_error(parent, source))?;
    let temporary_path = temporary.path().to_path_buf();
    set_owner_file(&temporary_path)?;
    (|| {
        temporary
            .write_all(bytes)
            .map_err(|source| crate::error::io_error(&temporary_path, source))?;
        temporary
            .flush()
            .map_err(|source| crate::error::io_error(&temporary_path, source))?;
        boundary(DurableBoundary::BeforeFileSync)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| crate::error::io_error(&temporary_path, source))?;
        boundary(DurableBoundary::AfterFileSync)?;
        boundary(DurableBoundary::BeforeReplace)?;
        replace_temporary(temporary, path)?;
        boundary(DurableBoundary::AfterReplace)?;
        set_owner_file(path)?;
        boundary(DurableBoundary::BeforeDirectorySync)?;
        sync_directory(parent)?;
        boundary(DurableBoundary::AfterDirectorySync)
    })()
}

#[cfg(not(windows))]
fn replace_temporary(temporary: tempfile::NamedTempFile, path: &Path) -> Result<(), HistoryError> {
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| crate::error::io_error(path, error.error))
}

#[cfg(windows)]
fn replace_temporary(temporary: tempfile::NamedTempFile, path: &Path) -> Result<(), HistoryError> {
    let temporary_path = temporary
        .into_temp_path()
        .keep()
        .map_err(|error| crate::error::io_error(path, error.error))?;
    let result = atomicwrites::replace_atomic(&temporary_path, path)
        .map_err(|source| crate::error::io_error(path, source));
    if result.is_err() {
        let _cleanup = fs::remove_file(&temporary_path);
    }
    result
}

pub(crate) fn read_json_bounded<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<T, HistoryError> {
    reject_symlink(path, false)?;
    let file = File::open(path).map_err(|source| crate::error::io_error(path, source))?;
    let metadata = file
        .metadata()
        .map_err(|source| crate::error::io_error(path, source))?;
    if !metadata.is_file() || metadata.len() > MAX_JOB_BYTES as u64 {
        return Err(HistoryError::OperationalState(format!(
            "{} is not a bounded regular durable-state file",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or_default());
    file.take((MAX_JOB_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| crate::error::io_error(path, source))?;
    if bytes.len() > MAX_JOB_BYTES {
        return Err(HistoryError::OperationalState(format!(
            "{} exceeds the {}-byte durable-state limit",
            path.display(),
            MAX_JOB_BYTES
        )));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        HistoryError::OperationalState(format!("{} is invalid JSON: {error}", path.display()))
    })
}

fn sync_directory(path: &Path) -> Result<(), HistoryError> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| crate::error::io_error(path, source))
    }
    #[cfg(not(unix))]
    {
        // The replace operation is write-through on supported Windows Rust targets. Opening a
        // directory for fsync is not portable through std, so no weaker copy fallback is used.
        let _ = path;
        Ok(())
    }
}

pub(crate) fn remove_file_durable(path: &Path) -> Result<(), HistoryError> {
    reject_symlink(path, false)?;
    fs::remove_file(path).map_err(|source| crate::error::io_error(path, source))?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injected_failures_leave_the_old_or_new_document_intact()
    -> Result<(), Box<dyn std::error::Error>> {
        let boundaries = [
            DurableBoundary::BeforeFileSync,
            DurableBoundary::AfterFileSync,
            DurableBoundary::BeforeReplace,
            DurableBoundary::AfterReplace,
            DurableBoundary::BeforeDirectorySync,
            DurableBoundary::AfterDirectorySync,
        ];
        for fail_at in boundaries {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("state.json");
            write_bytes_atomic(&path, br#"{"generation":1}"#)?;
            let result = write_bytes_atomic_with(&path, br#"{"generation":2}"#, |boundary| {
                if std::mem::discriminant(&boundary) == std::mem::discriminant(&fail_at) {
                    Err(HistoryError::OperationalState(format!(
                        "injected failure at {boundary:?}"
                    )))
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            let value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            let generation = value["generation"].as_u64();
            if matches!(
                fail_at,
                DurableBoundary::BeforeFileSync
                    | DurableBoundary::AfterFileSync
                    | DurableBoundary::BeforeReplace
            ) {
                assert_eq!(generation, Some(1));
            } else {
                assert_eq!(generation, Some(2));
            }
        }
        Ok(())
    }
}
