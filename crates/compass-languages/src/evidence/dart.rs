//! Universal evidence profile for Dart.

use std::collections::BTreeMap;
use std::path::Path;

use tree_sitter::Node;

use super::build::range_for_byte_span;
use super::model::{CandidateRelation, SemanticEvidenceBatch};
use super::shared::{self, LanguageProfile, ParsedImport, State};
use super::validate::EvidenceError;

struct Dart;

impl LanguageProfile for Dart {
    const LANGUAGE: &'static str = "dart";

    fn package_name(source: &[u8]) -> Option<String> {
        dart_library_name(source).or_else(|| dart_part_of_name(source))
    }

    fn declaration_kind(kind: &str) -> Option<&'static str> {
        let lower = kind.to_ascii_lowercase();
        shared::shared_declaration_kind(kind)
            .or_else(|| (lower == "variable_declaration").then_some("field"))
    }

    fn declaration_kind_for_node(node: Node<'_>, _source: &[u8]) -> Option<&'static str> {
        if node.kind() == "method_signature" {
            return dart_signature_kind(node);
        }
        if matches!(
            node.kind(),
            "constructor_signature"
                | "factory_constructor_signature"
                | "getter_signature"
                | "setter_signature"
                | "function_signature"
        ) {
            // These signatures are children of a method_signature wrapper in
            // the Dart grammar. The wrapper owns the declaration range; keep
            // the child from publishing a duplicate symbol. Top-level
            // function_signature and declaration-wrapped constructors remain
            // first-class evidence.
            if node
                .parent()
                .is_some_and(|parent| parent.kind() == "method_signature")
            {
                return None;
            }
            return dart_signature_kind(node);
        }
        if node.kind() == "declaration"
            && node.parent().is_some_and(|parent| {
                matches!(
                    parent.kind(),
                    "class_body" | "extension_body" | "mixin_body" | "enum_body"
                )
            })
            && node
                .named_children(&mut node.walk())
                .any(|child| child.kind() == "initialized_identifier_list")
        {
            return Some("field");
        }
        Self::declaration_kind(node.kind())
    }

    fn declaration_name_nodes_for_node(node: Node<'_>) -> Vec<Node<'_>> {
        if node.kind() == "method_signature" {
            return dart_signature_name(node).into_iter().collect();
        }
        if matches!(
            node.kind(),
            "constructor_signature"
                | "factory_constructor_signature"
                | "getter_signature"
                | "setter_signature"
                | "function_signature"
        ) {
            if node
                .parent()
                .is_some_and(|parent| parent.kind() == "method_signature")
            {
                return Vec::new();
            }
            return dart_signature_name(node).into_iter().collect();
        }
        if node.kind() == "declaration" {
            let mut cursor = node.walk();
            if let Some(list) = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "initialized_identifier_list")
            {
                let mut names = Vec::new();
                let mut list_cursor = list.walk();
                for initialized in list.named_children(&mut list_cursor) {
                    let mut initialized_cursor = initialized.walk();
                    if let Some(identifier) = initialized
                        .named_children(&mut initialized_cursor)
                        .find(|child| child.kind() == "identifier")
                    {
                        names.push(identifier);
                    }
                }
                if !names.is_empty() {
                    return names;
                }
            }
        }
        shared::declaration_name(node).into_iter().collect()
    }

    fn ignores_declaration(_node: Node<'_>, name_node: Node<'_>, source: &[u8]) -> bool {
        let prefix = source.get(..name_node.start_byte()).unwrap_or_default();
        prefix
            .iter()
            .rev()
            .skip_while(|byte| byte.is_ascii_whitespace())
            .take(2)
            .eq(b">=".iter())
    }

    fn base_type_relation(node: Node<'_>, _owner_kind: Option<&str>) -> Option<CandidateRelation> {
        let mut current = node.parent();
        for _ in 0..=4 {
            let Some(parent) = current else {
                break;
            };
            match parent.kind() {
                // The Dart grammar separates `extends` and `implements`
                // clauses into these containers rather than naming the
                // relation on the leaf type node.
                "interfaces" | "mixins" => return Some(CandidateRelation::Implements),
                "superclass" => return Some(CandidateRelation::Extends),
                _ => {}
            }
            current = parent.parent();
        }
        None
    }

    fn declaration_lookup_name(name: &str) -> String {
        name.split_once('(')
            .map_or_else(|| name.to_owned(), |(base, _)| base.trim().to_owned())
    }

    fn ignores_type_reference(spelling: &str) -> bool {
        matches!(
            spelling,
            "deferred" | "export" | "hide" | "import" | "library" | "part" | "show"
        )
    }

    fn parse_imports(statement: &str) -> Vec<ParsedImport> {
        parse_dart_import(statement)
    }

    fn has_source_supplement(_declaration_count: usize) -> bool {
        true
    }

    fn prefers_owner_local_calls() -> bool {
        true
    }

    fn prefers_constructor_declarations() -> bool {
        true
    }

    fn supports_implicit_constructors() -> bool {
        true
    }

    fn collect_source_supplement<'source>(
        state: &mut State<'source, Self>,
    ) -> Result<(), EvidenceError> {
        collect_dart_parts(state)?;
        collect_dart_receiver_calls(state)
    }
}

