use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::text::{
    anchor, calls, join_route_path, line_anchor, literal, matching_delimiter, normalize_route_path,
    split_top_level, text,
};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect(path: &Path, source: &[u8], _root: Node<'_>) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let mut facts = if is_laravel_route_file(path, body) {
        detect_laravel(path, source, body)
    } else {
        Vec::new()
    };
    if is_drupal_hook_file(path) {
        facts.extend(detect_drupal_hooks(path, source, body));
    }
    facts
}

pub(super) fn detect_drupal_routing(path: &Path, source: &[u8]) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let mut facts = Vec::new();
    let mut current_name = None::<String>;
    let mut current_path = None::<String>;
    let mut current_method = None::<String>;
    let mut current_handler = None::<(String, usize, String)>;
    let mut offset = 0_usize;

    let flush = |facts: &mut Vec<RawFrameworkFact>,
                 name: &mut Option<String>,
                 route_path: &mut Option<String>,
                 method: &mut Option<String>,
                 handler: &mut Option<(String, usize, String)>| {
        let Some(route_name) = name.take() else {
            return;
        };
        let Some(raw_path) = route_path.take() else {
            handler.take();
            method.take();
            return;
        };
        let Some((reference, line_start, line)) = handler.take() else {
            method.take();
            return;
        };
        let operation = method
            .take()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "ANY".to_owned());
        facts.push(RawFrameworkFact::Route(RawRouteFact {
            framework: "drupal".to_owned(),
            operation: operation.to_ascii_uppercase(),
            raw_path: raw_path.clone(),
            normalized_path: normalize_route_path(&raw_path),
            declaring_scope: route_name,
            anchor: line_anchor(path, source, line_start, &line),
            handler_reference: normalize_drupal_handler(&reference),
            middleware_references: Vec::new(),
            origin: RawFrameworkOrigin::Config,
            rule: Some("drupal-routing-yaml".to_owned()),
            detail: Map::new(),
        }));
    };

    for line in body.split_inclusive('\n') {
        let trimmed = line.trim();
        let indentation = line.len().saturating_sub(line.trim_start().len());
        if indentation == 0 && trimmed.ends_with(':') && !trimmed.starts_with(['#', '-', '{']) {
            flush(
                &mut facts,
                &mut current_name,
                &mut current_path,
                &mut current_method,
                &mut current_handler,
            );
            current_name = Some(trimmed.trim_end_matches(':').to_owned());
        } else if let Some((key, value)) = trimmed.split_once(':') {
            let value = unquote_yaml(value);
            match key.trim() {
                "path" => current_path = Some(value),
                "_controller" | "_form" | "_entity_form" | "_entity_view" | "_entity_list" => {
                    current_handler = Some((value, offset, line.to_owned()));
                }
                "_method" => current_method = Some(value.replace('|', ",")),
                _ => {}
            }
        }
        offset = offset.saturating_add(line.len());
    }
    flush(
        &mut facts,
        &mut current_name,
        &mut current_path,
        &mut current_method,
        &mut current_handler,
    );
    facts
}

fn detect_laravel(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    let Ok(route_call) = Regex::new(r"Route::([A-Za-z_][A-Za-z0-9_]*)\s*\(") else {
        return Vec::new();
    };
    let prefixes = laravel_prefixes(body);
    let mut facts = Vec::new();
    for capture in route_call.captures_iter(body) {
        let Some(method_match) = capture.get(1) else {
            continue;
        };
        let method = method_match.as_str().to_ascii_lowercase();
        if method == "prefix" {
            continue;
        }
        let Some(call_match) = capture.get(0) else {
            continue;
        };
        let open = call_match.end().saturating_sub(1);
        let Some(close) = matching_delimiter(body.as_bytes(), open, b'(', b')') else {
            continue;
        };
        let arguments = split_top_level(&body[open + 1..close]);
        let prefix = prefixes
            .iter()
            .filter(|(_, start, end)| *start < call_match.start() && call_match.start() < *end)
            .max_by_key(|(_, start, _)| *start)
            .map(|(prefix, _, _)| prefix.as_str())
            .unwrap_or_default();
        let call_anchor = anchor(path, source, call_match.start(), close + 1);
        if method == "resource" {
            let Some(resource) = arguments.first().and_then(|value| literal(value)) else {
                continue;
            };
            let Some(controller) = arguments.get(1).and_then(|value| laravel_controller(value))
            else {
                continue;
            };
            facts.extend(resource_routes(&resource, prefix, &controller, call_anchor));
            continue;
        }
        let (operations, path_index, handler_index) = if method == "match" {
            let Some(methods) = arguments.first() else {
                continue;
            };
            (
                array_literals(methods)
                    .into_iter()
                    .map(|value| value.to_ascii_uppercase())
                    .collect::<Vec<_>>(),
                1,
                2,
            )
        } else if method == "any" {
            (vec!["ANY".to_owned()], 0, 1)
        } else if is_http_method(&method) {
            (vec![method.to_ascii_uppercase()], 0, 1)
        } else {
            continue;
        };
        let Some(raw_path) = arguments.get(path_index).and_then(|value| literal(value)) else {
            continue;
        };
        let Some(handler) = arguments
            .get(handler_index)
            .and_then(|value| laravel_handler(value))
        else {
            continue;
        };
        let normalized_path = join_route_path(prefix, &raw_path);
        for operation in operations {
            facts.push(RawFrameworkFact::Route(RawRouteFact {
                framework: "laravel".to_owned(),
                operation,
                raw_path: raw_path.clone(),
                normalized_path: normalized_path.clone(),
                declaring_scope: path.to_string_lossy().replace('\\', "/"),
                anchor: call_anchor.clone(),
                handler_reference: handler.clone(),
                middleware_references: Vec::new(),
                origin: RawFrameworkOrigin::Ast,
                rule: Some("laravel-route-facade".to_owned()),
                detail: Map::new(),
            }));
        }
    }
    facts
}

