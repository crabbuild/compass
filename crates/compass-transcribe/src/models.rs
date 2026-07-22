//! Reproducible, bounded model artifact acquisition from official OpenAI repositories.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};
use sha2::Sha256;

const USER_AGENT: &str = "compass/0.1 model-fetch";
const MODEL_HOST: &str = "huggingface.co";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestKind {
    Sha256,
    GitBlobSha1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactSpec {
    pub name: &'static str,
    pub size: u64,
    pub digest_kind: DigestKind,
    pub digest: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpec {
    pub name: &'static str,
    pub repository: &'static str,
    pub revision: &'static str,
    pub artifacts: &'static [ArtifactSpec],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles {
    pub config: PathBuf,
    pub weights: PathBuf,
    pub generation_config: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("unsupported Whisper model {0:?}")]
    UnsupportedModel(String),
    #[error("could not determine Compass's model cache directory")]
    MissingCacheDirectory,
    #[error("could not {action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("model download failed for {url}: {message}")]
    Download { url: String, message: String },
    #[error("model artifact {path} has size {actual}; expected {expected}")]
    Size {
        path: PathBuf,
        actual: u64,
        expected: u64,
    },
    #[error("model artifact {path} failed digest verification")]
    Digest { path: PathBuf },
    #[error("invalid verification marker {path}: {message}")]
    Marker { path: PathBuf, message: String },
}

pub trait ArtifactFetcher {
    fn fetch(&self, url: &str, max_bytes: u64) -> Result<Box<dyn Read>, String>;
}

#[derive(Clone)]
pub struct HttpsArtifactFetcher {
    agent: ureq::Agent,
}

impl HttpsArtifactFetcher {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(5)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for HttpsArtifactFetcher {
    fn default() -> Self {
        Self::new(Duration::from_secs(15 * 60))
    }
}

impl ArtifactFetcher for HttpsArtifactFetcher {
    fn fetch(&self, url: &str, max_bytes: u64) -> Result<Box<dyn Read>, String> {
        let response = self
            .agent
            .get(url)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|error| error.to_string())?;
        let limit = max_bytes
            .checked_add(1)
            .ok_or_else(|| "artifact size limit overflowed".to_owned())?;
        Ok(Box::new(
            response
                .into_body()
                .into_with_config()
                .limit(limit)
                .reader(),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct ModelCache {
    root: PathBuf,
}

impl ModelCache {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn from_environment() -> Result<Self, ModelError> {
        if let Some(root) = std::env::var_os("COMPASS_CACHE_DIR") {
            return Ok(Self::new(PathBuf::from(root).join("models")));
        }
        if let Some(root) = std::env::var_os("XDG_CACHE_HOME") {
            return Ok(Self::new(PathBuf::from(root).join("compass/models")));
        }
        if cfg!(windows)
            && let Some(root) = std::env::var_os("LOCALAPPDATA")
        {
            return Ok(Self::new(PathBuf::from(root).join("compass/models")));
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|root| Self::new(root.join(".cache/compass/models")))
            .ok_or(ModelError::MissingCacheDirectory)
    }

    pub fn ensure_model(
        &self,
        name: &str,
        fetcher: &dyn ArtifactFetcher,
    ) -> Result<ModelFiles, ModelError> {
        let spec = model_spec(name).ok_or_else(|| ModelError::UnsupportedModel(name.to_owned()))?;
        self.ensure_spec(spec, fetcher)
    }

    fn ensure_spec(
        &self,
        spec: &ModelSpec,
        fetcher: &dyn ArtifactFetcher,
    ) -> Result<ModelFiles, ModelError> {
        let directory = self.root.join(spec.name).join(spec.revision);
        fs::create_dir_all(&directory).map_err(|source| ModelError::Io {
            action: "create model cache directory",
            path: directory.clone(),
            source,
        })?;
        for artifact in spec.artifacts {
            ensure_artifact(&directory, spec, artifact, fetcher)?;
        }
        Ok(ModelFiles {
            config: directory.join("config.json"),
            weights: directory.join("model.safetensors"),
            generation_config: directory.join("generation_config.json"),
        })
    }
}

#[must_use]
pub fn model_spec(name: &str) -> Option<&'static ModelSpec> {
    let canonical = match name {
        "large" => "large-v3",
        "turbo" => "large-v3-turbo",
        other => other,
    };
    MODEL_SPECS.iter().find(|spec| spec.name == canonical)
}

fn ensure_artifact(
    directory: &Path,
    model: &ModelSpec,
    artifact: &ArtifactSpec,
    fetcher: &dyn ArtifactFetcher,
) -> Result<(), ModelError> {
    let destination = directory.join(artifact.name);
    if verified_marker_matches(&destination, artifact)? {
        return Ok(());
    }
    if destination.exists() && verify_file(&destination, artifact)? {
        write_verified_marker(&destination, artifact)?;
        return Ok(());
    }

    let url = artifact_url(model, artifact);
    let mut reader =
        fetcher
            .fetch(&url, artifact.size)
            .map_err(|message| ModelError::Download {
                url: url.clone(),
                message,
            })?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(directory).map_err(|source| ModelError::Io {
            action: "create temporary model artifact",
            path: directory.to_path_buf(),
            source,
        })?;
    let mut hasher = ArtifactHasher::new(artifact);
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|source| ModelError::Io {
            action: "download model artifact",
            path: destination.clone(),
            source,
        })?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > artifact.size {
            return Err(ModelError::Size {
                path: destination,
                actual: total,
                expected: artifact.size,
            });
        }
        hasher.update(&buffer[..read]);
        temporary
            .write_all(&buffer[..read])
            .map_err(|source| ModelError::Io {
                action: "write model artifact",
                path: destination.clone(),
                source,
            })?;
    }
    if total != artifact.size {
        return Err(ModelError::Size {
            path: destination,
            actual: total,
            expected: artifact.size,
        });
    }
    if hasher.finish() != artifact.digest {
        return Err(ModelError::Digest { path: destination });
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| ModelError::Io {
            action: "sync model artifact",
            path: destination.clone(),
            source,
        })?;
    temporary
        .persist(&destination)
        .map_err(|error| ModelError::Io {
            action: "publish model artifact",
            path: destination.clone(),
            source: error.error,
        })?;
    write_verified_marker(&destination, artifact)
}

fn artifact_url(model: &ModelSpec, artifact: &ArtifactSpec) -> String {
    format!(
        "https://{MODEL_HOST}/openai/{}/resolve/{}/{}?download=true",
        model.repository, model.revision, artifact.name
    )
}

#[derive(Debug, Serialize, Deserialize)]
struct VerifiedMarker {
    size: u64,
    modified_nanos: u128,
    digest: String,
}

fn marker_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".verified.json");
    PathBuf::from(name)
}