fn dart_signature_inner(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() != "method_signature" {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|child| {
        matches!(
            child.kind(),
            "constructor_signature"
                | "factory_constructor_signature"
                | "getter_signature"
                | "setter_signature"
                | "function_signature"
        )
    })
}

fn dart_signature_kind(node: Node<'_>) -> Option<&'static str> {
    let node = dart_signature_inner(node)?;
    match node.kind() {
        "constructor_signature" | "factory_constructor_signature" => Some("constructor"),
        "getter_signature" | "setter_signature" => Some("property"),
        "function_signature" => {
            let mut current = node.parent();
            while let Some(parent) = current {
                if matches!(
                    parent.kind(),
                    "class_body" | "extension_body" | "mixin_body" | "enum_body"
                ) {
                    return Some("method");
                }
                current = parent.parent();
            }
            Some("function")
        }
        _ => None,
    }
}

fn dart_signature_name(node: Node<'_>) -> Option<Node<'_>> {
    let node = dart_signature_inner(node)?;
    let mut identifiers = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            identifiers.push(child);
        }
    }
    if matches!(
        node.kind(),
        "constructor_signature" | "factory_constructor_signature"
    ) {
        return identifiers
            .get(1)
            .copied()
            .or_else(|| identifiers.first().copied());
    }
    identifiers.first().copied()
}

pub(super) fn emit_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    shared::emit_tree_evidence::<Dart>(path, source_file, source, root)
}

fn dart_library_name(source: &[u8]) -> Option<String> {
    dart_directive_name(source, "library")
}

fn dart_part_of_name(source: &[u8]) -> Option<String> {
    dart_directive_name(source, "part of")
}

fn dart_directive_name(source: &[u8], keyword: &str) -> Option<String> {
    let text = std::str::from_utf8(source).ok()?;
    text.lines().take(128).find_map(|line| {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix(keyword)?.trim();
        let value = rest.trim_end_matches(';').trim().trim_matches(['\'', '"']);
        if value.is_empty()
            || !value
                .chars()
                .all(|character| character.is_alphanumeric() || "._/-:".contains(character))
        {
            return None;
        }
        Some(value.to_owned())
    })
}

