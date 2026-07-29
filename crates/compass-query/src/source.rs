use std::fs::{self, File};
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
    let path = canonical_root.join(relative_path);
    let parent = path.parent().unwrap_or(&canonical_root);
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
    let mut file = open_beneath(&canonical_root, relative_path).map_err(|error| {
        let unsupported = error.kind() == std::io::ErrorKind::Unsupported;
        let unsafe_path = unsupported || is_unsafe_open_error(&error);
        QueryError::new(
            if unsafe_path {
                QueryErrorKind::UnsafePath
            } else {
                QueryErrorKind::Internal
            },
            if unsafe_path {
                if unsupported {
                    "source_confinement_unsupported"
                } else {
                    "unsafe_source_path"
                }
            } else {
                "source_read_failed"
            },
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
fn is_unsafe_open_error(error: &std::io::Error) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| code == libc::ELOOP || code == libc::ENOTDIR)
}

#[cfg(not(unix))]
fn is_unsafe_open_error(_error: &std::io::Error) -> bool {
    false
}

#[cfg(unix)]
fn open_beneath(root: &Path, relative: &Path) -> std::io::Result<File> {
    use rustix::fs::{Mode, OFlags, open, openat};

    let mut directory = open(
        root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
    )?;
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
        };
        let last = index + 1 == components.len();
        let flags = OFlags::RDONLY
            | OFlags::CLOEXEC
            | OFlags::NOFOLLOW
            | if last {
                OFlags::empty()
            } else {
                OFlags::DIRECTORY
            };
        directory = openat(&directory, *name, flags, Mode::empty())?;
    }
    Ok(File::from(directory))
}

#[cfg(not(unix))]
fn open_beneath(_root: &Path, _relative: &Path) -> std::io::Result<File> {
    // Safe component-by-component, no-follow traversal is not available through
    // the standard library on these platforms. Fail closed instead of reopening
    // the canonicalize-then-open race.
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "race-resistant source confinement is unavailable on this platform",
    ))
}

pub(crate) enum VerifiedSource {
    Fresh { source: String, truncated: bool },
    Stale { actual: String },
}
