use std::fs;
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
    let bytes = fs::read(root.join(relative_path)).map_err(|error| {
        QueryError::new(
            QueryErrorKind::Internal,
            "source_read_failed",
            format!("{relative}: {error}"),
        )
    })?;
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    if digest != expected_digest {
        return Ok(VerifiedSource::Stale { actual: digest });
    }
    let limit = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    let truncated = bytes.len() > limit;
    let selected = bytes.get(..bytes.len().min(limit)).unwrap_or_default();
    Ok(VerifiedSource::Fresh {
        source: String::from_utf8_lossy(selected).into_owned(),
        truncated,
    })
}

pub(crate) enum VerifiedSource {
    Fresh { source: String, truncated: bool },
    Stale { actual: String },
}
