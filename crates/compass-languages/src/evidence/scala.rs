//! Universal evidence profile for Scala 2 and Scala 3.

use std::collections::BTreeMap;
use std::path::Path;

use tree_sitter::Node;

use super::model::SemanticEvidenceBatch;
use super::shared::{self, LanguageProfile, ParsedImport};
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

    fn declaration_name_nodes_for_node(node: Node<'_>) -> Vec<Node<'_>> {
        // Scala 3 wraps typed `val`/`var` bindings in an
        // `alternative_pattern` (for example `cache: Foo | Null`).  The
        // shared profile only checks immediate name children, so it misses
        // the binding when the pattern is nested and the source oracle sees
        // a real local field without a corresponding declaration.  Walk only
        // the pattern child—not the initializer—to recover every binding
        // while deliberately ignoring type alternatives such as `Null`.
        if matches!(
            node.kind(),
            "val_definition" | "var_definition" | "val_declaration" | "var_declaration"
        ) {
            let mut names = Vec::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if matches!(child.kind(), "modifiers" | "access_modifier" | "annotation") {
                    continue;
                }
                collect_scala_pattern_names(child, &mut names);
                if !names.is_empty() {
                    return names;
                }
            }
        }
        if node.kind().contains("function") || node.kind().contains("method") {
            let mut cursor = node.walk();
            if let Some(operator) = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "operator_identifier")
            {
                return vec![operator];
            }
        }
        shared::declaration_name(node).into_iter().collect()
    }

    fn declaration_name_is_valid(name: &str) -> bool {
        scala_reference_name(name)
    }

    fn reference_name_is_valid(name: &str) -> bool {
        scala_reference_name(name)
    }

    fn is_type_reference_node(node: Node<'_>) -> bool {
        matches!(
            node.kind(),
            "stable_type_identifier"
                | "type_identifier"
                | "simple_type"
                | "user_type"
                | "named_type"
                | "type_reference"
                | "class_type"
                | "projected_type"
        )
    }

    fn consumes_type_reference_children(node: Node<'_>) -> bool {
        matches!(node.kind(), "stable_type_identifier" | "projected_type")
    }

    fn split_type_reference(raw: &str) -> (Option<String>, String) {
        split_scala_type_reference(raw)
    }

    fn qualified_type_reference_target<'source>(
        state: &super::shared::State<'source, Self>,
        node: Node<'_>,
        raw: &str,
        qualifier: Option<&str>,
        spelling: &str,
    ) -> Option<String> {
        let qualifier = qualifier?.trim_end_matches(".type");
        let root = qualifier.split(['.', '#']).next().unwrap_or_default();
        if root.is_empty() {
            return None;
        }
        let base = state
            .source_type_name(root)
            .or_else(|| state.source_declared_type_at(root, node.start_byte()))?;
        let suffix = qualifier
            .get(root.len()..)
            .unwrap_or_default()
            .trim_start_matches(['.', '#']);
        let mut target = base;
        if !suffix.is_empty() {
            target.push('.');
            target.push_str(suffix);
        }
        target.push(if raw.contains('#') { '#' } else { '.' });
        target.push_str(spelling);
        Some(target)
    }

    fn parse_imports(statement: &str) -> Vec<ParsedImport> {
        parse_scala_import(statement)
    }

    fn should_collect_source_supplement(_source: &[u8], _declaration_count: usize) -> bool {
        true
    }

    fn prefers_constructor_declarations() -> bool {
        true
    }

    fn supports_implicit_constructors() -> bool {
        true
    }

    fn collect_source_supplement<'source>(
        state: &mut super::shared::State<'source, Self>,
    ) -> Result<(), EvidenceError> {
        collect_scala_receiver_calls(state)
    }
}

