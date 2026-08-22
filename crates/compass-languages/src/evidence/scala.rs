//! Universal evidence profile for Scala 2 and Scala 3.

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
