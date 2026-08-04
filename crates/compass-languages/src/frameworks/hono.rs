use std::path::Path;

use tree_sitter::Node;

use super::RawFrameworkFact;
use crate::Extraction;

/// Hono route methods, `on` method arrays, base paths, and child mounts are
/// selected by a dedicated pack while reusing the bounded TypeScript router
/// primitives shared with Fastify and Express.
pub(super) fn detect(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    super::typescript::detect_hono(path, source, root, extraction)
}
