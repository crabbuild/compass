//! Universal evidence profile for Scala 2 and Scala 3.

use std::path::Path;

use tree_sitter::Node;

use super::model::SemanticEvidenceBatch;
use super::shared::{self, LanguageProfile};
use super::validate::EvidenceError;

struct Scala;

impl LanguageProfile for Scala {
    const LANGUAGE: &'static str = "scala";

    fn package_name(source: &[u8]) -> Option<String> {
        shared::package_name_from_source(source)
    }

    fn declaration_kind(kind: &str) -> Option<&'static str> {
        let lower = kind.to_ascii_lowercase();
        shared::shared_declaration_kind(kind)
            .or_else(|| (lower.contains("val_") || lower.contains("var_")).then_some("field"))
    }
}

pub(super) fn emit_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    shared::emit_tree_evidence::<Scala>(path, source_file, source, root)
}
