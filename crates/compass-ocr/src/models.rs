//! Pinned PP-OCRv6 model acquisition and offline verification.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{OCR_PREPROCESSING_VERSION, OcrError, OcrProfileIdentity};

const MODEL_HOST: &str = "github.com";
const MODEL_ASSET_HOST: &str = "release-assets.githubusercontent.com";
const MODEL_REPOSITORY: &str = "GreatV/oar-ocr";
const MODEL_REVISION: &str = "v0.7.0";
const ENGINE_VERSION: &str = "0.9.2";
const USER_AGENT: &str = "compass/0.3 document-ocr";
const VERIFIED_MARKER_SCHEMA: &str = "compass.ocr.model-profile/1";
const MODEL_LICENSE: &str = "Apache-2.0 (PaddleOCR models and OAR-OCR runtime)";
const MODEL_CARD: &str = "https://github.com/GreatV/oar-ocr/releases/tag/v0.7.0";
const MODEL_INSTALL_LOCK: &str = ".install.lock";
const MODEL_INSTALL_LOCK_WAIT: Duration = Duration::from_secs(15 * 60);
const MODEL_INSTALL_LOCK_RETRY: Duration = Duration::from_millis(50);
const MODEL_REDIRECT_MAX_BYTES: usize = 8 * 1024;
const MODEL_ERROR_MAX_CHARS: usize = 1_024;
const VERIFIED_MARKER_MAX_BYTES: u64 = 16 * 1024;

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct VerifiedProfileMarker {
    schema: String,
    profile: String,
    engine: String,
    engine_version: String,
    repository: String,
    revision: String,
    manifest_digest: String,
    license: String,
    model_card: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelProfile {
    PpOcrV6Small,
    PpOcrV6Medium,
}

impl ModelProfile {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::PpOcrV6Small => "pp-ocrv6-small",
            Self::PpOcrV6Medium => "pp-ocrv6-medium",
        }
    }

    fn artifacts(self) -> &'static [ArtifactSpec] {
        match self {
            Self::PpOcrV6Small => &SMALL_ARTIFACTS,
            Self::PpOcrV6Medium => &MEDIUM_ARTIFACTS,
        }
    }
}

