use std::path::Path;

use tree_sitter::Node;

use super::RawFrameworkFact;
use crate::Extraction;

/// Fastify keeps route registration in a fluent application object. The
/// TypeScript adapter supplies the shared literal route and middleware parser;
/// this module is the focused runtime ownership boundary for the framework.
pub(super) fn detect(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    super::typescript::detect_fastify(path, source, root, extraction)
}
