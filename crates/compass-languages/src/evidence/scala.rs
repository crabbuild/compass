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

        if (trimmed.contains("def ") || trimmed.starts_with("extension"))
            && let Some(owner_offset) = scala_definition_owner_offset(trimmed)
            && let Some(owner) = state.source_callable_owner_for(
                line_start
                    .saturating_add(trim_offset)
                    .saturating_add(owner_offset),
            )
        {
            let bindings = scala_typed_bindings(trimmed)
                .into_iter()
                .filter_map(|(name, raw_type)| {
                    state
                        .source_type_name(&raw_type)
                        .map(|qualified| (name, qualified))
                })
                .collect::<BTreeMap<_, _>>();
            if !bindings.is_empty() {
                bindings_by_owner.insert(owner, bindings);
            }
        }

        for call in scala_calls(line_without_newline) {
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
                            (!qualifier.contains('.'))
                                .then(|| state.source_type_name(qualifier))
                                .flatten()
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