#[must_use]
pub fn profile_manifest_digest(profile: ModelProfile) -> String {
    let mut digest = Sha256::new();
    digest.update(ENGINE_VERSION.as_bytes());
    digest.update([0]);
    digest.update(MODEL_REPOSITORY.as_bytes());
    digest.update([0]);
    digest.update(MODEL_REVISION.as_bytes());
    for artifact in profile.artifacts() {
        digest.update([0]);
        digest.update(artifact.role.as_bytes());
        digest.update([0]);
        digest.update(artifact.name.as_bytes());
        digest.update(artifact.size.to_le_bytes());
        digest.update(artifact.sha256.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}

impl FromStr for ModelProfile {
    type Err = OcrError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pp-ocrv6-small" | "small" => Ok(Self::PpOcrV6Small),
            "pp-ocrv6-medium" | "medium" => Ok(Self::PpOcrV6Medium),
            _ => Err(OcrError::ModelUnavailable(format!(
                "unknown profile {value:?}; expected pp-ocrv6-small or pp-ocrv6-medium"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ArtifactSpec {
    role: &'static str,
    name: &'static str,
    size: u64,
    sha256: &'static str,
}

const DICTIONARY: ArtifactSpec = ArtifactSpec {
    role: "dictionary",
    name: "ppocrv6_dict.txt",
    size: 74_947,
    sha256: "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d",
};

const SMALL_ARTIFACTS: [ArtifactSpec; 3] = [
    ArtifactSpec {
        role: "detector",
        name: "pp-ocrv6_small_det.onnx",
        size: 9_880_512,
        sha256: "d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e",
    },
    ArtifactSpec {
        role: "recognizer",
        name: "pp-ocrv6_small_rec.onnx",
        size: 21_159_378,
        sha256: "5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634",
    },
    DICTIONARY,
];

const MEDIUM_ARTIFACTS: [ArtifactSpec; 3] = [
    ArtifactSpec {
        role: "detector",
        name: "pp-ocrv6_medium_det.onnx",
        size: 62_032_837,
        sha256: "eb13b44b25bb36f89528b68720af8a61d9cf381176107f465db1757b65d086e1",
    },
    ArtifactSpec {
        role: "recognizer",
        name: "pp-ocrv6_medium_rec.onnx",
        size: 76_554_979,
        sha256: "9c09abf0957f7968c7586464b7397b84ad2387a0497a351af40e9acc71b673ba",
    },
    DICTIONARY,
];

#[derive(Clone, Debug)]
pub struct ModelFiles {
    pub detector: PathBuf,
    pub recognizer: PathBuf,
    pub dictionary: PathBuf,
    pub identity: OcrProfileIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelStatus {
    pub profile: String,
    pub installed: bool,
    pub verified: bool,
    pub bytes: u64,
    pub license: String,
}

pub trait ArtifactFetcher {
    fn fetch(&self, url: &str, max_bytes: u64) -> Result<Box<dyn Read>, OcrError>;
}

#[derive(Clone)]
pub struct HttpsArtifactFetcher {
    agent: ureq::Agent,
}

impl Default for HttpsArtifactFetcher {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15 * 60)))
            .timeout_connect(Some(Duration::from_secs(30)))
            .max_redirects(0)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl ArtifactFetcher for HttpsArtifactFetcher {
    fn fetch(&self, url: &str, max_bytes: u64) -> Result<Box<dyn Read>, OcrError> {
        let mut current = url.to_owned();
        for redirect in 0..=3 {
            validate_model_url(&current, redirect > 0)?;
            let response = self
                .agent
                .get(&current)
                .header("User-Agent", USER_AGENT)
                .call()
                .map_err(|error| OcrError::ModelUnavailable(bounded_error(&error.to_string())))?;
            if response.status().is_redirection() {
                if redirect == 3 {
                    return Err(OcrError::ModelVerification(
                        "model download exceeded the redirect limit".to_owned(),
                    ));
                }
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| {
                        OcrError::ModelVerification(
                            "model download redirect has no valid location".to_owned(),
                        )
                    })?;
                if location.len() > MODEL_REDIRECT_MAX_BYTES {
                    return Err(OcrError::ModelVerification(
                        "model download redirect exceeds its byte limit".to_owned(),
                    ));
                }
                current = location.to_owned();
                continue;
            }
            if !response.status().is_success() {
                return Err(OcrError::ModelUnavailable(format!(
                    "model host returned HTTP {}",
                    response.status()
                )));
            }
            let limit = max_bytes
                .checked_add(1)
                .ok_or_else(|| OcrError::ModelVerification("model size overflow".to_owned()))?;
            return Ok(Box::new(
                response
                    .into_body()
                    .into_with_config()
                    .limit(limit)
                    .reader(),
            ));
        }
        Err(OcrError::ModelVerification(
            "model download redirect handling failed".to_owned(),
        ))
    }
}

fn validate_model_url(url: &str, redirected: bool) -> Result<(), OcrError> {
    let parsed = ureq::http::Uri::try_from(url)
        .map_err(|error| OcrError::ModelVerification(error.to_string()))?;
    let allowed_host = if redirected {
        matches!(parsed.host(), Some(MODEL_HOST) | Some(MODEL_ASSET_HOST))
    } else {
        parsed.host() == Some(MODEL_HOST)
    };
    let unsafe_authority = parsed
        .authority()
        .is_none_or(|authority| authority.as_str().contains('@'));
    let unsafe_port = parsed.port_u16().is_some_and(|port| port != 443);
    if parsed.scheme_str() != Some("https") || !allowed_host || unsafe_authority || unsafe_port {
        return Err(OcrError::ModelVerification(
            "model URL is outside the HTTPS host allowlist".to_owned(),
        ));
    }
    Ok(())
}

fn bounded_error(message: &str) -> String {
    message.chars().take(MODEL_ERROR_MAX_CHARS).collect()
}

#[derive(Debug)]
struct ModelInstallGuard {
    file: File,
}

impl ModelInstallGuard {
    fn acquire(directory: &Path) -> Result<Self, OcrError> {
        Self::acquire_with_timeout(directory, MODEL_INSTALL_LOCK_WAIT)
    }

    fn acquire_with_timeout(directory: &Path, timeout: Duration) -> Result<Self, OcrError> {
        let path = directory.join(MODEL_INSTALL_LOCK);
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && (!metadata.is_file() || metadata.file_type().is_symlink())
        {
            return Err(OcrError::ModelVerification(format!(
                "model install lock is not a regular file: {}",
                path.display()
            )));
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(|source| OcrError::Io {
            path: path.clone(),
            source,
        })?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { file }),
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(MODEL_INSTALL_LOCK_RETRY);
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(OcrError::ModelUnavailable(format!(
                        "timed out waiting for another model installation at {}; retry `compass models install`",
                        path.display()
                    )));
                }
                Err(std::fs::TryLockError::Error(source)) => {
                    return Err(OcrError::Io { path, source });
                }
            }
        }
    }
}

impl Drop for ModelInstallGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Clone, Debug)]
pub struct ModelCache {
    root: PathBuf,
}

