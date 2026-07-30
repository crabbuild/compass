use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use serde_json::Map;
use tree_sitter::Node;

use super::text::{join_route_path, line_anchor, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect(path: &Path, source: &[u8], _root: Node<'_>) -> Vec<RawFrameworkFact> {
    let body = text(source);
    if !body.contains("import Vapor") {
        return Vec::new();
    }
    let Ok(group) =
        Regex::new(r#"^\s*let\s+([A-Za-z_]\w*)\s*=\s*([A-Za-z_]\w*)\.grouped\(\s*([^)]+)\)"#)
    else {
        return Vec::new();
    };
    let Ok(route) =
        Regex::new(r#"^\s*([A-Za-z_]\w*)\.(get|post|put|patch|delete|options|head)\(\s*(.+)\)"#)
    else {
        return Vec::new();
    };
    let mut prefixes = HashMap::<String, String>::new();
    let mut facts = Vec::new();
    let mut offset = 0_usize;
    for line in body.split_inclusive('\n') {
        if let Some(capture) = group.captures(line)
            && let (Some(child), Some(parent), Some(segments)) =
                (capture.get(1), capture.get(2), capture.get(3))
        {
            let parent_prefix = prefixes
                .get(parent.as_str())
                .map(String::as_str)
                .unwrap_or_default();
            if let Some(prefix) = vapor_path(segments.as_str()) {
                prefixes.insert(
                    child.as_str().to_owned(),
                    join_route_path(parent_prefix, &prefix),
                );
            }
        }
        let Some(capture) = route.captures(line) else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let (Some(receiver), Some(operation), Some(arguments)) =
            (capture.get(1), capture.get(2), capture.get(3))
        else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let (path_arguments, handler, opaque_handler) =
            if let Some((path_arguments, handler)) = arguments.as_str().rsplit_once("use:") {
                (
                    path_arguments,
                    handler.trim().trim_end_matches(',').trim(),
                    false,
                )
            } else if line.contains(") {") {
                (arguments.as_str(), "", true)
            } else {
                offset = offset.saturating_add(line.len());
                continue;
            };
        let Some(raw_path) = vapor_path(path_arguments.trim().trim_end_matches(',')) else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        if !opaque_handler && (handler.is_empty() || handler.starts_with('{')) {
            offset = offset.saturating_add(line.len());
            continue;
        }
        let prefix = prefixes
            .get(receiver.as_str())
            .map(String::as_str)
            .unwrap_or_default();
        let anchor = line_anchor(path, source, offset, line);
        facts.push(RawFrameworkFact::Route(RawRouteFact {
            framework: "vapor".to_owned(),
            operation: operation.as_str().to_ascii_uppercase(),
            raw_path: raw_path.clone(),
            normalized_path: join_route_path(prefix, &raw_path),
            declaring_scope: receiver.as_str().to_owned(),
            handler_reference: if opaque_handler {
                format!("opaque_closure_at_line_{}", anchor.start_line)
            } else {
                handler.to_owned()
            },
            anchor,
            middleware_references: Vec::new(),
            origin: RawFrameworkOrigin::Ast,
            rule: Some("vapor-route-call".to_owned()),
            detail: if opaque_handler {
                Map::from_iter([("opaque_handler".into(), true.into())])
            } else {
                Map::new()
            },
        }));
        offset = offset.saturating_add(line.len());
    }
    facts
}

fn vapor_path(arguments: &str) -> Option<String> {
    let segments = arguments
        .split(',')
        .map(str::trim)
        .map(|segment| {
            if let Some(literal) = super::text::literal(segment) {
                Some(literal)
            } else {
                segment
                    .strip_prefix('.')
                    .filter(|segment| {
                        segment
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric() || character == '_')
                    })
                    .map(|segment| format!(":{segment}"))
            }
        })
        .collect::<Option<Vec<_>>>()?;
    (!segments.is_empty()).then(|| format!("/{}", segments.join("/")))
}
