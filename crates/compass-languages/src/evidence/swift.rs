//! Universal evidence profile for Swift.

use std::path::Path;

use tree_sitter::Node;

use super::model::{CandidateRelation, SemanticEvidenceBatch};
use super::shared::{self, LanguageProfile, State};
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

    fn declaration_name_nodes_for_node(node: Node<'_>) -> Vec<Node<'_>> {
        // Swift's grammar represents an extension target as `user_type`,
        // including qualified names such as `Foo.Bar`. The shared fallback
        // intentionally only knows the common identifier node kinds, so keep
        // this language-specific name recovery at the Swift boundary.
        if node.kind() == "class_declaration" {
            let mut cursor = node.walk();
            if let Some(user_type) = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "user_type")
            {
                return vec![user_type];
            }
        }
        shared::declaration_name(node).into_iter().collect()
    }

    fn declaration_name_is_valid(name: &str) -> bool {
        let name = name.trim();
        !name.is_empty() && name.len() <= 512 && name.split('.').all(shared::valid_name)
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

    fn should_collect_source_supplement(_source: &[u8], _declaration_count: usize) -> bool {
        true
    }

    fn collect_source_supplement<'source>(
        state: &mut State<'source, Self>,
    ) -> Result<(), EvidenceError> {
        collect_swift_source_declarations(state)
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

const MAX_SWIFT_SUPPLEMENT_TOKENS: usize = 200_000;

#[derive(Clone, Debug)]
struct SwiftSourceToken {
    text: String,
    start: usize,
    end: usize,
}

/// Recover declaration forms that the pinned tree-sitter Swift grammar can
/// omit inside conditional-compilation blocks or deeply nested local scopes.
/// This supplement is declaration-only: calls and type references still come
/// from the AST, and every recovered declaration is source-bounded and
/// deduplicated against an overlapping parser declaration.
fn collect_swift_source_declarations<'source>(
    state: &mut State<'source, Swift>,
) -> Result<(), EvidenceError> {
    let Ok(text) = std::str::from_utf8(state.source) else {
        return Ok(());
    };
    let masked = mask_swift_source(text.as_bytes());
    let tokens = swift_source_tokens(&masked);
    let brace_pairs = swift_brace_pairs(&masked);
    let brace_depths = swift_brace_depths(&masked);

    for (index, token) in tokens.iter().enumerate() {
        let Some((kind, name, name_start, name_end)) =
            swift_declaration_header(&tokens, index, state, &masked, &brace_pairs, &brace_depths)
        else {
            continue;
        };
        let start = swift_declaration_start(&masked, token.start);
        let end = swift_declaration_end(&masked, name_end, &brace_pairs);
        if end <= start {
            continue;
        }
        // Parser recovery can let a declaration's AST range run past its
        // lexical closing brace (notably across `#if` blocks).  A top-level
        // source declaration must never inherit that stale parser owner.
        let parent = (!swift_is_top_level(&brace_depths, start))
            .then(|| swift_parent_index(state, start, &masked, &brace_pairs))
            .flatten();
        if swift_existing_declaration(state, &name, kind, start, end).is_some() {
            continue;
        }
        let parent_scope = parent
            .and_then(|parent| state.declarations.get(parent))
            .map_or(state.file_scope_id.as_str(), |declaration| {
                declaration.body_scope_id.as_str()
            })
            .to_owned();
        let _ = state.add_source_declaration(
            kind,
            &name,
            start,
            end,
            name_start,
            name_end,
            parent,
            &parent_scope,
        )?;
    }
    Ok(())
}