impl ModelCache {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn from_environment() -> Result<Self, OcrError> {
        if let Some(root) = std::env::var_os("COMPASS_CACHE_DIR") {
            return Ok(Self::new(PathBuf::from(root).join("models/ocr")));
        }
        if let Some(root) = std::env::var_os("XDG_CACHE_HOME") {
            return Ok(Self::new(PathBuf::from(root).join("compass/models/ocr")));
        }
        if cfg!(windows)
            && let Some(root) = std::env::var_os("LOCALAPPDATA")
        {
            return Ok(Self::new(PathBuf::from(root).join("Compass/models/ocr")));
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|root| Self::new(root.join(".cache/compass/models/ocr")))
            .ok_or_else(|| {
                OcrError::ModelUnavailable(
                    "could not determine Compass's model cache directory".to_owned(),
                )
            })
    }

    fn profile_dir(&self, profile: ModelProfile) -> PathBuf {
        self.root.join(profile.name()).join(ENGINE_VERSION)
    }

    pub fn install(
        &self,
        profile: ModelProfile,
        fetcher: &dyn ArtifactFetcher,
    ) -> Result<ModelFiles, OcrError> {
        let directory = self.profile_dir(profile);
        fs::create_dir_all(&directory).map_err(|source| OcrError::Io {
            path: directory.clone(),
            source,
        })?;
        let _guard = ModelInstallGuard::acquire(&directory)?;
        for artifact in profile.artifacts() {
            ensure_artifact(&directory, artifact, fetcher)?;
        }
        let files = verify_artifacts(&directory, profile)?;
        write_verified_marker(&directory, profile)?;
        Ok(files)
    }

    pub fn verify(&self, profile: ModelProfile) -> Result<ModelFiles, OcrError> {
        let directory = self.profile_dir(profile);
        if !verify_marker(&directory, profile)? {
            return Err(OcrError::ModelUnavailable(format!(
                "profile {} has no current verification marker; run `compass models install {}`",
                profile.name(),
                profile.name()
            )));
        }
        verify_artifacts(&directory, profile)
    }

    #[must_use]
    pub fn status(&self, profile: ModelProfile) -> ModelStatus {
        let installed = profile
            .artifacts()
            .iter()
            .all(|artifact| is_regular_file(&self.profile_dir(profile).join(artifact.name)));
        ModelStatus {
            profile: profile.name().to_owned(),
            installed,
            verified: self.verify(profile).is_ok(),
            bytes: profile
                .artifacts()
                .iter()
                .map(|artifact| artifact.size)
                .sum(),
            license: MODEL_LICENSE.to_owned(),
        }
    }
}

