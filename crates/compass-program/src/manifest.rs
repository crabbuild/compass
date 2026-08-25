use std::collections::BTreeMap;

use compass_ir::{canonical_json_bytes, hex_sha256};

use crate::{
    ArtifactManifest, ManagedAnalyzerProfile, ManagedAnalyzerState, ProviderError,
    normalize_source_path,
};

pub const SCIP_MANIFEST_SCHEMA: &str = "compass.scip-manifest/1";
pub const MANAGED_ANALYZER_PROFILE_SCHEMA: &str = "compass.managed-analyzer-profile/1";

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
    if let Some(profile) = &manifest.managed_analyzer {
        validate_managed_analyzer_profile(profile)?;
    }
    Ok(())
}

pub fn source_inventory_digest(
    source_digests: &BTreeMap<String, String>,
) -> Result<String, ProviderError> {
    let mut normalized = BTreeMap::new();
    for (path, digest) in source_digests {
        let path = normalize_source_path(path)?;
        if !is_digest(digest) {
            return Err(ProviderError::InvalidInput(format!(
                "invalid source inventory digest for {path}"
            )));
        }
        if normalized.insert(path.clone(), digest).is_some() {
            return Err(ProviderError::InvalidInput(format!(
                "duplicate normalized source inventory path {path}"
            )));
        }
    }
    canonical_json_bytes(&normalized)
        .map(|bytes| hex_sha256(&bytes))
        .map_err(|error| ProviderError::InvalidInput(error.to_string()))
}

pub fn managed_analyzer_profile_digest(
    profile: &ManagedAnalyzerProfile,
) -> Result<String, ProviderError> {
    validate_managed_analyzer_profile(profile)?;
    canonical_json_bytes(profile)
        .map(|bytes| hex_sha256(&bytes))
        .map_err(|error| ProviderError::InvalidInput(error.to_string()))
}

pub(crate) fn validate_managed_artifact_context(
    manifest: Option<&ArtifactManifest>,
    project_digest: &str,
) -> Result<(), ProviderError> {
    let Some(profile) = manifest.and_then(|manifest| manifest.managed_analyzer.as_ref()) else {
        return Ok(());
    };
    validate_managed_analyzer_profile(profile)?;
    if !is_digest(project_digest) || profile.source_inventory_digest != project_digest {
        return Err(ProviderError::StaleAnalyzerProfile(
            "source inventory digest mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn validate_managed_analyzer_profile(
    profile: &ManagedAnalyzerProfile,
) -> Result<(), ProviderError> {
    if profile.schema != MANAGED_ANALYZER_PROFILE_SCHEMA {
        return Err(ProviderError::UnsupportedArtifact(format!(
            "unsupported managed analyzer profile schema {}",
            profile.schema
        )));
    }
    if profile.language != "python" || profile.provider != "scip-python" {
        return Err(ProviderError::UnsupportedArtifact(format!(
            "unsupported managed analyzer profile {}/{}",
            profile.language, profile.provider
        )));
    }
    if profile.protocol_version != "scip/1" {
        return Err(ProviderError::UnsupportedArtifact(format!(
            "unsupported managed analyzer protocol {}",
            profile.protocol_version
        )));
    }
    for (field, value) in [
        ("provider_version", profile.provider_version.as_str()),
        ("protocol_version", profile.protocol_version.as_str()),
        (
            "environment.implementation",
            profile.environment.implementation.as_str(),
        ),
        (
            "environment.python_version",
            profile.environment.python_version.as_str(),
        ),
        (
            "environment.platform",
            profile.environment.platform.as_str(),
        ),
    ] {
        if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err(ProviderError::InvalidInput(format!(
                "invalid managed analyzer {field}"
            )));
        }
    }
    for (field, digest) in [
        (
            "source_inventory_digest",
            profile.source_inventory_digest.as_str(),
        ),
        (
            "environment_digest",
            profile.environment.environment_digest.as_str(),
        ),
        (
            "project_configuration_digest",
            profile.environment.project_configuration_digest.as_str(),
        ),
        (
            "typeshed_digest",
            profile.environment.typeshed_digest.as_str(),
        ),
        ("stubs_digest", profile.environment.stubs_digest.as_str()),
    ] {
        if !is_digest(digest) {
            return Err(ProviderError::InvalidInput(format!(
                "invalid managed analyzer {field}"
            )));
        }
    }
    validate_profile_paths("source_roots", &profile.environment.source_roots)?;
    validate_profile_paths("import_roots", &profile.environment.import_roots)?;
    if profile.environment.editable_packages.len() > 256 {
        return Err(ProviderError::ResourceLimit(
            "managed analyzer editable_packages exceeds 256 entries".to_owned(),
        ));
    }
    let mut editable_packages = BTreeMap::new();
    for package in &profile.environment.editable_packages {
        let root = normalize_source_path(&package.root)?;
        if package.name.is_empty()
            || package.name.len() > 256
            || package.name.chars().any(char::is_control)
            || root != package.root
            || !is_digest(&package.digest)
            || editable_packages
                .insert((package.name.clone(), root), ())
                .is_some()
        {
            return Err(ProviderError::InvalidInput(
                "managed analyzer editable_packages is not canonical".to_owned(),
            ));
        }
    }
    if profile.permissions.allow_dependency_network
        || profile.permissions.allow_package_install
        || profile.permissions.allow_project_execution
    {
        return Err(ProviderError::AnalyzerPermissionDenied(
            "managed SCIP artifacts must use the offline profile".to_owned(),
        ));
    }
    match profile.state {
        ManagedAnalyzerState::Complete => Ok(()),
        ManagedAnalyzerState::TimedOut => Err(ProviderError::AnalyzerTimedOut),
        ManagedAnalyzerState::Cancelled => Err(ProviderError::AnalyzerCancelled),
        ManagedAnalyzerState::PermissionDenied => Err(ProviderError::AnalyzerPermissionDenied(
            "artifact producer denied a required permission".to_owned(),
        )),
        ManagedAnalyzerState::Partial => Err(ProviderError::AnalyzerIncomplete(
            "artifact producer reported partial evidence".to_owned(),
        )),
        ManagedAnalyzerState::Failed => Err(ProviderError::AnalyzerIncomplete(
            "artifact producer failed".to_owned(),
        )),
    }
}

fn validate_profile_paths(field: &str, paths: &[String]) -> Result<(), ProviderError> {
    if paths.len() > 256 {
        return Err(ProviderError::ResourceLimit(format!(
            "managed analyzer {field} exceeds 256 entries"
        )));
    }
    let mut normalized = BTreeMap::new();
    for path in paths {
        let value = normalize_source_path(path)?;
        if value != *path || normalized.insert(value.clone(), ()).is_some() {
            return Err(ProviderError::InvalidInput(format!(
                "managed analyzer {field} is not canonical"
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
