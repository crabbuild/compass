use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{anchor, join_route_path, line_anchor, normalize_route_path, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    let mut masked = source.to_vec();
    mask_comments(root, &mut masked);
    let body = text(&masked);
    let evidence = EvidenceSet::new()
        .direct_if(
            body.contains("Microsoft.AspNetCore.Mvc"),
            "aspnet",
            EvidenceKind::Import,
            "Microsoft.AspNetCore.Mvc",
        )
        .supporting_if(
            body.contains("[ApiController]"),
            "aspnet",
            EvidenceKind::DecoratorOrAttribute,
            "ApiController",
        )
        .direct_if(
            body.contains("WebApplication.CreateBuilder") && body.contains(".Map"),
            "aspnet",
            EvidenceKind::Receiver,
            "ASP.NET Core WebApplication minimal route receiver",
        );
    if !evidence.activates("aspnet") {
        return Vec::new();
    }
    let Ok(route_attribute) = Regex::new(r#"\[Route\(\s*"([^"]*)"\s*\)\]"#) else {
        return Vec::new();
    };
    let Ok(http_attribute) =
        Regex::new(r#"\[Http(Get|Post|Put|Patch|Delete|Head|Options)(?:\(\s*"([^"]*)"\s*\))?\]"#)
    else {
        return Vec::new();
    };
    let Ok(class) = Regex::new(r"\bclass\s+([A-Za-z_][A-Za-z0-9_]*)") else {
        return Vec::new();
    };
    let Ok(method) = Regex::new(
        r"\b(?:public|protected|private|internal|static|virtual|override|async|\s)+[A-Za-z0-9_<>,.?\[\]\s]+\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
    ) else {
        return Vec::new();
    };
    let mut class_name = None::<String>;
    let mut class_prefix = String::new();
    let mut pending_route = None::<String>;
    let mut pending_action_route = None::<String>;
    let mut pending_http = Vec::<(String, String, usize, String)>::new();
    let mut facts = collect_minimal_api_routes(path, source, body);
    let mut offset = 0_usize;
    for line in body.split_inclusive('\n') {
        if let Some(capture) = route_attribute.captures(line)
            && let Some(value) = capture.get(1)
        {
            if class_name.is_some() {
                pending_action_route = Some(value.as_str().to_owned());
            } else {
                pending_route = Some(value.as_str().to_owned());
            }
        }
        for capture in http_attribute.captures_iter(line) {
            let Some(operation) = capture.get(1) else {
                continue;
            };
            pending_http.push((
                operation.as_str().to_ascii_uppercase(),
                capture
                    .get(2)
                    .map(|value| value.as_str().to_owned())
                    .unwrap_or_default(),
                offset,
                line.to_owned(),
            ));
        }
        if let Some(name) = class.captures(line).and_then(|capture| capture.get(1)) {
            class_name = Some(name.as_str().to_owned());
            class_prefix = pending_route
                .take()
                .unwrap_or_default()
                .replace("[controller]", name.as_str().trim_end_matches("Controller"));
            pending_http.clear();
            offset = offset.saturating_add(line.len());
            continue;
        }
        if (!pending_http.is_empty() || pending_action_route.is_some())
            && let (Some(class_name), Some(method_name)) = (
                class_name.as_deref(),
                method
                    .captures(line)
                    .and_then(|capture| capture.get(1))
                    .map(|value| value.as_str()),
            )
        {
            let action_route = pending_action_route.take();
            let pending_http = if pending_http.is_empty() {
                vec![("ANY".to_owned(), String::new(), offset, line.to_owned())]
            } else {
                std::mem::take(&mut pending_http)
            };
            for (operation, action_path, anchor_offset, anchor_line) in pending_http {
                let template = action_route.as_deref().unwrap_or(&action_path);
                let expanded_action = template
                    .replace("[controller]", class_name.trim_end_matches("Controller"))
                    .replace("[action]", method_name);
                let normalized_path = if let Some(absolute) = expanded_action.strip_prefix("~/") {
                    normalize_route_path(absolute)
                } else if expanded_action.starts_with('/') || class_prefix.is_empty() {
                    normalize_route_path(&expanded_action)
                } else {
                    join_route_path(&class_prefix, &expanded_action)
                };
                facts.push(RawFrameworkFact::Route(RawRouteFact {
                    framework: "aspnet".to_owned(),
                    operation,
                    raw_path: template.to_owned(),
                    normalized_path,
                    declaring_scope: class_name.to_owned(),
                    anchor: line_anchor(path, source, anchor_offset, &anchor_line),
                    handler_reference: format!("{class_name}.{method_name}"),
                    middleware_references: Vec::new(),
                    origin: RawFrameworkOrigin::Ast,
                    rule: Some(if action_route.is_some() {
                        "aspnet-action-route-attribute".to_owned()
                    } else {
                        "aspnet-http-attribute".to_owned()
                    }),
                    detail: Map::new(),
                }));
            }
        }
        offset = offset.saturating_add(line.len());
    }
    facts
}

fn mask_comments(node: Node<'_>, source: &mut [u8]) {
    if node.kind() == "comment" {
        for byte in source
            .get_mut(node.start_byte()..node.end_byte())
            .into_iter()
            .flatten()
            .filter(|byte| **byte != b'\n' && **byte != b'\r')
        {
            *byte = b' ';
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        mask_comments(child, source);
    }
}

fn collect_minimal_api_routes(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    if !body.contains("WebApplication.CreateBuilder") {
        return Vec::new();
    }
    let Ok(group_pattern) = Regex::new(
        r#"(?m)\b(?:var|RouteGroupBuilder)\s+([A-Za-z_]\w*)\s*=\s*([A-Za-z_]\w*)\.MapGroup\(\s*"([^"]*)"\s*\)"#,
    ) else {
        return Vec::new();
    };
    let mut prefixes = HashMap::<String, String>::new();
    for capture in group_pattern.captures_iter(body) {
        let (Some(child), Some(parent), Some(prefix)) =
            (capture.get(1), capture.get(2), capture.get(3))
        else {
            continue;
        };
        let parent_prefix = prefixes
            .get(parent.as_str())
            .map(String::as_str)
            .unwrap_or_default();
        prefixes.insert(
            child.as_str().to_owned(),
            join_route_path(parent_prefix, prefix.as_str()),
        );
    }

    let Ok(route_pattern) = Regex::new(
        r#"(?s)\b([A-Za-z_]\w*)\.Map(Get|Post|Put|Patch|Delete|Options|Head)\s*\(\s*"([^"]*)"\s*,\s*([^,;\r\n]+)"#,
    ) else {
        return Vec::new();
    };
    let Ok(reference_pattern) = Regex::new(r"^[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*$") else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    for capture in route_pattern.captures_iter(body) {
        let (Some(whole), Some(receiver), Some(operation), Some(raw_path), Some(handler)) = (
            capture.get(0),
            capture.get(1),
            capture.get(2),
            capture.get(3),
            capture.get(4),
        ) else {
            continue;
        };
        let handler = handler.as_str().trim().trim_end_matches(')').trim();
        let (handler_reference, detail) = if reference_pattern.is_match(handler) {
            (handler.to_owned(), Map::new())
        } else {
            (
                format!("opaque_minimal_handler_at_{}", whole.start()),
                Map::from_iter([("opaque_handler".into(), Value::Bool(true))]),
            )
        };
        let receiver_prefix = prefixes
            .get(receiver.as_str())
            .map(String::as_str)
            .unwrap_or_default();
        facts.push(RawFrameworkFact::Route(RawRouteFact {
            framework: "aspnet".to_owned(),
            operation: operation.as_str().to_ascii_uppercase(),
            raw_path: raw_path.as_str().to_owned(),
            normalized_path: join_route_path(receiver_prefix, raw_path.as_str()),
            declaring_scope: receiver.as_str().to_owned(),
            anchor: anchor(path, source, whole.start(), whole.end()),
            handler_reference,
            middleware_references: Vec::new(),
            origin: RawFrameworkOrigin::Ast,
            rule: None,
            detail,
        }));
    }
    facts
}
