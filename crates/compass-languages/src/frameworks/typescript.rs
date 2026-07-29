use std::collections::HashSet;
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
};
use crate::{Extraction, RawNodeRecord, make_id};

const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "options", "head", "all",
];

pub(super) fn detect(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    if source.is_empty() {
        return Vec::new();
    }
    let text = std::str::from_utf8(source).unwrap_or_default();
    let mut facts = Vec::new();
    let receivers = express_receivers(root, source);
    if !receivers.is_empty() {
        collect_express_routes(root, source, path, &receivers, &mut facts);
    }
    if text.contains("@nestjs/") {
        collect_nest_routes(root, source, path, &mut facts);
    }
    if text.contains("react-router") {
        collect_react_router_routes(root, source, path, &mut facts);
    }
    if text.contains("vue-router") || text.contains("createRouter") {
        collect_vue_router_routes(root, source, path, &mut facts);
    }
    facts
}

pub(super) fn attach_import_aliases(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) {
    attach_default_export_identities(path, source, root, extraction);
    let mut aliases = Vec::new();
    collect_import_aliases(root, source, &mut aliases);
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
    let mut receivers = HashSet::new();
    collect_express_receivers(root, source, &mut receivers);
    receivers
}

fn collect_express_receivers(node: Node<'_>, source: &[u8], receivers: &mut HashSet<String>) {
    if node.kind() == "variable_declarator"
        && let (Some(name), Some(value)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("value"),
        )
    {
        let variable = node_text(name, source).trim();
        let expression = node_text(value, source).trim();
        if is_identifier(variable)
            && matches!(
                expression,
                "express()" | "Router()" | "express.Router()" | "router()"
            )
        {
            receivers.insert(variable.to_owned());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_express_receivers(child, source, receivers);
    }
}

fn collect_express_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    receivers: &HashSet<String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
    {
        let function = node_text(function, source).trim();
        if let Some((receiver, method)) = function.rsplit_once('.')
            && receivers.contains(receiver)
            && HTTP_METHODS.contains(&method)
            && let Some(arguments) = node.child_by_field_name("arguments")
        {
            let arguments = split_arguments(node_text(arguments, source));
            if let Some(raw_path) = arguments
                .first()
                .and_then(|argument| string_literal(argument))
            {
                let stages = arguments
                    .iter()
                    .skip(1)
                    .filter_map(|argument| handler_reference(argument))
                    .collect::<Vec<_>>();
                if let Some((handler, middleware)) = stages.split_last() {
                    facts.push(RawFrameworkFact::Route(RawRouteFact {
                        framework: "express".to_owned(),
                        operation: if method == "all" {
                            "ANY".to_owned()
                        } else {
                            method.to_ascii_uppercase()
                        },
                        raw_path: raw_path.clone(),
                        normalized_path: normalize_path(&raw_path),
                        declaring_scope: module_scope(path),
                        anchor: anchor(path, node),
                        handler_reference: handler.clone(),
                        middleware_references: middleware.to_vec(),
                        origin: RawFrameworkOrigin::Ast,
                        rule: None,
                        detail: Map::from_iter([(
                            "receiver".into(),
                            Value::String(receiver.to_owned()),
                        )]),
                    }));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_express_routes(child, source, path, receivers, facts);
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
    let controller_prefix = decorator_argument(&class_text, "Controller");
    let is_resolver = has_decorator(&class_text, "Resolver");
    if controller_prefix.is_none() && !is_resolver {
        return;
    }
    collect_nest_methods(
        class,
        source,
        path,
        class_name,
        controller_prefix.as_deref().unwrap_or_default(),
        is_resolver,
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
                let local = decorator_argument(&method_text, decorator).unwrap_or_default();
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
            let local = decorator_argument(&method_text, "RequestMapping").unwrap_or_default();
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
                    let field = decorator_argument(&method_text, decorator)
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| method_name.to_owned());
                    facts.push(route_fact(
                        "nestjs-graphql",
                        operation,
                        &format!("/graphql/{field}"),
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
            if has_decorator(&method_text, decorator) {
                let subject = decorator_argument(&method_text, decorator)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| method_name.to_owned());
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
            facts,
        );
    }
}

fn collect_react_router_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if matches!(node.kind(), "jsx_self_closing_element" | "jsx_element") {
        let text = node_text(node, source);
        if text.trim_start().starts_with("<Route")
            && let Some(raw_path) = jsx_string_attribute(text, "path")
            && let Some(handler) = jsx_component_attribute(text, "element")
        {
            facts.push(route_fact(
                "react-router",
                "PAGE",
                &raw_path,
                &handler,
                path,
                node,
                Map::new(),
            ));
        }
    }
    collect_route_config(node, source, path, "react-router", facts);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_react_router_routes(child, source, path, facts);
    }
}

fn collect_vue_router_routes(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    facts: &mut Vec<RawFrameworkFact>,
) {
    collect_route_config(node, source, path, "vue-router", facts);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_vue_router_routes(child, source, path, facts);
    }
}

fn collect_route_config(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    framework: &str,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if !matches!(node.kind(), "object" | "object_pattern") {
        return;
    }
    let text = node_text(node, source);
    let Some(raw_path) = object_string_property(text, "path") else {
        return;
    };
    let handler = object_identifier_property(text, "component")
        .or_else(|| object_identifier_property(text, "element"))
        .or_else(|| object_identifier_property(text, "Component"));
    let Some(handler) = handler else {
        return;
    };
    let middleware = ["loader", "action"]
        .into_iter()
        .filter_map(|property| object_identifier_property(text, property))
        .collect();
    let mut fact = match route_fact(
        framework,
        "PAGE",
        &raw_path,
        &handler,
        path,
        node,
        Map::new(),
    ) {
        RawFrameworkFact::Route(route) => route,
        RawFrameworkFact::Domain(_) => unreachable!(),
    };
    fact.middleware_references = middleware;
    facts.push(RawFrameworkFact::Route(fact));
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
    let pattern = Regex::new(&format!(
        r#"(?s)@\s*{}\s*\(\s*["'`]([^"'`]*?)["'`]"#,
        regex::escape(name)
    ))
    .ok()?;
    pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
        .or_else(|| has_decorator(source, name).then(String::new))
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

fn object_string_property(source: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r#"(?m)(?:^|[,{{]\s*){}\s*:\s*["']([^"']+)["']"#,
        regex::escape(name)
    ))
    .ok()?;
    pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
}

fn object_identifier_property(source: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r"(?m)(?:^|[,\{{]\s*){}\s*:\s*([A-Za-z_$][A-Za-z0-9_$.]*)",
        regex::escape(name)
    ))
    .ok()?;
    pattern
        .captures(source)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
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
