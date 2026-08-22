//! Universal evidence profile for Groovy and Gradle scripts.

use std::path::Path;

use tree_sitter::Node;

use super::model::SemanticEvidenceBatch;
use super::shared::{self, LanguageProfile, State};
use super::validate::EvidenceError;

struct Groovy;

impl LanguageProfile for Groovy {
    const LANGUAGE: &'static str = "groovy";

    fn package_name(source: &[u8]) -> Option<String> {
        shared::package_name_from_source(source)
    }

    fn has_source_supplement(declaration_count: usize) -> bool {
        declaration_count <= 1
    }

    fn should_collect_source_supplement(source: &[u8], declaration_count: usize) -> bool {
        Self::has_source_supplement(declaration_count)
            || std::str::from_utf8(source).is_ok_and(|text| {
                text.lines()
                    .any(|line| groovy_spock_feature_declaration(line.trim()).is_some())
            })
    }

    fn declaration_name_is_valid(name: &str) -> bool {
        shared::valid_name(name)
            || (!name.is_empty()
                && name.len() <= 512
                && name.chars().all(|character| !character.is_control()))
    }

    fn collect_source_supplement<'source>(
        state: &mut State<'source, Self>,
    ) -> Result<(), EvidenceError> {
        collect_groovy_source(state)
    }
}

pub(super) fn emit_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    shared::emit_tree_evidence::<Groovy>(path, source_file, source, root)
}

/// The pinned Groovy grammar intentionally exposes each top-level form as a
/// bounded `command` node. Keep Groovy on the universal evidence route by
/// extracting declaration/call spans from that command text rather than
/// reintroducing a raw graph fallback. The scanner is line- and
/// brace-bounded, preserves exact byte ranges, and remains fail-closed for
/// ambiguous method spellings.
fn collect_groovy_source<'source>(state: &mut State<'source, Groovy>) -> Result<(), EvidenceError> {
    // A lossy conversion can expand one invalid source byte into multiple
    // replacement bytes. Do not publish scanner offsets derived from it;
    // tree-sitter evidence above remains available and this omission is
    // reported as explicit incomplete input.
    let Ok(text) = std::str::from_utf8(state.source) else {
        return Ok(());
    };
    // The pinned grammar can recover a quoted feature declaration even when
    // the specification imports its base class through a project-local alias
    // (or the fixture intentionally omits the import). Treat the quoted
    // declaration itself as the bounded syntax signal; no test relationship
    // is inferred here.
    let spock_source = text.contains("spock.lang.Specification")
        || text
            .lines()
            .any(|line| groovy_spock_feature_declaration(line.trim()).is_some());
    let mut depth = 0_i32;
    let mut classes: Vec<(usize, usize, i32)> = Vec::new();
    let mut method: Option<(usize, usize)> = None;
    let mut line_start = 0_usize;
    for line in text.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches(['\r', '\n']);
        let line_end = line_start.saturating_add(line_without_newline.len());
        let trimmed = line_without_newline.trim();
        while classes.last().is_some_and(|(_, end, _)| line_start >= *end) {
            classes.pop();
        }
        if method.is_some_and(|(_, end)| line_start >= end) {
            method = None;
        }

        if let Some((kind, name, name_offset)) = groovy_type_declaration(trimmed) {
            let parent = classes.last().map(|(index, _, _)| *index);
            let parent_scope = parent
                .and_then(|index| state.declarations.get(index))
                .map_or(state.file_scope_id.as_str(), |decl| {
                    decl.body_scope_id.as_str()
                })
                .to_owned();
            let body_end = matching_brace_end(state.source, line_start, line_end);
            let end = body_end.max(line_end);
            if let Some(index) = state.add_source_declaration(
                kind,
                &name,
                line_start,
                end,
                line_start.saturating_add(name_offset),
                line_start
                    .saturating_add(name_offset)
                    .saturating_add(name.len()),
                parent,
                &parent_scope,
            )? {
                classes.push((index, end, depth));
                state.emit_source_calls(line_start, line_end, index)?;
            }
            depth = depth.saturating_add(brace_delta(trimmed));
            line_start = line_start.saturating_add(line.len());
            continue;
        }

        let active_class = classes.last().map(|(index, _, _)| *index);
        if let Some(class_index) = active_class
            && let Some((name, name_start, name_end)) = spock_source
                .then(|| groovy_spock_feature_declaration(trimmed))
                .flatten()
        {
            let parent_scope = state
                .declarations
                .get(class_index)
                .map_or(state.file_scope_id.as_str(), |decl| {
                    decl.body_scope_id.as_str()
                })
                .to_owned();
            let body_end = matching_brace_end(state.source, line_start, line_end);
            let end = body_end.max(line_end);
            if let Some(index) = state.add_source_declaration(
                "method",
                &name,
                line_start,
                end,
                line_start.saturating_add(name_start),
                line_start.saturating_add(name_end),
                Some(class_index),
                &parent_scope,
            )? {
                method = Some((index, end));
                state.emit_source_calls(line_start, line_end, index)?;
            }
        } else if let Some(class_index) = active_class
            && let Some((name, constructor, name_offset)) = groovy_method_declaration(trimmed)
        {
            let parent_scope = state
                .declarations
                .get(class_index)
                .map_or(state.file_scope_id.as_str(), |decl| {
                    decl.body_scope_id.as_str()
                })
                .to_owned();
            let body_end = matching_brace_end(state.source, line_start, line_end);
            let end = body_end.max(line_end);
            let kind = if constructor { "constructor" } else { "method" };
            if let Some(index) = state.add_source_declaration(
                kind,
                &name,
                line_start,
                end,
                line_start.saturating_add(name_offset),
                line_start
                    .saturating_add(name_offset)
                    .saturating_add(name.len()),
                Some(class_index),
                &parent_scope,
            )? {
                method = Some((index, end));
                state.emit_source_calls(line_start, line_end, index)?;
            }
        } else if let Some((method_index, method_end)) = method
            && line_start < method_end
        {
            state.emit_source_calls(line_start, line_end, method_index)?;
        }

        depth = depth.saturating_add(brace_delta(trimmed));
        line_start = line_start.saturating_add(line.len());
    }
    Ok(())
}

