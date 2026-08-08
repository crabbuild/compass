use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use flate2::read::GzDecoder;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::Outcome;

const RELEASE_MANIFEST_URL: &str =
    "https://github.com/crabbuild/compass/releases/latest/download/compass-release.json";
const RELEASE_DOWNLOAD_BASE_URL: &str = "https://github.com/crabbuild/compass/releases/download";
const RELEASE_MANIFEST_SCHEMA: &str = "compass.release/1";
const USER_AGENT: &str = concat!("compass/", env!("CARGO_PKG_VERSION"));
const MANIFEST_LIMIT: usize = 64 * 1024;
const MAX_RELEASE_ARTIFACTS: usize = 32;
const MAX_TARGET_LENGTH: usize = 128;
const ARCHIVE_LIMIT: usize = 512 * 1024 * 1024;
const BINARY_LIMIT: u64 = 512 * 1024 * 1024;
const SUPPORTED_TARGETS: [&str; 6] = [
    "aarch64-apple-darwin",
    "aarch64-pc-windows-msvc",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
];

#[derive(Debug, Deserialize)]
struct ReleaseManifest {
    schema: String,
    version: String,
    tag: String,
    artifacts: Vec<ReleaseArtifact>,
}

#[derive(Debug, Deserialize)]
struct ReleaseArtifact {
    target: String,
    archive: String,
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Eq, PartialEq)]
struct ReleasePlan {
    version: Version,
    tag: String,
    archive: String,
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VersionDecision {
    Upgrade,
    Current,
    Newer,
}

pub(crate) fn command_upgrade(arguments: &[String]) -> Outcome {
    if let Some(argument) = arguments.first() {
        return Outcome::failure_with_code(format!("error: unexpected argument '{argument}'"), 2);
    }

    match upgrade() {
        Ok(message) => Outcome::success(message),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn upgrade() -> Result<String, String> {
    let target = current_target()?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("invalid installed Compass version: {error}"))?;
    let agent = http_agent();
    let manifest = fetch(&agent, RELEASE_MANIFEST_URL, MANIFEST_LIMIT)
        .map_err(|error| format!("could not download Compass release manifest: {error}"))?;
    let plan = parse_release_manifest(&manifest, target)?;
    let latest = &plan.version;

    match version_decision(&current, latest) {
        VersionDecision::Current => {
            return Ok(format!("Compass {current} is already the latest version."));
        }
        VersionDecision::Newer => {
            return Ok(format!(
                "Compass {current} is newer than the latest release ({latest}); no downgrade was performed."
            ));
        }
        VersionDecision::Upgrade => {}
    }

    let temporary = tempfile::tempdir()
        .map_err(|error| format!("could not create upgrade directory: {error}"))?;
    let archive_path = temporary.path().join(&plan.archive);
    let archive_url = format!("{RELEASE_DOWNLOAD_BASE_URL}/{}/{}", plan.tag, plan.archive);

    let downloaded = download(&agent, &archive_url, &archive_path, ARCHIVE_LIMIT)
        .map_err(|error| format!("could not download {}: {error}", plan.archive))?;
    if downloaded != plan.bytes {
        return Err(format!(
            "release archive size mismatch for {}: expected {} bytes, downloaded {downloaded}",
            plan.archive, plan.bytes
        ));
    }
    verify_checksum(&archive_path, &plan.sha256)?;

    let executable_name = if target.contains("windows") {
        "compass.exe"
    } else {
        "compass"
    };
    let packaged_path = PathBuf::from(format!("compass-{target}")).join(executable_name);
    let staged_path = temporary.path().join(format!("staged-{executable_name}"));
    extract_executable(&archive_path, &packaged_path, &staged_path)?;
    validate_executable(&staged_path, latest)?;
    self_replace::self_replace(&staged_path).map_err(|error| {
        let installed = std::env::current_exe()
            .map_or_else(|_| "the running executable".to_owned(), |path| path.display().to_string());
        format!(
            "could not replace {installed}: {error}. Check that the executable is writable by the current user"
        )
    })?;

    Ok(format!("Upgraded Compass from {current} to {latest}."))
}

fn http_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5 * 60)))
        .max_redirects(5)
        .build();
    config.into()
}

fn fetch(agent: &ureq::Agent, url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let response = agent
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| error.to_string())?;
    let limit = u64::try_from(max_bytes)
        .map_err(|_| "download size limit is invalid".to_owned())?
        .checked_add(1)
        .ok_or_else(|| "download size limit overflowed".to_owned())?;
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(limit)
        .reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > max_bytes {
        return Err(format!("response exceeded {max_bytes} bytes"));
    }
    Ok(bytes)
}

