use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{anchor, join_route_path, literal, normalize_route_path, split_top_level, text};
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
    let Ok(document) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(body) else {
        return Vec::new();
    };
    let serde_yaml_ng::Value::Mapping(routes) = document else {
        return Vec::new();
    };
    let mut entries = routes
        .into_iter()
        .filter_map(|(name, value)| {
            let route_name = yaml_string(&name)?;
            let serde_yaml_ng::Value::Mapping(fields) = value else {
                return None;
            };
            let raw_path = fields.get(yaml_key("path")).and_then(yaml_string)?;
            let defaults = fields
                .get(yaml_key("defaults"))
                .and_then(|value| match value {
                    serde_yaml_ng::Value::Mapping(fields) => Some(fields),
                    _ => None,
                })
                .unwrap_or(&fields);
            let (handler_key, reference) = [
                "_controller",
                "_form",
                "_entity_form",
                "_entity_view",
                "_entity_list",
            ]
            .into_iter()
            .find_map(|key| {
                defaults
                    .get(yaml_key(key))
                    .and_then(yaml_string)
                    .map(|value| (key, value))
            })?;
            let requirements = fields
                .get(yaml_key("requirements"))
                .and_then(|value| match value {
                    serde_yaml_ng::Value::Mapping(fields) => Some(fields),
                    _ => None,
                })
                .unwrap_or(&fields);
            let methods = requirements
                .get(yaml_key("_method"))
                .map(yaml_methods)
                .filter(|methods| !methods.is_empty())
                .unwrap_or_else(|| vec!["ANY".to_owned()]);
            Some((route_name, raw_path, handler_key, reference, methods))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut facts = Vec::new();
    for (route_name, raw_path, handler_key, reference, operations) in entries {
        let needle = format!("{handler_key}: {reference}");
        let start = body
            .find(&needle)
            .unwrap_or_else(|| body.find(&reference).unwrap_or(0));
        let end = start.saturating_add(needle.len().min(source.len().saturating_sub(start)));
        let anchor = anchor(path, source, start, end);
        let handler_reference = normalize_drupal_handler(&reference);
        for operation in operations {
            facts.push(RawFrameworkFact::Route(RawRouteFact {
                framework: "drupal".to_owned(),
                operation,
                raw_path: raw_path.clone(),
                normalized_path: normalize_route_path(&raw_path),
                declaring_scope: route_name.clone(),
                anchor: anchor.clone(),
                handler_reference: handler_reference.clone(),
                middleware_references: Vec::new(),
                origin: RawFrameworkOrigin::Config,
                rule: Some("drupal-routing-yaml".to_owned()),
                detail: Map::new(),
            }));
        }
    }
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
        if matches!(method, "resource" | "apiresource") {
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
            let actions = resource_actions(source, call, method == "apiresource");
            facts.extend(resource_routes(
                &resource,
                prefix,
                &controller,
                &actions,
                call_anchor,
            ));
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
    actions: &BTreeSet<String>,
    call_anchor: super::RawFrameworkAnchor,
) -> Vec<RawFrameworkFact> {
    let base = join_route_path(prefix, resource);
    let parameter = resource
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("resource")
        .to_owned();
    let parameter = singular_resource_name(&parameter);
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
    .filter(|(_, _, action)| actions.contains(*action))
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

fn singular_resource_name(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.ends_with("ies") && value.len() > 3 {
        return format!("{}y", &value[..value.len().saturating_sub(3)]);
    }
    if ["sses", "shes", "ches", "xes", "zes"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
        && value.len() > 2
    {
        return value[..value.len().saturating_sub(2)].to_owned();
    }
    if lower.ends_with('s') && !lower.ends_with("ss") && value.len() > 1 {
        return value[..value.len().saturating_sub(1)].to_owned();
    }
    value.to_owned()
}

fn resource_actions(source: &[u8], call: &LaravelCall, api_resource: bool) -> BTreeSet<String> {
    let mut actions = [
        "index", "create", "store", "show", "edit", "update", "destroy",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    if api_resource {
        actions.remove("create");
        actions.remove("edit");
    }
    let suffix = source
        .get(call.end..)
        .and_then(|value| std::str::from_utf8(value).ok())
        .unwrap_or_default();
    let Ok(modifier) = Regex::new(r"(?s)^\s*->\s*(only|except)\s*\(([^)]*)\)") else {
        return actions;
    };
    let Some(capture) = modifier.captures(suffix) else {
        return actions;
    };
    let selected = capture
        .get(2)
        .map(|value| array_literals(value.as_str()))
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();
    if capture.get(1).is_some_and(|value| value.as_str() == "only") {
        actions.retain(|action| selected.contains(action));
    } else {
        actions.retain(|action| !selected.contains(action));
    }
    actions
}

fn detect_drupal_hooks(path: &Path, source: &[u8], body: &str) -> Vec<RawFrameworkFact> {
    let module = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let Ok(documented) = Regex::new(
        r"(?is)/\*\*.*?Implements\s+hook_([A-Za-z0-9_]+)\s*\(\s*\)\s*\..*?\*/\s*function\s+([A-Za-z0-9_]+)\s*\(",
    ) else {
        return Vec::new();
    };
    let Ok(placeholder) = Regex::new(r"(?i)\bfunction\s+(hook_[A-Za-z0-9_]+)\s*\(") else {
        return Vec::new();
    };
    let mut hooks = documented
        .captures_iter(body)
        .filter_map(|capture| {
            let hook = capture.get(1)?.as_str();
            let function = capture.get(2)?;
            (function.as_str() == format!("{module}_{hook}"))
                .then(|| (function.start(), function.end(), hook.to_owned()))
        })
        .collect::<Vec<_>>();
    hooks.extend(placeholder.captures_iter(body).filter_map(|capture| {
        let function = capture.get(1)?;
        let hook = function.as_str().get("hook_".len()..)?.to_owned();
        Some((function.start(), function.end(), hook))
    }));
    let mut seen = BTreeSet::new();
    hooks.sort();
    hooks
        .into_iter()
        .filter(|(start, end, _)| seen.insert((*start, *end)))
        .filter_map(|(start, end, hook)| {
            let name = body.get(start..end)?;
            Some(RawFrameworkFact::Route(RawRouteFact {
                framework: "drupal".to_owned(),
                operation: "HOOK".to_owned(),
                raw_path: format!("hook://hook_{hook}"),
                normalized_path: format!("/__hook/hook_{hook}"),
                declaring_scope: path.to_string_lossy().replace('\\', "/"),
                anchor: anchor(path, source, start, end),
                handler_reference: name.to_owned(),
                middleware_references: Vec::new(),
                origin: RawFrameworkOrigin::Ast,
                rule: Some("drupal-hook-implementation".to_owned()),
                detail: Map::from_iter([("hook".into(), Value::String(hook))]),
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

fn drupal_methods(value: &str) -> Vec<String> {
    let mut methods = value
        .split(['|', ','])
        .map(str::trim)
        .filter(|method| !method.is_empty())
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();
    methods.sort();
    methods.dedup();
    methods
}

fn yaml_key(value: &str) -> serde_yaml_ng::Value {
    serde_yaml_ng::Value::String(value.to_owned())
}

fn yaml_string(value: &serde_yaml_ng::Value) -> Option<String> {
    match value {
        serde_yaml_ng::Value::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn yaml_methods(value: &serde_yaml_ng::Value) -> Vec<String> {
    match value {
        serde_yaml_ng::Value::String(value) => drupal_methods(value),
        serde_yaml_ng::Value::Sequence(values) => values
            .iter()
            .filter_map(yaml_string)
            .flat_map(|value| drupal_methods(&value))
            .collect(),
        _ => Vec::new(),
    }
}
