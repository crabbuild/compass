use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{anchor, join_route_path, literal, normalize_route_path, split_top_level, text};
use super::{
    RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
    UniversalDetectionContext,
};
use crate::{CandidateRelation, HierarchyConstraint, SemanticRole, SymbolNamespace};

const LARAVEL_ROUTE_FACADE: &str = "illuminate\\support\\facades\\route";

#[derive(Clone, Debug)]
struct LaravelCall {
    method: String,
    arguments: Vec<String>,
    start: usize,
    end: usize,
    group_body: Option<(usize, usize)>,
}

pub(super) fn detect(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    if context.evidence.pipeline.language != "php" {
        return Vec::new();
    }
    let route_calls = universal_laravel_call_sites(context);
    let mut calls = Vec::new();
    collect_laravel_calls(context.root, context.source, &route_calls, &mut calls);
    let laravel_manifest = context.project.is_some_and(|project| {
        project.has_any_dependency(super::pack::PHP_FRAMEWORKS_DESCRIPTOR.dependency_markers)
            && project.has_dependency("laravel/framework")
    });
    let drupal_manifest = context
        .project
        .is_some_and(|project| project.has_dependency("drupal/core"));
    let type_bindings = unique_type_bindings(context);
    let mut facts = if !calls.is_empty() || (laravel_manifest && is_routes_directory(context.path))
    {
        detect_laravel(context.path, context.source, &calls, &type_bindings)
    } else {
        Vec::new()
    };
    if drupal_manifest || is_drupal_hook_file(context.path) {
        facts.extend(detect_drupal_hooks(context));
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

fn detect_laravel(
    path: &Path,
    source: &[u8],
    calls: &[LaravelCall],
    type_bindings: &BTreeMap<String, String>,
) -> Vec<RawFrameworkFact> {
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
                .and_then(|value| laravel_controller(value, type_bindings))
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
            .and_then(|value| laravel_handler(value, type_bindings))
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

fn universal_laravel_call_sites(
    context: &UniversalDetectionContext<'_, '_>,
) -> BTreeSet<(u64, u64, String)> {
    let occurrences = context
        .evidence
        .occurrences
        .iter()
        .map(|occurrence| (occurrence.id.as_str(), occurrence))
        .collect::<BTreeMap<_, _>>();
    context
        .evidence
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .filter(|candidate| {
            matches!(
                candidate.constraints.hierarchy.as_ref(),
                Some(HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name,
                    ..
                }) if receiver_qualified_name.eq_ignore_ascii_case(LARAVEL_ROUTE_FACADE)
            )
        })
        .filter_map(|candidate| {
            let occurrence = occurrences.get(candidate.occurrence_id.as_deref()?)?;
            (occurrence.role == SemanticRole::Call).then(|| {
                (
                    occurrence.range.start_byte,
                    occurrence.range.end_byte,
                    candidate.target_spelling.to_ascii_lowercase(),
                )
            })
        })
        .collect()
}

fn unique_type_bindings(context: &UniversalDetectionContext<'_, '_>) -> BTreeMap<String, String> {
    let mut grouped = BTreeMap::<String, BTreeSet<String>>::new();
    for binding in &context.evidence.bindings {
        if binding.namespace != Some(SymbolNamespace::Type) {
            continue;
        }
        grouped
            .entry(binding.spelling.to_ascii_lowercase())
            .or_default()
            .insert(binding.qualified_target.to_ascii_lowercase());
    }
    grouped
        .into_iter()
        .filter_map(|(spelling, targets)| {
            if targets.len() != 1 {
                return None;
            }
            targets.into_iter().next().map(|target| (spelling, target))
        })
        .collect()
}

fn collect_laravel_calls(
    node: Node<'_>,
    source: &[u8],
    route_calls: &BTreeSet<(u64, u64, String)>,
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
        && route_calls.contains(&(
            u64::try_from(name.start_byte()).unwrap_or(u64::MAX),
            u64::try_from(name.end_byte()).unwrap_or(u64::MAX),
            node_text(name, source).to_ascii_lowercase(),
        ))
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
        collect_laravel_calls(child, source, route_calls, calls);
    }
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

fn detect_drupal_hooks(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let module = context
        .path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let body = text(context.source);
    let Ok(documented) = Regex::new(
        r"(?is)/\*\*.*?Implements\s+hook_([A-Za-z0-9_]+)\s*\(\s*\)\s*\..*?\*/\s*function\s+([A-Za-z0-9_]+)\s*\(",
    ) else {
        return Vec::new();
    };
    let declarations = context
        .evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "function")
        .collect::<Vec<_>>();
    let mut hooks = documented
        .captures_iter(body)
        .filter_map(|capture| {
            let hook = capture.get(1)?.as_str();
            let function = capture.get(2)?;
            if function.as_str() != format!("{module}_{hook}") {
                return None;
            }
            declarations
                .iter()
                .find(|declaration| {
                    declaration.name == function.as_str()
                        && declaration.range.start_byte
                            == u64::try_from(function.start()).unwrap_or(u64::MAX)
                        && declaration.range.end_byte
                            == u64::try_from(function.end()).unwrap_or(u64::MAX)
                })
                .map(|declaration| ((*declaration).clone(), hook.to_owned()))
        })
        .collect::<Vec<_>>();
    hooks.extend(declarations.iter().filter_map(|declaration| {
        declaration
            .name
            .get("hook_".len()..)
            .filter(|_| {
                declaration
                    .name
                    .get(.."hook_".len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("hook_"))
            })
            .filter(|hook| !hook.is_empty())
            .map(|hook| ((*declaration).clone(), hook.to_owned()))
    }));
    hooks.sort_by(|left, right| {
        left.0
            .range
            .start_byte
            .cmp(&right.0.range.start_byte)
            .then_with(|| left.0.id.cmp(&right.0.id))
            .then_with(|| left.1.cmp(&right.1))
    });
    let mut seen = BTreeSet::new();
    hooks
        .into_iter()
        .filter(|(declaration, hook)| seen.insert((declaration.id.clone(), hook.clone())))
        .map(|(declaration, hook)| {
            RawFrameworkFact::Route(RawRouteFact {
                framework: "drupal".to_owned(),
                operation: "HOOK".to_owned(),
                raw_path: format!("hook://hook_{hook}"),
                normalized_path: format!("/__hook/hook_{hook}"),
                declaring_scope: declaration.range.source_file.clone(),
                anchor: framework_anchor(&declaration.range),
                handler_reference: declaration.qualified_name.replace('\\', "."),
                middleware_references: Vec::new(),
                origin: RawFrameworkOrigin::Ast,
                rule: Some("drupal-hook-implementation".to_owned()),
                detail: Map::from_iter([
                    ("hook".into(), Value::String(hook)),
                    (
                        "declarationId".into(),
                        Value::String(declaration.id.clone()),
                    ),
                ]),
            })
        })
        .collect()
}

fn framework_anchor(range: &crate::EvidenceRange) -> RawFrameworkAnchor {
    RawFrameworkAnchor {
        source_file: range.source_file.clone(),
        start_byte: range.start_byte,
        end_byte: range.end_byte,
        start_line: range.start_line,
        start_column: range.start_column,
        end_line: range.end_line,
        end_column: range.end_column,
    }
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

fn laravel_handler(value: &str, type_bindings: &BTreeMap<String, String>) -> Option<String> {
    if let Some(handler) = literal(value) {
        return Some(handler.to_ascii_lowercase().replace('@', "."));
    }
    let parts = value
        .trim()
        .strip_prefix('[')?
        .strip_suffix(']')
        .map(split_top_level)?;
    let controller = parts
        .first()
        .and_then(|part| laravel_controller(part, type_bindings))?;
    let action = parts
        .get(1)
        .and_then(|part| literal(part))?
        .to_ascii_lowercase();
    Some(format!("{controller}.{action}"))
}

fn laravel_controller(value: &str, type_bindings: &BTreeMap<String, String>) -> Option<String> {
    let value = value
        .trim()
        .strip_suffix("::class")?
        .trim_start_matches('\\');
    if !is_php_qualified_name(value) {
        return None;
    }
    let first = value.split('\\').next().unwrap_or(value);
    let resolved = type_bindings.get(&first.to_ascii_lowercase()).map_or_else(
        || value.to_ascii_lowercase(),
        |target| {
            let suffix = value.strip_prefix(first).unwrap_or_default();
            format!("{target}{suffix}")
        },
    );
    Some(resolved.replace('\\', "."))
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