fn download(
    agent: &ureq::Agent,
    url: &str,
    destination: &Path,
    max_bytes: usize,
) -> Result<u64, String> {
    let response = agent
        .get(url)
        .header("Accept", "application/octet-stream")
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| error.to_string())?;
    let limit = u64::try_from(max_bytes)
        .map_err(|_| "download size limit is invalid".to_owned())?
        .checked_add(1)
        .ok_or_else(|| "download size limit overflowed".to_owned())?;
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(limit)
        .reader();
    let mut output = File::create(destination).map_err(|error| error.to_string())?;
    let written = io::copy(&mut reader, &mut output).map_err(|error| error.to_string())?;
    if written > u64::try_from(max_bytes).map_err(|_| "download size limit is invalid")? {
        return Err(format!("response exceeded {max_bytes} bytes"));
    }
    output.sync_all().map_err(|error| error.to_string())?;
    Ok(written)
}

fn parse_release_manifest(bytes: &[u8], target: &str) -> Result<ReleasePlan, String> {
    let manifest: ReleaseManifest = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid Compass release manifest: {error}"))?;
    release_plan(manifest, target)
}

fn release_plan(manifest: ReleaseManifest, target: &str) -> Result<ReleasePlan, String> {
    if manifest.schema != RELEASE_MANIFEST_SCHEMA {
        return Err(format!(
            "unsupported Compass release manifest schema '{}'",
            manifest.schema
        ));
    }
    let version = Version::parse(&manifest.version)
        .map_err(|error| format!("invalid Compass release version: {error}"))?;
    if !version.pre.is_empty() {
        return Err(format!(
            "Compass release manifest version '{}' is a prerelease",
            manifest.version
        ));
    }
    let expected_tag = format!("compass-v{version}");
    if manifest.tag != expected_tag {
        return Err(format!(
            "Compass release manifest tag '{}' does not match version {version}",
            manifest.tag
        ));
    }
    if manifest.artifacts.is_empty() || manifest.artifacts.len() > MAX_RELEASE_ARTIFACTS {
        return Err(format!(
            "Compass release manifest must contain between 1 and {MAX_RELEASE_ARTIFACTS} artifacts"
        ));
    }

    let mut seen = BTreeSet::new();
    let mut selected = None;
    for artifact in manifest.artifacts {
        if artifact.target.is_empty()
            || artifact.target.len() > MAX_TARGET_LENGTH
            || !artifact.target.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b'.')
            })
        {
            return Err(format!(
                "Compass release manifest contains invalid target '{}'",
                artifact.target
            ));
        }
        if !seen.insert(artifact.target.clone()) {
            return Err(format!(
                "Compass release manifest contains duplicate target '{}'",
                artifact.target
            ));
        }
        let expected_archive = format!("compass-{}.tar.gz", artifact.target);
        if artifact.archive != expected_archive {
            return Err(format!(
                "Compass release manifest archive '{}' does not match target '{}'",
                artifact.archive, artifact.target
            ));
        }
        if artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "Compass release manifest contains an invalid SHA-256 digest for '{}'",
                artifact.target
            ));
        }
        if artifact.bytes == 0 || artifact.bytes > ARCHIVE_LIMIT as u64 {
            return Err(format!(
                "Compass release manifest contains an invalid archive size for '{}'",
                artifact.target
            ));
        }
        if artifact.target == target {
            selected = Some(ReleasePlan {
                version: version.clone(),
                tag: expected_tag.clone(),
                archive: artifact.archive,
                sha256: artifact.sha256,
                bytes: artifact.bytes,
            });
        }
    }
    selected.ok_or_else(|| format!("Compass release manifest is missing target '{target}'"))
}

fn version_decision(current: &Version, latest: &Version) -> VersionDecision {
    match current.cmp(latest) {
        std::cmp::Ordering::Less => VersionDecision::Upgrade,
        std::cmp::Ordering::Equal => VersionDecision::Current,
        std::cmp::Ordering::Greater => VersionDecision::Newer,
    }
}

fn supported_target(target: &str) -> Option<&'static str> {
    SUPPORTED_TARGETS
        .iter()
        .copied()
        .find(|candidate| *candidate == target)
}

fn current_target() -> Result<&'static str, String> {
    let target = env!("COMPASS_BUILD_TARGET");
    supported_target(target).ok_or_else(|| format!("unsupported upgrade target: {target}"))
}