fn collect_dart_parts<'source>(state: &mut State<'source, Dart>) -> Result<(), EvidenceError> {
    let Ok(text) = std::str::from_utf8(state.source) else {
        return Ok(());
    };
    let mut line_start = 0_usize;
    for line in text.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches(['\r', '\n']);
        let trimmed = line_without_newline.trim();
        let line_end = line_start.saturating_add(line_without_newline.len());
        let range_start =
            line_start.saturating_add(line_without_newline.len().saturating_sub(trimmed.len()));
        let range = range_for_byte_span(state.source_file, state.source, range_start, line_end);
        if trimmed.starts_with("import ") || trimmed.starts_with("export ") {
            state.emit_imports(parse_dart_import(trimmed), range.clone())?;
        }
        let target = trimmed
            .strip_prefix("part of ")
            .or_else(|| trimmed.strip_prefix("part "))
            .and_then(dart_directive_target);
        if let Some(target) = target {
            state.emit_embedding(&target, range)?;
        }
        line_start = line_start.saturating_add(line.len());
    }
    Ok(())
}

fn collect_dart_receiver_calls<'source>(
    state: &mut State<'source, Dart>,
) -> Result<(), EvidenceError> {
    let Ok(text) = std::str::from_utf8(state.source) else {
        return Ok(());
    };
    let mut bindings_by_owner = BTreeMap::<usize, BTreeMap<String, String>>::new();
    let mut lexical_state = DartLexicalState::default();
    let mut brace_depth = 0_isize;
    let mut active_owner = None::<(usize, isize)>;
    let mut pending_owner = None::<usize>;
    let mut line_start = 0_usize;
    for line in text.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches(['\r', '\n']);
        let trim_offset = line_without_newline
            .len()
            .saturating_sub(line_without_newline.trim_start().len());
        let trimmed = line_without_newline.trim();
        let line_owner = trimmed.find('(').and_then(|open| {
            state.source_callable_owner_for(
                line_start.saturating_add(trim_offset).saturating_add(open),
            )
        });

        if let Some(owner) = line_owner {
            let bindings = dart_typed_bindings(trimmed)
                .into_iter()
                .filter_map(|(name, raw_type)| {
                    state
                        .source_type_name_or_namespace(&raw_type)
                        .map(|qualified| (name, qualified))
                })
                .collect::<BTreeMap<_, _>>();
            if !bindings.is_empty() {
                bindings_by_owner.insert(owner, bindings);
            }
        }

        let scan = dart_calls(line_without_newline, &mut lexical_state);
        let header_owner = trimmed.find('(').and_then(|open| {
            state.source_callable_owner_for(
                line_start.saturating_add(trim_offset).saturating_add(open),
            )
        });
        if let Some(owner) = header_owner {
            if scan.brace_delta > 0 {
                active_owner = Some((owner, brace_depth.saturating_add(1)));
                pending_owner = None;
            } else if line_without_newline.contains(';') {
                pending_owner = None;
            } else {
                pending_owner = Some(owner);
            }
        } else if pending_owner.is_some() && scan.brace_delta > 0 {
            active_owner = pending_owner
                .take()
                .map(|owner| (owner, brace_depth.saturating_add(1)));
        }
        let inherited_owner = active_owner.map(|(owner, _)| owner);
        for call in scan.calls {
            let start = line_start.saturating_add(call.start);
            let end = line_start.saturating_add(call.end);
            let owner = state
                .source_callable_owner_for(start)
                .or(line_owner)
                .or(inherited_owner);
            if let Some(owner) = owner {
                state.record_source_call_owner(start, owner);
            }
            if call.qualifier.is_none() {
                state.emit_source_local_call(
                    owner,
                    &call.spelling,
                    None,
                    start,
                    end,
                    call.constructor,
                )?;
                continue;
            }
            let Some(owner) = owner else { continue };
            let receiver = call.qualifier.as_deref().and_then(|qualifier| {
                (!qualifier.contains('.'))
                    .then(|| bindings_by_owner.get(&owner))
                    .flatten()
                    .and_then(|bindings| bindings.get(qualifier))
                    .cloned()
                    .or_else(|| {
                        (!qualifier.contains('.'))
                            .then(|| state.source_type_name_or_namespace(qualifier))
                            .flatten()
                    })
            });
            if call.constructor {
                state.emit_source_local_call(
                    Some(owner),
                    &call.spelling,
                    call.qualifier.as_deref(),
                    start,
                    end,
                    true,
                )?;
                continue;
            }
            let Some(receiver) = receiver else { continue };
            state.emit_source_receiver_call(
                owner,
                &receiver,
                &call.spelling,
                call.qualifier.as_deref(),
                start,
                end,
            )?;
        }
        brace_depth = brace_depth.saturating_add(scan.brace_delta);
        if active_owner.is_some_and(|(_, body_depth)| brace_depth < body_depth) {
            active_owner = None;
        }
        line_start = line_start.saturating_add(line.len());
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct DartCall {
    qualifier: Option<String>,
    spelling: String,
    start: usize,
    end: usize,
    constructor: bool,
}

#[derive(Default)]
struct DartLexicalState {
    block_comment: bool,
    triple_quote: Option<u8>,
}

struct DartScan {
    calls: Vec<DartCall>,
    brace_delta: isize,
}

fn dart_typed_bindings(line: &str) -> Vec<(String, String)> {
    let Some(open) = line.find('(') else {
        return Vec::new();
    };
    let Some(close) = line[open.saturating_add(1)..].find(')') else {
        return Vec::new();
    };
    let close = open.saturating_add(1).saturating_add(close);
    line[open.saturating_add(1)..close]
        .split(',')
        .filter_map(|parameter| {
            let parameter = parameter
                .split_once('=')
                .map_or(parameter, |(value, _)| value)
                .trim();
            let tokens = parameter
                .split_whitespace()
                .filter(|token| !matches!(*token, "required" | "covariant" | "final"))
                .collect::<Vec<_>>();
            let name = tokens.last()?.trim_start_matches("this.");
            if !shared::valid_name(name) || tokens.len() < 2 {
                return None;
            }
            let raw_type = tokens[..tokens.len().saturating_sub(1)].join(" ");
            let raw_type = raw_type.trim().trim_end_matches('?');
            (!raw_type.is_empty()).then(|| (name.to_owned(), raw_type.to_owned()))
        })
        .collect()
}

fn dart_calls(line: &str, state: &mut DartLexicalState) -> DartScan {
    let bytes = line.as_bytes();
    let code = dart_code_mask(bytes, state);
    let mut calls = Vec::new();
    let mut brace_delta = 0_isize;
    for (index, byte) in bytes.iter().enumerate() {
        if !code[index] {
            continue;
        }
        match byte {
            b'{' => brace_delta = brace_delta.saturating_add(1),
            b'}' => brace_delta = brace_delta.saturating_sub(1),
            _ => {}
        }
    }
    let mut cursor = 0_usize;
    while cursor < bytes.len() {
        if !code[cursor] || !dart_identifier_start(bytes[cursor]) {
            cursor = cursor.saturating_add(1);
            continue;
        }
        let first_start = cursor;
        cursor = cursor.saturating_add(1);
        while cursor < bytes.len() && dart_identifier_continue(bytes[cursor]) {
            cursor = cursor.saturating_add(1);
        }
        let first_end = cursor;
        let mut lookahead = skip_dart_space(bytes, cursor);
        if bytes.get(lookahead) != Some(&b'.') {
            if bytes.get(lookahead) == Some(&b'(')
                && !dart_call_keyword(&line[first_start..first_end])
            {
                calls.push(DartCall {
                    qualifier: None,
                    spelling: line[first_start..first_end].to_owned(),
                    start: first_start,
                    end: first_end,
                    constructor: dart_previous_word(line, first_start) == Some("new"),
                });
            }
            cursor = first_end;
            continue;
        }

        let qualifier_start = first_start;
        loop {
            lookahead = skip_dart_space(bytes, lookahead.saturating_add(1));
            if !dart_identifier_start(bytes.get(lookahead).copied().unwrap_or_default()) {
                break;
            }
            let last_start = lookahead;
            lookahead = lookahead.saturating_add(1);
            while lookahead < bytes.len() && dart_identifier_continue(bytes[lookahead]) {
                lookahead = lookahead.saturating_add(1);
            }
            let last_end = lookahead;
            let after = skip_dart_space(bytes, lookahead);
            if bytes.get(after) == Some(&b'(') {
                let qualifier = line[qualifier_start..last_start]
                    .trim()
                    .trim_end_matches('.')
                    .to_owned();
                calls.push(DartCall {
                    constructor: dart_previous_word(line, qualifier_start) == Some("new"),
                    qualifier: Some(qualifier),
                    spelling: line[last_start..last_end].to_owned(),
                    start: last_start,
                    end: last_end,
                });
                cursor = last_end;
                break;
            }
            if bytes.get(after) != Some(&b'.') {
                cursor = last_end;
                break;
            }
            lookahead = after;
        }
    }
    DartScan { calls, brace_delta }
}

fn dart_code_mask(bytes: &[u8], state: &mut DartLexicalState) -> Vec<bool> {
    let mut code = vec![true; bytes.len()];
    let mut index = 0_usize;
    while index < bytes.len() {
        if state.block_comment {
            code[index] = false;
            if bytes.get(index..index.saturating_add(2)) == Some(b"*/") {
                code[index.saturating_add(1)] = false;
                state.block_comment = false;
                index = index.saturating_add(2);
            } else {
                index = index.saturating_add(1);
            }
            continue;
        }
        if let Some(quote) = state.triple_quote {
            code[index] = false;
            if bytes.get(index..index.saturating_add(3)) == Some(&[quote, quote, quote]) {
                code[index.saturating_add(1)] = false;
                code[index.saturating_add(2)] = false;
                state.triple_quote = None;
                index = index.saturating_add(3);
            } else {
                index = index.saturating_add(1);
            }
            continue;
        }
        if bytes.get(index..index.saturating_add(2)) == Some(b"//") {
            code[index..].fill(false);
            break;
        }
        if bytes.get(index..index.saturating_add(2)) == Some(b"/*") {
            code[index] = false;
            if let Some(next) = code.get_mut(index.saturating_add(1)) {
                *next = false;
            }
            state.block_comment = true;
            index = index.saturating_add(2);
            continue;
        }
        if matches!(bytes.get(index), Some(b'\'' | b'"')) {
            let quote = bytes[index];
            code[index] = false;
            if bytes.get(index..index.saturating_add(3)) == Some(&[quote, quote, quote]) {
                code[index.saturating_add(1)] = false;
                code[index.saturating_add(2)] = false;
                state.triple_quote = Some(quote);
                index = index.saturating_add(3);
                continue;
            }
            index = index.saturating_add(1);
            let mut escaped = false;
            while index < bytes.len() {
                code[index] = false;
                if escaped {
                    escaped = false;
                } else if bytes[index] == b'\\' {
                    escaped = true;
                } else if bytes[index] == quote {
                    index = index.saturating_add(1);
                    break;
                }
                index = index.saturating_add(1);
            }
            continue;
        }
        index = index.saturating_add(1);
    }
    code
}

fn dart_previous_word(line: &str, start: usize) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut end = start;
    while end > 0 && bytes[end.saturating_sub(1)].is_ascii_whitespace() {
        end = end.saturating_sub(1);
    }
    let mut begin = end;
    while begin > 0 && dart_identifier_continue(bytes[begin.saturating_sub(1)]) {
        begin = begin.saturating_sub(1);
    }
    (begin < end).then(|| line.get(begin..end)).flatten()
}

