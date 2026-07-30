use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{
    anchor, join_route_path, line_anchor, literal, normalize_route_path, split_top_level, text,
};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

const LARAVEL_ROUTE_FACADE: &str = "Illuminate\\Support\\Facades\\Route";

#[derive(Clone, Debug)]
struct LaravelCall {
    method: String,
    arguments: Vec<String>,
    start: usize,
    end: usize,
    group_body: Option<(usize, usize)>,
}

pub(super) fn detect(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let imports = php_imports(root, source);
    let mut calls = Vec::new();
    collect_laravel_calls(root, source, &imports, &mut calls);
    let evidence = EvidenceSet::new()
        .direct_if(
            !calls.is_empty(),
            "laravel",
            EvidenceKind::Receiver,
            LARAVEL_ROUTE_FACADE,
        )
        .supporting_if(
            is_routes_directory(path),
            "laravel",
            EvidenceKind::Convention,
            "routes/",
        )
        .direct_if(
            is_drupal_hook_file(path),
            "drupal",
            EvidenceKind::ConfigurationContract,
            "Drupal hook extension",
        );
    let mut facts = if evidence.activates("laravel") {
        detect_laravel(path, source, &calls)
    } else {
        Vec::new()
    };
    if evidence.activates("drupal") {
        facts.extend(detect_drupal_hooks(path, source, body));
    }
    facts
}

pub(super) fn detect_drupal_routing(path: &Path, source: &[u8]) -> Vec<RawFrameworkFact> {
    let direct_contract = path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".routing.yml") || name.ends_with(".routing.yaml"));
    let evidence = EvidenceSet::new().direct_if(
        direct_contract,
        "drupal",
        EvidenceKind::ConfigurationContract,
        "Drupal routing YAML",
    );
    if !evidence.activates("drupal") {
        return Vec::new();
    }
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

