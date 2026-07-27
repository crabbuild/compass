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

const RELEASE_API_URL: &str = "https://api.github.com/repos/crabbuild/compass/releases/latest";
const USER_AGENT: &str = concat!("compass/", env!("CARGO_PKG_VERSION"));
const METADATA_LIMIT: usize = 1024 * 1024;
const CHECKSUM_LIMIT: usize = 4096;
const ARCHIVE_LIMIT: usize = 512 * 1024 * 1024;
const BINARY_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
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
    let release: Release = serde_json::from_slice(&fetch(&agent, RELEASE_API_URL, METADATA_LIMIT)?)
        .map_err(|error| format!("invalid GitHub release metadata: {error}"))?;
    let latest = release_version(&release)?;

    match version_decision(&current, &latest) {
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

    let archive_name = format!("compass-{target}.tar.gz");
    let checksum_name = format!("{archive_name}.sha256");
    let archive_asset = release_asset(&release, &archive_name)?;
    let checksum_asset = release_asset(&release, &checksum_name)?;
    let temporary = tempfile::tempdir()
        .map_err(|error| format!("could not create upgrade directory: {error}"))?;
    let archive_path = temporary.path().join(&archive_name);

    download(
        &agent,
        &archive_asset.browser_download_url,
        &archive_path,
        ARCHIVE_LIMIT,
    )
    .map_err(|error| format!("could not download {archive_name}: {error}"))?;
    let checksum = fetch(&agent, &checksum_asset.browser_download_url, CHECKSUM_LIMIT)
        .map_err(|error| format!("could not download {checksum_name}: {error}"))?;
    verify_checksum(&archive_path, &archive_name, &checksum)?;

    let executable_name = if target.contains("windows") {
        "compass.exe"
    } else {
        "compass"
    };
    let packaged_path = PathBuf::from(format!("compass-{target}")).join(executable_name);
    let staged_path = temporary.path().join(format!("staged-{executable_name}"));
    extract_executable(&archive_path, &packaged_path, &staged_path)?;
    validate_executable(&staged_path, &latest)?;
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
        .header("Accept", "application/vnd.github+json")
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
) -> Result<(), String> {
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
    output.sync_all().map_err(|error| error.to_string())
}

fn release_version(release: &Release) -> Result<Version, String> {
    if release.draft || release.prerelease {
        return Err("GitHub latest release is not a stable published release".to_owned());
    }
    let raw = release
        .tag_name
        .strip_prefix("compass-v")
        .ok_or_else(|| format!("invalid Compass release tag '{}'", release.tag_name))?;
    let version =
        Version::parse(raw).map_err(|error| format!("invalid Compass release tag: {error}"))?;
    if !version.pre.is_empty() {
        return Err(format!(
            "latest Compass release '{}' is a prerelease",
            release.tag_name
        ));
    }
    Ok(version)
}

fn release_asset<'a>(release: &'a Release, name: &str) -> Result<&'a ReleaseAsset, String> {
    let mut matches = release.assets.iter().filter(|asset| asset.name == name);
    let asset = matches
        .next()
        .ok_or_else(|| format!("latest Compass release is missing asset {name}"))?;
    if matches.next().is_some() {
        return Err(format!(
            "latest Compass release contains duplicate asset {name}"
        ));
    }
    Ok(asset)
}

fn version_decision(current: &Version, latest: &Version) -> VersionDecision {
    match current.cmp(latest) {
        std::cmp::Ordering::Less => VersionDecision::Upgrade,
        std::cmp::Ordering::Equal => VersionDecision::Current,
        std::cmp::Ordering::Greater => VersionDecision::Newer,
    }
}

fn supported_target(target: &str) -> Option<&'static str> {
    match target {
        "x86_64-apple-darwin" => Some("x86_64-apple-darwin"),
        "aarch64-apple-darwin" => Some("aarch64-apple-darwin"),
        "x86_64-unknown-linux-gnu" => Some("x86_64-unknown-linux-gnu"),
        "aarch64-unknown-linux-gnu" => Some("aarch64-unknown-linux-gnu"),
        "x86_64-pc-windows-msvc" => Some("x86_64-pc-windows-msvc"),
        "aarch64-pc-windows-msvc" => Some("aarch64-pc-windows-msvc"),
        _ => None,
    }
}

fn current_target() -> Result<&'static str, String> {
    let target = env!("COMPASS_BUILD_TARGET");
    supported_target(target).ok_or_else(|| format!("unsupported upgrade target: {target}"))
}

fn verify_checksum(archive: &Path, archive_name: &str, checksum: &[u8]) -> Result<(), String> {
    let checksum =
        std::str::from_utf8(checksum).map_err(|_| "release checksum is not UTF-8".to_owned())?;
    let mut fields = checksum.split_whitespace();
    let expected = fields
        .next()
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "release checksum does not contain a valid SHA-256 digest".to_owned())?;
    let filename = fields
        .next()
        .ok_or_else(|| "release checksum does not name its archive".to_owned())?;
    if fields.next().is_some() || filename.trim_start_matches('*') != archive_name {
        return Err(format!(
            "release checksum does not match archive {archive_name}"
        ));
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

    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.to_owned(),
            draft: false,
            prerelease: false,
            assets: Vec::new(),
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

        assert_eq!(release_version(&release("compass-v1.2.3")), Ok(current));
        assert!(release_version(&release("v1.2.3")).is_err());
        assert!(release_version(&release("compass-v1.2.4-beta.1")).is_err());
        let mut prerelease = release("compass-v1.2.4");
        prerelease.prerelease = true;
        assert!(release_version(&prerelease).is_err());
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
    fn release_assets_must_be_present_once() {
        let mut release = release("compass-v1.2.3");
        release.assets.push(ReleaseAsset {
            name: "compass-a.tar.gz".to_owned(),
            browser_download_url: "https://example.test/a".to_owned(),
        });
        assert!(release_asset(&release, "compass-a.tar.gz").is_ok());
        assert!(release_asset(&release, "missing").is_err());
        release.assets.push(ReleaseAsset {
            name: "compass-a.tar.gz".to_owned(),
            browser_download_url: "https://example.test/b".to_owned(),
        });
        assert!(release_asset(&release, "compass-a.tar.gz").is_err());
    }

    #[test]
    fn checksum_verification_requires_the_expected_name_and_digest()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("compass-test.tar.gz");
        std::fs::write(&archive, b"verified archive")?;
        let digest = format!("{:x}", Sha256::digest(b"verified archive"));
        let checksum = format!("{digest}  compass-test.tar.gz\n");
        assert!(verify_checksum(&archive, "compass-test.tar.gz", checksum.as_bytes()).is_ok());
        assert!(verify_checksum(&archive, "other.tar.gz", checksum.as_bytes()).is_err());
        assert!(
            verify_checksum(
                &archive,
                "compass-test.tar.gz",
                b"0000000000000000000000000000000000000000000000000000000000000000  compass-test.tar.gz"
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
