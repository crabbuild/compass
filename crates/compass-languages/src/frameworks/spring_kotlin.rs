use std::path::Path;

use tree_sitter::Node;

use super::RawFrameworkFact;

/// Kotlin keeps the established JVM syntax extractor, but owns a dedicated
/// adapter entry point so Spring can evolve without coupling its pack to other
/// Kotlin framework conventions.
pub(super) fn detect(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    super::java::detect(path, source, root)
}