fn collect_scala_pattern_names<'tree>(node: Node<'tree>, names: &mut Vec<Node<'tree>>) {
    match node.kind() {
        "typed_pattern" => {
            let mut cursor = node.walk();
            if let Some(name) = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "identifier")
            {
                names.push(name);
            }
        }
        // In `x: T | Null`, the `Null` alternative is represented as a bare
        // identifier.  It is a type, not another binding, so recurse only
        // through nested typed/pattern nodes here.
        "alternative_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "identifier" {
                    collect_scala_pattern_names(child, names);
                }
            }
        }
        "identifier" => names.push(node),
        "type_identifier" | "operator_identifier" | "wildcard" => {}
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_scala_pattern_names(child, names);
            }
        }
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

fn parse_scala_import(statement: &str) -> Vec<ParsedImport> {
    let trimmed = statement.trim();
    let (reexport, rest) = if let Some(rest) = trimmed.strip_prefix("export") {
        (true, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix("import") {
        (false, rest.trim())
    } else {
        return Vec::new();
    };
    let rest = rest.trim_end_matches(';').trim();
    if let Some(open) = rest.find(".{")
        && let Some(close) = rest.rfind('}')
        && close > open.saturating_add(2)
    {
        let prefix = rest[..open].trim();
        if prefix.is_empty() {
            return Vec::new();
        }
        return rest[open.saturating_add(2)..close]
            .split(',')
            .filter_map(|selector| scala_selector(prefix, selector.trim(), reexport))
            .collect();
    }
    let target = rest.trim();
    if target.is_empty() {
        return Vec::new();
    }
    if let Some(prefix) = target.strip_suffix("._") {
        return vec![ParsedImport {
            target: prefix.to_owned(),
            binding_spelling: format!("{prefix}.*"),
            local_spelling: "*".to_owned(),
            qualifier: Some(prefix.to_owned()),
            alias: false,
            prefix: false,
            reexport,
        }];
    }
    let spelling = target.rsplit('.').next().unwrap_or(target).trim();
    if !shared::valid_name(spelling) {
        return Vec::new();
    }
    vec![ParsedImport {
        target: target.to_owned(),
        binding_spelling: spelling.to_owned(),
        local_spelling: spelling.to_owned(),
        qualifier: None,
        alias: false,
        prefix: false,
        reexport,
    }]
}

fn scala_selector(prefix: &str, selector: &str, reexport: bool) -> Option<ParsedImport> {
    if selector == "_" {
        return Some(ParsedImport {
            target: prefix.to_owned(),
            binding_spelling: format!("{prefix}.*"),
            local_spelling: "*".to_owned(),
            qualifier: Some(prefix.to_owned()),
            alias: false,
            prefix: false,
            reexport,
        });
    }
    let (name, alias) = selector
        .split_once("=>")
        .map_or((selector.trim(), None), |(name, alias)| {
            (name.trim(), Some(alias.trim()))
        });
    if alias == Some("_") || !shared::valid_name(name) {
        return None;
    }
    let spelling = alias.unwrap_or(name);
    if !shared::valid_name(spelling) {
        return None;
    }
    Some(ParsedImport {
        target: format!("{prefix}.{name}"),
        binding_spelling: spelling.to_owned(),
        local_spelling: spelling.to_owned(),
        qualifier: None,
        alias: alias.is_some(),
        prefix: false,
        reexport,
    })
}

fn collect_scala_receiver_calls<'source>(
    state: &mut super::shared::State<'source, Scala>,
) -> Result<(), EvidenceError> {
    let Ok(text) = std::str::from_utf8(state.source) else {
        return Ok(());
    };
    let mut bindings_by_owner = BTreeMap::<usize, BTreeMap<String, String>>::new();
    let mut line_start = 0_usize;
    for line in text.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches(['\r', '\n']);
        let trimmed = line_without_newline.trim();
        let trim_offset = line_without_newline
            .len()
            .saturating_sub(line_without_newline.trim_start().len());

        let line_owner = state.source_callable_owner_for(line_start.saturating_add(trim_offset));
        if let Some(owner) = line_owner.or_else(|| {
            scala_definition_owner_offset(trimmed).and_then(|owner_offset| {
                state.source_callable_owner_for(
                    line_start
                        .saturating_add(trim_offset)
                        .saturating_add(owner_offset),
                )
            })
        }) {
            let bindings = scala_typed_bindings(trimmed)
                .into_iter()
                .chain(scala_value_typed_bindings(trimmed))
                .filter_map(|(name, raw_type)| {
                    state
                        .source_type_name(&raw_type)
                        .map(|qualified| (name, qualified))
                })
                .collect::<BTreeMap<_, _>>();
            if !bindings.is_empty() {
                bindings_by_owner.entry(owner).or_default().extend(bindings);
            }
        }

        let mut calls = scala_calls(line_without_newline);
        calls.extend(scala_operator_calls(line_without_newline));
        for call in calls {
            let start = line_start.saturating_add(call.start);
            let end = line_start.saturating_add(call.end);
            let Some(owner) = state.source_callable_owner_for(start) else {
                continue;
            };
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
            let receiver = call
                .qualifier
                .as_deref()
                .and_then(|qualifier| {
                    (!qualifier.contains('.'))
                        .then(|| bindings_by_owner.get(&owner))
                        .flatten()
                        .and_then(|bindings| bindings.get(qualifier))
                        .cloned()
                        .or_else(|| {
                            state
                                .source_type_name(qualifier)
                                .or_else(|| state.source_type_name_or_namespace(qualifier))
                        })
                })
                .or_else(|| {
                    call.qualifier
                        .is_none()
                        .then(|| state.source_enclosing_type(start))
                        .flatten()
                });
            let Some(receiver) = receiver else {
                continue;
            };
            state.emit_source_receiver_call(
                owner,
                &receiver,
                &call.spelling,
                call.qualifier.as_deref(),
                start,
                end,
            )?;
        }
        line_start = line_start.saturating_add(line.len());
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct ScalaCall {
    qualifier: Option<String>,
    spelling: String,
    start: usize,
    end: usize,
    constructor: bool,
}

fn scala_calls(line: &str) -> Vec<ScalaCall> {
    let bytes = line.as_bytes();
    let mut calls = Vec::new();
    let mut cursor = 0_usize;
    while cursor < bytes.len() {
        if !scala_identifier_start(bytes[cursor]) {
            cursor = cursor.saturating_add(1);
            continue;
        }
        let first_start = cursor;
        cursor = cursor.saturating_add(1);
        while cursor < bytes.len() && scala_identifier_continue(bytes[cursor]) {
            cursor = cursor.saturating_add(1);
        }
        let first_end = cursor;
        let mut lookahead = skip_ascii_space(bytes, cursor);
        if bytes.get(lookahead) == Some(&b'(') {
            let spelling = &line[first_start..first_end];
            let previous = previous_scala_word(bytes, first_start);
            if !scala_call_keyword(spelling) && previous != Some("def") {
                calls.push(ScalaCall {
                    qualifier: None,
                    spelling: spelling.to_owned(),
                    start: first_start,
                    end: first_end,
                    constructor: previous == Some("new"),
                });
            }
            continue;
        }
        if bytes.get(lookahead) != Some(&b'.') {
            continue;
        }

        let qualifier_start = first_start;
        loop {
            lookahead = skip_ascii_space(bytes, lookahead.saturating_add(1));
            if !scala_identifier_start(bytes.get(lookahead).copied().unwrap_or_default()) {
                break;
            }
            let last_start = lookahead;
            lookahead = lookahead.saturating_add(1);
            while lookahead < bytes.len() && scala_identifier_continue(bytes[lookahead]) {
                lookahead = lookahead.saturating_add(1);
            }
            let last_end = lookahead;
            let after = skip_ascii_space(bytes, lookahead);
            if bytes.get(after) == Some(&b'(') {
                let spelling = &line[last_start..last_end];
                if !scala_call_keyword(spelling) {
                    calls.push(ScalaCall {
                        qualifier: Some(
                            line[qualifier_start..last_start]
                                .trim()
                                .trim_end_matches('.')
                                .to_owned(),
                        ),
                        spelling: spelling.to_owned(),
                        start: last_start,
                        end: last_end,
                        constructor: previous_scala_word(bytes, qualifier_start) == Some("new"),
                    });
                }
                cursor = last_end;
                break;
            }
            if bytes.get(after) != Some(&b'.') {
                break;
            }
            lookahead = after;
        }
    }
    calls
}

fn scala_typed_bindings(line: &str) -> Vec<(String, String)> {
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
            let (name, raw_type) = parameter.split_once(':')?;
            let name = name
                .trim()
                .rsplit(|character: char| !scala_identifier_continue(character as u8))
                .next()
                .unwrap_or_default()
                .trim();
            let raw_type = raw_type.split(['=', '{']).next().unwrap_or_default().trim();
            (scala_identifier(name) && !raw_type.is_empty())
                .then(|| (name.to_owned(), raw_type.to_owned()))
        })
        .collect()
}

