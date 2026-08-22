//! Universal evidence profile for Dart.

use std::path::Path;

use tree_sitter::Node;

use super::super::model::SemanticEvidenceBatch;
use super::super::validate::EvidenceError;
use super::common::{self, LanguageProfile};

struct Dart;

impl LanguageProfile for Dart {
    const LANGUAGE: &'static str = "dart";

    fn declaration_kind(kind: &str) -> Option<&'static str> {
        let lower = kind.to_ascii_lowercase();
        common::shared_declaration_kind(kind)
            .or_else(|| (lower == "variable_declaration").then_some("field"))
    }

    fn declaration_lookup_name(name: &str) -> String {
        name.split_once('(')
            .map_or_else(|| name.to_owned(), |(base, _)| base.trim().to_owned())
    }
}

pub(super) fn emit_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    common::emit_tree_evidence::<Dart>(path, source_file, source, root)
}
