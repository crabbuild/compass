use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use compass_files::{FileError, write_json_atomic};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::CoreError;

pub(crate) const BUILD_STATE_FILE: &str = ".compass_build_state.json";
const BUILD_STATE_SCHEMA: &str = "compass.build-state/1";
const HASH_BUFFER_BYTES: usize = 1024 * 1024;

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
    pub program_analysis: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedStats {
    pub files: usize,
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
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
                    || ArtifactSeal::capture(&output_dir.join("graph.json")),
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
            producer: env!("CARGO_PKG_VERSION").to_owned(),
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
        || state.profile != *profile
        || !state.manifest.matches(manifest_path)
        || !state.graph.matches(&output_dir.join("graph.json"))
        || state.profile.program_analysis
            != state
                .program
                .as_ref()
                .is_some_and(|seal| seal.matches(&output_dir.join("program.json")))
        || !state
            .required
            .iter()
            .all(|(name, seal)| seal.matches(&output_dir.join(name)))
    {
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
    fn verified_state_rejects_schema_profile_artifact_and_interruption_changes()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let output = directory.path();
        let manifest = output.join("manifest.json");
        let graph = output.join("graph.json");
        let program = output.join("program.json");
        let required = output.join(".compass_root");
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
            program_analysis: true,
        };
        let state = BuildState::capture(
            output,
            profile.clone(),
            &manifest,
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

        let state_path = output.join(BUILD_STATE_FILE);
        let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&state_path)?)?;
        document["schema"] = serde_json::Value::String("unsupported".to_owned());
        fs::write(&state_path, serde_json::to_vec(&document)?)?;
        assert!(load_verified(output, &profile, &manifest, true)?.is_none());
        Ok(())
    }
}
