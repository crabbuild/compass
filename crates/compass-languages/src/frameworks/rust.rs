use std::path::Path;

use regex::Regex;
use serde_json::Map;
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{anchor, join_route_path, literal, normalize_route_path, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect_axum(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    detect_selected(path, source, root, Some("axum"))
}

pub(super) fn detect_non_axum(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    detect_selected(path, source, root, Some("non-axum"))
}

fn detect_selected(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    selected: Option<&str>,
) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let evidence = EvidenceSet::new()
        .direct_if(
            body.contains("axum::") || body.contains("use axum"),
            "axum",
            EvidenceKind::Import,
            "axum",
        )
        .direct_if(
            body.contains("actix_web"),
            "actix",
            EvidenceKind::Import,
            "actix_web",
        )
        .direct_if(
            body.contains("rocket::"),
            "rocket",
            EvidenceKind::Import,
            "rocket",
        )
        .direct_if(
            body.contains("#[rocket::"),
            "rocket",
            EvidenceKind::Macro,
            "rocket route attribute",
        );
    let axum = selected.is_none_or(|framework| framework == "axum") && evidence.activates("axum");
    let actix = selected.is_none_or(|framework| framework == "actix" || framework == "non-axum")
        && evidence.activates("actix");
    let rocket = selected.is_none_or(|framework| framework == "rocket" || framework == "non-axum")
        && evidence.activates("rocket");
    if !axum && !actix && !rocket {
        return Vec::new();
    }
    let Ok(attribute) = Regex::new(
        r#"(?s)#\[(?:(rocket|actix_web)::)?(get|post|put|patch|delete|head)\(\s*["']([^"']+)["'][^\]]*\)\]"#,
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
    collect_rust_calls(root, source, path, framework, "", &mut facts);
    let masked_body = masked_rust_source(root, source);

    let functions = function
        .captures_iter(&masked_body)
        .filter_map(|capture| {
            let whole = capture.get(0)?;
            let handler = capture.get(1)?;
            Some((whole.start(), handler.as_str().to_owned()))
        })
        .collect::<Vec<_>>();
    for capture in attribute.captures_iter(&masked_body) {
        let Some(whole) = capture.get(0) else {
            continue;
        };
        let (Some(operation), Some(raw_path)) = (capture.get(2), capture.get(3)) else {
            continue;
        };
        let Some((_, handler)) = functions.iter().find(|(start, _)| *start >= whole.end()) else {
            continue;
        };
        let framework = match capture.get(1).map(|value| value.as_str()) {
            Some("rocket") => "rocket",
            Some("actix_web") => "actix",
            _ if actix => "actix",
            _ => "rocket",
        };
        facts.push(RawFrameworkFact::Route(RawRouteFact {
            framework: framework.to_owned(),
            operation: operation.as_str().to_ascii_uppercase(),
            raw_path: raw_path.as_str().to_owned(),
            normalized_path: normalize_route_path(raw_path.as_str()),
            declaring_scope: path.to_string_lossy().into_owned(),
            anchor: anchor(path, source, whole.start(), whole.end()),
            handler_reference: handler.clone(),
            middleware_references: Vec::new(),
            origin: RawFrameworkOrigin::Ast,
            rule: Some("rust-route-attribute".to_owned()),
            detail: Map::new(),
        }));
    }
    if let Some(selected) = selected {
        facts.retain(|fact| {
            let framework = match fact {
                RawFrameworkFact::Route(route) => route.framework.as_str(),
                RawFrameworkFact::Domain(domain) => domain.framework.as_str(),
                RawFrameworkFact::Annotation(annotation) => annotation.framework.as_str(),
            };
            if selected == "non-axum" {
                framework != "axum"
            } else {
                framework == selected
            }
        });
    }
    facts
}

fn masked_rust_source(root: Node<'_>, source: &[u8]) -> String {
    let mut masked = source.to_vec();
    mask_rust_comments(root, &mut masked);
    String::from_utf8(masked).unwrap_or_default()
}

fn mask_rust_comments(node: Node<'_>, source: &mut [u8]) {
    if matches!(node.kind(), "line_comment" | "block_comment") {
        let start = node.start_byte().min(source.len());
        let end = node.end_byte().min(source.len());
        for byte in &mut source[start..end] {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        mask_rust_comments(child, source);
    }
}

fn collect_rust_calls(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    default_framework: &str,
    prefix: &str,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
    {
        let method = rust_call_method(function, source);
        let arguments = node.child_by_field_name("arguments");
        let argument_values = arguments
            .map(|arguments| named_children(arguments))
            .unwrap_or_default();
        let receiver_prefix = function
            .child_by_field_name("value")
            .map(|value| rust_chain_prefix(value, source))
            .unwrap_or_default();
        let mut child_prefix = append_prefix(prefix, &receiver_prefix);
        if matches!(method.as_deref(), Some("nest" | "scope"))
            && let Some(route_prefix) = argument_values
                .first()
                .and_then(|argument| literal(node_text(*argument, source)))
        {
            child_prefix = join_route_path(prefix, &route_prefix);
        }
        if method.as_deref() == Some("route") {
            let (raw_path, method_expression) = if argument_values
                .first()
                .and_then(|argument| literal(node_text(*argument, source)))
                .is_some()
            {
                (
                    argument_values
                        .first()
                        .and_then(|argument| literal(node_text(*argument, source))),
                    argument_values.get(1).copied(),
                )
            } else {
                (
                    function
                        .child_by_field_name("value")
                        .and_then(|value| rust_resource_path(value, source)),
                    argument_values.first().copied(),
                )
            };
            if let (Some(raw_path), Some(method_expression)) = (raw_path, method_expression) {
                let framework =
                    route_framework(method_expression, function, source, default_framework);
                for (operation, handler) in rust_method_handlers(method_expression, source) {
                    if handler.is_empty() {
                        continue;
                    }
                    let mut fact = route_fact(
                        path,
                        source,
                        framework,
                        &operation,
                        &raw_path,
                        &handler,
                        node.start_byte(),
                        node.end_byte(),
                        "rust-router-call",
                    );
                    if !child_prefix.is_empty()
                        && let RawFrameworkFact::Route(route) = &mut fact
                    {
                        route.normalized_path =
                            join_route_path(&child_prefix, &route.normalized_path);
                    }
                    facts.push(fact);
                }
            }
        }
        let mut cursor = node.walk();
        let function_id = function.id();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            let next_prefix = if function_id == child.id() {
                prefix
            } else {
                &child_prefix
            };
            collect_rust_calls(child, source, path, default_framework, next_prefix, facts);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_rust_calls(child, source, path, default_framework, prefix, facts);
    }
}

fn rust_call_method(function: Node<'_>, source: &[u8]) -> Option<String> {
    if function.kind() == "field_expression" {
        return function
            .child_by_field_name("field")
            .map(|field| node_text(field, source).to_owned());
    }
    if matches!(function.kind(), "scoped_identifier" | "identifier") {
        return Some(
            node_text(function, source)
                .rsplit("::")
                .next()
                .unwrap_or_default()
                .to_owned(),
        );
    }
    None
}

fn append_prefix(base: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return base.to_owned();
    }
    if base.is_empty() || base == suffix {
        return suffix.to_owned();
    }
    let suffix = suffix.trim_matches('/');
    if base.ends_with(&format!("/{suffix}")) {
        base.to_owned()
    } else {
        join_route_path(base, suffix)
    }
}

fn rust_chain_prefix(node: Node<'_>, source: &[u8]) -> String {
    if node.kind() != "call_expression" {
        return node
            .child_by_field_name("value")
            .map(|value| rust_chain_prefix(value, source))
            .unwrap_or_default();
    }
    let Some(function) = node.child_by_field_name("function") else {
        return String::new();
    };
    let base = function
        .child_by_field_name("value")
        .map(|value| rust_chain_prefix(value, source))
        .unwrap_or_default();
    let Some(method) = rust_call_method(function, source) else {
        return base;
    };
    if matches!(method.as_str(), "nest" | "scope")
        && let Some(prefix) = node
            .child_by_field_name("arguments")
            .and_then(|arguments| named_children(arguments).first().copied())
            .and_then(|argument| literal(node_text(argument, source)))
    {
        return append_prefix(&base, &prefix);
    }
    base
}

fn rust_resource_path(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let function = node.child_by_field_name("function")?;
    if rust_call_method(function, source).as_deref() != Some("resource") {
        return function
            .child_by_field_name("value")
            .and_then(|value| rust_resource_path(value, source));
    }
    node.child_by_field_name("arguments")
        .and_then(|arguments| named_children(arguments).first().copied())
        .and_then(|argument| literal(node_text(argument, source)))
}

fn rust_method_handlers(node: Node<'_>, source: &[u8]) -> Vec<(String, String)> {
    if node.kind() != "call_expression" {
        return Vec::new();
    }
    let Some(function) = node.child_by_field_name("function") else {
        return Vec::new();
    };
    let Some(method) = rust_call_method(function, source) else {
        return Vec::new();
    };
    let arguments = node
        .child_by_field_name("arguments")
        .map(|arguments| named_children(arguments))
        .unwrap_or_default();
    let value = function.child_by_field_name("value");
    if is_http_method(&method) {
        let mut output = value
            .map(|value| rust_method_handlers(value, source))
            .unwrap_or_default();
        let handler = arguments
            .first()
            .map(|argument| clean_rust_handler(node_text(*argument, source)))
            .unwrap_or_default();
        output.push((method.to_ascii_uppercase(), handler));
        return output;
    }
    if method == "to" {
        let handler = arguments
            .first()
            .map(|argument| clean_rust_handler(node_text(*argument, source)))
            .unwrap_or_default();
        return value
            .map(|value| {
                rust_method_handlers(value, source)
                    .into_iter()
                    .map(|(operation, _)| (operation, handler.clone()))
                    .collect()
            })
            .unwrap_or_default();
    }
    if matches!(function.kind(), "scoped_identifier" | "identifier") && is_http_method(&method) {
        let handler = arguments
            .first()
            .map(|argument| clean_rust_handler(node_text(*argument, source)))
            .unwrap_or_default();
        return vec![(method.to_ascii_uppercase(), handler)];
    }
    Vec::new()
}

fn is_http_method(method: &str) -> bool {
    matches!(
        method.to_ascii_lowercase().as_str(),
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options" | "trace" | "connect"
    )
}

fn clean_rust_handler(value: &str) -> String {
    value.trim().trim_start_matches('&').replace("::", ".")
}

fn route_framework<'framework>(
    method_expression: Node<'_>,
    function: Node<'_>,
    source: &[u8],
    default_framework: &'framework str,
) -> &'framework str {
    let method_text = node_text(method_expression, source);
    let function_text = node_text(function, source);
    if method_text.contains("web::")
        || function_text.contains("web::")
        || function_text.contains("App")
        || function_text.contains("scope")
        || function_text.contains("resource")
    {
        "actix"
    } else {
        default_framework
    }
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn node_text<'source>(node: Node<'_>, source: &'source [u8]) -> &'source str {
    std::str::from_utf8(
        &source[node.start_byte().min(source.len())..node.end_byte().min(source.len())],
    )
    .unwrap_or_default()
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
