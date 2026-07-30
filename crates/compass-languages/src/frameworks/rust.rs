use std::path::Path;

use regex::Regex;
use serde_json::Map;
use tree_sitter::Node;

use super::text::{line_anchor, normalize_route_path, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect(path: &Path, source: &[u8], _root: Node<'_>) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let axum = body.contains("axum::") || body.contains("use axum");
    let actix = body.contains("actix_web");
    let rocket = body.contains("rocket::") || body.contains("#[rocket::");
    if !axum && !actix && !rocket {
        return Vec::new();
    }
    let Ok(route) = Regex::new(
        r#"\.route\(\s*["']([^"']+)["']\s*,\s*(?:(?:web::)?(get|post|put|patch|delete|head)\s*\(\s*([A-Za-z_][A-Za-z0-9_:]*)\s*\)|web::(get|post|put|patch|delete|head)\s*\(\s*\)\s*\.to\(\s*([A-Za-z_][A-Za-z0-9_:]*)\s*\))"#,
    ) else {
        return Vec::new();
    };
    let Ok(attribute) = Regex::new(
        r#"^\s*#\[(?:(?:rocket|actix_web)::)?(get|post|put|patch|delete|head)\(\s*["']([^"']+)["'][^\]]*\)\]"#,
    ) else {
        return Vec::new();
    };
    let Ok(function) = Regex::new(r"\b(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)") else {
        return Vec::new();
    };

    let framework = if axum {
        "axum"
    } else if actix {
        "actix"
    } else {
        "rocket"
    };
    let mut facts = Vec::new();
    for capture in route.captures_iter(body) {
        let Some(whole) = capture.get(0) else {
            continue;
        };
        let operation = capture.get(2).or_else(|| capture.get(4));
        let handler = capture.get(3).or_else(|| capture.get(5));
        let (Some(raw_path), Some(operation), Some(handler)) = (capture.get(1), operation, handler)
        else {
            continue;
        };
        facts.push(route_fact(
            path,
            source,
            framework,
            operation.as_str(),
            raw_path.as_str(),
            handler.as_str(),
            whole.start(),
            whole.end(),
            "rust-router-call",
        ));
    }

    let mut pending = None;
    let mut offset = 0_usize;
    for line in body.split_inclusive('\n') {
        if let Some(capture) = attribute.captures(line)
            && let (Some(operation), Some(raw_path)) = (capture.get(1), capture.get(2))
        {
            pending = Some((
                operation.as_str().to_owned(),
                raw_path.as_str().to_owned(),
                offset,
                line.to_owned(),
            ));
        } else if let Some((operation, raw_path, anchor_offset, anchor_line)) = pending.take()
            && let Some(handler) = function.captures(line).and_then(|capture| capture.get(1))
        {
            facts.push(RawFrameworkFact::Route(RawRouteFact {
                framework: if actix { "actix" } else { "rocket" }.to_owned(),
                operation: operation.to_ascii_uppercase(),
                raw_path: raw_path.clone(),
                normalized_path: normalize_route_path(&raw_path),
                declaring_scope: path.to_string_lossy().into_owned(),
                anchor: line_anchor(path, source, anchor_offset, &anchor_line),
                handler_reference: handler.as_str().to_owned(),
                middleware_references: Vec::new(),
                origin: RawFrameworkOrigin::Ast,
                rule: Some("rust-route-attribute".to_owned()),
                detail: Map::new(),
            }));
        }
        offset = offset.saturating_add(line.len());
    }
    facts
}

#[allow(clippy::too_many_arguments)]
fn route_fact(
    path: &Path,
    source: &[u8],
    framework: &str,
    operation: &str,
    raw_path: &str,
    handler: &str,
    start: usize,
    end: usize,
    rule: &str,
) -> RawFrameworkFact {
    RawFrameworkFact::Route(RawRouteFact {
        framework: framework.to_owned(),
        operation: operation.to_ascii_uppercase(),
        raw_path: raw_path.to_owned(),
        normalized_path: normalize_route_path(raw_path),
        declaring_scope: path.to_string_lossy().into_owned(),
        anchor: super::text::anchor(path, source, start, end),
        handler_reference: handler.replace("::", "."),
        middleware_references: Vec::new(),
        origin: RawFrameworkOrigin::Ast,
        rule: Some(rule.to_owned()),
        detail: Map::new(),
    })
}