fn verified_marker_matches(path: &Path, artifact: &ArtifactSpec) -> Result<bool, ModelError> {
    let metadata = match path.metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(ModelError::Io {
                action: "inspect model artifact",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.len() != artifact.size {
        return Ok(false);
    }
    let marker = marker_path(path);
    let raw = match fs::read(&marker) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(ModelError::Io {
                action: "read model verification marker",
                path: marker,
                source,
            });
        }
    };
    let parsed: VerifiedMarker =
        serde_json::from_slice(&raw).map_err(|error| ModelError::Marker {
            path: marker,
            message: error.to_string(),
        })?;
    Ok(parsed.size == artifact.size
        && parsed.digest == artifact.digest
        && modified_nanos(&metadata) == Some(parsed.modified_nanos))
}

fn verify_file(path: &Path, artifact: &ArtifactSpec) -> Result<bool, ModelError> {
    let metadata = path.metadata().map_err(|source| ModelError::Io {
        action: "inspect model artifact",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() != artifact.size {
        return Ok(false);
    }
    let mut file = File::open(path).map_err(|source| ModelError::Io {
        action: "open model artifact",
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = ArtifactHasher::new(artifact);
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| ModelError::Io {
            action: "verify model artifact",
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish() == artifact.digest)
}

fn write_verified_marker(path: &Path, artifact: &ArtifactSpec) -> Result<(), ModelError> {
    let metadata = path.metadata().map_err(|source| ModelError::Io {
        action: "inspect verified model artifact",
        path: path.to_path_buf(),
        source,
    })?;
    let marker = VerifiedMarker {
        size: metadata.len(),
        modified_nanos: modified_nanos(&metadata).ok_or_else(|| ModelError::Marker {
            path: path.to_path_buf(),
            message: "modification time predates the Unix epoch".to_owned(),
        })?,
        digest: artifact.digest.to_owned(),
    };
    let marker_path = marker_path(path);
    let parent = marker_path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| ModelError::Io {
            action: "create model verification marker",
            path: marker_path.clone(),
            source,
        })?;
    serde_json::to_writer(&mut temporary, &marker).map_err(|error| ModelError::Marker {
        path: marker_path.clone(),
        message: error.to_string(),
    })?;
    temporary.flush().map_err(|source| ModelError::Io {
        action: "flush model verification marker",
        path: marker_path.clone(),
        source,
    })?;
    temporary
        .persist(&marker_path)
        .map_err(|error| ModelError::Io {
            action: "publish model verification marker",
            path: marker_path,
            source: error.error,
        })?;
    Ok(())
}

fn modified_nanos(metadata: &fs::Metadata) -> Option<u128> {
    metadata
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

enum ArtifactHasher {
    Sha256(Sha256),
    GitBlobSha1(Sha1),
}

impl ArtifactHasher {
    fn new(artifact: &ArtifactSpec) -> Self {
        match artifact.digest_kind {
            DigestKind::Sha256 => Self::Sha256(Sha256::new()),
            DigestKind::GitBlobSha1 => {
                let mut hasher = Sha1::new();
                hasher.update(format!("blob {}\0", artifact.size));
                Self::GitBlobSha1(hasher)
            }
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha256(hasher) => hasher.update(bytes),
            Self::GitBlobSha1(hasher) => hasher.update(bytes),
        }
    }

    fn finish(self) -> String {
        match self {
            Self::Sha256(hasher) => format!("{:x}", hasher.finalize()),
            Self::GitBlobSha1(hasher) => format!("{:x}", hasher.finalize()),
        }
    }
}

macro_rules! artifacts {
    ($config_size:literal, $config:literal, $generation_size:literal, $generation:literal, $weights_size:literal, $weights:literal) => {
        &[
            ArtifactSpec {
                name: "config.json",
                size: $config_size,
                digest_kind: DigestKind::GitBlobSha1,
                digest: $config,
            },
            ArtifactSpec {
                name: "generation_config.json",
                size: $generation_size,
                digest_kind: DigestKind::GitBlobSha1,
                digest: $generation,
            },
            ArtifactSpec {
                name: "model.safetensors",
                size: $weights_size,
                digest_kind: DigestKind::Sha256,
                digest: $weights,
            },
        ]
    };
}

pub static MODEL_SPECS: &[ModelSpec] = &[
    ModelSpec {
        name: "tiny",
        repository: "whisper-tiny",
        revision: "169d4a4341b33bc18d8881c4b69c2e104e1cc0af",
        artifacts: artifacts!(
            1983,
            "417aa9de49a132dd3eb6a56d3be2718b15f08917",
            3747,
            "4b26dd66b8f7bca37d851d259fdc118315cacc62",
            151061672,
            "7ebd0e69e78190ffe1438491fa05cc1f5c1aa3a4c4db3bc1723adbb551ea2395"
        ),
    },
    ModelSpec {
        name: "tiny.en",
        repository: "whisper-tiny.en",
        revision: "87c7102498dcde7456f24cfd30239ca606ed9063",
        artifacts: artifacts!(
            1937,
            "31c9f364d610705cd391e465d49df5f8e77fd868",
            1621,
            "974cd4239d99b2a5e21151e9d5023f073bbfc4d6",
            151060136,
            "db59695928ded6043adaef491a53ef4e12da9611184d77c53baa691a60b958ad"
        ),
    },
    ModelSpec {
        name: "base",
        repository: "whisper-base",
        revision: "e37978b90ca9030d5170a5c07aadb050351a65bb",
        artifacts: artifacts!(
            1983,
            "708cca09e7e8a9364e444efe8b4f9c2691fc5f51",
            3807,
            "38eda87a0b22fdb85af5dfa14cf065147337137a",
            290403936,
            "07cadb9f25677c8d50df603e66a98fbd842cce45047139baeb16e6219a1e807b"
        ),
    },
    ModelSpec {
        name: "base.en",
        repository: "whisper-base.en",
        revision: "911407f4214e0e1d82085af863093ec0b66f9cd6",
        artifacts: artifacts!(
            1937,
            "0e9af059a5d452f97d3493ffb4da98712c8fd711",
            1531,
            "1cde9fc2a32ad94f4db760e67a2375e229215f58",
            290401888,
            "d4dd5542fd6a1d35639e21384238f3bfe6c557c849d392b5905d33ee29e71db5"
        ),
    },
    ModelSpec {
        name: "small",
        repository: "whisper-small",
        revision: "973afd24965f72e36ca33b3055d56a652f456b4d",
        artifacts: artifacts!(
            1967,
            "113bb3efe3a7396f2ea629eef12637bd8085238d",
            3868,
            "d3823179a367fa912e3b911b3d2e71ab2a028290",
            966995080,
            "1d7734884874f1a1513ed9aa760a4f8e97aaa02fd6d93a3a85d27b2ae9ca596b"
        ),
    },
    ModelSpec {
        name: "small.en",
        repository: "whisper-small.en",
        revision: "e8727524f962ee844a7319d92be39ac1bd25655a",
        artifacts: artifacts!(
            1943,
            "441d82a4be08ee59609c374a4d5082641677466e",
            1931,
            "6b3e70541b0f1565a9d01901d0112abc19c88265",
            966992008,
            "6014ac49b506df900f66f4aca6b0801eed7245594ace97bcaf73e0ae5b863066"
        ),
    },
    ModelSpec {
        name: "medium",
        repository: "whisper-medium",
        revision: "abdf7c39ab9d0397620ccaea8974cc764cd0953e",
        artifacts: artifacts!(
            1991,
            "eda91ab1edc34a7c54512ed7a53e643b202387af",
            3755,
            "dda8239f83dec12c4a47e4b4c82af3e7f3391855",
            3055544304,
            "62f73550fa6db24b0c6f6c5962bd0dae80fa644e93cde9cd9c3792971b47fd28"
        ),
    },
    ModelSpec {
        name: "medium.en",
        repository: "whisper-medium.en",
        revision: "2e98eb6279edf5095af0c8dedb36bdec0acd172b",
        artifacts: artifacts!(
            1945,
            "64c74fbc447ac03a91057284c2bd1dc94dcbc427",
            1947,
            "025030a7a902c19626f704dd56329ae2d162762b",
            3055540208,
            "8f731340f569588236c21f794c33e83b7cc5a297511d2bdc30117ea969004311"
        ),
    },
    ModelSpec {
        name: "large-v1",
        repository: "whisper-large",
        revision: "4ef9b41f0d4fe232daafdb5f76bb1dd8b23e01d7",
        artifacts: artifacts!(
            1990,
            "0c2e9761f048be4bf412ddbb63bcd8715b82783e",
            3850,
            "84475ad47acf3a724346bbcb8baa87a6016b4c43",
            6173370152,
            "27d753181b54178da228555dbc57fe639f6624ca470d83d35e500b22df0ab7e6"
        ),
    },
    ModelSpec {
        name: "large-v2",
        repository: "whisper-large-v2",
        revision: "ae4642769ce2ad8fc292556ccea8e901f1530655",
        artifacts: artifacts!(
            1993,
            "1ce74630ed587e80f3db2b3d434f7026327f131e",
            4294,
            "11d6bb6b68b567e1ab71ace46594e7d5311d4271",
            6173370152,
            "57a1ba2a82c093cabff2541409ae778c97145378b9ddfa722763cb1cb8f9020b"
        ),
    },
    ModelSpec {
        name: "large-v3",
        repository: "whisper-large-v3",
        revision: "06f233fe06e710322aca913c1bc4249a0d71fce1",
        artifacts: artifacts!(
            1272,
            "14c6c8cf48b64ebb1cb8b637e2b0fab3a9774972",
            3903,
            "f3294dfe3654ac4e362570867369ec48104af59f",
            3087130976,
            "a8e94b85976e5864ba3e9525c7e6c83b2a1eca42d4b797a0c7c24d778e40fd95"
        ),
    },
    ModelSpec {
        name: "large-v3-turbo",
        repository: "whisper-large-v3-turbo",
        revision: "41f01f3fe87f28c78e2fbf8b568835947dd65ed9",
        artifacts: artifacts!(
            1256,
            "ad2f44ff1ed66e12765b2392dc041469db91a462",
            3772,
            "cbe752958dc3e4671b0e0220aa1c545423a6d5f5",
            1617824864,
            "542566a422ae4f3fd23f1ba11add198fca01bbf82e66e6a2857b3f608b1eb9d1"
        ),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::Cursor;

    struct StaticFetcher {
        body: Vec<u8>,
        calls: Cell<usize>,
    }

    impl ArtifactFetcher for StaticFetcher {
        fn fetch(&self, _url: &str, _max_bytes: u64) -> Result<Box<dyn Read>, String> {
            self.calls.set(self.calls.get() + 1);
            Ok(Box::new(Cursor::new(self.body.clone())))
        }
    }

    const TEST_ARTIFACT: ArtifactSpec = ArtifactSpec {
        name: "config.json",
        size: 3,
        digest_kind: DigestKind::Sha256,
        digest: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    };
    const TEST_MODEL: ModelSpec = ModelSpec {
        name: "test",
        repository: "test",
        revision: "0123456789abcdef0123456789abcdef01234567",
        artifacts: &[TEST_ARTIFACT],
    };

    #[test]
    fn official_model_catalog_is_pinned_and_complete() {
        assert_eq!(MODEL_SPECS.len(), 12);
        assert_eq!(model_spec("large"), model_spec("large-v3"));
        assert_eq!(model_spec("turbo"), model_spec("large-v3-turbo"));
        for model in MODEL_SPECS {
            assert_eq!(model.revision.len(), 40);
            assert_eq!(model.artifacts.len(), 3);
            assert!(model.artifacts.iter().all(|artifact| artifact.size > 0));
            assert!(
                model
                    .artifacts
                    .iter()
                    .all(|artifact| artifact.digest.len() == 40 || artifact.digest.len() == 64)
            );
        }
    }

    #[test]
    fn artifact_download_is_verified_atomic_and_warm_cached()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let fetcher = StaticFetcher {
            body: b"abc".to_vec(),
            calls: Cell::new(0),
        };
        ensure_artifact(directory.path(), &TEST_MODEL, &TEST_ARTIFACT, &fetcher)?;
        assert_eq!(fs::read(directory.path().join("config.json"))?, b"abc");
        assert_eq!(fetcher.calls.get(), 1);
        ensure_artifact(directory.path(), &TEST_MODEL, &TEST_ARTIFACT, &fetcher)?;
        assert_eq!(fetcher.calls.get(), 1);
        Ok(())
    }

    #[test]
    fn wrong_digest_never_publishes_artifact() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let fetcher = StaticFetcher {
            body: b"abd".to_vec(),
            calls: Cell::new(0),
        };
        let result = ensure_artifact(directory.path(), &TEST_MODEL, &TEST_ARTIFACT, &fetcher);
        assert!(matches!(result, Err(ModelError::Digest { .. })));
        assert!(!directory.path().join("config.json").exists());
        Ok(())
    }

    #[test]
    fn oversized_response_is_rejected_before_publish() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let fetcher = StaticFetcher {
            body: b"abcd".to_vec(),
            calls: Cell::new(0),
        };
        let result = ensure_artifact(directory.path(), &TEST_MODEL, &TEST_ARTIFACT, &fetcher);
        assert!(matches!(result, Err(ModelError::Size { actual: 4, .. })));
        assert!(!directory.path().join("config.json").exists());
        Ok(())
    }

    #[test]
    fn git_blob_digest_matches_git_object_contract() {
        let artifact = ArtifactSpec {
            name: "config.json",
            size: 3,
            digest_kind: DigestKind::GitBlobSha1,
            digest: "f2ba8f84ab5c1bce84a7b441cb1959cfc7093b7f",
        };
        let mut hasher = ArtifactHasher::new(&artifact);
        hasher.update(b"abc");
        assert_eq!(hasher.finish(), artifact.digest);
    }

    struct ErrorFetcher;

    impl ArtifactFetcher for ErrorFetcher {
        fn fetch(&self, _url: &str, _max_bytes: u64) -> Result<Box<dyn Read>, String> {
            Err("offline".to_owned())
        }
    }

    struct ReadError;

    impl Read for ReadError {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("broken stream"))
        }
    }

    struct ReadErrorFetcher;

    impl ArtifactFetcher for ReadErrorFetcher {
        fn fetch(&self, _url: &str, _max_bytes: u64) -> Result<Box<dyn Read>, String> {
            Ok(Box::new(ReadError))
        }
    }

    #[test]
    fn download_failures_short_reads_and_invalid_existing_files_are_explicit()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        assert!(matches!(
            ensure_artifact(directory.path(), &TEST_MODEL, &TEST_ARTIFACT, &ErrorFetcher),
            Err(ModelError::Download { .. })
        ));
        assert!(matches!(
            ensure_artifact(
                directory.path(),
                &TEST_MODEL,
                &TEST_ARTIFACT,
                &ReadErrorFetcher
            ),
            Err(ModelError::Io {
                action: "download model artifact",
                ..
            })
        ));

        let short = StaticFetcher {
            body: b"ab".to_vec(),
            calls: Cell::new(0),
        };
        assert!(matches!(
            ensure_artifact(directory.path(), &TEST_MODEL, &TEST_ARTIFACT, &short),
            Err(ModelError::Size {
                actual: 2,
                expected: 3,
                ..
            })
        ));

        fs::write(directory.path().join("config.json"), b"bad")?;
        let good = StaticFetcher {
            body: b"abc".to_vec(),
            calls: Cell::new(0),
        };
        ensure_artifact(directory.path(), &TEST_MODEL, &TEST_ARTIFACT, &good)?;
        assert_eq!(good.calls.get(), 1);
        assert_eq!(fs::read(directory.path().join("config.json"))?, b"abc");
        Ok(())
    }

    #[test]
    fn verification_markers_must_match_size_digest_and_modification_time()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.json");
        fs::write(&path, b"abc")?;
        assert!(!verified_marker_matches(&path, &TEST_ARTIFACT)?);
        fs::write(marker_path(&path), b"not json")?;
        assert!(matches!(
            verified_marker_matches(&path, &TEST_ARTIFACT),
            Err(ModelError::Marker { .. })
        ));
        fs::write(
            marker_path(&path),
            serde_json::to_vec(&VerifiedMarker {
                size: 3,
                modified_nanos: 0,
                digest: "wrong".to_owned(),
            })?,
        )?;
        assert!(!verified_marker_matches(&path, &TEST_ARTIFACT)?);
        write_verified_marker(&path, &TEST_ARTIFACT)?;
        assert!(verified_marker_matches(&path, &TEST_ARTIFACT)?);

        let wrong_size = ArtifactSpec {
            size: 4,
            ..TEST_ARTIFACT
        };
        assert!(!verified_marker_matches(&path, &wrong_size)?);
        assert!(!verify_file(&path, &wrong_size)?);
        Ok(())
    }

    #[test]
    fn model_cache_builds_expected_layout_and_rejects_unknown_names()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let cache = ModelCache::new(directory.path().to_path_buf());
        assert!(matches!(
            cache.ensure_model("unknown", &ErrorFetcher),
            Err(ModelError::UnsupportedModel(_))
        ));
        let fetcher = StaticFetcher {
            body: b"abc".to_vec(),
            calls: Cell::new(0),
        };
        let files = cache.ensure_spec(&TEST_MODEL, &fetcher)?;
        assert!(
            files
                .config
                .ends_with("test/0123456789abcdef0123456789abcdef01234567/config.json")
        );
        assert!(files.weights.ends_with("model.safetensors"));
        assert!(files.generation_config.ends_with("generation_config.json"));
        assert!(model_spec("missing").is_none());
        assert!(
            artifact_url(&TEST_MODEL, &TEST_ARTIFACT)
                .starts_with("https://huggingface.co/openai/test/resolve/")
        );
        Ok(())
    }
}
