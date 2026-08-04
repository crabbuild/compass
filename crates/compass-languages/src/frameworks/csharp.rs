use std::path::Path;

use regex::Regex;
use serde_json::Map;
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{join_route_path, line_anchor, normalize_route_path, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect(path: &Path, source: &[u8], _root: Node<'_>) -> Vec<RawFrameworkFact> {
    let body = text(source);
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
    let mut facts = Vec::new();
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
