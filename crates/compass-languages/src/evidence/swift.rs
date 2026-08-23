//! Universal evidence profile for Swift.

use std::path::Path;

use tree_sitter::Node;

use super::model::{CandidateRelation, SemanticEvidenceBatch};
use super::shared::{self, LanguageProfile};
use super::validate::EvidenceError;

struct Swift;

impl LanguageProfile for Swift {
    const LANGUAGE: &'static str = "swift";

    fn emits_module_declarations() -> bool {
        true
    }

    fn declaration_kind_for_node(node: Node<'_>, source: &[u8]) -> Option<&'static str> {
        if node.kind() == "typealias_declaration" {
            return Some("type_alias");
        }
        if node.kind() == "class_declaration"
            && let Some(text) = source
                .get(node.start_byte()..node.end_byte())
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
        {
            let header = text.split('{').next().unwrap_or(text);
            let has_keyword = |keyword: &str| {
                header
                    .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                    .any(|token| token == keyword)
            };
            if has_keyword("enum") {
                return Some("enum");
            }
            if has_keyword("struct") {
                return Some("struct");
            }
            if has_keyword("extension") {
                return Some("extension");
            }
        }
        if node.kind() == "function_declaration" && swift_callable_is_member(node) {
            return Some("method");
        }
        shared::shared_declaration_kind(node.kind())
    }

    fn base_type_relation(node: Node<'_>, owner_kind: Option<&str>) -> Option<CandidateRelation> {
        let mut current = node.parent();
        for _ in 0..=4 {
            let Some(parent) = current else {
                break;
            };
            if parent.kind() == "inheritance_specifier" {
                // Swift structs and enums can conform to protocols but cannot
                // extend a superclass.  Keep class inheritance conservative;
                // the resolver reclassifies class-to-protocol conformances
                // once the target declaration kind is known.
                return Some(
                    if matches!(owner_kind, Some("struct" | "enum" | "extension")) {
                        CandidateRelation::Implements
                    } else {
                        CandidateRelation::Extends
                    },
                );
            }
            current = parent.parent();
        }
        None
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

fn swift_callable_is_member(node: Node<'_>) -> bool {
    let mut ancestor = node.parent();
    for _ in 0..32 {
        let Some(current) = ancestor else {
            return false;
        };
        if matches!(
            current.kind(),
            "class_body" | "protocol_body" | "enum_class_body" | "extension_body"
        ) {
            return true;
        }
        ancestor = current.parent();
    }
    false
}
