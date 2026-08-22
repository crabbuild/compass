//! Universal evidence profile for Dart.

use std::path::Path;

use tree_sitter::Node;

use super::build::range_for_byte_span;
use super::model::SemanticEvidenceBatch;
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

    fn collect_source_supplement<'source>(
        state: &mut State<'source, Self>,
    ) -> Result<(), EvidenceError> {
        collect_dart_parts(state)
    }
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
