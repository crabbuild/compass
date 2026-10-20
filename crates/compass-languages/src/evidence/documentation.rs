//! Language-specific AST attachment for bounded leading documentation.
use tree_sitter::Node;

use super::build::range_for_node;
use super::model::{
    DeclarationDocumentationFact, DeclarationDocumentationKind, EvidenceRange,
    MAX_DECLARATION_DOCUMENTATION_BYTES,
};

const MAX_ATTACHMENT_NODES: usize = 1024;
const MAX_ATTACHMENT_SPAN_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub(super) struct DocumentationOmission {
    pub range: EvidenceRange,
    pub message: &'static str,
}

pub(super) fn leading_documentation(
    source_file: &str,
    source: &[u8],
    declaration: Node<'_>,
    language: &str,
) -> Result<Option<DeclarationDocumentationFact>, DocumentationOmission> {
    let kind = match language {
        "rust" => DeclarationDocumentationKind::RustOuter,
        "go" => DeclarationDocumentationKind::GoLeading,
        "typescript" => DeclarationDocumentationKind::TypeScriptJsDoc,
        _ => return Ok(None),
    };
    if declaration.has_error() || declaration.is_missing() {
        return Ok(None);
    }
    let mut owner = declaration;
    if language == "go" && matches!(owner.kind(), "type_spec" | "type_alias") {
        let Some(parent) = owner
            .parent()
            .filter(|parent| parent.kind() == "type_declaration")
        else {
            return Ok(None);
        };
        let mut cursor = parent.walk();
        let count = parent
            .named_children(&mut cursor)
            .filter(|child| matches!(child.kind(), "type_spec" | "type_alias"))
            .take(2)
            .count();
        if count != 1 {
            return Ok(None);
        }
        owner = parent;
    }
    if language == "typescript"
        && let Some(parent) = owner
            .parent()
            .filter(|parent| parent.kind() == "export_statement")
        && parent
            .child_by_field_name("declaration")
            .is_some_and(|child| child.id() == owner.id())
    {
        owner = parent;
    }
    if owner.has_error() || owner.is_missing() {
        return Ok(None);
    }
    let mut comments = Vec::new();
    let mut crossed_attributes_after_comments = false;
    let mut boundary = owner;
    let mut previous = boundary.prev_named_sibling();
    let mut steps = 0;
    while let Some(node) = previous {
        steps += 1;
        if steps > MAX_ATTACHMENT_NODES
            || owner.start_byte().saturating_sub(node.start_byte()) > MAX_ATTACHMENT_SPAN_BYTES
        {
            return Err(DocumentationOmission {
                range: range_for_node(source_file, declaration),
                message: "leading documentation attachment exceeded its bounded AST walk",
            });
        }
        if node.has_error() || node.is_missing() {
            break;
        }
        let Some(gap) = source.get(node.end_byte()..boundary.start_byte()) else {
            return Err(DocumentationOmission {
                range: range_for_node(source_file, declaration),
                message: "leading documentation attachment has an invalid source interval",
            });
        };
        if !gap.iter().all(u8::is_ascii_whitespace) {
            break;
        }
        let attribute = (language == "rust" && node.kind() == "attribute_item")
            || (language == "typescript" && node.kind() == "decorator");
        if attribute {
            if !comments.is_empty() {
                crossed_attributes_after_comments = true;
            }
            boundary = node;
            previous = node.prev_named_sibling();
            continue;
        }
        if !is_documentation(node, language, source) {
            break;
        }
        if crossed_attributes_after_comments {
            return Err(DocumentationOmission {
                range: range_for_node(source_file, declaration),
                message: "multiple documentation groups separated by attributes are outside this attachment profile",
            });
        }
        if language == "go"
            && (node.end_position().row.checked_add(1) != Some(boundary.start_position().row)
                || !starts_own_line(node, source))
        {
            break;
        }
        comments.push(node);
        boundary = node;
        previous = node.prev_named_sibling();
    }
    let (Some(first), Some(last)) = (comments.last(), comments.first()) else {
        return Ok(None);
    };
    let start = first.start_byte();
    let end = last.end_byte();
    let mut range = range_for_node(source_file, *first);
    let last_range = range_for_node(source_file, *last);
    range.end_byte = last_range.end_byte;
    range.end_line = last_range.end_line;
    range.end_column = last_range.end_column;
    if end.saturating_sub(start) > MAX_DECLARATION_DOCUMENTATION_BYTES {
        return Err(DocumentationOmission {
            range,
            message: "leading documentation exceeds the 4096-byte source excerpt limit",
        });
    }
    let Some(bytes) = source.get(start..end) else {
        return Err(DocumentationOmission {
            range,
            message: "leading documentation source range is unavailable",
        });
    };
    let Ok(content) = std::str::from_utf8(bytes) else {
        return Err(DocumentationOmission {
            range,
            message: "leading documentation is not valid UTF-8 source text",
        });
    };
    Ok(Some(DeclarationDocumentationFact {
        kind,
        range,
        content: content.to_owned(),
    }))
}

fn is_documentation(node: Node<'_>, language: &str, source: &[u8]) -> bool {
    match language {
        "rust" => {
            matches!(node.kind(), "line_comment" | "block_comment")
                && node.child_by_field_name("outer").is_some()
                && node.child_by_field_name("inner").is_none()
        }
        "go" => node.kind() == "comment",
        "typescript" => {
            node.kind() == "comment"
                && source
                    .get(node.start_byte()..node.end_byte())
                    .is_some_and(|text| text.starts_with(b"/**") && text.ends_with(b"*/"))
        }
        _ => false,
    }
}

fn starts_own_line(node: Node<'_>, source: &[u8]) -> bool {
    let Some(prefix) = source.get(..node.start_byte()) else {
        return false;
    };
    let start = prefix
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position.saturating_add(1));
    prefix[start..].iter().all(u8::is_ascii_whitespace)
}
