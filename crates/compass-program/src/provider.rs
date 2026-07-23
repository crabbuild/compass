use std::collections::BTreeMap;
use std::io::{Read, Seek};

use compass_ir::ProviderDescriptor;
use serde::{Deserialize, Serialize};

use crate::EvidenceBatch;

#[derive(Clone, Copy, Debug)]
pub struct FileInput<'a> {
    pub source_file: &'a str,
    pub language: &'a str,
    pub source: &'a [u8],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub schema: String,
    pub index_sha256: String,
    pub documents: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactLimits {
    pub max_artifact_bytes: u64,
    pub max_document_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_records: u64,
}

impl Default for ArtifactLimits {
    fn default() -> Self {
        Self {
            max_artifact_bytes: 2 * 1024 * 1024 * 1024,
            max_document_bytes: 64 * 1024 * 1024,
            max_metadata_bytes: 8 * 1024 * 1024,
            max_records: 50_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ArtifactInput<'a> {
    pub logical_name: &'a str,
    pub input_digest: &'a str,
    pub byte_len: u64,
    pub manifest: Option<&'a ArtifactManifest>,
    pub source_digests: &'a BTreeMap<String, String>,
    pub source_texts: &'a BTreeMap<String, Vec<u8>>,
    pub limits: ArtifactLimits,
}

#[derive(Clone, Copy, Debug)]
pub struct ProjectFile<'a> {
    pub source_file: &'a str,
    pub language: &'a str,
    pub source_digest: &'a str,
    pub source: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub struct ProjectInput<'a> {
    pub repository_digest: &'a str,
    pub build_context_digest: &'a str,
    pub files: &'a [ProjectFile<'a>],
}

pub trait SyntaxProvider {
    fn descriptor(&self, input: &FileInput<'_>) -> ProviderDescriptor;

    fn analyze_file(
        &mut self,
        input: FileInput<'_>,
    ) -> Result<Option<EvidenceBatch>, ProviderError>;
}

pub trait ArtifactReader: Read + Seek {}
impl<T: Read + Seek> ArtifactReader for T {}

pub trait ArtifactProvider {
    fn descriptor(&self, input: &ArtifactInput<'_>) -> ProviderDescriptor;

    fn analyze_artifact(
        &self,
        input: ArtifactInput<'_>,
        reader: &mut dyn ArtifactReader,
    ) -> Result<EvidenceBatch, ProviderError>;
}

pub trait ProjectAnalyzer {
    fn descriptor(&self, repository_digest: &str, build_context_digest: &str)
    -> ProviderDescriptor;

    fn analyze_project(&self, input: ProjectInput<'_>) -> Result<EvidenceBatch, ProviderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("unsafe source path {0}")]
    UnsafePath(String),
    #[error("invalid provider input: {0}")]
    InvalidInput(String),
    #[error("provider resource limit exceeded: {0}")]
    ResourceLimit(String),
    #[error("malformed provider artifact: {0}")]
    MalformedArtifact(String),
    #[error("unsupported provider artifact: {0}")]
    UnsupportedArtifact(String),
    #[error("provider I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