fn dart_call_keyword(spelling: &str) -> bool {
    matches!(
        spelling,
        "as" | "assert"
            | "await"
            | "case"
            | "catch"
            | "const"
            | "do"
            | "else"
            | "for"
            | "if"
            | "in"
            | "is"
            | "new"
            | "on"
            | "rethrow"
            | "return"
            | "show"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "try"
            | "var"
            | "while"
            | "with"
            | "yield"
    )
}

fn skip_dart_space(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index = index.saturating_add(1);
    }
    index
}

fn dart_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn dart_identifier_continue(byte: u8) -> bool {
    dart_identifier_start(byte) || byte.is_ascii_digit()
}

fn dart_directive_target(rest: &str) -> Option<String> {
    let value = rest
        .trim()
        .trim_end_matches(';')
        .trim()
        .trim_matches(['\'', '"']);
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_dart_import(statement: &str) -> Vec<ParsedImport> {
    let trimmed = statement.trim();
    let (reexport, rest) = if let Some(rest) = trimmed.strip_prefix("export") {
        (true, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix("import") {
        (false, rest.trim())
    } else {
        return Vec::new();
    };
    let Some((uri, suffix)) = dart_quoted_prefix(rest) else {
        return Vec::new();
    };
    let suffix = suffix.trim().trim_end_matches(';').trim();
    let prefix = dart_import_prefix(suffix);
    let shown = dart_import_clause(suffix, "show");
    let specs = if shown.is_empty() { Vec::new() } else { shown };
    if !specs.is_empty() {
        return specs
            .into_iter()
            .map(|name| {
                let target = format!("{uri}.{name}");
                if let Some(prefix) = prefix.as_deref() {
                    ParsedImport {
                        target,
                        binding_spelling: format!("{prefix}.{name}"),
                        local_spelling: name,
                        qualifier: Some(prefix.to_owned()),
                        alias: true,
                        prefix: false,
                        reexport,
                    }
                } else {
                    ParsedImport {
                        target,
                        binding_spelling: name.clone(),
                        local_spelling: name,
                        qualifier: None,
                        alias: false,
                        prefix: false,
                        reexport,
                    }
                }
            })
            .collect();
    }
    let binding_spelling = prefix.clone().unwrap_or_else(|| uri.clone());
    vec![ParsedImport {
        target: uri,
        binding_spelling: binding_spelling.clone(),
        local_spelling: prefix
            .as_ref()
            .map_or_else(|| binding_spelling.clone(), |_| "*".to_owned()),
        qualifier: prefix.clone(),
        alias: prefix.is_some(),
        prefix: prefix.is_some(),
        reexport,
    }]
}

fn dart_quoted_prefix(value: &str) -> Option<(String, &str)> {
    let quote = value.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let end = value
        .as_bytes()
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, byte)| (*byte == quote).then_some(index))?;
    let uri = value.get(1..end)?.to_owned();
    Some((uri, value.get(end.saturating_add(1)..)?))
}

fn dart_import_prefix(suffix: &str) -> Option<String> {
    let tokens = suffix.split_whitespace().collect::<Vec<_>>();
    let index = tokens.iter().position(|token| *token == "as")?;
    let prefix = tokens.get(index.saturating_add(1))?.trim_matches(';');
    shared::valid_name(prefix).then(|| prefix.to_owned())
}

fn dart_import_clause(suffix: &str, keyword: &str) -> Vec<String> {
    let start = suffix
        .split_whitespace()
        .position(|token| token == keyword)
        .map(|index| {
            suffix
                .split_whitespace()
                .take(index)
                .map(str::len)
                .sum::<usize>()
                .saturating_add(index)
                .saturating_add(keyword.len())
        });
    let Some(start) = start else {
        return Vec::new();
    };
    let rest = suffix.get(start..).unwrap_or_default();
    let end = ["show", "hide"]
        .iter()
        .filter_map(|other| rest.find(&format!(" {other}")))
        .min()
        .unwrap_or(rest.len());
    rest.get(..end)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| shared::valid_name(name))
        .map(str::to_owned)
        .collect()
}
