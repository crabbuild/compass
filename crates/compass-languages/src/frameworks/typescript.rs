use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
    RawRouteStageFact, RawRouteStageRole,
};
use crate::{Extraction, RawNodeRecord, make_id};

const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "options", "head", "all",
];

#[derive(Clone, Debug, Eq, PartialEq)]
struct ImportAlias {
    local: String,
    imported: String,
    module: String,
    anchor: RawFrameworkAnchor,
}

pub(super) fn detect_express(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    if source.is_empty() {
        return Vec::new();
    }
    let body = std::str::from_utf8(source).unwrap_or_default();
    let mut imports = Vec::new();
    collect_import_aliases(path, root, source, &mut imports);
    attach_import_aliases(path, source, root, extraction, &imports);
    let imports_module = |expected: &str| {
        imports.iter().any(|alias| {
            alias.module == expected
                || (expected.ends_with('/') && alias.module.starts_with(expected))
                || (expected == "react-router" && alias.module.starts_with("react-router-"))
        })
    };
    let mut facts = Vec::new();
    let receivers = express_receivers(root, source);
    let mounts = express_mounts(root, source, &receivers);
    let evidence = EvidenceSet::new().direct_if(
        !receivers.is_empty()
            && (imports_module("express")
                || source_has_express_require(root, source)
                || body.contains("require(\"express\")")
                || body.contains("require('express')")),
        "express",
        EvidenceKind::Receiver,
        "express application/router",
    );
    if evidence.activates("express") {
        collect_express_mount_facts(root, source, path, &receivers, &imports, &mut facts);
        collect_express_routes(root, source, path, &receivers, &mounts, &mut facts);
    }
    facts
}

/// Fastify and Hono use the same JavaScript/TypeScript call shapes as
/// Express, but their receiver construction and mount conventions differ.
/// Keep those framework decisions behind one small router seam so the packs
/// can share literal parsing, import identity, middleware ordering, and path
/// normalization without making the Express adapter a catch-all detector.
pub(super) fn detect_fastify(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    detect_node_router(path, source, root, extraction, NodeRouterKind::Fastify)
}

pub(super) fn detect_hono(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    detect_node_router(path, source, root, extraction, NodeRouterKind::Hono)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NodeRouterKind {
    Fastify,
    Hono,
}

impl NodeRouterKind {
    fn framework(self) -> &'static str {
        match self {
            Self::Fastify => "fastify",
            Self::Hono => "hono",
        }
    }

    fn module(self) -> &'static str {
        match self {
            Self::Fastify => "fastify",
            Self::Hono => "hono",
        }
    }

    fn constructor_import(self, imported: &str) -> bool {
        match self {
            Self::Fastify => imported == "default" || imported == "*",
            Self::Hono => imported == "Hono" || imported == "default" || imported == "*",
        }
    }

    fn route_methods(self) -> &'static [&'static str] {
        match self {
            Self::Fastify => &[
                "get", "post", "put", "patch", "delete", "options", "head", "trace", "all",
            ],
            Self::Hono => &[
                "get", "post", "put", "patch", "delete", "options", "head", "trace", "all",
            ],
        }
    }

    fn route_method_pattern(self) -> String {
        let methods = self.route_methods().join("|");
        if matches!(self, Self::Hono) {
            format!("{methods}|on")
        } else {
            methods
        }
    }
}

fn detect_node_router(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
    kind: NodeRouterKind,
) -> Vec<RawFrameworkFact> {
    if source.is_empty() {
        return Vec::new();
    }
    let mut imports = Vec::new();
    collect_import_aliases(path, root, source, &mut imports);
    attach_import_aliases(path, source, root, extraction, &imports);
    let module = kind.module();
    let imported = imports
        .iter()
        .any(|alias| alias.module == module || alias.module.starts_with(&format!("{module}/")));
    let direct = imported || source_has_module_require(root, source, module);
    if !direct {
        return Vec::new();
    }
    let receivers = node_router_receivers(root, source, &imports, kind);
    if receivers.is_empty() {
        return Vec::new();
    }
    let mounts = node_router_mounts(root, source, &receivers, kind);
    let mut facts = Vec::new();
    collect_node_router_mount_facts(root, source, path, &receivers, &imports, kind, &mut facts);
    collect_node_router_routes(root, source, path, &receivers, &mounts, kind, &mut facts);
    facts
}

fn node_router_receivers(
    root: Node<'_>,
    source: &[u8],
    imports: &[ImportAlias],
    kind: NodeRouterKind,
) -> HashSet<String> {
    let constructors = imports
        .iter()
        .filter(|alias| {
            (alias.module == kind.module()
                || alias.module.starts_with(&format!("{}/", kind.module())))
                && kind.constructor_import(&alias.imported)
        })
        .map(|alias| alias.local.clone())
        .collect::<HashSet<_>>();
    let body = std::str::from_utf8(source).unwrap_or_default();
    let mut receivers = HashSet::new();
    collect_node_router_receivers(root, source, &constructors, kind, &mut receivers);

    // CommonJS bindings do not appear in import aliases. Keep the binding
    // evidence exact and let the normal AST receiver walk handle construction.
    if let Ok(pattern) = Regex::new(&format!(
        r#"(?m)^\s*(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*require\(\s*["']{}["']\s*\)\s*;?"#,
        regex::escape(kind.module())
    )) {
        let mut require_constructors = constructors;
        require_constructors.extend(
            pattern
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned())),
        );
        collect_node_router_receivers(root, source, &require_constructors, kind, &mut receivers);
    }
    receivers
}

