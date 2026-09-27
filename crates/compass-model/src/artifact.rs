//! One bounded read of a graph artifact, shared by all derived query views.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::GraphError;

/// Bytes and content identity from one bounded read of an opened graph file.
///
/// Derive related projections from this value to keep them bound to the same
/// bytes even when the path is replaced. This read does not lock an external
/// writer; publishers should continue to use atomic replacement. Decoding and
/// validation are performed by the document/projection methods.
#[derive(Debug)]
pub struct GraphArtifact {
    path: PathBuf,
    pub(crate) bytes: Vec<u8>,
    pub(crate) signature: GraphSignature,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GraphSignature {
    pub(crate) len: u64,
    pub(crate) digest: [u8; 32],
}

impl GraphArtifact {
    /// Read a `.json` artifact using the configured graph-size limit.
    pub fn load(path: &Path) -> Result<Self, GraphError> {
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            return Err(GraphError::InvalidExtension(path.to_path_buf()));
        }
        Self::load_for_recluster(path)
    }

    pub(crate) fn load_for_recluster(path: &Path) -> Result<Self, GraphError> {
        let file = File::open(path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                GraphError::NotFound(crate::graph::absolute_path(path))
            } else {
                GraphError::Read {
                    path: crate::graph::absolute_path(path),
                    source,
                }
            }
        })?;
        Self::read_opened(path, file, crate::graph::graph_size_cap())
    }

    fn read_opened(path: &Path, file: File, cap: u64) -> Result<Self, GraphError> {
        let size = file
            .metadata()
            .map_err(|source| GraphError::Read {
                path: crate::graph::absolute_path(path),
                source,
            })?
            .len();
        if size > cap {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size,
                cap,
            });
        }
        Self::read_bounded(path, file, cap)
    }

    fn read_bounded(path: &Path, reader: impl Read, cap: u64) -> Result<Self, GraphError> {
        let mut bytes = Vec::new();
        reader
            .take(cap.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| GraphError::Read {
                path: crate::graph::absolute_path(path),
                source,
            })?;
        let len = bytes.len() as u64;
        if len > cap {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size: len,
                cap,
            });
        }
        let digest = Sha256::digest(&bytes).into();
        Ok(Self {
            path: path.to_path_buf(),
            bytes,
            signature: GraphSignature { len, digest },
        })
    }

    /// Original path used for disposable caches and adjacent sidecars.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// SHA-256 of the exact bytes used to derive every view of this artifact.
    #[must_use]
    pub fn artifact_digest(&self) -> String {
        self.signature
            .digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opened_artifact_and_all_views_keep_the_original_bytes_after_replacement()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        let before = br#"{"nodes":[{"id":"a","label":"Alpha"}],"links":[]}"#;
        std::fs::write(&path, before)?;
        let opened = File::open(&path)?;
        let mut next = tempfile::NamedTempFile::new_in(directory.path())?;
        std::io::Write::write_all(
            &mut next,
            br#"{"nodes":[{"id":"b","label":"Omega"}],"links":[]}"#,
        )?;
        next.persist(&path)?;
        let snapshot = GraphArtifact::read_opened(&path, opened, 1024)?;
        assert_eq!(
            snapshot.artifact_digest(),
            format!("{:x}", Sha256::digest(before))
        );
        assert_eq!(snapshot.document()?.nodes[0].label(), "Alpha");
        assert_eq!(snapshot.traversal_document()?.nodes[0].label(), "Alpha");
        assert_eq!(
            GraphArtifact::load(&path)?.document()?.nodes[0].label(),
            "Omega"
        );
        Ok(())
    }

    #[test]
    fn artifact_read_is_bounded_even_without_a_metadata_size_hint() {
        assert!(matches!(
            GraphArtifact::read_bounded(Path::new("graph.json"), &b"123456789"[..], 8),
            Err(GraphError::TooLarge {
                size: 9,
                cap: 8,
                ..
            })
        ));
        assert!(GraphArtifact::read_bounded(Path::new("graph.json"), &b"12345678"[..], 8).is_ok());
    }
}
