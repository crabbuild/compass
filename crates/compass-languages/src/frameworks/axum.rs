use std::path::Path;

use tree_sitter::Node;

use super::RawFrameworkFact;

/// Axum route calls and nested router scopes are intentionally isolated from
/// the Actix/Rocket attribute adapter. This keeps mixed Rust workspaces
/// deterministic and gives each framework a qualification seam of its own.
pub(super) fn detect(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    super::rust::detect_axum(path, source, root)
}
