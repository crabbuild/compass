//! Universal evidence profile for Swift.

use std::path::Path;

use tree_sitter::Node;

use super::model::SemanticEvidenceBatch;
use super::shared::{self, LanguageProfile};
use super::validate::EvidenceError;

struct Swift;

impl LanguageProfile for Swift {
    const LANGUAGE: &'static str = "swift";

    fn emits_module_declarations() -> bool {
        true
    }
}

pub(super) fn emit_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    shared::emit_tree_evidence::<Swift>(path, source_file, source, root)
}