fn scala_value_typed_bindings(line: &str) -> Vec<(String, String)> {
    let bytes = line.as_bytes();
    let mut bindings = Vec::new();
    for keyword in ["val", "var"] {
        let mut search = 0_usize;
        while let Some(relative) = line.get(search..).and_then(|rest| rest.find(keyword)) {
            let start = search.saturating_add(relative);
            let before_ok =
                start == 0 || !scala_identifier_continue(bytes[start.saturating_sub(1)]);
            let after = start.saturating_add(keyword.len());
            if !before_ok
                || bytes
                    .get(after)
                    .is_some_and(|byte| scala_identifier_continue(*byte))
            {
                search = after;
                continue;
            }
            let rest = line.get(after..).unwrap_or_default().trim_start();
            let name_len = rest
                .char_indices()
                .take_while(|(_, character)| character.is_ascii_alphanumeric() || *character == '_')
                .map(|(index, character)| index + character.len_utf8())
                .last()
                .unwrap_or_default();
            let name = rest.get(..name_len).unwrap_or_default();
            let Some(colon) = rest.get(name_len..).and_then(|value| value.find(':')) else {
                search = after;
                continue;
            };
            let type_start = name_len.saturating_add(colon).saturating_add(1);
            let raw_type = rest
                .get(type_start..)
                .unwrap_or_default()
                .split(['=', ';', '\n', '\r'])
                .next()
                .unwrap_or_default()
                .trim();
            if scala_identifier(name) && !raw_type.is_empty() {
                bindings.push((name.to_owned(), raw_type.to_owned()));
            }
            search = after;
        }
    }
    bindings
}