fn verify_checksum(archive: &Path, expected: &str) -> Result<(), String> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("release manifest does not contain a valid SHA-256 digest".to_owned());
    }

    let mut file = File::open(archive)
        .map_err(|error| format!("could not read downloaded archive: {error}"))?;
    let mut digest = Sha256::new();
    io::copy(&mut file, &mut digest)
        .map_err(|error| format!("could not hash downloaded archive: {error}"))?;
    let actual = format!("{:x}", digest.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("release archive failed SHA-256 verification".to_owned());
    }
    Ok(())
}

fn extract_executable(
    archive_path: &Path,
    packaged_path: &Path,
    destination: &Path,
) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|error| format!("could not open verified release archive: {error}"))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut found = false;

    for entry in archive
        .entries()
        .map_err(|error| format!("invalid release archive: {error}"))?
    {
        let mut entry = entry.map_err(|error| format!("invalid release archive: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("invalid release archive path: {error}"))?;
        if path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            return Err(format!(
                "release archive contains unsafe path {}",
                path.display()
            ));
        }
        if path.as_ref() != packaged_path {
            continue;
        }
        if found || !entry.header().entry_type().is_file() {
            return Err(format!(
                "release archive contains an invalid {} entry",
                packaged_path.display()
            ));
        }
        let mut output = File::create(destination)
            .map_err(|error| format!("could not stage Compass executable: {error}"))?;
        let written = io::copy(&mut entry.by_ref().take(BINARY_LIMIT + 1), &mut output)
            .map_err(|error| format!("could not extract Compass executable: {error}"))?;
        if written > BINARY_LIMIT {
            return Err("release executable exceeded the size limit".to_owned());
        }
        output
            .sync_all()
            .map_err(|error| format!("could not sync staged Compass executable: {error}"))?;
        found = true;
    }

    if !found {
        return Err(format!(
            "release archive is missing {}",
            packaged_path.display()
        ));
    }
    make_executable(destination)?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)
        .map_err(|error| format!("could not inspect staged Compass executable: {error}"))?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| format!("could not make staged Compass executable runnable: {error}"))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn validate_executable(path: &Path, expected: &Version) -> Result<(), String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| format!("could not run staged Compass executable: {error}"))?;
    if !output.status.success() {
        return Err("staged Compass executable failed its version check".to_owned());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "staged Compass version output is not UTF-8".to_owned())?;
    let expected_output = format!("compass {expected}");
    if stdout.trim() != expected_output {
        return Err(format!(
            "staged Compass executable reported '{}', expected '{expected_output}'",
            stdout.trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> ReleaseManifest {
        ReleaseManifest {
            schema: RELEASE_MANIFEST_SCHEMA.to_owned(),
            version: version.to_owned(),
            tag: format!("compass-v{version}"),
            artifacts: SUPPORTED_TARGETS
                .iter()
                .map(|target| ReleaseArtifact {
                    target: (*target).to_owned(),
                    archive: format!("compass-{target}.tar.gz"),
                    sha256: "a".repeat(64),
                    bytes: 42,
                })
                .collect(),
        }
    }

    #[test]
    fn version_policy_upgrades_only_to_a_newer_stable_release() {
        let current = Version::new(1, 2, 3);
        assert_eq!(
            version_decision(&current, &Version::new(1, 2, 4)),
            VersionDecision::Upgrade
        );
        assert_eq!(
            version_decision(&current, &Version::new(1, 2, 3)),
            VersionDecision::Current
        );
        assert_eq!(
            version_decision(&current, &Version::new(1, 2, 2)),
            VersionDecision::Newer
        );
    }

    #[test]
    fn published_targets_are_selected_exactly() {
        for target in [
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
            "aarch64-pc-windows-msvc",
        ] {
            assert_eq!(supported_target(target), Some(target));
        }
        assert_eq!(supported_target("x86_64-unknown-linux-musl"), None);
        assert_eq!(supported_target("x86_64-pc-windows-gnu"), None);
    }

    #[test]
    fn release_manifest_selects_one_validated_target() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = serde_json::json!({
            "schema": RELEASE_MANIFEST_SCHEMA,
            "version": "1.2.3",
            "tag": "compass-v1.2.3",
            "artifacts": SUPPORTED_TARGETS.iter().map(|target| serde_json::json!({
                "target": target,
                "archive": format!("compass-{target}.tar.gz"),
                "sha256": "a".repeat(64),
                "bytes": 42,
            })).collect::<Vec<_>>(),
        });
        let bytes = serde_json::to_vec(&fixture)?;
        let plan =
            parse_release_manifest(&bytes, "aarch64-apple-darwin").map_err(io::Error::other)?;
        assert_eq!(plan.version, Version::new(1, 2, 3));
        assert_eq!(plan.tag, "compass-v1.2.3");
        assert_eq!(plan.archive, "compass-aarch64-apple-darwin.tar.gz");
        assert_eq!(plan.sha256, "a".repeat(64));
        assert_eq!(plan.bytes, 42);
        Ok(())
    }

    #[test]
    fn release_manifest_rejects_unknown_schema_prerelease_and_mismatched_tag() {
        let mut unknown = manifest("1.2.3");
        unknown.schema = "compass.release/2".to_owned();
        assert!(release_plan(unknown, "aarch64-apple-darwin").is_err());

        assert!(release_plan(manifest("1.2.4-beta.1"), "aarch64-apple-darwin").is_err());

        let mut mismatched = manifest("1.2.3");
        mismatched.tag = "compass-v1.2.4".to_owned();
        assert!(release_plan(mismatched, "aarch64-apple-darwin").is_err());
    }

    #[test]
    fn release_manifest_rejects_incomplete_or_ambiguous_artifacts() {
        let mut incomplete = manifest("1.2.3");
        incomplete
            .artifacts
            .retain(|artifact| artifact.target != "aarch64-apple-darwin");
        assert!(release_plan(incomplete, "aarch64-apple-darwin").is_err());

        let mut duplicate = manifest("1.2.3");
        duplicate.artifacts[5] = ReleaseArtifact {
            target: duplicate.artifacts[0].target.clone(),
            archive: duplicate.artifacts[0].archive.clone(),
            sha256: duplicate.artifacts[0].sha256.clone(),
            bytes: duplicate.artifacts[0].bytes,
        };
        assert!(release_plan(duplicate, "aarch64-apple-darwin").is_err());
    }

    #[test]
    fn release_manifest_accepts_a_bounded_future_target() {
        let mut future = manifest("1.2.3");
        future.artifacts.push(ReleaseArtifact {
            target: "riscv64gc-unknown-linux-gnu".to_owned(),
            archive: "compass-riscv64gc-unknown-linux-gnu.tar.gz".to_owned(),
            sha256: "b".repeat(64),
            bytes: 42,
        });
        assert!(release_plan(future, "aarch64-apple-darwin").is_ok());
    }

    #[test]
    fn release_manifest_rejects_invalid_artifact_contracts() {
        let mut invalid_archive = manifest("1.2.3");
        invalid_archive.artifacts[0].archive = "../compass.tar.gz".to_owned();
        assert!(release_plan(invalid_archive, "aarch64-apple-darwin").is_err());

        let mut invalid_digest = manifest("1.2.3");
        invalid_digest.artifacts[0].sha256 = "not-a-digest".to_owned();
        assert!(release_plan(invalid_digest, "aarch64-apple-darwin").is_err());

        let mut invalid_size = manifest("1.2.3");
        invalid_size.artifacts[0].bytes = 0;
        assert!(release_plan(invalid_size, "aarch64-apple-darwin").is_err());
    }

    #[test]
    fn checksum_verification_requires_the_expected_digest() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("compass-test.tar.gz");
        std::fs::write(&archive, b"verified archive")?;
        let digest = format!("{:x}", Sha256::digest(b"verified archive"));
        assert!(verify_checksum(&archive, &digest).is_ok());
        assert!(
            verify_checksum(
                &archive,
                "0000000000000000000000000000000000000000000000000000000000000000"
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn extraction_selects_only_the_packaged_compass_binary()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let archive_path = directory.path().join("release.tar.gz");
        let archive_file = File::create(&archive_path)?;
        let encoder = flate2::write::GzEncoder::new(archive_file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let contents = b"compass executable";
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(contents.len())?);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append_data(
            &mut header,
            "compass-aarch64-apple-darwin/compass",
            &contents[..],
        )?;
        let encoder = archive.into_inner()?;
        encoder.finish()?;

        let destination = directory.path().join("staged-compass");
        extract_executable(
            &archive_path,
            Path::new("compass-aarch64-apple-darwin/compass"),
            &destination,
        )
        .map_err(io::Error::other)?;
        assert_eq!(std::fs::read(destination)?, contents);
        assert!(
            extract_executable(
                &archive_path,
                Path::new("compass-x86_64-apple-darwin/compass"),
                &directory.path().join("missing")
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn unexpected_arguments_fail_before_upgrade_work() {
        let outcome = command_upgrade(&["--force".to_owned()]);
        assert_eq!(outcome.code, 2);
        assert!(outcome.stderr.contains("unexpected argument '--force'"));
    }
}
