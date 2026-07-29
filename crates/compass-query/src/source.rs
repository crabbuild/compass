use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path};

use sha2::{Digest, Sha256};

use crate::cql::{QueryError, QueryErrorKind};

pub(crate) fn verified_source(
    root: &Path,
    relative: &str,
    expected_digest: &str,
    max_bytes: u64,
) -> Result<VerifiedSource, QueryError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative.contains('\\')
        || relative_path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(QueryError::new(
            QueryErrorKind::UnsafePath,
            "unsafe_source_path",
            format!("source path is not repository-relative: {relative}"),
        ));
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        QueryError::new(
            QueryErrorKind::Internal,
            "source_read_failed",
            format!("{}: {error}", root.display()),
        )
    })?;
    let path = root.join(relative_path);
    let parent = path.parent().unwrap_or(root);
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        QueryError::new(
            QueryErrorKind::Internal,
            "source_read_failed",
            format!("{}: {error}", parent.display()),
        )
    })?;
    if !canonical_parent.starts_with(&canonical_root)
        || fs::symlink_metadata(&path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
    {
        return Err(QueryError::new(
            QueryErrorKind::UnsafePath,
            "unsafe_source_path",
            format!("source path escapes the repository: {relative}"),
        ));
    }
    let mut file = open_no_follow(&path).map_err(|error| {
        QueryError::new(
            QueryErrorKind::Internal,
            "source_read_failed",
            format!("{relative}: {error}"),
        )
    })?;
    let limit = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    let retained_capacity = limit.min(1024 * 1024);
    let mut selected = Vec::with_capacity(retained_capacity);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            QueryError::new(
                QueryErrorKind::Internal,
                "source_read_failed",
                format!("{relative}: {error}"),
            )
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
        let remaining = limit.saturating_sub(selected.len());
        selected.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    let digest = format!("sha256:{:x}", digest.finalize());
    if digest != expected_digest {
        return Ok(VerifiedSource::Stale { actual: digest });
    }
    Ok(VerifiedSource::Fresh {
        source: String::from_utf8_lossy(&selected).into_owned(),
        truncated: total > max_bytes,
    })
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

pub(crate) enum VerifiedSource {
    Fresh { source: String, truncated: bool },
    Stale { actual: String },
}
