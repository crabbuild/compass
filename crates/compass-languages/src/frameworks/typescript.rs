use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
};
use crate::{Extraction, RawNodeRecord, make_id};

const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "options", "head", "all",
];

pub(super) fn detect(
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
    collect_import_aliases(root, source, &mut imports);
    attach_import_aliases(path, source, root, extraction, &imports);
    let imports_module = |expected: &str| {
        imports.iter().any(|(_, _, module, _)| {
            module == expected
                || (expected.ends_with('/') && module.starts_with(expected))
                || (expected == "react-router" && module.starts_with("react-router-"))
        })
    };
    let mut facts = Vec::new();
    let receivers = express_receivers(root, source);
    let mounts = express_mounts(root, source, &receivers);
    let evidence = EvidenceSet::new()
        .direct_if(
            !receivers.is_empty()
                && (imports_module("express")
                    || source_has_express_require(root, source)
                    || body.contains("require(\"express\")")
                    || body.contains("require('express')")),
            "express",
            EvidenceKind::Receiver,
            "express application/router",
        )
        .direct_if(
            imports_module("@nestjs/"),
            "nestjs",
            EvidenceKind::Import,
            "@nestjs/",
        )
        .direct_if(
            imports_module("react-router"),
            "react-router",
            EvidenceKind::Import,
            "react-router",
        )
        .direct_if(
            imports_module("vue-router"),
            "vue-router",
            EvidenceKind::Import,
            "vue-router",
        );
    if evidence.activates("express") {
        collect_express_routes(root, source, path, &receivers, &mounts, &mut facts);
    }
    if evidence.activates("nestjs") {
        collect_nest_routes(root, source, path, &mut facts);
    }
    if evidence.activates("react-router") {
        collect_react_router_routes(root, source, path, &mut facts, "");
    }
    if evidence.activates("vue-router") {
        collect_vue_router_routes(root, source, path, &mut facts);
    }
    facts
}

fn attach_import_aliases(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
    aliases: &[(String, String, String, u64)],
) {
    attach_default_export_identities(path, source, root, extraction);
    let mut aliases = aliases.to_vec();
    aliases.sort();
    aliases.dedup();
    let source_file = path.to_string_lossy().into_owned();
    for (local, imported, module, line) in aliases {
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
                ("source_location".into(), Value::String(format!("L{line}"))),
                ("line_start".into(), Value::from(line)),
                ("line_end".into(), Value::from(line)),
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
    node: Node<'_>,
    source: &[u8],
    aliases: &mut Vec<(String, String, String, u64)>,
) {
    if node.kind() == "import_statement" {
        let text = node_text(node, source);
        if let Some(module) = import_module(text) {
            let line = u64::try_from(node.start_position().row + 1).unwrap_or(u64::MAX);
            aliases.extend(
                parse_import_bindings(text)
                    .into_iter()
                    .map(|(local, imported)| (local, imported, module.clone(), line)),
            );
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_import_aliases(child, source, aliases);
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
        if let Some((handler, opaque_handler)) = direct_object_handler_property(node, source) {
            let middleware = ["loader", "action"]
                .into_iter()
                .filter_map(|property| direct_object_identifier_property(node, source, property))
                .collect();
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
                RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => unreachable!(),
            };
            fact.middleware_references = middleware;
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

fn direct_object_identifier_property(node: Node<'_>, source: &[u8], name: &str) -> Option<String> {
    let value = direct_object_property(node, source, name)?.trim();
    is_reference(value).then(|| value.to_owned())
}

fn direct_object_handler_property(node: Node<'_>, source: &[u8]) -> Option<(String, bool)> {
    for name in ["component", "element", "Component"] {
        let Some(value) = direct_object_property(node, source, name) else {
            continue;
        };
        let value = value.trim();
        if is_reference(value) {
            return Some((value.to_owned(), false));
        }
        if let Some(tag) = value.strip_prefix('<').and_then(|value| {
            value
                .split(|character: char| {
                    character.is_whitespace() || matches!(character, '/' | '>')
                })
                .next()
        }) && is_reference(tag)
        {
            return Some((tag.to_owned(), false));
        }
        if value.contains("=>") || value.contains("import(") || value.starts_with("lazy(") {
            return Some((
                format!("opaque_route_handler_at_{}", node.start_byte()),
                true,
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