fn resource_routes(
    resource: &str,
    prefix: &str,
    controller: &str,
    call_anchor: super::RawFrameworkAnchor,
) -> Vec<RawFrameworkFact> {
    let base = join_route_path(prefix, resource);
    let parameter = resource
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("resource")
        .trim_end_matches('s');
    [
        ("GET", base.clone(), "index"),
        ("GET", format!("{base}/create"), "create"),
        ("POST", base.clone(), "store"),
        ("GET", format!("{base}/{{{parameter}}}"), "show"),
        ("GET", format!("{base}/{{{parameter}}}/edit"), "edit"),
        ("PUT", format!("{base}/{{{parameter}}}"), "update"),
        ("PATCH", format!("{base}/{{{parameter}}}"), "update"),
        ("DELETE", format!("{base}/{{{parameter}}}"), "destroy"),
    ]
    .into_iter()
    .map(|(operation, route_path, action)| {
        RawFrameworkFact::Route(RawRouteFact {
            framework: "laravel".to_owned(),
            operation: operation.to_owned(),
            raw_path: resource.to_owned(),
            normalized_path: normalize_route_path(&route_path),
            declaring_scope: controller.to_owned(),
            anchor: call_anchor.clone(),
            handler_reference: format!("{controller}.{action}"),
            middleware_references: Vec::new(),
            origin: RawFrameworkOrigin::Ast,
            rule: Some("laravel-resource-expansion".to_owned()),
            detail: Map::from_iter([(
                "resourceAction".to_owned(),
                Value::String(action.to_owned()),
            )]),
        })
    })
    .collect()
}

fn detect_drupal_hooks(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    let Ok(function) = Regex::new(r"(?i)\bfunction\s+(hook_[A-Za-z0-9_]+)\s*\(") else {
        return Vec::new();
    };
    function
        .captures_iter(body)
        .filter_map(|capture| {
            let name = capture.get(1)?;
            Some(RawFrameworkFact::Route(RawRouteFact {
                framework: "drupal".to_owned(),
                operation: "HOOK".to_owned(),
                raw_path: format!("hook://{}", name.as_str()),
                normalized_path: format!("/__hook/{}", name.as_str()),
                declaring_scope: path.to_string_lossy().replace('\\', "/"),
                anchor: anchor(path, source, name.start(), name.end()),
                handler_reference: name.as_str().to_owned(),
                middleware_references: Vec::new(),
                origin: RawFrameworkOrigin::Ast,
                rule: Some("drupal-hook-implementation".to_owned()),
                detail: Map::new(),
            }))
        })
        .collect()
}

fn laravel_prefixes(source: &str) -> Vec<(String, usize, usize)> {
    let mut prefixes = Vec::new();
    for call in calls(source, "Route::prefix") {
        let Some(prefix) = split_top_level(call.arguments)
            .first()
            .and_then(|value| literal(value))
        else {
            continue;
        };
        let suffix = &source[call.end..];
        let Some(group) = suffix.find("->group") else {
            continue;
        };
        let group_start = call.end + group;
        let Some(open) = source[group_start..]
            .find('{')
            .map(|value| group_start + value)
        else {
            continue;
        };
        let Some(close) = matching_delimiter(source.as_bytes(), open, b'{', b'}') else {
            continue;
        };
        prefixes.push((prefix, open, close));
    }
    prefixes
}

fn laravel_handler(value: &str) -> Option<String> {
    if let Some(handler) = literal(value) {
        return Some(handler.replace('@', "."));
    }
    let parts = value
        .trim()
        .strip_prefix('[')?
        .strip_suffix(']')
        .map(split_top_level)?;
    let controller = parts.first().and_then(|part| laravel_controller(part))?;
    let action = parts.get(1).and_then(|part| literal(part));
    Some(action.map_or(controller.clone(), |action| {
        format!("{controller}.{action}")
    }))
}

fn laravel_controller(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('\\');
    let value = value.strip_suffix("::class").unwrap_or(value);
    (!value.is_empty()).then(|| value.replace('\\', "."))
}

fn array_literals(value: &str) -> Vec<String> {
    value
        .trim()
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .map(split_top_level)
        .unwrap_or_default()
        .into_iter()
        .filter_map(literal)
        .collect()
}

fn is_http_method(method: &str) -> bool {
    matches!(
        method,
        "get" | "post" | "put" | "patch" | "delete" | "options" | "head"
    )
}

fn is_laravel_route_file(path: &Path, source: &str) -> bool {
    source.contains("Illuminate\\Support\\Facades\\Route")
        || path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .is_some_and(|parent| parent.eq_ignore_ascii_case("routes"))
}

fn is_drupal_hook_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("module" | "theme" | "install" | "inc")
    )
}

fn normalize_drupal_handler(value: &str) -> String {
    value
        .trim()
        .trim_matches(['\'', '"'])
        .trim_start_matches('\\')
        .replace('\\', ".")
        .replace("::", ".")
}

fn unquote_yaml(value: &str) -> String {
    value.trim().trim_matches(['\'', '"']).trim().to_owned()
}