fn groovy_type_declaration(line: &str) -> Option<(&'static str, String, usize)> {
    let tokens = line
        .split_whitespace()
        .map(|token| token.trim_matches(['@', '{', ';', ',']))
        .collect::<Vec<_>>();
    for (index, token) in tokens.iter().enumerate() {
        let kind = match *token {
            "class" => "class",
            "interface" => "interface",
            "trait" => "trait",
            "enum" => "enum",
            _ => continue,
        };
        let name = tokens
            .get(index.saturating_add(1))?
            .trim_matches(['{', ';']);
        if !shared::valid_name(name) {
            return None;
        }
        let offset = line.find(name)?;
        return Some((kind, name.to_owned(), offset));
    }
    None
}

fn groovy_method_declaration(line: &str) -> Option<(String, bool, usize)> {
    let open = line.find('(')?;
    let before = line.get(..open)?.trim_end();
    let name_end = before.len();
    let name_start = before
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_ascii_alphanumeric() && *character != '_')
        .map_or(0, |(index, _)| index.saturating_add(1));
    let name = before.get(name_start..name_end)?.trim();
    if !shared::valid_name(name)
        || matches!(
            name,
            "if" | "for" | "while" | "switch" | "catch" | "try" | "return" | "assert"
        )
    {
        return None;
    }
    let constructor = name.chars().next().is_some_and(char::is_uppercase);
    let has_return_shape = before[..name_start]
        .split_whitespace()
        .any(|token| token == "def" || !token.is_empty());
    (has_return_shape || constructor).then(|| (name.to_owned(), constructor, name_start))
}

fn groovy_spock_feature_declaration(line: &str) -> Option<(String, usize, usize)> {
    let rest = line.strip_prefix("def")?.trim_start();
    let quote = rest.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let mut escaped = false;
    let closing = rest
        .as_bytes()
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, byte)| {
            if escaped {
                escaped = false;
                return None;
            }
            if *byte == b'\\' {
                escaped = true;
                return None;
            }
            (*byte == quote).then_some(index)
        })?;
    if !rest
        .get(closing.saturating_add(1)..)?
        .trim_start()
        .starts_with('(')
    {
        return None;
    }
    let name = rest.get(1..closing)?.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return None;
    }
    let rest_offset = line.len().saturating_sub(rest.len());
    Some((
        name.to_owned(),
        rest_offset.saturating_add(1),
        rest_offset.saturating_add(closing),
    ))
}

fn brace_delta(line: &str) -> i32 {
    let mut delta = 0_i32;
    let mut quote = None;
    for character in line.chars() {
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '{' {
            delta = delta.saturating_add(1);
        } else if character == '}' {
            delta = delta.saturating_sub(1);
        }
    }
    delta
}

fn matching_brace_end(source: &[u8], line_start: usize, line_end: usize) -> usize {
    let Some(open) = source
        .get(line_start..line_end)
        .and_then(|line| line.iter().position(|byte| *byte == b'{'))
        .map(|offset| line_start.saturating_add(offset))
    else {
        return line_end;
    };
    let mut depth = 0_i32;
    let mut quote = None;
    for (offset, byte) in source.iter().enumerate().skip(open) {
        let character = char::from(*byte);
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
            continue;
        }
        if character == '{' {
            depth = depth.saturating_add(1);
        } else if character == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return offset.saturating_add(1);
            }
        }
    }
    source.len()
}