fn scala_operator_calls(line: &str) -> Vec<ScalaCall> {
    let mut calls = Vec::new();
    let mut characters = line.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        if !scala_operator_character(character) {
            continue;
        }
        let mut end = start.saturating_add(character.len_utf8());
        while let Some(&(next_start, next_character)) = characters.peek() {
            if !scala_operator_character(next_character) {
                break;
            }
            end = next_start.saturating_add(next_character.len_utf8());
            characters.next();
        }
        let spelling = line.get(start..end).unwrap_or_default();
        if spelling.is_empty()
            || matches!(
                spelling,
                "=" | "=>" | "<-" | ":=" | "<:" | ":>" | "<%" | ">:"
            )
            || spelling.contains('=')
        {
            continue;
        }
        let left = line.get(..start).unwrap_or_default().trim_end();
        if left.is_empty()
            || left.ends_with("def")
            || left.ends_with("val")
            || left.ends_with("var")
        {
            continue;
        }
        let qualifier_start = left
            .char_indices()
            .rev()
            .find(|(_, character)| {
                !(character.is_ascii_alphanumeric() || *character == '_' || *character == '.')
            })
            .map_or(0, |(index, character)| index + character.len_utf8());
        let qualifier = left.get(qualifier_start..).unwrap_or_default().trim();
        if !scala_identifier_or_path(qualifier) {
            continue;
        }
        calls.push(ScalaCall {
            qualifier: Some(qualifier.to_owned()),
            spelling: spelling.to_owned(),
            start,
            end,
            constructor: false,
        });
    }
    calls
}