fn collect_node_router_receivers(
    node: Node<'_>,
    source: &[u8],
    constructors: &HashSet<String>,
    kind: NodeRouterKind,
    receivers: &mut HashSet<String>,
) {
    if matches!(kind, NodeRouterKind::Fastify)
        && matches!(node.kind(), "required_parameter" | "optional_parameter")
        && node_text(node, source).contains("FastifyInstance")
        && let Some(name) = node
            .child_by_field_name("pattern")
            .or_else(|| node.child_by_field_name("name"))
        && is_identifier(node_text(name, source))
    {
        receivers.insert(node_text(name, source).to_owned());
    }
    if node.kind() == "variable_declarator"
        && let (Some(name), Some(value)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("value"),
        )
    {
        let variable = node_text(name, source).trim();
        let expression = node_text(value, source).trim();
        if is_identifier(variable) && node_router_constructor_call(expression, constructors, kind) {
            receivers.insert(variable.to_owned());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_node_router_receivers(child, source, constructors, kind, receivers);
    }
}

fn node_router_constructor_call(
    expression: &str,
    constructors: &HashSet<String>,
    kind: NodeRouterKind,
) -> bool {
    let expression = expression.trim();
    match kind {
        NodeRouterKind::Fastify => {
            let callee = expression
                .split_once('(')
                .map_or(expression, |(callee, _)| callee.trim());
            is_identifier(callee)
                && (constructors.contains(callee)
                    || callee.eq_ignore_ascii_case("fastify")
                    || callee.eq_ignore_ascii_case("Fastify"))
                || expression.starts_with("require(\"fastify\")")
                || expression.starts_with("require('fastify')")
        }
        NodeRouterKind::Hono => {
            let Some(constructed) = expression.strip_prefix("new ") else {
                return false;
            };
            let callee = constructed
                .split_once('(')
                .map_or(constructed, |(callee, _)| callee.trim());
            is_identifier(callee) && (constructors.contains(callee) || callee == "Hono")
                || constructed.starts_with("(require(\"hono\").Hono)")
                || constructed.starts_with("(require('hono').Hono)")
        }
    }
}

fn node_router_mounts(
    root: Node<'_>,
    source: &[u8],
    receivers: &HashSet<String>,
    kind: NodeRouterKind,
) -> HashMap<String, String> {
    let mut mounts = HashMap::new();
    collect_node_router_mounts(root, source, receivers, kind, &mut mounts);
    mounts
}

fn collect_node_router_mounts(
    node: Node<'_>,
    source: &[u8],
    receivers: &HashSet<String>,
    kind: NodeRouterKind,
    mounts: &mut HashMap<String, String>,
) {
    let is_mount = node.kind() == "call_expression"
        && node
            .child_by_field_name("function")
            .is_some_and(|function| {
                let Some((parent, method)) = node_text(function, source).trim().rsplit_once('.')
                else {
                    return false;
                };
                receivers.contains(parent)
                    && ((matches!(kind, NodeRouterKind::Hono) && method == "route")
                        || (matches!(kind, NodeRouterKind::Fastify) && method == "register"))
            });
    if is_mount {
        let function = node
            .child_by_field_name("function")
            .map(|function| node_text(function, source).trim().to_owned())
            .unwrap_or_default();
        let Some((parent, method)) = function.rsplit_once('.') else {
            return;
        };
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return;
        };
        let argument_text = split_arguments(node_text(arguments, source));
        let prefix = if method == "route" {
            argument_text
                .first()
                .and_then(|value| string_literal(value))
        } else {
            argument_text.get(1).and_then(|value| {
                object_property_text(value, "prefix").and_then(|value| string_literal(&value))
            })
        };
        let child = argument_text
            .first()
            .and_then(|value| {
                if method == "route" {
                    argument_text.get(1)
                } else {
                    Some(value)
                }
            })
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| is_identifier(value));
        if let (Some(prefix), Some(child)) = (prefix, child)
            && receivers.contains(child)
        {
            let parent_prefix = mounts.get(parent).map(String::as_str).unwrap_or_default();
            mounts.insert(child.to_owned(), join_paths(parent_prefix, &prefix));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_node_router_mounts(child, source, receivers, kind, mounts);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_node_router_mount_facts(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    receivers: &HashSet<String>,
    imports: &[ImportAlias],
    kind: NodeRouterKind,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && let Some((parent, method)) = node_text(function, source).trim().rsplit_once('.')
        && receivers.contains(parent)
        && ((matches!(kind, NodeRouterKind::Hono) && method == "route")
            || (matches!(kind, NodeRouterKind::Fastify) && method == "register"))
        && let Some(arguments) = node.child_by_field_name("arguments")
    {
        let arguments = split_arguments(node_text(arguments, source));
        let (target, prefix) = if method == "route" {
            (
                arguments.get(1).map(String::as_str),
                arguments.first().and_then(|value| string_literal(value)),
            )
        } else {
            (
                arguments.first().map(String::as_str),
                arguments.get(1).and_then(|value| {
                    object_property_text(value, "prefix").and_then(|value| string_literal(&value))
                }),
            )
        };
        if let (Some(target), Some(prefix)) = (target.map(str::trim), prefix)
            && is_identifier(target)
            && (receivers.contains(target) || imports.iter().any(|alias| alias.local == target))
        {
            facts.push(router_mount_fact(
                kind.framework(),
                path,
                node,
                parent,
                target,
                &prefix,
                imports,
            ));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_node_router_mount_facts(child, source, path, receivers, imports, kind, facts);
    }
}

fn collect_node_router_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    receivers: &HashSet<String>,
    mounts: &HashMap<String, String>,
    kind: NodeRouterKind,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
    {
        let function = node_text(function, source).trim();
        let mut receiver = None;
        let mut method = None;
        let mut chained_path = None;
        if let Some((candidate, candidate_method)) = function.rsplit_once('.')
            && receivers.contains(candidate)
            && (kind.route_methods().contains(&candidate_method)
                || (matches!(kind, NodeRouterKind::Hono) && candidate_method == "on"))
        {
            receiver = Some(candidate.to_owned());
            method = Some(candidate_method.to_owned());
        } else if matches!(kind, NodeRouterKind::Hono)
            && let Ok(pattern) = Regex::new(&format!(
                r#"^([A-Za-z_$][\w$]*)\.basePath\(\s*["']([^"']+)["']\s*\)\.({})$"#,
                kind.route_method_pattern()
            ))
            && let Some(capture) = pattern.captures(function)
            && let (Some(candidate), Some(prefix), Some(candidate_method)) =
                (capture.get(1), capture.get(2), capture.get(3))
            && receivers.contains(candidate.as_str())
        {
            receiver = Some(candidate.as_str().to_owned());
            method = Some(candidate_method.as_str().to_owned());
            chained_path = Some(prefix.as_str().to_owned());
        }

        if let (Some(receiver), Some(method), Some(arguments)) = (
            receiver.as_deref(),
            method.as_deref(),
            node.child_by_field_name("arguments"),
        ) {
            let arguments = split_arguments(node_text(arguments, source));
            let (methods, path_index) = if method == "on" {
                let methods = arguments
                    .first()
                    .map(|value| string_array_literals(value))
                    .unwrap_or_default();
                (methods, 1_usize)
            } else {
                (vec![method.to_owned()], 0_usize)
            };
            let raw_path = arguments
                .get(path_index)
                .and_then(|argument| string_literal(argument))
                .map(|route_path| {
                    chained_path
                        .as_deref()
                        .map_or(route_path.clone(), |prefix| join_paths(prefix, &route_path))
                });
            if let Some(raw_path) = raw_path {
                let raw_stages = arguments.iter().skip(path_index + 1).collect::<Vec<_>>();
                let (handler, middleware, mut detail) = router_stages(&raw_stages, kind, node);
                if let Some(handler) = handler {
                    detail.insert("receiver".into(), Value::String(receiver.to_owned()));
                    if let Some(prefix) = mounts.get(receiver).filter(|prefix| !prefix.is_empty()) {
                        detail.insert("mount_prefix".into(), Value::String(prefix.clone()));
                    }
                    for operation in methods {
                        facts.push(RawFrameworkFact::Route(RawRouteFact {
                            framework: kind.framework().to_owned(),
                            operation: if operation.eq_ignore_ascii_case("all") {
                                "ANY".to_owned()
                            } else {
                                operation.to_ascii_uppercase()
                            },
                            raw_path: raw_path.clone(),
                            normalized_path: normalize_path(&join_paths(
                                mounts.get(receiver).map(String::as_str).unwrap_or_default(),
                                &raw_path,
                            )),
                            declaring_scope: module_scope(path),
                            anchor: anchor(path, node),
                            handler_reference: handler.clone(),
                            middleware_references: middleware.clone(),
                            stages: Vec::new(),
                            origin: RawFrameworkOrigin::Ast,
                            rule: None,
                            detail: detail.clone(),
                        }));
                    }
                }
            }
        }

        if matches!(kind, NodeRouterKind::Fastify)
            && function
                .rsplit_once('.')
                .is_some_and(|(receiver, method)| receivers.contains(receiver) && method == "route")
            && let Some(arguments) = node.child_by_field_name("arguments")
            && let Some(route_object) = arguments.named_child(0)
        {
            let object = node_text(route_object, source);
            let Some(raw_path) = object_property_text(object, "url")
                .and_then(|value| string_literal(&value))
                .or_else(|| {
                    object_property_text(object, "path").and_then(|value| string_literal(&value))
                })
            else {
                return;
            };
            let methods = object_property_text(object, "method")
                .map(|value| string_array_literals(&value))
                .filter(|values| !values.is_empty())
                .unwrap_or_else(|| {
                    object_property_text(object, "method")
                        .and_then(|value| string_literal(&value))
                        .into_iter()
                        .collect()
                });
            if methods.is_empty() {
                return;
            }
            let Some(handler_value) = object_property_text(object, "handler") else {
                return;
            };
            let (handler, opaque) = handler_from_value(&handler_value, node);
            let receiver = function
                .rsplit_once('.')
                .map(|(receiver, _)| receiver)
                .unwrap_or_default();
            let mut detail =
                Map::from_iter([("receiver".into(), Value::String(receiver.to_owned()))]);
            if let Some(prefix) = mounts.get(receiver).filter(|prefix| !prefix.is_empty()) {
                detail.insert("mount_prefix".into(), Value::String(prefix.clone()));
            }
            if opaque {
                detail.insert("opaque_handler".into(), Value::Bool(true));
            }
            let middleware = object_hook_references(object);
            for operation in methods {
                facts.push(RawFrameworkFact::Route(RawRouteFact {
                    framework: "fastify".to_owned(),
                    operation: if operation.eq_ignore_ascii_case("all") {
                        "ANY".to_owned()
                    } else {
                        operation.to_ascii_uppercase()
                    },
                    raw_path: raw_path.clone(),
                    normalized_path: normalize_path(&join_paths(
                        mounts.get(receiver).map(String::as_str).unwrap_or_default(),
                        &raw_path,
                    )),
                    declaring_scope: module_scope(path),
                    anchor: anchor(path, node),
                    handler_reference: handler.clone(),
                    middleware_references: middleware.clone(),
                    stages: Vec::new(),
                    origin: RawFrameworkOrigin::Ast,
                    rule: None,
                    detail: detail.clone(),
                }));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_node_router_routes(child, source, path, receivers, mounts, kind, facts);
    }
}

fn router_stages(
    stages: &[&String],
    kind: NodeRouterKind,
    node: Node<'_>,
) -> (Option<String>, Vec<String>, Map<String, Value>) {
    let mut detail = Map::new();
    let mut candidates = Vec::new();
    let mut middleware = Vec::new();
    for stage in stages {
        if matches!(kind, NodeRouterKind::Fastify) && stage.trim_start().starts_with('{') {
            if let Some(value) = object_property_text(stage, "handler") {
                let (handler, opaque) = handler_from_value(&value, node);
                if opaque {
                    detail.insert("opaque_handler".into(), Value::Bool(true));
                }
                candidates.push(handler);
            }
            middleware.extend(object_hook_references(stage));
        } else if let Some(reference) = handler_reference(stage) {
            candidates.push(reference);
        } else if is_inline_handler_expression(stage) {
            candidates.push(format!("opaque_inline_handler_at_{}", node.start_byte()));
            detail.insert("opaque_handler".into(), Value::Bool(true));
        }
    }
    let handler = candidates.pop();
    middleware.extend(candidates);
    (handler, middleware, detail)
}

fn handler_from_value(value: &str, node: Node<'_>) -> (String, bool) {
    if let Some(reference) = handler_reference(value) {
        return (reference, false);
    }
    (
        format!("opaque_inline_handler_at_{}", node.start_byte()),
        true,
    )
}

fn object_hook_references(value: &str) -> Vec<String> {
    let Some(value) = value
        .trim()
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    else {
        return Vec::new();
    };
    super::text::split_top_level(value)
        .into_iter()
        .filter_map(|entry| {
            let entry = entry.trim();
            let (key, value) = entry.split_once(':').unwrap_or((entry, entry));
            let key = key
                .trim()
                .trim_matches(|character| matches!(character, '\'' | '"' | '`'));
            matches!(
                key,
                "onRequest"
                    | "preParsing"
                    | "preValidation"
                    | "preHandler"
                    | "onSend"
                    | "onResponse"
            )
            .then(|| list_or_reference(value))
        })
        .flatten()
        .collect()
}

fn list_or_reference(value: &str) -> Vec<String> {
    let value = value.trim();
    let value = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    split_arguments(value)
        .into_iter()
        .filter_map(|value| handler_reference(&value))
        .collect()
}

fn object_property_text(value: &str, name: &str) -> Option<String> {
    let value = value.trim().strip_prefix('{')?.strip_suffix('}')?;
    super::text::split_top_level(value)
        .into_iter()
        .find_map(|entry| {
            let entry = entry.trim();
            if entry == name {
                return Some(name.to_owned());
            }
            let (key, value) = entry.split_once(':')?;
            let key = key
                .trim()
                .trim_matches(|character| matches!(character, '\'' | '"' | '`'));
            (key == name).then(|| value.trim().to_owned())
        })
}

fn string_array_literals(value: &str) -> Vec<String> {
    let value = value
        .trim()
        .strip_suffix("as const")
        .map_or(value, str::trim);
    let value = value
        .trim()
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    split_arguments(value)
        .into_iter()
        .filter_map(|value| string_literal(&value))
        .collect()
}

fn source_has_module_require(node: Node<'_>, source: &[u8], module: &str) -> bool {
    if node.kind() == "call_expression"
        && node
            .child_by_field_name("function")
            .is_some_and(|function| node_text(function, source).trim() == "require")
        && node
            .child_by_field_name("arguments")
            .is_some_and(|arguments| {
                split_arguments(node_text(arguments, source))
                    .first()
                    .and_then(|argument| string_literal(argument))
                    .is_some_and(|value| value == module)
            })
    {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .any(|child| source_has_module_require(child, source, module))
}

pub(super) fn detect_non_express(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    if source.is_empty() {
        return Vec::new();
    }
    let mut imports = Vec::new();
    collect_import_aliases(path, root, source, &mut imports);
    attach_import_aliases(path, source, root, extraction, &imports);
    let imports_module = |expected: &str| {
        imports.iter().any(|alias| {
            alias.module == expected
                || (expected.ends_with('/') && alias.module.starts_with(expected))
                || (expected == "react-router" && alias.module.starts_with("react-router-"))
        })
    };
    let mut facts = Vec::new();
    let evidence = EvidenceSet::new()
        .direct_if(
            imports_module("@angular/router"),
            "angular-router",
            EvidenceKind::Import,
            "@angular/router",
        )
        .direct_if(
            imports_module("@nestjs/"),
            "nestjs",
            EvidenceKind::Import,
            "@nestjs/",
        )
        .direct_if(
            imports_module("vue-router"),
            "vue-router",
            EvidenceKind::Import,
            "vue-router",
        );
    if evidence.activates("nestjs") {
        collect_nest_routes(root, source, path, &mut facts);
    }
    if evidence.activates("angular-router") {
        collect_angular_router_routes(root, source, path, &mut facts);
    }
    if evidence.activates("vue-router") {
        collect_vue_router_routes(root, source, path, &mut facts);
    }
    facts
}

/// Dedicated React Router owner used after the universal pack cut. The
/// broader `typescript-web` adapter retains Angular/Nest/Vue behavior and
/// import identity, but no longer publishes React Router route facts.
pub(super) fn detect_react_router(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    if source.is_empty() {
        return Vec::new();
    }
    let mut imports = Vec::new();
    collect_import_aliases(path, root, source, &mut imports);
    attach_import_aliases(path, source, root, extraction, &imports);
    let file_route = detect_react_router_file_route(path, source, root);
    let active = imports.iter().any(|alias| {
        alias.module == "react-router"
            || alias.module == "react-router-dom"
            || alias.module.starts_with("react-router/")
            || alias.module.starts_with("react-router-dom/")
    });
    if !active {
        return file_route
            .map(|route| vec![RawFrameworkFact::Route(route)])
            .unwrap_or_default();
    }
    let mut facts = Vec::new();
    if let Some(route) = file_route {
        facts.push(RawFrameworkFact::Route(route));
    }
    collect_react_router_routes(root, source, path, &mut facts, "");
    facts
}

/// React Router's framework mode uses a file-based route convention in
/// `app/routes` or `src/routes`. Those modules frequently import only generated
/// route types, so the import-gated config detector above cannot activate for
/// them. Emit the route inventory and named loader/action stages from the
/// stable file convention while leaving target selection to the resolver.
fn detect_react_router_file_route(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
) -> Option<RawRouteFact> {
    let portable = path.to_string_lossy().replace('\\', "/");
    let (marker, relative) = ["app/routes/", "src/routes/"].iter().find_map(|marker| {
        portable
            .find(marker)
            .map(|index| (*marker, &portable[index + marker.len()..]))
    })?;
    let extension = Path::new(relative)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    // SvelteKit's `src/routes` convention is intentionally distinct from
    // React Router's file routes.  Its `+page/+server` modules otherwise
    // satisfy the broad directory heuristic and receive a misleading PAGE
    // route in projects that have no React Router dependency.
    if Path::new(relative)
        .file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|stem| {
            matches!(
                stem,
                "+page" | "+server" | "+layout" | "+error" | "+loading"
            )
        })
    {
        return None;
    }
    if !matches!(
        extension,
        "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs"
    ) {
        return None;
    }
    let route_file = relative
        .rsplit_once('.')
        .map_or(relative, |(stem, _)| stem)
        .trim_matches('/');
    if route_file.is_empty() || route_file.ends_with("routes") {
        return None;
    }
    let normalized_path = react_router_file_path(route_file);
    let syntax = super::typescript_syntax::TypeScriptSyntax::new(root, source);
    let default_reference = react_router_default_export_reference(syntax);
    let mut stages = Vec::new();
    let default_anchor = react_router_named_declaration_anchor(syntax, path, &default_reference)
        .unwrap_or_else(|| anchor(path, root));
    stages.push(RawRouteStageFact {
        role: RawRouteStageRole::RouteComponent,
        position: 0,
        reference: default_reference.clone(),
        anchor: default_anchor,
        origin: RawFrameworkOrigin::Convention,
        detail: Map::from_iter([(
            "convention".to_owned(),
            Value::String("react-router-file-route".to_owned()),
        )]),
    });
    for (name, role) in [
        ("loader", RawRouteStageRole::Loader),
        ("action", RawRouteStageRole::Action),
    ] {
        if !react_router_has_declaration(syntax, name) {
            continue;
        }
        let stage_anchor = react_router_named_declaration_anchor(syntax, path, name)
            .unwrap_or_else(|| anchor(path, root));
        stages.push(RawRouteStageFact {
            role,
            position: u32::try_from(stages.len()).unwrap_or(u32::MAX),
            reference: name.to_owned(),
            anchor: stage_anchor,
            origin: RawFrameworkOrigin::Ast,
            detail: Map::from_iter([("property".to_owned(), Value::String(name.to_owned()))]),
        });
    }
    Some(RawRouteFact {
        framework: "react-router".to_owned(),
        operation: "PAGE".to_owned(),
        raw_path: normalized_path.clone(),
        normalized_path,
        declaring_scope: marker.trim_end_matches('/').to_owned(),
        anchor: anchor(path, root),
        handler_reference: default_reference,
        middleware_references: Vec::new(),
        stages,
        origin: RawFrameworkOrigin::Convention,
        rule: Some("react-router-file-route-convention".to_owned()),
        detail: Map::from_iter([("file_route".to_owned(), Value::Bool(true))]),
    })
}

fn react_router_file_path(route_file: &str) -> String {
    let mut segments = Vec::new();
    for piece in route_file.split('/') {
        let mut pieces = piece.split('.').filter(|part| !part.is_empty());
        for part in pieces.by_ref() {
            if part == "_index" || part.starts_with('_') {
                continue;
            }
            if part == "route" {
                continue;
            }
            let segment = part
                .strip_prefix('$')
                .map_or_else(|| part.to_owned(), |name| format!("{{{name}}}"));
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
    }
    if segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", segments.join("/"))
    }
}

fn react_router_default_export_reference(
    syntax: super::typescript_syntax::TypeScriptSyntax<'_, '_>,
) -> String {
    for statement in syntax
        .descendants(syntax.root())
        .into_iter()
        .filter(|node| node.kind() == "export_statement")
    {
        if !syntax
            .text(statement)
            .is_some_and(|text| text.trim_start().starts_with("export default"))
        {
            continue;
        }
        let mut cursor = statement.walk();
        for child in statement.named_children(&mut cursor) {
            if let Some(name) = child
                .child_by_field_name("name")
                .and_then(|name| syntax.text(name))
                && (child.kind().contains("function") || child.kind().contains("class"))
            {
                return name.to_owned();
            }
            if matches!(child.kind(), "identifier" | "member_expression") {
                return syntax.text(child).unwrap_or("default").to_owned();
            }
        }
    }
    "default".to_owned()
}

fn react_router_has_declaration(
    syntax: super::typescript_syntax::TypeScriptSyntax<'_, '_>,
    expected: &str,
) -> bool {
    syntax.descendants(syntax.root()).into_iter().any(|node| {
        matches!(
            node.kind(),
            "function_declaration" | "class_declaration" | "variable_declarator"
        ) && node
            .child_by_field_name("name")
            .and_then(|name| syntax.text(name))
            .is_some_and(|name| name == expected)
    })
}

fn react_router_named_declaration_anchor(
    syntax: super::typescript_syntax::TypeScriptSyntax<'_, '_>,
    path: &Path,
    expected: &str,
) -> Option<RawFrameworkAnchor> {
    syntax
        .descendants(syntax.root())
        .into_iter()
        .find_map(|node| {
            if !matches!(
                node.kind(),
                "function_declaration" | "class_declaration" | "variable_declarator"
            ) {
                return None;
            }
            let name = node.child_by_field_name("name")?;
            (syntax.text(name) == Some(expected)).then(|| RawFrameworkAnchor {
                source_file: path.to_string_lossy().into_owned(),
                start_byte: name.start_byte() as u64,
                end_byte: name.end_byte() as u64,
                start_line: u32::try_from(name.start_position().row + 1).unwrap_or(u32::MAX),
                start_column: u32::try_from(name.start_position().column).unwrap_or(u32::MAX),
                end_line: u32::try_from(name.end_position().row + 1).unwrap_or(u32::MAX),
                end_column: u32::try_from(name.end_position().column).unwrap_or(u32::MAX),
            })
        })
}

fn attach_import_aliases(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
    aliases: &[ImportAlias],
) {
    attach_default_export_identities(path, source, root, extraction);
    let mut aliases = aliases.to_vec();
    aliases.sort_by(|left, right| {
        (
            left.local.as_str(),
            left.imported.as_str(),
            left.module.as_str(),
            left.anchor.start_byte,
            left.anchor.end_byte,
        )
            .cmp(&(
                right.local.as_str(),
                right.imported.as_str(),
                right.module.as_str(),
                right.anchor.start_byte,
                right.anchor.end_byte,
            ))
    });
    aliases.dedup();
    let source_file = path.to_string_lossy().into_owned();
    for alias in aliases {
        let ImportAlias {
            local,
            imported,
            module,
            anchor,
        } = alias;
        if extraction.nodes.iter().any(|node| {
            node.attributes.get("local_name").and_then(Value::as_str) == Some(local.as_str())
                && node.attributes.get("imported_name").and_then(Value::as_str)
                    == Some(imported.as_str())
        }) {
            continue;
        }
        extraction.nodes.push(RawNodeRecord {
            id: make_id(&["framework-import", &source_file, &module, &imported, &local]),
            attributes: Map::from_iter([
                ("label".into(), Value::String(local.clone())),
                ("name".into(), Value::String(local.clone())),
                (
                    "qualified_name".into(),
                    Value::String(format!("{module}.{imported}")),
                ),
                ("symbol_kind".into(), Value::String("import".into())),
                ("local_name".into(), Value::String(local)),
                ("imported_name".into(), Value::String(imported)),
                ("module".into(), Value::String(module)),
                ("source_file".into(), Value::String(source_file.clone())),
                (
                    "source_location".into(),
                    Value::String(format!("L{}", anchor.start_line)),
                ),
                ("line_start".into(), Value::from(anchor.start_line)),
                ("line_end".into(), Value::from(anchor.end_line)),
                ("column_start".into(), Value::from(anchor.start_column)),
                ("column_end".into(), Value::from(anchor.end_column)),
                ("start_byte".into(), Value::from(anchor.start_byte)),
                ("end_byte".into(), Value::from(anchor.end_byte)),
                ("file_type".into(), Value::String("code".into())),
                ("language".into(), Value::String("typescript".into())),
                ("_origin".into(), Value::String("ast".into())),
                (
                    "extractor".into(),
                    Value::String("compass.frameworks.typescript.imports".into()),
                ),
            ]),
        });
    }
}

fn attach_default_export_identities(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) {
    let mut exports = Vec::new();
    collect_default_export_identities(root, source, &mut exports);
    let source_file = path.to_string_lossy();
    for (name, line) in exports {
        if let Some(target) = extraction.nodes.iter_mut().find(|node| {
            node.attributes.get("source_file").and_then(Value::as_str) == Some(source_file.as_ref())
                && node.attributes.get("line_start").and_then(Value::as_u64) == Some(line)
                && node.label().trim_start_matches('.').trim_end_matches("()") == name
        }) {
            target
                .attributes
                .insert("export_name".into(), Value::String("default".into()));
        }
    }
}

fn collect_default_export_identities(
    node: Node<'_>,
    source: &[u8],
    exports: &mut Vec<(String, u64)>,
) {
    if node.kind() == "export_statement"
        && node_text(node, source)
            .trim_start()
            .starts_with("export default")
    {
        let mut cursor = node.walk();
        if let Some(declaration) = node.named_children(&mut cursor).find(|child| {
            matches!(
                child.kind(),
                "function_declaration" | "class_declaration" | "abstract_class_declaration"
            )
        }) && let Some(name) = declaration
            .child_by_field_name("name")
            .map(|name| node_text(name, source))
            .filter(|name| is_identifier(name))
        {
            exports.push((
                name.to_owned(),
                u64::try_from(declaration.start_position().row + 1).unwrap_or(u64::MAX),
            ));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_default_export_identities(child, source, exports);
    }
}

fn collect_import_aliases(
    path: &Path,
    node: Node<'_>,
    source: &[u8],
    aliases: &mut Vec<ImportAlias>,
) {
    if node.kind() == "import_statement" {
        let text = node_text(node, source);
        if let Some(module) = import_module(text) {
            let anchor = anchor(path, node);
            aliases.extend(
                parse_import_bindings(text)
                    .into_iter()
                    .map(|(local, imported)| ImportAlias {
                        local,
                        imported,
                        module: module.clone(),
                        anchor: anchor.clone(),
                    }),
            );
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_import_aliases(path, child, source, aliases);
    }
}

fn import_module(source: &str) -> Option<String> {
    let pattern = Regex::new(r#"\bfrom\s+["']([^"']+)["']"#).ok()?;
    pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
}

fn parse_import_bindings(source: &str) -> Vec<(String, String)> {
    let before_from = source.split(" from ").next().unwrap_or(source);
    let bindings = before_from
        .trim()
        .strip_prefix("import")
        .unwrap_or_default()
        .trim();
    let mut output = Vec::new();
    if let Some(namespace) = bindings.strip_prefix("* as ").map(str::trim)
        && is_identifier(namespace)
    {
        output.push((namespace.to_owned(), "*".to_owned()));
        return output;
    }
    if let (Some(open), Some(close)) = (bindings.find('{'), bindings.rfind('}')) {
        let default = bindings[..open].trim().trim_end_matches(',').trim();
        if is_identifier(default) {
            output.push((default.to_owned(), "default".to_owned()));
        }
        for binding in bindings[open + 1..close].split(',').map(str::trim) {
            if binding.is_empty() {
                continue;
            }
            let (imported, local) = binding
                .split_once(" as ")
                .map_or((binding, binding), |(imported, local)| {
                    (imported.trim(), local.trim())
                });
            if is_identifier(imported) && is_identifier(local) {
                output.push((local.to_owned(), imported.to_owned()));
            }
        }
    } else if is_identifier(bindings) {
        output.push((bindings.to_owned(), "default".to_owned()));
    }
    output
}

fn express_receivers(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut constructors = HashSet::new();
    collect_express_require_aliases(root, source, &mut constructors);
    let mut receivers = HashSet::new();
    collect_express_receivers(root, source, &constructors, &mut receivers);
    let body = std::str::from_utf8(source).unwrap_or_default();
    if let Ok(require_alias) = Regex::new(
        r#"(?m)^\s*(?:const|let|var)\s+([A-Za-z_]\w*)\s*=\s*require\(\s*["']express["']\s*\)\s*;"#,
    ) {
        for capture in require_alias.captures_iter(body) {
            if let Some(alias) = capture.get(1)
                && let Ok(constructed) = Regex::new(&format!(
                    r"(?m)^\s*(?:const|let|var)\s+([A-Za-z_]\w*)\s*=\s*{}(?:\.Router)?\(\s*\)\s*;",
                    regex::escape(alias.as_str())
                ))
            {
                receivers.extend(
                    constructed
                        .captures_iter(body)
                        .filter_map(|capture| capture.get(1).map(|name| name.as_str().to_owned())),
                );
            }
        }
    }
    receivers
}

fn collect_express_require_aliases(
    node: Node<'_>,
    source: &[u8],
    constructors: &mut HashSet<String>,
) {
    if node.kind() == "variable_declarator"
        && let (Some(name), Some(value)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("value"),
        )
        && is_identifier(node_text(name, source).trim())
        && node_text(value, source).trim().starts_with("require(")
        && node_text(value, source).contains("express")
    {
        constructors.insert(node_text(name, source).trim().to_owned());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_express_require_aliases(child, source, constructors);
    }
}

fn collect_express_receivers(
    node: Node<'_>,
    source: &[u8],
    constructors: &HashSet<String>,
    receivers: &mut HashSet<String>,
) {
    if node.kind() == "variable_declarator"
        && let (Some(name), Some(value)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("value"),
        )
    {
        let variable = node_text(name, source).trim();
        let expression = node_text(value, source).trim();
        if is_identifier(variable)
            && (matches!(
                expression,
                "express()" | "Router()" | "express.Router()" | "router()"
            ) || expression.starts_with("require(\"express\")")
                || expression.starts_with("require('express')")
                || constructors.iter().any(|constructor| {
                    expression == format!("{constructor}()")
                        || expression == format!("{constructor}.Router()")
                }))
        {
            receivers.insert(variable.to_owned());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_express_receivers(child, source, constructors, receivers);
    }
}

fn collect_express_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    receivers: &HashSet<String>,
    mounts: &HashMap<String, String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
    {
        let function = node_text(function, source).trim();
        let parsed = function.rsplit_once('.').and_then(|(receiver, method)| {
            if receivers.contains(receiver) && HTTP_METHODS.contains(&method) {
                return Some((receiver, method, None));
            }
            let pattern = Regex::new(&format!(
                r#"^([A-Za-z_]\w*)\.route\(\s*["']([^"']+)["']\s*\)\.({})$"#,
                HTTP_METHODS.join("|")
            ))
            .ok()?;
            let capture = pattern.captures(function)?;
            let receiver = capture.get(1)?.as_str();
            receivers.contains(receiver).then_some((
                receiver,
                capture.get(3)?.as_str(),
                capture.get(2).map(|value| value.as_str().to_owned()),
            ))
        });
        if let Some((receiver, method, chained_path)) = parsed
            && let Some(arguments) = node.child_by_field_name("arguments")
        {
            let arguments = split_arguments(node_text(arguments, source));
            let is_chained = chained_path.is_some();
            let raw_path = chained_path.or_else(|| {
                arguments
                    .first()
                    .and_then(|argument| string_literal(argument))
            });
            if let Some(raw_path) = raw_path {
                let raw_stages = if is_chained {
                    arguments.iter().collect::<Vec<_>>()
                } else {
                    arguments.iter().skip(1).collect::<Vec<_>>()
                };
                let last_stage = raw_stages.last().copied();
                let inline_handler = last_stage.is_some_and(|argument| {
                    handler_reference(argument).is_none() && is_inline_handler_expression(argument)
                });
                let stages = raw_stages
                    .iter()
                    .filter_map(|argument| handler_reference(argument))
                    .collect::<Vec<_>>();
                let (handler, middleware, mut detail) = if inline_handler {
                    let handler = format!("opaque_inline_handler_at_{}", node.start_byte());
                    let middleware = raw_stages
                        .iter()
                        .take(raw_stages.len().saturating_sub(1))
                        .filter_map(|argument| handler_reference(argument))
                        .collect::<Vec<_>>();
                    (
                        handler,
                        middleware,
                        Map::from_iter([("opaque_handler".into(), Value::Bool(true))]),
                    )
                } else if let Some((handler, middleware)) = stages.split_last() {
                    (handler.clone(), middleware.to_vec(), Map::new())
                } else {
                    (String::new(), Vec::new(), Map::new())
                };
                if !handler.is_empty() {
                    detail.insert("receiver".into(), Value::String(receiver.to_owned()));
                    if let Some(prefix) = mounts.get(receiver).filter(|prefix| !prefix.is_empty()) {
                        detail.insert("mount_prefix".into(), Value::String(prefix.clone()));
                    }
                    facts.push(RawFrameworkFact::Route(RawRouteFact {
                        framework: "express".to_owned(),
                        operation: if method == "all" {
                            "ANY".to_owned()
                        } else {
                            method.to_ascii_uppercase()
                        },
                        raw_path: raw_path.clone(),
                        normalized_path: normalize_path(&join_paths(
                            mounts.get(receiver).map(String::as_str).unwrap_or_default(),
                            &raw_path,
                        )),
                        declaring_scope: module_scope(path),
                        anchor: anchor(path, node),
                        handler_reference: handler,
                        middleware_references: middleware,
                        stages: Vec::new(),
                        origin: RawFrameworkOrigin::Ast,
                        rule: None,
                        detail,
                    }));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_express_routes(child, source, path, receivers, mounts, facts);
    }
}

fn source_has_express_require(node: Node<'_>, source: &[u8]) -> bool {
    if node.kind() == "call_expression"
        && node
            .child_by_field_name("function")
            .is_some_and(|function| node_text(function, source).trim() == "require")
        && node
            .child_by_field_name("arguments")
            .is_some_and(|arguments| {
                split_arguments(node_text(arguments, source))
                    .first()
                    .and_then(|argument| string_literal(argument))
                    .is_some_and(|module| module == "express")
            })
    {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .any(|child| source_has_express_require(child, source))
}

fn express_mounts(
    root: Node<'_>,
    source: &[u8],
    receivers: &HashSet<String>,
) -> HashMap<String, String> {
    let mut mounts = HashMap::new();
    collect_express_mounts(root, source, receivers, &mut mounts);
    mounts
}

fn collect_express_mounts(
    node: Node<'_>,
    source: &[u8],
    receivers: &HashSet<String>,
    mounts: &mut HashMap<String, String>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && let Some((parent, "use")) = node_text(function, source).trim().rsplit_once('.')
        && receivers.contains(parent)
        && let Some(arguments) = node.child_by_field_name("arguments")
    {
        let arguments = split_arguments(node_text(arguments, source));
        if let Some(prefix) = arguments
            .first()
            .and_then(|argument| string_literal(argument))
            && let Some(child) = arguments.get(1).and_then(|argument| {
                let candidate = argument.trim();
                is_identifier(candidate).then_some(candidate)
            })
            && receivers.contains(child)
        {
            let parent_prefix = mounts.get(parent).map(String::as_str).unwrap_or_default();
            mounts.insert(child.to_owned(), join_paths(parent_prefix, &prefix));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_express_mounts(child, source, receivers, mounts);
    }
}

fn collect_express_mount_facts(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    receivers: &HashSet<String>,
    imports: &[ImportAlias],
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && let Some((parent, "use")) = node_text(function, source).trim().rsplit_once('.')
        && receivers.contains(parent)
        && let Some(arguments) = node.child_by_field_name("arguments")
    {
        let arguments = split_arguments(node_text(arguments, source));
        if let (Some(prefix), Some(target)) = (
            arguments
                .first()
                .and_then(|argument| string_literal(argument)),
            arguments.get(1).map(|argument| argument.trim()),
        ) && is_identifier(target)
            && (receivers.contains(target) || imports.iter().any(|alias| alias.local == target))
        {
            facts.push(router_mount_fact(
                "express", path, node, parent, target, &prefix, imports,
            ));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_express_mount_facts(child, source, path, receivers, imports, facts);
    }
}

fn router_mount_fact(
    framework: &str,
    path: &Path,
    node: Node<'_>,
    parent: &str,
    target: &str,
    prefix: &str,
    imports: &[ImportAlias],
) -> RawFrameworkFact {
    let target_module = imports
        .iter()
        .find(|alias| alias.local == target)
        .map(|alias| alias.module.clone())
        .unwrap_or_default();
    RawFrameworkFact::Domain(RawDomainFact {
        framework: framework.to_owned(),
        kind: "router_mount".to_owned(),
        name: target.to_owned(),
        declaring_scope: module_scope(path),
        anchor: anchor(path, node),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::from_iter([
            ("parent_receiver".into(), Value::String(parent.to_owned())),
            ("target_receiver".into(), Value::String(target.to_owned())),
            ("target_module".into(), Value::String(target_module)),
            ("mount_prefix".into(), Value::String(prefix.to_owned())),
        ]),
    })
}

fn collect_nest_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "class_declaration" {
        collect_nest_class(node, source, path, facts);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_nest_routes(child, source, path, facts);
    }
}

fn collect_nest_class(
    class: Node<'_>,
    source: &[u8],
    path: &Path,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let Some(name_node) = class.child_by_field_name("name") else {
        return;
    };
    let class_name = node_text(name_node, source);
    let class_text = decorator_context(class, source);
    let controller_present = has_decorator(&class_text, "Controller");
    let controller_prefix = controller_present
        .then(|| decorator_argument(&class_text, "Controller"))
        .flatten();
    let is_resolver = has_decorator(&class_text, "Resolver");
    let is_gateway = has_decorator(&class_text, "WebSocketGateway");
    if (!controller_present && !is_resolver && !is_gateway)
        || (controller_present && controller_prefix.is_none())
    {
        return;
    }
    collect_nest_methods(
        class,
        source,
        path,
        class_name,
        controller_prefix.as_deref().unwrap_or_default(),
        is_resolver,
        is_gateway,
        facts,
    );
}

#[allow(clippy::too_many_arguments)]
fn collect_nest_methods(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    class_name: &str,
    controller_prefix: &str,
    is_resolver: bool,
    is_gateway: bool,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "method_definition" {
        let method_name = node
            .child_by_field_name("name")
            .map(|name| node_text(name, source))
            .unwrap_or_default();
        let method_text = decorator_context(node, source);
        let handler = format!("{class_name}.{method_name}");
        for (decorator, operation) in [
            ("Get", "GET"),
            ("Post", "POST"),
            ("Put", "PUT"),
            ("Patch", "PATCH"),
            ("Delete", "DELETE"),
            ("Options", "OPTIONS"),
            ("Head", "HEAD"),
            ("All", "ANY"),
        ] {
            if has_decorator(&method_text, decorator) {
                let Some(local) = decorator_argument(&method_text, decorator) else {
                    continue;
                };
                let route = join_paths(controller_prefix, &local);
                facts.push(route_fact(
                    "nestjs",
                    operation,
                    &route,
                    &handler,
                    path,
                    node,
                    Map::new(),
                ));
            }
        }
        if has_decorator(&method_text, "RequestMapping") {
            let Some(local) = decorator_argument(&method_text, "RequestMapping") else {
                return;
            };
            let operation =
                request_mapping_method(&method_text).unwrap_or_else(|| "ANY".to_owned());
            let route = join_paths(controller_prefix, &local);
            facts.push(route_fact(
                "nestjs",
                &operation,
                &route,
                &handler,
                path,
                node,
                Map::new(),
            ));
        }
        if is_resolver {
            for (decorator, operation) in [("Query", "QUERY"), ("Mutation", "MUTATION")] {
                if has_decorator(&method_text, decorator) {
                    let Some(argument) = decorator_argument(&method_text, decorator) else {
                        continue;
                    };
                    let field = if argument.is_empty() {
                        method_name.to_owned()
                    } else {
                        argument
                    };
                    facts.push(route_fact(
                        "nestjs-graphql",
                        operation,
                        "/graphql",
                        &handler,
                        path,
                        node,
                        Map::from_iter([("graphql_field".into(), Value::String(field.to_owned()))]),
                    ));
                }
            }
        }
        for (decorator, kind, transport, relationship) in [
            ("MessagePattern", "message", "microservice", "handles"),
            ("EventPattern", "event", "microservice", "handles"),
            ("SubscribeMessage", "message", "websocket", "subscribes"),
        ] {
            if decorator == "SubscribeMessage" && !is_gateway {
                continue;
            }
            if has_decorator(&method_text, decorator) {
                let Some(argument) = decorator_argument(&method_text, decorator) else {
                    continue;
                };
                let subject = if argument.is_empty() {
                    method_name.to_owned()
                } else {
                    argument
                };
                facts.push(RawFrameworkFact::Domain(RawDomainFact {
                    framework: "nestjs".to_owned(),
                    kind: kind.to_owned(),
                    name: subject.clone(),
                    declaring_scope: module_scope(path),
                    anchor: anchor(path, node),
                    origin: RawFrameworkOrigin::Ast,
                    detail: Map::from_iter([
                        ("transport".into(), Value::String(transport.to_owned())),
                        ("subject".into(), Value::String(subject.clone())),
                        ("handler_reference".into(), Value::String(handler.clone())),
                        (
                            "relationship".into(),
                            Value::String(relationship.to_owned()),
                        ),
                    ]),
                }));
                if decorator == "SubscribeMessage" {
                    facts.push(RawFrameworkFact::Domain(RawDomainFact {
                        framework: "nestjs".to_owned(),
                        kind: kind.to_owned(),
                        name: subject.clone(),
                        declaring_scope: module_scope(path),
                        anchor: anchor(path, node),
                        origin: RawFrameworkOrigin::Ast,
                        detail: Map::from_iter([
                            ("transport".into(), Value::String(transport.to_owned())),
                            ("subject".into(), Value::String(subject)),
                            ("handler_reference".into(), Value::String(handler.clone())),
                            ("relationship".into(), Value::String("registers".to_owned())),
                        ]),
                    }));
                }
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_nest_methods(
            child,
            source,
            path,
            class_name,
            controller_prefix,
            is_resolver,
            is_gateway,
            facts,
        );
    }
}

fn collect_react_router_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    facts: &mut Vec<RawFrameworkFact>,
    parent_path: &str,
) {
    let mut next_parent = parent_path.to_owned();
    if matches!(node.kind(), "jsx_self_closing_element" | "jsx_element") {
        let text = node_text(node, source);
        if is_route_jsx_element(text)
            && let Some(raw_path) = jsx_string_attribute(text, "path")
            && let Some(handler) = jsx_component_attribute(text, "element")
                .or_else(|| jsx_reference_attribute(text, "Component"))
        {
            let route_path = join_paths(parent_path, &raw_path);
            facts.push(route_fact(
                "react-router",
                "PAGE",
                &route_path,
                &handler,
                path,
                node,
                Map::new(),
            ));
            next_parent = route_path;
        }
    }
    if node.parent().is_none() {
        collect_router_config_calls(node, source, path, "react-router", facts);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_react_router_routes(child, source, path, facts, &next_parent);
    }
}

fn collect_vue_router_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.parent().is_none() {
        collect_router_config_calls(node, source, path, "vue-router", facts);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_vue_router_routes(child, source, path, facts);
    }
}

fn collect_angular_router_routes(
    root: Node<'_>,
    source: &[u8],
    path: &Path,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let mut arrays = BTreeMap::new();
    collect_named_route_arrays(root, source, &mut arrays);
    let mut collected = HashSet::new();
    for array in arrays.values().filter(|array| {
        array
            .parent()
            .is_some_and(|declaration| node_text(declaration, source).contains(": Routes"))
    }) {
        if collected.insert(array.id()) {
            collect_route_config_with_parent(*array, source, path, "angular-router", facts, "");
        }
    }
    collect_angular_config_calls(root, source, path, &arrays, &mut collected, facts);
}

fn collect_named_route_arrays<'tree>(
    node: Node<'tree>,
    source: &[u8],
    arrays: &mut BTreeMap<String, Node<'tree>>,
) {
    if node.kind() == "variable_declarator"
        && let (Some(name), Some(value)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("value"),
        )
        && name.kind() == "identifier"
        && value.kind() == "array"
    {
        arrays.insert(node_text(name, source).to_owned(), value);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_named_route_arrays(child, source, arrays);
    }
}

fn collect_angular_config_calls(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    arrays: &BTreeMap<String, Node<'_>>,
    collected: &mut HashSet<usize>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && matches!(
            node_text(function, source).trim().rsplit('.').next(),
            Some("provideRouter" | "forRoot" | "forChild" | "resetConfig")
        )
        && let Some(arguments) = node.child_by_field_name("arguments")
    {
        let mut cursor = arguments.walk();
        if let Some(argument) = arguments.named_children(&mut cursor).next() {
            let array = if argument.kind() == "array" {
                Some(argument)
            } else if argument.kind() == "identifier" {
                arrays.get(node_text(argument, source)).copied()
            } else {
                None
            };
            if let Some(array) = array.filter(|array| collected.insert(array.id())) {
                collect_route_config_with_parent(array, source, path, "angular-router", facts, "");
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_angular_config_calls(child, source, path, arrays, collected, facts);
    }
}

fn collect_router_config_calls(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    framework: &str,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && matches!(
            node_text(function, source).trim().rsplit('.').next(),
            Some(
                "createBrowserRouter" | "createHashRouter" | "createMemoryRouter" | "createRouter"
            )
        )
        && let Some(arguments) = node.child_by_field_name("arguments")
    {
        let mut cursor = arguments.walk();
        if let Some(argument) = arguments.named_children(&mut cursor).next() {
            collect_route_config_with_parent(argument, source, path, framework, facts, "");
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_router_config_calls(child, source, path, framework, facts);
    }
}

fn collect_route_config_with_parent(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    framework: &str,
    facts: &mut Vec<RawFrameworkFact>,
    parent_path: &str,
) {
    let mut current_parent = parent_path.to_owned();
    if matches!(node.kind(), "object" | "object_pattern")
        && let Some(raw_path) = direct_object_string_property(node, source, "path")
    {
        current_parent = join_paths(parent_path, &raw_path);
        if let Some((handler, opaque_handler, handler_node, handler_property)) =
            direct_object_handler_property(node, source)
        {
            let mut stages = Vec::new();
            let mut legacy_middleware = Vec::new();
            for (property, role) in [
                ("loader", RawRouteStageRole::Loader),
                ("action", RawRouteStageRole::Action),
                ("errorElement", RawRouteStageRole::Boundary),
                ("errorComponent", RawRouteStageRole::Boundary),
            ] {
                let Some((raw_value, value_node)) =
                    direct_object_value_property(node, source, property)
                else {
                    continue;
                };
                let raw_value = raw_value.trim();
                let reference = if is_reference(raw_value) {
                    raw_value.to_owned()
                } else if role == RawRouteStageRole::Boundary {
                    jsx_tag_reference(raw_value).unwrap_or_else(|| {
                        format!("opaque_route_handler_at_{}", value_node.start_byte())
                    })
                } else {
                    continue;
                };
                if role == RawRouteStageRole::Loader {
                    // Preserve the legacy projection while the typed stage
                    // remains authoritative for resolver/publication code.
                    legacy_middleware.push(reference.clone());
                }
                stages.push(RawRouteStageFact {
                    role,
                    position: u32::try_from(stages.len()).unwrap_or(u32::MAX),
                    reference,
                    anchor: anchor(path, value_node),
                    origin: RawFrameworkOrigin::Ast,
                    detail: Map::from_iter([(
                        "property".to_owned(),
                        Value::String(property.to_owned()),
                    )]),
                });
            }
            // Preserve the established handler stage for Angular/Vue route
            // config adapters. React Router framework mode is the one pack
            // that needs the more specific route-component stage; changing
            // unrelated adapters would silently break their public route
            // contract and downstream qualification fixtures.
            let component_role = if framework == "react-router" {
                RawRouteStageRole::RouteComponent
            } else {
                RawRouteStageRole::Handler
            };
            stages.push(RawRouteStageFact {
                role: component_role,
                position: u32::try_from(stages.len()).unwrap_or(u32::MAX),
                reference: handler.clone(),
                anchor: anchor(path, handler_node),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::from_iter([("property".to_owned(), Value::String(handler_property))]),
            });
            let detail = if opaque_handler {
                Map::from_iter([("opaque_handler".into(), Value::Bool(true))])
            } else {
                Map::new()
            };
            let mut fact = match route_fact(
                framework,
                "PAGE",
                &current_parent,
                &handler,
                path,
                node,
                detail,
            ) {
                RawFrameworkFact::Route(route) => route,
                RawFrameworkFact::Domain(_)
                | RawFrameworkFact::Annotation(_)
                | RawFrameworkFact::Role(_)
                | RawFrameworkFact::Relation(_)
                | RawFrameworkFact::Configuration(_)
                | RawFrameworkFact::FileSet(_) => unreachable!(),
            };
            fact.middleware_references = legacy_middleware;
            fact.stages = stages;
            facts.push(RawFrameworkFact::Route(fact));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        let child_parent = if child.kind() == "pair"
            && child
                .child_by_field_name("key")
                .is_some_and(|key| node_text(key, source).trim_matches(['"', '\'']) == "children")
        {
            current_parent.as_str()
        } else {
            parent_path
        };
        collect_route_config_with_parent(child, source, path, framework, facts, child_parent);
    }
}

fn route_fact(
    framework: &str,
    operation: &str,
    raw_path: &str,
    handler: &str,
    source_path: &Path,
    node: Node<'_>,
    detail: Map<String, Value>,
) -> RawFrameworkFact {
    RawFrameworkFact::Route(RawRouteFact {
        framework: framework.to_owned(),
        operation: operation.to_owned(),
        raw_path: raw_path.to_owned(),
        normalized_path: normalize_path(raw_path),
        declaring_scope: module_scope(source_path),
        anchor: anchor(source_path, node),
        handler_reference: handler.to_owned(),
        middleware_references: Vec::new(),
        stages: Vec::new(),
        origin: RawFrameworkOrigin::Ast,
        rule: None,
        detail,
    })
}

fn split_arguments(value: &str) -> Vec<String> {
    let value = value.trim();
    let value = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value);
    let mut output = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
            continue;
        }
        match character {
            '(' | '[' | '{' | '<' => depth = depth.saturating_add(1),
            ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                output.push(value[start..index].trim().to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < value.len() {
        output.push(value[start..].trim().to_owned());
    }
    output
}

fn handler_reference(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value
        .strip_prefix("async ")
        .unwrap_or(value)
        .trim_start_matches("await ");
    if is_reference(value) {
        return Some(value.to_owned());
    }
    let (callee, _) = value.split_once('(')?;
    let callee = callee.trim();
    is_reference(callee).then(|| callee.to_owned())
}

fn is_inline_handler_expression(value: &str) -> bool {
    let value = value.trim();
    value.contains("=>")
        || value.starts_with("function")
        || value.starts_with("async function")
        || value.starts_with("async(")
        || value.starts_with("async (")
}

fn string_literal(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() < 2 {
        return None;
    }
    let first = value.chars().next()?;
    let last = value.chars().last()?;
    if first == last && matches!(first, '\'' | '"') {
        return Some(value[1..value.len() - 1].to_owned());
    }
    if first == '`' && last == '`' && !value.contains("${") {
        return Some(value[1..value.len() - 1].to_owned());
    }
    None
}

fn decorator_argument(source: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r"@\s*{}\b", regex::escape(name))).ok()?;
    let marker = pattern.find(source)?;
    let rest = source[marker.end()..].trim_start();
    let Some(rest) = rest.strip_prefix('(') else {
        return Some(String::new());
    };
    let mut depth = 1_u32;
    let mut quote = None;
    let mut escaped = false;
    let mut close = None;
    for (offset, character) in rest.char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
            continue;
        }
        match character {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close = Some(offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let body = rest.get(..close?)?.trim();
    if body.is_empty() {
        return Some(String::new());
    }
    if let Some(value) = string_literal(body) {
        return Some(value);
    }
    if body.starts_with('[')
        && body.ends_with(']')
        && let Some(value) = super::text::split_top_level(&body[1..body.len() - 1])
            .into_iter()
            .find_map(string_literal)
    {
        return Some(value);
    }
    Regex::new(r#"(?:^|\b)(?:path|value|name)\s*:\s*["'`]([^"'`]+)["'`]"#)
        .ok()?
        .captures(body)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
}

fn has_decorator(source: &str, name: &str) -> bool {
    Regex::new(&format!(r"@\s*{}\b", regex::escape(name)))
        .is_ok_and(|pattern| pattern.is_match(source))
}

fn request_mapping_method(source: &str) -> Option<String> {
    let pattern = Regex::new(r"RequestMethod\.(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD)").ok()?;
    pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
}

fn jsx_string_attribute(source: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r#"\b{}\s*=\s*["']([^"']+)["']"#,
        regex::escape(name)
    ))
    .ok()?;
    pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
}

fn jsx_component_attribute(source: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r"\b{}\s*=\s*\{{\s*<\s*([A-Z][A-Za-z0-9_$.]*)",
        regex::escape(name)
    ))
    .ok()?;
    pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
}

fn is_route_jsx_element(source: &str) -> bool {
    let trimmed = source.trim_start();
    let Some(rest) = trimmed.strip_prefix("<Route") else {
        return false;
    };
    rest.chars()
        .next()
        .is_some_and(|character| matches!(character, ' ' | '\t' | '\n' | '/' | '>'))
}

fn jsx_reference_attribute(source: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r"\b{}\s*=\s*\{{\s*([A-Z][A-Za-z0-9_$.]*)\s*\}}",
        regex::escape(name)
    ))
    .ok()?;
    pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
}

fn direct_object_string_property(node: Node<'_>, source: &[u8], name: &str) -> Option<String> {
    direct_object_property(node, source, name).and_then(string_literal)
}

fn direct_object_value_property<'tree>(
    node: Node<'tree>,
    source: &[u8],
    name: &str,
) -> Option<(String, Node<'tree>)> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "pair")
        .find_map(|pair| {
            let key = pair.child_by_field_name("key")?;
            let key = node_text(key, source).trim().trim_matches(['"', '\'', '`']);
            if key != name {
                return None;
            }
            let value = pair
                .child_by_field_name("value")
                .or_else(|| pair.named_child(1))?;
            Some((node_text(value, source).to_owned(), value))
        })
}

fn jsx_tag_reference(value: &str) -> Option<String> {
    let value = value.trim().strip_prefix('<')?;
    let tag = value
        .split(|character: char| character.is_whitespace() || matches!(character, '/' | '>'))
        .next()?;
    is_reference(tag).then(|| tag.to_owned())
}

fn direct_object_handler_property<'tree>(
    node: Node<'tree>,
    source: &[u8],
) -> Option<(String, bool, Node<'tree>, String)> {
    for name in [
        "component",
        "element",
        "Component",
        "loadComponent",
        "loadChildren",
    ] {
        let Some((value, value_node)) = direct_object_value_property(node, source, name) else {
            continue;
        };
        let value = value.trim();
        if is_reference(value) {
            return Some((value.to_owned(), false, value_node, name.to_owned()));
        }
        if let Some(tag) = value.strip_prefix('<').and_then(|value| {
            value
                .split(|character: char| {
                    character.is_whitespace() || matches!(character, '/' | '>')
                })
                .next()
        }) && is_reference(tag)
        {
            return Some((tag.to_owned(), false, value_node, name.to_owned()));
        }
        if value.contains("=>") || value.contains("import(") || value.starts_with("lazy(") {
            return Some((
                format!("opaque_route_handler_at_{}", node.start_byte()),
                true,
                value_node,
                name.to_owned(),
            ));
        }
    }
    None
}

fn direct_object_property<'a>(node: Node<'_>, source: &'a [u8], name: &str) -> Option<&'a str> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "pair")
        .find_map(|pair| {
            let key = pair.child_by_field_name("key")?;
            let key = node_text(key, source)
                .trim()
                .trim_matches(|character| matches!(character, '\'' | '"' | '`'));
            (key == name)
                .then(|| pair.child_by_field_name("value"))
                .flatten()
                .map(|value| node_text(value, source))
        })
}

fn normalize_path(path: &str) -> String {
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let Ok(express) = Regex::new(r":([A-Za-z_][A-Za-z0-9_]*)") else {
        return path;
    };
    express.replace_all(&path, "{$1}").into_owned()
}

fn join_paths(prefix: &str, suffix: &str) -> String {
    let prefix = prefix.trim_matches('/');
    let suffix = suffix.trim_matches('/');
    match (prefix.is_empty(), suffix.is_empty()) {
        (true, true) => "/".to_owned(),
        (false, true) => format!("/{prefix}"),
        (true, false) => format!("/{suffix}"),
        (false, false) => format!("/{prefix}/{suffix}"),
    }
}

fn module_scope(path: &Path) -> String {
    path.with_extension("")
        .to_string_lossy()
        .replace(['/', '\\'], ".")
        .trim_start_matches('.')
        .to_owned()
}

fn anchor(path: &Path, node: Node<'_>) -> RawFrameworkAnchor {
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().into_owned(),
        start_byte: node.start_byte() as u64,
        end_byte: node.end_byte() as u64,
        start_line: u32::try_from(node.start_position().row + 1).unwrap_or(u32::MAX),
        start_column: u32::try_from(node.start_position().column).unwrap_or(u32::MAX),
        end_line: u32::try_from(node.end_position().row + 1).unwrap_or(u32::MAX),
        end_column: u32::try_from(node.end_position().column).unwrap_or(u32::MAX),
    }
}

fn node_text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or_default()
}

fn decorator_context(node: Node<'_>, source: &[u8]) -> String {
    let start = node.start_byte().min(source.len());
    let preceding = &source[..start];
    let boundary = preceding
        .iter()
        .rposition(|byte| matches!(*byte, b'{' | b'}' | b';'))
        .map_or(0, |index| index + 1);
    String::from_utf8_lossy(&source[boundary..node.end_byte().min(source.len())]).into_owned()
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character == '_' || character == '$' || character.is_alphabetic())
        && chars
            .all(|character| character == '_' || character == '$' || character.is_alphanumeric())
}

fn is_reference(value: &str) -> bool {
    value.split('.').all(is_identifier)
}