fn swift_declaration_header(
    tokens: &[SwiftSourceToken],
    index: usize,
    state: &State<'_, Swift>,
    masked: &[u8],
    brace_pairs: &std::collections::BTreeMap<usize, usize>,
    brace_depths: &[(usize, isize)],
) -> Option<(&'static str, String, usize, usize)> {
    let token = tokens.get(index)?;
    let keyword = token.text.as_str();
    match keyword {
        "func" => {
            let name_token = tokens.get(index.saturating_add(1))?;
            let name = swift_token_name(&name_token.text)?;
            let kind = if !swift_is_top_level(brace_depths, token.start)
                && swift_parent_index(state, token.start, masked, brace_pairs)
                    .and_then(|parent| state.declarations.get(parent))
                    .is_some_and(|declaration| swift_nominal_kind(&declaration.kind))
            {
                "method"
            } else {
                "function"
            };
            Some((kind, name, name_token.start, name_token.end))
        }
        "deinit" => Some(("method", "deinit".to_owned(), token.start, token.end)),
        "let" | "var" => {
            let name_token = tokens.get(index.saturating_add(1))?;
            let name = swift_token_name(&name_token.text)?;
            let parent = (!swift_is_top_level(brace_depths, token.start))
                .then(|| swift_parent_index(state, token.start, masked, brace_pairs))
                .flatten();
            let field_scope = parent.is_none()
                || parent
                    .and_then(|parent| state.declarations.get(parent))
                    .is_some_and(|declaration| swift_nominal_kind(&declaration.kind));
            field_scope.then_some(("field", name, name_token.start, name_token.end))
        }
        "class" | "struct" | "enum" | "actor" | "protocol" => {
            let name_token = tokens.get(index.saturating_add(1))?;
            let name = swift_token_name(&name_token.text)?;
            let kind = match keyword {
                "class" | "actor" => "class",
                "struct" => "struct",
                "enum" => "enum",
                "protocol" => "protocol",
                _ => return None,
            };
            Some((kind, name, name_token.start, name_token.end))
        }
        _ => None,
    }
}

fn swift_token_name(value: &str) -> Option<String> {
    let name = value.trim_matches('`');
    shared::valid_name(name).then(|| name.to_owned())
}

fn swift_nominal_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class" | "enum" | "extension" | "interface" | "protocol" | "struct" | "trait"
    )
}

fn swift_parent_index(
    state: &State<'_, Swift>,
    start: usize,
    masked: &[u8],
    brace_pairs: &std::collections::BTreeMap<usize, usize>,
) -> Option<usize> {
    state
        .declarations
        .iter()
        .enumerate()
        .filter(|(_, declaration)| {
            declaration.start <= start
                && start < swift_body_end(masked, declaration.start, declaration.end, brace_pairs)
                && (swift_nominal_kind(&declaration.kind)
                    || matches!(
                        declaration.kind.as_str(),
                        "constructor" | "function" | "method"
                    ))
        })
        .max_by_key(|(_, declaration)| declaration.start)
        .map(|(index, _)| index)
}

fn swift_is_top_level(depths: &[(usize, isize)], start: usize) -> bool {
    let depth = depths
        .binary_search_by_key(&start, |(position, _)| *position)
        .map_or_else(|index| index.checked_sub(1), Some)
        .and_then(|index| depths.get(index).map(|(_, depth)| *depth))
        .unwrap_or(0);
    depth == 0
}

fn swift_body_end(
    source: &[u8],
    declaration_start: usize,
    declaration_end: usize,
    brace_pairs: &std::collections::BTreeMap<usize, usize>,
) -> usize {
    let mut parentheses = 0_usize;
    let mut brackets = 0_usize;
    let limit = declaration_end.min(source.len());
    for (index, &byte) in source
        .iter()
        .enumerate()
        .take(limit)
        .skip(declaration_start)
    {
        match byte {
            b'(' => parentheses = parentheses.saturating_add(1),
            b')' => parentheses = parentheses.saturating_sub(1),
            b'[' => brackets = brackets.saturating_add(1),
            b']' => brackets = brackets.saturating_sub(1),
            b'{' if parentheses == 0 && brackets == 0 => {
                return brace_pairs
                    .get(&index)
                    .copied()
                    .map_or(declaration_end, |end| end.saturating_add(1));
            }
            _ => {}
        }
    }
    declaration_end
}