fn detect_laravel(path: &Path, source: &[u8], calls: &[LaravelCall]) -> Vec<RawFrameworkFact> {
    let prefixes = laravel_prefixes(calls);
    let mut facts = Vec::new();
    for call in calls {
        let method = call.method.as_str();
        if method == "prefix" {
            continue;
        }
        let prefix = prefixes
            .iter()
            .filter(|(_, start, end)| *start < call.start && call.start < *end)
            .max_by_key(|(_, start, _)| *start)
            .map(|(prefix, _, _)| prefix.as_str())
            .unwrap_or_default();
        let call_anchor = anchor(path, source, call.start, call.end);
        if method == "resource" {
            let Some(resource) = call.arguments.first().and_then(|value| literal(value)) else {
                continue;
            };
            let Some(controller) = call
                .arguments
                .get(1)
                .and_then(|value| laravel_controller(value))
            else {
                continue;
            };
            facts.extend(resource_routes(&resource, prefix, &controller, call_anchor));
            continue;
        }
        let (operations, path_index, handler_index) = if method == "match" {
            let Some(methods) = call.arguments.first() else {
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
        } else if is_http_method(method) {
            (vec![method.to_ascii_uppercase()], 0, 1)
        } else {
            continue;
        };
        let Some(raw_path) = call
            .arguments
            .get(path_index)
            .and_then(|value| literal(value))
        else {
            continue;
        };
        let Some(handler) = call
            .arguments
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

fn php_imports(root: Node<'_>, source: &[u8]) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    collect_php_imports(root, source, &mut imports);
    imports
}

fn collect_php_imports(node: Node<'_>, source: &[u8], imports: &mut HashMap<String, String>) {
    if node.kind() == "namespace_use_declaration" {
        parse_php_import_declaration(node_text(node, source), imports);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_php_imports(child, source, imports);
    }
}

fn parse_php_import_declaration(declaration: &str, imports: &mut HashMap<String, String>) {
    let declaration = declaration.trim();
    let Some(body) = declaration
        .get(3..)
        .filter(|_| {
            declaration
                .get(..3)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("use"))
        })
        .map(str::trim)
        .and_then(|value| value.strip_suffix(';'))
        .map(str::trim)
    else {
        return;
    };
    if starts_with_keyword(body, "function") || starts_with_keyword(body, "const") {
        return;
    }
    if let (Some(open), Some(close)) = (body.find('{'), body.rfind('}')) {
        if open >= close {
            return;
        }
        let prefix = body[..open].trim().trim_end_matches('\\');
        for entry in split_top_level(&body[open + 1..close]) {
            add_php_import(prefix, entry, imports);
        }
    } else {
        for entry in split_top_level(body) {
            add_php_import("", entry, imports);
        }
    }
}

fn add_php_import(prefix: &str, entry: &str, imports: &mut HashMap<String, String>) {
    let entry = entry.trim();
    if starts_with_keyword(entry, "function") || starts_with_keyword(entry, "const") {
        return;
    }
    let parts = entry.split_whitespace().collect::<Vec<_>>();
    let Some(imported) = parts.first().copied() else {
        return;
    };
    let alias = if parts.len() == 3 && parts[1].eq_ignore_ascii_case("as") {
        parts[2]
    } else if parts.len() == 1 {
        imported.rsplit('\\').next().unwrap_or_default()
    } else {
        return;
    };
    let target = if prefix.is_empty() {
        normalize_php_name(imported)
    } else {
        normalize_php_name(&format!("{prefix}\\{imported}"))
    };
    if is_php_qualified_name(alias) && !target.is_empty() {
        imports.insert(alias.to_owned(), target);
    }
}

fn collect_laravel_calls(
    node: Node<'_>,
    source: &[u8],
    imports: &HashMap<String, String>,
    calls: &mut Vec<LaravelCall>,
) {
    if node.kind() == "scoped_call_expression"
        && let (Some(scope), Some(name), Some(arguments)) = (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
            node.child_by_field_name("arguments"),
        )
        && matches!(scope.kind(), "name" | "qualified_name")
        && name.kind() == "name"
        && resolves_laravel_route(node_text(scope, source), imports)
    {
        let arguments_text = node_text(arguments, source).trim();
        if let Some(arguments_text) = arguments_text
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
        {
            calls.push(LaravelCall {
                method: node_text(name, source).to_ascii_lowercase(),
                arguments: split_top_level(arguments_text)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                start: node.start_byte(),
                end: node.end_byte(),
                group_body: scoped_group_body(node, source),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_laravel_calls(child, source, imports, calls);
    }
}

fn resolves_laravel_route(scope: &str, imports: &HashMap<String, String>) -> bool {
    let scope = normalize_php_name(scope);
    scope == LARAVEL_ROUTE_FACADE
        || imports
            .get(&scope)
            .is_some_and(|target| target == LARAVEL_ROUTE_FACADE)
}

fn scoped_group_body(call: Node<'_>, source: &[u8]) -> Option<(usize, usize)> {
    let parent = call.parent()?;
    if parent.kind() != "member_call_expression" {
        return None;
    }
    let object = parent.child_by_field_name("object")?;
    let name = parent.child_by_field_name("name")?;
    if object.id() != call.id() || node_text(name, source) != "group" {
        return None;
    }
    let arguments = parent.child_by_field_name("arguments")?;
    let body = find_descendant(arguments, "compound_statement")?;
    Some((body.start_byte(), body.end_byte()))
}

fn find_descendant<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .find_map(|child| find_descendant(child, kind))
}

fn starts_with_keyword(value: &str, keyword: &str) -> bool {
    value
        .get(..keyword.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(keyword))
        && value
            .as_bytes()
            .get(keyword.len())
            .is_some_and(u8::is_ascii_whitespace)
}

fn normalize_php_name(value: &str) -> String {
    value.trim().trim_start_matches('\\').to_owned()
}

fn node_text<'source>(node: Node<'_>, source: &'source [u8]) -> &'source str {
    node.utf8_text(source).unwrap_or_default()
}

fn is_php_qualified_name(value: &str) -> bool {
    !value.is_empty()
        && value.split('\\').all(|segment| {
            let mut characters = segment.chars();
            characters
                .next()
                .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
                && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
        })
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

fn laravel_prefixes(calls: &[LaravelCall]) -> Vec<(String, usize, usize)> {
    calls
        .iter()
        .filter(|call| call.method == "prefix")
        .filter_map(|call| {
            let prefix = call.arguments.first().and_then(|value| literal(value))?;
            let (start, end) = call.group_body?;
            Some((prefix, start, end))
        })
        .collect()
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
    let action = parts.get(1).and_then(|part| literal(part))?;
    Some(format!("{controller}.{action}"))
}

fn laravel_controller(value: &str) -> Option<String> {
    let value = value
        .trim()
        .strip_suffix("::class")?
        .trim_start_matches('\\');
    is_php_qualified_name(value).then(|| value.replace('\\', "."))
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

fn is_drupal_hook_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("module" | "theme" | "install" | "inc")
    )
}

fn is_routes_directory(path: &Path) -> bool {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .is_some_and(|parent| parent.eq_ignore_ascii_case("routes"))
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