fn scala_definition_owner_offset(line: &str) -> Option<usize> {
    let def_start = line
        .split_whitespace()
        .scan(0_usize, |offset, token| {
            line[*offset..].find(token).map(|relative| {
                let absolute = offset.saturating_add(relative);
                *offset = absolute.saturating_add(token.len());
                absolute
            })
        })
        .find(|start| {
            line.get(*start..)
                .is_some_and(|rest| rest.starts_with("def"))
        })?;
    let after = def_start.saturating_add(3);
    let relative = line[after..]
        .find(|character: char| character.is_ascii_alphabetic() || character == '_')?;
    Some(after.saturating_add(relative))
}

fn skip_ascii_space(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index = index.saturating_add(1);
    }
    index
}

fn previous_scala_word(bytes: &[u8], end: usize) -> Option<&str> {
    let mut cursor = end;
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
        cursor = cursor.saturating_sub(1);
    }
    let word_end = cursor;
    while cursor > 0 && scala_identifier_continue(bytes[cursor - 1]) {
        cursor = cursor.saturating_sub(1);
    }
    std::str::from_utf8(bytes.get(cursor..word_end)?).ok()
}

fn scala_call_keyword(value: &str) -> bool {
    matches!(
        value,
        "catch" | "do" | "for" | "if" | "match" | "return" | "switch" | "throw" | "try" | "while"
    )
}

fn scala_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && scala_identifier_start(bytes[0])
        && bytes[1..].iter().copied().all(scala_identifier_continue)
}

fn scala_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn scala_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn scala_operator_character(character: char) -> bool {
    matches!(
        character,
        '!' | '#'
            | '%'
            | '&'
            | '*'
            | '+'
            | '-'
            | '/'
            | ':'
            | '<'
            | '='
            | '>'
            | '?'
            | '@'
            | '^'
            | '|'
            | '~'
    ) || matches!(
        character,
        '\u{2190}'..='\u{21ff}'
            | '\u{2200}'..='\u{22ff}'
            | '\u{2300}'..='\u{23ff}'
            | '\u{2500}'..='\u{257f}'
            | '\u{25a0}'..='\u{25ff}'
            | '\u{2600}'..='\u{27ff}'
            | '\u{2900}'..='\u{29ff}'
            | '\u{2a00}'..='\u{2aff}'
            | '\u{2b00}'..='\u{2bff}'
    )
}

fn scala_reference_name(value: &str) -> bool {
    let value = value.trim();
    (!value.is_empty() && value.len() <= 512)
        && (shared::valid_name(value) || value.chars().all(scala_operator_character))
}

fn scala_identifier_or_path(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.split('.').all(scala_identifier)
}

fn split_scala_type_reference(raw: &str) -> (Option<String>, String) {
    let cleaned = raw
        .trim()
        .trim_matches(['`', '\'', '"'])
        .trim_end_matches(['?', '!']);
    let separator = cleaned
        .rfind('#')
        .map(|index| (index, 1_usize))
        .or_else(|| cleaned.rfind("::").map(|index| (index, 2_usize)))
        .or_else(|| cleaned.rfind('.').map(|index| (index, 1_usize)));
    let Some((index, width)) = separator else {
        return (None, cleaned.to_owned());
    };
    let qualifier = cleaned.get(..index).unwrap_or_default().trim();
    let spelling = cleaned
        .get(index.saturating_add(width)..)
        .unwrap_or_default()
        .trim();
    if qualifier.is_empty() || spelling.is_empty() {
        (None, cleaned.to_owned())
    } else {
        (Some(qualifier.to_owned()), spelling.to_owned())
    }
}