fn swift_existing_declaration(
    state: &State<'_, Swift>,
    name: &str,
    kind: &str,
    start: usize,
    end: usize,
) -> Option<usize> {
    state
        .declarations
        .iter()
        .enumerate()
        .filter(|(_, declaration)| {
            declaration.name == name
                && declaration.start < end
                && start < declaration.end
                && (declaration.kind == kind
                    || (matches!(kind, "function" | "method")
                        && matches!(declaration.kind.as_str(), "function" | "method")))
        })
        .max_by_key(|(_, declaration)| declaration.start)
        .map(|(index, _)| index)
}

fn swift_declaration_start(source: &[u8], token_start: usize) -> usize {
    let line_start = source
        .get(..token_start)
        .and_then(|prefix| prefix.iter().rposition(|byte| *byte == b'\n'))
        .map_or(0, |newline| newline.saturating_add(1));
    source[line_start..token_start]
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map_or(token_start, |offset| line_start.saturating_add(offset))
}

fn swift_declaration_end(
    source: &[u8],
    name_end: usize,
    brace_pairs: &std::collections::BTreeMap<usize, usize>,
) -> usize {
    let mut index = name_end;
    let mut parentheses = 0_usize;
    let mut brackets = 0_usize;
    while index < source.len() {
        match source[index] {
            b'(' => parentheses = parentheses.saturating_add(1),
            b')' => parentheses = parentheses.saturating_sub(1),
            b'[' => brackets = brackets.saturating_add(1),
            b']' => brackets = brackets.saturating_sub(1),
            b'{' if parentheses == 0 && brackets == 0 => {
                return brace_pairs
                    .get(&index)
                    .copied()
                    .map_or(source.len(), |end| end.saturating_add(1));
            }
            b'\n' if parentheses == 0 && brackets == 0 => return index,
            b';' if parentheses == 0 && brackets == 0 => return index.saturating_add(1),
            _ => {}
        }
        index = index.saturating_add(1);
    }
    source.len()
}

fn swift_brace_pairs(source: &[u8]) -> std::collections::BTreeMap<usize, usize> {
    let mut stack = Vec::new();
    let mut pairs = std::collections::BTreeMap::new();
    for (index, byte) in source.iter().copied().enumerate() {
        match byte {
            b'{' => stack.push(index),
            b'}' => {
                if let Some(open) = stack.pop() {
                    pairs.insert(open, index);
                }
            }
            _ => {}
        }
    }
    pairs
}

fn swift_brace_depths(source: &[u8]) -> Vec<(usize, isize)> {
    let mut depth = 0_isize;
    let mut depths = Vec::new();
    for (index, byte) in source.iter().copied().enumerate() {
        match byte {
            b'{' => {
                depth = depth.saturating_add(1);
                depths.push((index, depth));
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                depths.push((index, depth));
            }
            _ => {}
        }
    }
    depths
}

fn swift_source_tokens(source: &[u8]) -> Vec<SwiftSourceToken> {
    let mut tokens = Vec::new();
    let mut index = 0_usize;
    while index < source.len() && tokens.len() < MAX_SWIFT_SUPPLEMENT_TOKENS {
        if source[index] == b'`' {
            let start = index;
            index = index.saturating_add(1);
            while index < source.len() && source[index] != b'`' {
                index = index.saturating_add(1);
            }
            if index < source.len() {
                index = index.saturating_add(1);
            }
            let text = String::from_utf8_lossy(&source[start..index]).into_owned();
            tokens.push(SwiftSourceToken {
                text,
                start,
                end: index,
            });
            continue;
        }
        if !swift_identifier_start(source[index]) {
            index = index.saturating_add(1);
            continue;
        }
        let start = index;
        index = index.saturating_add(1);
        while index < source.len() && swift_identifier_continue(source[index]) {
            index = index.saturating_add(1);
        }
        tokens.push(SwiftSourceToken {
            text: String::from_utf8_lossy(&source[start..index]).into_owned(),
            start,
            end: index,
        });
    }
    tokens
}

