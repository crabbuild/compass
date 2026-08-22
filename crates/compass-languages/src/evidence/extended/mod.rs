//! AST-first universal evidence entry points for the extended language wave.
//!
//! Shared traversal and validation live in [`common`]. Each language keeps a
//! separate profile so syntax-specific policy and source supplements stay
//! reviewable without duplicating the evidence contract.

use std::path::Path;

use tree_sitter::Node;

use super::model::SemanticEvidenceBatch;
use super::validate::EvidenceError;

mod common;
pub(super) mod dart;
pub(super) mod groovy;
pub(super) mod scala;
pub(super) mod swift;

pub(super) fn emit_dart_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    dart::emit_tree_evidence(path, source_file, source, root)
}

pub(super) fn emit_groovy_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    groovy::emit_tree_evidence(path, source_file, source, root)
}

pub(super) fn emit_scala_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    scala::emit_tree_evidence(path, source_file, source, root)
}

pub(super) fn emit_swift_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    swift::emit_tree_evidence(path, source_file, source, root)
}
