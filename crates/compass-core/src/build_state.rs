use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use compass_files::{FileError, write_json_atomic};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::CoreError;

pub(crate) const BUILD_STATE_FILE: &str = "build-state.json";
const BUILD_STATE_SCHEMA: &str = "compass.build-state/1";
const HASH_BUFFER_BYTES: usize = 1024 * 1024;

fn current_build_fingerprint() -> String {
    let mut digest = Sha256::new();
    for component in [
        compass_model::code_graph::CODE_GRAPH_SCHEMA_V1,
        compass_graph::V1_PUBLICATION_SEMANTICS_VERSION,
        compass_languages::EXTRACTION_SEMANTICS_VERSION,
        compass_files::AST_CACHE_VERSION,
    ] {
        digest.update(component.as_bytes());
        digest.update([0]);
    }
    digest.update(compass_files::CACHE_ENCODING_VERSION.to_le_bytes());
    format!("sha256:{:x}", digest.finalize())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ArtifactSeal {
    pub bytes: u64,
    pub sha256: String,
}

impl ArtifactSeal {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }

    pub(crate) fn capture(path: &Path) -> Result<Self, CoreError> {
        let metadata = fs::metadata(path).map_err(|source| FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(CoreError::InvalidBuildState(format!(
                "artifact is not a regular file: {}",
                path.display()
            )));
        }
        let mut file = File::open(path).map_err(|source| FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
        let mut bytes = 0_u64;
        loop {
            let read = file.read(&mut buffer).map_err(|source| FileError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
            bytes = bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        }
        Ok(Self {
            bytes,
            sha256: format!("{:x}", digest.finalize()),
        })
    }

    fn matches(&self, path: &Path) -> bool {
        fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == self.bytes)
            && Self::capture(path).is_ok_and(|actual| actual == *self)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct BuildProfile {
    pub purpose: String,
    pub no_cluster: bool,
    pub no_viz: bool,
    pub resolution: f64,
    pub exclude_hubs: Option<f64>,
    #[serde(default)]
    pub code_only: bool,
    pub program_analysis: bool,
    pub graph_storage: String,
    #[serde(
        default = "legacy_default_inference_level",
        skip_serializing_if = "inference_level_is_legacy_max"
    )]
    pub inference_level: String,
    #[serde(default = "default_max_source_bytes")]
    pub max_source_bytes: u64,
    #[serde(default = "default_document_processing_identity")]
    pub document_processing_identity: String,
}

// Build-state schema 1 omitted the historical max profile. Keep interpreting
// an absent field as max even though new builds default to low. New low
// profiles serialize the field explicitly, so the first build after the
// cutover cannot reuse a max graph as though it were low.
fn legacy_default_inference_level() -> String {
    compass_graph::InferenceLevel::Max.as_str().to_owned()
}

fn inference_level_is_legacy_max(level: &str) -> bool {
    level == compass_graph::InferenceLevel::Max.as_str()
}

const fn default_max_source_bytes() -> u64 {
    crate::pipeline::DEFAULT_MAX_SOURCE_BYTES
}

fn default_document_processing_identity() -> String {
    crate::PreparedDocumentSet::default().cache_identity
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedStats {
    pub files: usize,
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    #[serde(default)]
    pub omitted_nodes: usize,
    #[serde(default)]
    pub omitted_edges: usize,
    #[serde(default)]
    pub identity_collisions: usize,
    pub program_modules: usize,
    pub program_summaries: usize,
    pub program_providers: usize,
    pub program_conflicts: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BuildState {
    schema: String,
    producer: String,
    profile: BuildProfile,
    manifest: ArtifactSeal,
    graph: ArtifactSeal,
    program: Option<ArtifactSeal>,
    required: BTreeMap<String, ArtifactSeal>,
    pub stats: SavedStats,
}

impl BuildState {
    pub(crate) fn capture(
        output_dir: &Path,
        profile: BuildProfile,
        manifest_path: &Path,
        graph_seal: Option<ArtifactSeal>,
        program_seal: Option<ArtifactSeal>,
        required_paths: &[PathBuf],
        stats: SavedStats,
    ) -> Result<Self, CoreError> {
        let program = if profile.program_analysis {
            Some(program_seal.map_or_else(
                || ArtifactSeal::capture(&output_dir.join("program.json")),
                Ok,
            )?)
        } else {
            None
        };
        let ((manifest, graph), required) = rayon::join(
            || {
                rayon::join(
                    || ArtifactSeal::capture(manifest_path),
                    || match graph_seal {
                        Some(seal) => Ok(seal),
                        None => ArtifactSeal::capture(&output_dir.join("graph.json")),
                    },
                )
            },
            || {
                required_paths
                    .par_iter()
                    .map(|path| {
                        let name = path
                            .strip_prefix(output_dir)
                            .map_err(|_| {
                                CoreError::InvalidBuildState(format!(
                                    "required artifact is outside output directory: {}",
                                    path.display()
                                ))
                            })?
                            .to_string_lossy()
                            .into_owned();
                        Ok((name, ArtifactSeal::capture(path)?))
                    })
                    .collect::<Result<BTreeMap<_, _>, CoreError>>()
            },
        );
        Ok(Self {
            schema: BUILD_STATE_SCHEMA.to_owned(),
            producer: current_build_fingerprint(),
            profile,
            manifest: manifest?,
            graph: graph?,
            program,
            required: required?,
            stats,
        })
    }

    pub(crate) fn save(&self, output_dir: &Path) -> Result<(), CoreError> {
        write_json_atomic(output_dir.join(BUILD_STATE_FILE), self, true)?;
        Ok(())
    }
}

pub(crate) fn load_verified(
    output_dir: &Path,
    profile: &BuildProfile,
    manifest_path: &Path,
    prior_build_complete: bool,
) -> Result<Option<BuildState>, CoreError> {
    if !prior_build_complete {
        return Ok(None);
    }
    let bytes = match fs::read(output_dir.join(BUILD_STATE_FILE)) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let state = match serde_json::from_slice::<BuildState>(&bytes) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };
    if state.schema != BUILD_STATE_SCHEMA
        || state.producer != current_build_fingerprint()
        || state.profile != *profile
        || !state.manifest.matches(manifest_path)
    {
        return Ok(None);
    }
    let (graph_matches, (program_matches, required_match)) = rayon::join(
        || state.graph.matches(&output_dir.join("graph.json")),
        || {
            rayon::join(
                || match (&state.program, state.profile.program_analysis) {
                    (Some(seal), true) => seal.matches(&output_dir.join("program.json")),
                    (None, false) => true,
                    _ => false,
                },
                || {
                    state
                        .required
                        .par_iter()
                        .all(|(name, seal)| seal.matches(&output_dir.join(name)))
                },
            )
        },
    );
    if !graph_matches || !program_matches || !required_match {
        return Ok(None);
    }
    Ok(Some(state))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io::{Seek, SeekFrom, Write};

    use super::*;

    #[test]
    fn artifact_seal_rejects_same_size_modification() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("artifact");
        fs::write(&path, b"original")?;
        let seal = ArtifactSeal::capture(&path)?;
        let mut file = fs::OpenOptions::new().write(true).open(&path)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(b"modified")?;
        assert_eq!(fs::metadata(&path)?.len(), seal.bytes);
        assert!(!seal.matches(&path));
        Ok(())
    }

    #[test]
    fn legacy_build_profiles_default_to_max_inference() -> Result<(), Box<dyn Error>> {
        let profile = BuildProfile {
            purpose: "update".to_owned(),
            no_cluster: false,
            no_viz: true,
            resolution: 1.0,
            exclude_hubs: None,
            code_only: true,
            program_analysis: false,
            graph_storage: "json".to_owned(),
            inference_level: legacy_default_inference_level(),
            max_source_bytes: default_max_source_bytes(),
            document_processing_identity: default_document_processing_identity(),
        };
        let document = serde_json::to_value(&profile)?;
        assert!(document.get("inference_level").is_none());

        let restored: BuildProfile = serde_json::from_value(document)?;
        assert_eq!(restored.inference_level, "max");
        assert_eq!(restored, profile);
        Ok(())
    }

    #[test]
    fn verified_state_rejects_schema_profile_artifact_and_interruption_changes()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let output = directory.path();
        let manifest = output.join("manifest.json");
        let graph = output.join("graph.json");
        let program = output.join("program.json");
        let required = output.join("source-root.txt");
        fs::write(&manifest, b"manifest")?;
        fs::write(&graph, b"graph")?;
        fs::write(&program, b"program")?;
        fs::write(&required, b"root")?;
        let profile = BuildProfile {
            purpose: "update".to_owned(),
            no_cluster: false,
            no_viz: true,
            resolution: 1.0,
            exclude_hubs: None,
            code_only: false,
            program_analysis: true,
            graph_storage: "json".to_owned(),
            inference_level: legacy_default_inference_level(),
            max_source_bytes: default_max_source_bytes(),
            document_processing_identity: default_document_processing_identity(),
        };
        let state = BuildState::capture(
            output,
            profile.clone(),
            &manifest,
            None,
            None,
            std::slice::from_ref(&required),
            SavedStats::default(),
        )?;
        state.save(output)?;
        assert!(load_verified(output, &profile, &manifest, true)?.is_some());
        assert!(load_verified(output, &profile, &manifest, false)?.is_none());

        let mut mismatch = profile.clone();
        mismatch.no_cluster = true;
        assert!(load_verified(output, &mismatch, &manifest, true)?.is_none());

        fs::write(&program, b"PROGRAM")?;
        assert!(load_verified(output, &profile, &manifest, true)?.is_none());
        fs::write(&program, b"program")?;
        fs::write(&graph, b"GRAPH")?;
        assert!(load_verified(output, &profile, &manifest, true)?.is_none());
        fs::write(&graph, b"graph")?;
        fs::write(&required, b"ROOT")?;
        assert!(load_verified(output, &profile, &manifest, true)?.is_none());
        fs::write(&required, b"root")?;
        assert!(load_verified(output, &profile, &manifest, true)?.is_some());

        let state_path = output.join(BUILD_STATE_FILE);
        let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&state_path)?)?;
        document["schema"] = serde_json::Value::String("unsupported".to_owned());
        fs::write(&state_path, serde_json::to_vec(&document)?)?;
        assert!(load_verified(output, &profile, &manifest, true)?.is_none());

        state.save(output)?;
        let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&state_path)?)?;
        document["producer"] = serde_json::Value::String("legacy-builder".to_owned());
        fs::write(&state_path, serde_json::to_vec(&document)?)?;
        assert!(load_verified(output, &profile, &manifest, true)?.is_none());
        Ok(())
    }
}
