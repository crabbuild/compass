use std::path::Path;

use tree_sitter::Node;

use super::RawFrameworkFact;
use crate::Extraction;

/// Express owns the receiver/mount/handler interpretation for JavaScript and
/// TypeScript. The shared TypeScript module only supplies the language-level
/// traversal helpers; keeping this adapter separate prevents unrelated router
/// conventions from activating in the Express pack.
pub(super) fn detect(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    super::typescript::detect_express(path, source, root, extraction)
}
