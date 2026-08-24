use std::path::Path;

use super::RawFrameworkFact;
use crate::{Extraction, ProjectEvidence};
use tree_sitter::Node;

/// Remix route modules are file conventions with named data handlers. The
/// shared file-route implementation owns path normalization, convention
/// anchors, and synthetic default identities; this adapter only selects the
/// Remix project boundary.
pub(super) fn detect(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    project: Option<&ProjectEvidence>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    super::file_routes::detect_remix(path, source, root, project, extraction)
}
