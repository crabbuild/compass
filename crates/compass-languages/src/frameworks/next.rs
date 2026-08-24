use std::path::Path;

use super::RawFrameworkFact;
use crate::{Extraction, ProjectEvidence};
use tree_sitter::Node;

/// Next.js owns file-system route conventions; the shared file-route module
/// supplies bounded path normalization and handler identity construction.
pub(super) fn detect(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    project: Option<&ProjectEvidence>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    super::file_routes::detect_next(path, source, root, project, extraction)
}
