use std::collections::BTreeMap;

use compass_ir::hex_sha256;

use crate::{ArtifactManifest, ProviderError, normalize_source_path};

pub const SCIP_MANIFEST_SCHEMA: &str = "compass.scip-manifest/1";

pub fn parse_artifact_manifest(
    bytes: &[u8],
    index_digest: &str,
) -> Result<ArtifactManifest, ProviderError> {
    let manifest: ArtifactManifest = serde_json::from_slice(bytes)?;
    validate_manifest(&manifest, index_digest)?;
    Ok(manifest)
}

pub(crate) fn validate_manifest(
    manifest: &ArtifactManifest,
    index_digest: &str,
) -> Result<(), ProviderError> {
    if manifest.schema != SCIP_MANIFEST_SCHEMA {
        return Err(ProviderError::UnsupportedArtifact(format!(
            "unsupported SCIP manifest schema {}",
            manifest.schema
        )));
    }
    if !is_digest(&manifest.index_sha256) || manifest.index_sha256 != index_digest {
        return Err(ProviderError::InvalidInput(
            "SCIP manifest index digest mismatch".to_owned(),
        ));
    }
    let mut normalized = BTreeMap::new();
    for (path, digest) in &manifest.documents {
        let path = normalize_source_path(path)?;
        if !is_digest(digest) {
            return Err(ProviderError::InvalidInput(format!(
                "invalid source digest for {path}"
            )));
        }
        if normalized.insert(path.clone(), digest).is_some() {
            return Err(ProviderError::InvalidInput(format!(
                "duplicate normalized manifest path {path}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn manifest_digest(manifest: Option<&ArtifactManifest>) -> String {
    manifest.map_or_else(
        || hex_sha256(b"raw-scip-unverified"),
        |manifest| {
            let bytes = compass_ir::canonical_json_bytes(manifest)
                .unwrap_or_else(|_| b"invalid-manifest".to_vec());
            hex_sha256(&bytes)
        },
    )
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::parse_artifact_manifest;

    #[test]
    fn validates_manifest_schema_digest_and_paths() {
        let digest = "a".repeat(64);
        let valid = format!(
            r#"{{"schema":"compass.scip-manifest/1","index_sha256":"{digest}","documents":{{"src/lib.rs":"{}"}}}}"#,
            "b".repeat(64)
        );
        assert!(parse_artifact_manifest(valid.as_bytes(), &digest).is_ok());
        let unsafe_path = valid.replace("src/lib.rs", "../lib.rs");
        assert!(parse_artifact_manifest(unsafe_path.as_bytes(), &digest).is_err());
        assert!(parse_artifact_manifest(valid.as_bytes(), &"c".repeat(64)).is_err());
    }
}