fn verify_artifacts(directory: &Path, profile: ModelProfile) -> Result<ModelFiles, OcrError> {
    let mut digests = BTreeMap::new();
    for artifact in profile.artifacts() {
        let path = directory.join(artifact.name);
        if !verify_artifact(&path, artifact)? {
            return Err(OcrError::ModelUnavailable(format!(
                "profile {} is missing or invalid; run `compass models install {}`",
                profile.name(),
                profile.name()
            )));
        }
        digests.insert(artifact.role.to_owned(), artifact.sha256.to_owned());
    }
    Ok(ModelFiles {
        detector: directory.join(profile.artifacts()[0].name),
        recognizer: directory.join(profile.artifacts()[1].name),
        dictionary: directory.join(profile.artifacts()[2].name),
        identity: OcrProfileIdentity {
            engine: "oar-ocr".to_owned(),
            engine_version: ENGINE_VERSION.to_owned(),
            profile: profile.name().to_owned(),
            model_digests: digests,
            languages: vec!["mul".to_owned()],
            preprocessing_version: OCR_PREPROCESSING_VERSION,
        },
    })
}

fn verify_marker(directory: &Path, profile: ModelProfile) -> Result<bool, OcrError> {
    let path = directory.join("verified.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(OcrError::Io { path, source }),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > VERIFIED_MARKER_MAX_BYTES
    {
        return Ok(false);
    }
    let file = File::open(&path).map_err(|source| OcrError::Io {
        path: path.clone(),
        source,
    })?;
    let opened_metadata = file.metadata().map_err(|source| OcrError::Io {
        path: path.clone(),
        source,
    })?;
    if !opened_metadata.is_file() || opened_metadata.len() > VERIFIED_MARKER_MAX_BYTES {
        return Ok(false);
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.take(VERIFIED_MARKER_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| OcrError::Io {
            path: path.clone(),
            source,
        })?;
    if bytes.len() as u64 > VERIFIED_MARKER_MAX_BYTES {
        return Ok(false);
    }
    let Ok(value) = serde_json::from_slice::<VerifiedProfileMarker>(&bytes) else {
        return Ok(false);
    };
    Ok(value == expected_verified_marker(profile))
}

fn write_verified_marker(directory: &Path, profile: ModelProfile) -> Result<(), OcrError> {
    let destination = directory.join("verified.json");
    let marker = expected_verified_marker(profile);
    let mut temporary =
        tempfile::NamedTempFile::new_in(directory).map_err(|source| OcrError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    serde_json::to_writer(&mut temporary, &marker)
        .map_err(|error| OcrError::ModelVerification(error.to_string()))?;
    temporary.flush().map_err(|source| OcrError::Io {
        path: destination.clone(),
        source,
    })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| OcrError::Io {
            path: destination.clone(),
            source,
        })?;
    temporary
        .persist(&destination)
        .map_err(|error| OcrError::Io {
            path: destination,
            source: error.error,
        })?;
    sync_directory(directory)?;
    Ok(())
}

fn expected_verified_marker(profile: ModelProfile) -> VerifiedProfileMarker {
    VerifiedProfileMarker {
        schema: VERIFIED_MARKER_SCHEMA.to_owned(),
        profile: profile.name().to_owned(),
        engine: "oar-ocr".to_owned(),
        engine_version: ENGINE_VERSION.to_owned(),
        repository: MODEL_REPOSITORY.to_owned(),
        revision: MODEL_REVISION.to_owned(),
        manifest_digest: profile_manifest_digest(profile),
        license: MODEL_LICENSE.to_owned(),
        model_card: MODEL_CARD.to_owned(),
    }
}

pub fn install_profile(profile: ModelProfile) -> Result<ModelFiles, OcrError> {
    crate::ensure_managed_runtime_available()?;
    ModelCache::from_environment()?.install(profile, &HttpsArtifactFetcher::default())
}

pub fn verify_profile(profile: ModelProfile) -> Result<ModelFiles, OcrError> {
    ModelCache::from_environment()?.verify(profile)
}

pub fn list_profiles() -> Result<Vec<ModelStatus>, OcrError> {
    let cache = ModelCache::from_environment()?;
    Ok([ModelProfile::PpOcrV6Small, ModelProfile::PpOcrV6Medium]
        .into_iter()
        .map(|profile| cache.status(profile))
        .collect())
}

fn ensure_artifact(
    directory: &Path,
    artifact: &ArtifactSpec,
    fetcher: &dyn ArtifactFetcher,
) -> Result<(), OcrError> {
    let destination = directory.join(artifact.name);
    if verify_artifact(&destination, artifact)? {
        return Ok(());
    }
    let url = format!(
        "https://{MODEL_HOST}/{MODEL_REPOSITORY}/releases/download/{MODEL_REVISION}/{}",
        artifact.name
    );
    let mut reader = fetcher.fetch(&url, artifact.size)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(directory).map_err(|source| OcrError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|source| OcrError::Io {
            path: destination.clone(),
            source,
        })?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| OcrError::ModelVerification("model size overflow".to_owned()))?;
        if total > artifact.size {
            return Err(OcrError::ModelVerification(format!(
                "{} exceeds its declared size",
                artifact.name
            )));
        }
        hasher.update(&buffer[..read]);
        temporary
            .write_all(&buffer[..read])
            .map_err(|source| OcrError::Io {
                path: destination.clone(),
                source,
            })?;
    }
    let digest = format!("{:x}", hasher.finalize());
    if total != artifact.size || digest != artifact.sha256 {
        return Err(OcrError::ModelVerification(format!(
            "{} failed size or SHA-256 verification",
            artifact.name
        )));
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| OcrError::Io {
            path: destination.clone(),
            source,
        })?;
    temporary
        .persist(&destination)
        .map_err(|error| OcrError::Io {
            path: destination.clone(),
            source: error.error,
        })?;
    sync_directory(directory)?;
    Ok(())
}