fn swift_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn swift_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn mask_swift_source(source: &[u8]) -> Vec<u8> {
    let mut masked = source.to_vec();
    let mut index = 0_usize;
    let mut block_comment_depth = 0_usize;
    while index < source.len() {
        if block_comment_depth > 0 {
            if source.get(index..index.saturating_add(2)) == Some(b"/*") {
                mask_swift_bytes(&mut masked, index, index.saturating_add(2));
                block_comment_depth = block_comment_depth.saturating_add(1);
                index = index.saturating_add(2);
            } else if source.get(index..index.saturating_add(2)) == Some(b"*/") {
                mask_swift_bytes(&mut masked, index, index.saturating_add(2));
                block_comment_depth = block_comment_depth.saturating_sub(1);
                index = index.saturating_add(2);
            } else {
                if source[index] != b'\n' && source[index] != b'\r' {
                    masked[index] = b' ';
                }
                index = index.saturating_add(1);
            }
            continue;
        }
        if source.get(index..index.saturating_add(2)) == Some(b"//") {
            mask_swift_bytes(&mut masked, index, index.saturating_add(2));
            index = index.saturating_add(2);
            while index < source.len() && source[index] != b'\n' {
                if source[index] != b'\r' {
                    masked[index] = b' ';
                }
                index = index.saturating_add(1);
            }
            continue;
        }
        if source.get(index..index.saturating_add(2)) == Some(b"/*") {
            mask_swift_bytes(&mut masked, index, index.saturating_add(2));
            block_comment_depth = 1;
            index = index.saturating_add(2);
            continue;
        }
        if source[index] == b'"' {
            index = mask_swift_string(source, &mut masked, index, 0);
            continue;
        }
        if source[index] == b'#' {
            let mut hashes = 0_usize;
            while source.get(index.saturating_add(hashes)) == Some(&b'#') {
                hashes = hashes.saturating_add(1);
            }
            if hashes > 0 && source.get(index.saturating_add(hashes)) == Some(&b'"') {
                index = mask_swift_string(source, &mut masked, index, hashes);
                continue;
            }
        }
        index = index.saturating_add(1);
    }
    masked
}

fn mask_swift_string(source: &[u8], masked: &mut [u8], start: usize, raw_hashes: usize) -> usize {
    let quote_start = start.saturating_add(raw_hashes);
    let triple = source.get(quote_start..quote_start.saturating_add(3)) == Some(b"\"\"\"");
    let marker_len = if triple { 3 } else { 1 };
    let mut index = quote_start.saturating_add(marker_len);
    mask_swift_bytes(masked, start, index);
    while index < source.len() {
        if source.get(index..index.saturating_add(marker_len))
            == Some(&source[quote_start..quote_start.saturating_add(marker_len)])
            && source.get(
                index.saturating_add(marker_len)..index.saturating_add(marker_len + raw_hashes),
            ) == Some(&source[start..quote_start])
        {
            let end = index.saturating_add(marker_len + raw_hashes);
            mask_swift_bytes(masked, index, end);
            return end;
        }
        if source[index] == b'\\' && raw_hashes == 0 {
            mask_swift_bytes(masked, index, index.saturating_add(2));
            index = index.saturating_add(2);
        } else {
            if source[index] != b'\n' && source[index] != b'\r' {
                masked[index] = b' ';
            }
            index = index.saturating_add(1);
        }
    }
    source.len()
}

fn mask_swift_bytes(masked: &mut [u8], start: usize, end: usize) {
    for byte in masked
        .get_mut(start..end.min(masked.len()))
        .into_iter()
        .flatten()
    {
        if *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }
}
