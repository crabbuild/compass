use std::path::Path;

use regex::Regex;
use serde_json::Map;
use tree_sitter::Node;

use super::text::{join_route_path, line_anchor, literal, normalize_route_path, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect(path: &Path, source: &[u8], _root: Node<'_>) -> Vec<RawFrameworkFact> {
    let body = text(source);
    if !is_rails_routes(path, body) {
        return Vec::new();
    }
    let Ok(scope) = Regex::new(
        r#"^\s*(?:scope\s+((?:'[^']*')|(?:"[^"]*"))|namespace\s+:([A-Za-z_][A-Za-z0-9_]*))\s+do\b"#,
    ) else {
        return Vec::new();
    };

    let mut facts = Vec::new();
    let mut prefixes = Vec::<String>::new();
    let mut offset = 0_usize;
    for line in body.split_inclusive('\n') {
        if let Some(capture) = scope.captures(line) {
            let prefix = capture
                .get(1)
                .and_then(|value| literal(value.as_str()))
                .or_else(|| capture.get(2).map(|value| value.as_str().to_owned()));
            if let Some(prefix) = prefix {
                prefixes.push(prefix);
            }
            offset = offset.saturating_add(line.len());
            continue;
        }
        if line.trim() == "end" && !prefixes.is_empty() {
            prefixes.pop();
            offset = offset.saturating_add(line.len());
            continue;
        }
        let Some((operation, raw_path, handler, suffix)) = parse_route_line(line) else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let handler = rails_handler(&handler);
        let prefix = prefixes.join("/");
        let normalized_path = if prefix.is_empty() {
            normalize_route_path(&raw_path)
        } else {
            join_route_path(&prefix, &raw_path)
        };
        let operations = if operation == "match" {
            Some(rails_via(suffix))
                .filter(|methods| !methods.is_empty())
                .unwrap_or_else(|| vec!["ANY".to_owned()])
        } else {
            vec![operation.to_ascii_uppercase()]
        };
        for operation in operations {
            facts.push(RawFrameworkFact::Route(RawRouteFact {
                framework: "rails".to_owned(),
                operation,
                raw_path: raw_path.clone(),
                normalized_path: normalized_path.clone(),
                declaring_scope: path.to_string_lossy().replace('\\', "/"),
                anchor: line_anchor(path, source, offset, line),
                handler_reference: handler.clone(),
                middleware_references: Vec::new(),
                origin: RawFrameworkOrigin::Ast,
                rule: Some("rails-routes-dsl".to_owned()),
                detail: Map::new(),
            }));
        }
        offset = offset.saturating_add(line.len());
    }
    facts
}

fn parse_route_line(line: &str) -> Option<(&str, String, String, &str)> {
    let line = line.trim();
    let split = line.find(char::is_whitespace)?;
    let operation = &line[..split];
    if !matches!(
        operation,
        "get" | "post" | "put" | "patch" | "delete" | "options" | "head" | "match"
    ) {
        return None;
    }
    let (raw_path, rest) = quoted_prefix(line[split..].trim_start())?;
    let rest = rest.trim_start();
    let rest = if let Some(rest) = rest.strip_prefix(',') {
        rest.trim_start().strip_prefix("to:")?.trim_start()
    } else {
        rest.strip_prefix("=>")?.trim_start()
    };
    let (handler, suffix) = quoted_prefix(rest)?;
    Some((operation, raw_path, handler, suffix))
}

fn quoted_prefix(value: &str) -> Option<(String, &str)> {
    let quote = value.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let end = value.as_bytes()[1..]
        .iter()
        .position(|byte| *byte == quote)?
        + 1;
    Some((value[1..end].to_owned(), &value[end + 1..]))
}

fn rails_via(suffix: &str) -> Vec<String> {
    let Some((_, value)) = suffix.split_once("via:") else {
        return Vec::new();
    };
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|method| {
            method
                .trim()
                .trim_start_matches(':')
                .trim_matches(['\'', '"'])
                .to_ascii_uppercase()
        })
        .filter(|method| !method.is_empty())
        .collect()
}

fn rails_handler(value: &str) -> String {
    let Some((controller, action)) = value.split_once('#') else {
        return value.to_owned();
    };
    let mut namespaces = controller.split('/').collect::<Vec<_>>();
    let controller = namespaces.pop().unwrap_or(controller);
    namespaces
        .into_iter()
        .map(camelize)
        .chain(std::iter::once(format!(
            "{}Controller",
            camelize(controller)
        )))
        .chain(std::iter::once(action.to_owned()))
        .collect::<Vec<_>>()
        .join(".")
}

fn camelize(value: &str) -> String {
    value
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + characters.as_str()
            })
        })
        .collect()
}

fn is_rails_routes(path: &Path, source: &str) -> bool {
    source.contains(".routes.draw do")
        || path
            .components()
            .rev()
            .take(2)
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            == ["routes.rb", "config"]
}