fn verify_artifact(path: &Path, artifact: &ArtifactSpec) -> Result<bool, OcrError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(OcrError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != artifact.size {
        return Ok(false);
    }
    let file = File::open(path).map_err(|source| OcrError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let opened_metadata = file.metadata().map_err(|source| OcrError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !opened_metadata.is_file() || opened_metadata.len() != artifact.size {
        return Ok(false);
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut total = 0_u64;
    let mut bounded = file.take(artifact.size.saturating_add(1));
    loop {
        let read = bounded.read(&mut buffer).map_err(|source| OcrError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
    }
    Ok(total == artifact.size && format!("{:x}", hasher.finalize()) == artifact.sha256)
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

fn sync_directory(path: &Path) -> Result<(), OcrError> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| OcrError::Io {
                path: path.to_path_buf(),
                source,
            })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::Cursor;

    use super::*;

    struct StaticFetcher {
        body: Vec<u8>,
        calls: Cell<usize>,
    }

    impl ArtifactFetcher for StaticFetcher {
        fn fetch(&self, _url: &str, _max_bytes: u64) -> Result<Box<dyn Read>, OcrError> {
            self.calls.set(self.calls.get() + 1);
            Ok(Box::new(Cursor::new(self.body.clone())))
        }
    }

    #[test]
    fn profiles_are_pinned_and_complete() {
        for profile in [ModelProfile::PpOcrV6Small, ModelProfile::PpOcrV6Medium] {
            assert_eq!(profile.artifacts().len(), 3);
            assert!(profile.artifacts().iter().all(|artifact| {
                artifact.size > 0
                    && artifact.sha256.len() == 64
                    && artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            }));
        }
    }

    #[test]
    fn model_urls_require_https_and_the_fixed_release_hosts() {
        assert!(
            validate_model_url(
                "https://github.com/GreatV/oar-ocr/releases/download/v0.7.0/model.onnx",
                false
            )
            .is_ok()
        );
        assert!(validate_model_url(
            "https://release-assets.githubusercontent.com/github-production-release-asset/model",
            true
        )
        .is_ok());
        assert!(validate_model_url("http://github.com/model", false).is_err());
        assert!(validate_model_url("https://example.com/model", true).is_err());
        assert!(validate_model_url("https://user@github.com/model", true).is_err());
        assert!(validate_model_url("https://github.com:444/model", true).is_err());
    }

    #[test]
    fn concurrent_model_install_lock_is_bounded_and_reusable()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let first =
            ModelInstallGuard::acquire_with_timeout(directory.path(), Duration::from_secs(1))?;
        let blocked = ModelInstallGuard::acquire_with_timeout(directory.path(), Duration::ZERO);
        assert!(matches!(blocked, Err(OcrError::ModelUnavailable(_))));
        drop(first);
        let second = ModelInstallGuard::acquire_with_timeout(directory.path(), Duration::ZERO)?;
        drop(second);
        Ok(())
    }

    #[test]
    fn wrong_download_never_publishes() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let fetcher = StaticFetcher {
            body: b"wrong".to_vec(),
            calls: Cell::new(0),
        };
        let result = ensure_artifact(directory.path(), &DICTIONARY, &fetcher);
        assert!(result.is_err());
        assert!(!directory.path().join(DICTIONARY.name).exists());
        assert_eq!(fetcher.calls.get(), 1);
        Ok(())
    }

    #[test]
    fn verified_download_is_atomic_reusable_and_revision_marked()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let artifact = ArtifactSpec {
            role: "fixture",
            name: "fixture.bin",
            size: 7,
            sha256: "f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d",
        };
        let fetcher = StaticFetcher {
            body: b"fixture".to_vec(),
            calls: Cell::new(0),
        };
        ensure_artifact(directory.path(), &artifact, &fetcher)?;
        ensure_artifact(directory.path(), &artifact, &fetcher)?;
        assert_eq!(fetcher.calls.get(), 1);
        assert_eq!(fs::read(directory.path().join("fixture.bin"))?, b"fixture");

        write_verified_marker(directory.path(), ModelProfile::PpOcrV6Small)?;
        let marker: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("verified.json"))?)?;
        assert_eq!(marker["schema"], VERIFIED_MARKER_SCHEMA);
        assert_eq!(marker["revision"], "v0.7.0");
        assert_eq!(
            marker["manifest_digest"],
            profile_manifest_digest(ModelProfile::PpOcrV6Small)
        );
        assert!(verify_marker(directory.path(), ModelProfile::PpOcrV6Small)?);
        let mut unknown = marker.clone();
        unknown["unexpected"] = serde_json::json!(true);
        fs::write(
            directory.path().join("verified.json"),
            serde_json::to_vec(&unknown)?,
        )?;
        assert!(!verify_marker(
            directory.path(),
            ModelProfile::PpOcrV6Small
        )?);
        fs::write(
            directory.path().join("verified.json"),
            serde_json::to_vec(&marker)?,
        )?;
        let mut stale = marker;
        stale["revision"] = serde_json::json!("mutable-branch");
        fs::write(
            directory.path().join("verified.json"),
            serde_json::to_vec(&stale)?,
        )?;
        assert!(!verify_marker(
            directory.path(),
            ModelProfile::PpOcrV6Small
        )?);
        let oversized = File::create(directory.path().join("verified.json"))?;
        oversized.set_len(VERIFIED_MARKER_MAX_BYTES + 1)?;
        assert!(!verify_marker(
            directory.path(),
            ModelProfile::PpOcrV6Small
        )?);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn verification_rejects_symlinked_artifacts_and_markers()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let target = directory.path().join("target.bin");
        fs::write(&target, b"fixture")?;
        let link = directory.path().join("fixture.bin");
        symlink(&target, &link)?;
        let artifact = ArtifactSpec {
            role: "fixture",
            name: "fixture.bin",
            size: 7,
            sha256: "f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d",
        };
        assert!(!verify_artifact(&link, &artifact)?);

        let marker_target = directory.path().join("marker-target.json");
        fs::write(
            &marker_target,
            serde_json::to_vec(&expected_verified_marker(ModelProfile::PpOcrV6Small))?,
        )?;
        symlink(&marker_target, directory.path().join("verified.json"))?;
        assert!(!verify_marker(
            directory.path(),
            ModelProfile::PpOcrV6Small
        )?);
        Ok(())
    }
}
