use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{anchor, join_route_path, literal, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

const HTTP_METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "options", "head"];

pub(super) fn detect(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let evidence = EvidenceSet::new().direct_if(
        body.contains("import Vapor"),
        "vapor",
        EvidenceKind::Import,
        "Vapor",
    );
    if !evidence.activates("vapor") {
        return Vec::new();
    }
    let mut prefixes = HashMap::new();
    collect_vapor_bindings(root, source, &mut prefixes);
    let mut facts = Vec::new();
    collect_vapor_calls(root, source, path, &prefixes, &mut facts);
    facts
}

fn collect_vapor_bindings(node: Node<'_>, source: &[u8], prefixes: &mut HashMap<String, String>) {
    if node.kind() == "property_declaration"
        && let Some(name) = node
            .child_by_field_name("name")
            .and_then(|name| name.child_by_field_name("bound_identifier"))
        && let Some(value) = node.child_by_field_name("value")
        && value.kind() == "call_expression"
        && let Some((navigation, arguments, _)) = call_parts(value)
        && navigation_method(navigation, source).is_some_and(|method| method == "grouped")
        && let Some(target) = navigation.child_by_field_name("target")
        && let Some((receiver, parent_prefix)) = vapor_target(target, source, prefixes)
        && let Some(prefix) = first_argument_literal(arguments, source)
    {
        let name = node_text(name, source).to_owned();
        let _ = receiver;
        prefixes.insert(name, join_route_path(&parent_prefix, &prefix));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_vapor_bindings(child, source, prefixes);
    }
}

fn collect_vapor_calls(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    prefixes: &HashMap<String, String>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let mut skip_children = HashSet::new();
    if node.kind() == "call_expression"
        && let Some((navigation, arguments, lambda)) = call_parts(node)
        && let Some(method) = navigation_method(navigation, source)
    {
        if method == "group"
            && let Some(lambda) = lambda
            && let Some(prefix) = first_argument_literal(arguments, source)
            && let Some(target) = navigation.child_by_field_name("target")
            && let Some((_, parent_prefix)) = vapor_target(target, source, prefixes)
            && let Some(parameter) = lambda_parameter_name(lambda, source)
        {
            let mut local = prefixes.clone();
            local.insert(parameter, join_route_path(&parent_prefix, &prefix));
            if let Some(body) = lambda_statements(lambda) {
                collect_vapor_calls(body, source, path, &local, facts);
            }
            skip_children.insert(lambda.id());
        }
        if (HTTP_METHODS.contains(&method.as_str()) || method == "on")
            && let Some(target) = navigation.child_by_field_name("target")
            && let Some((receiver, prefix)) = vapor_target(target, source, prefixes)
        {
            collect_vapor_route(
                node, &method, arguments, lambda, receiver, &prefix, source, path, facts,
            );
        }
    }
    let mut cursor = node.walk();
    for child in node
        .children(&mut cursor)
        .filter(|child| child.is_named() && !skip_children.contains(&child.id()))
    {
        collect_vapor_calls(child, source, path, prefixes, facts);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_vapor_route(
    node: Node<'_>,
    method: &str,
    arguments: Node<'_>,
    lambda: Option<Node<'_>>,
    receiver: String,
    prefix: &str,
    source: &[u8],
    path: &Path,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let values = value_arguments(arguments);
    let mut start = 0_usize;
    let operation = if method == "on" {
        let Some(first) = values
            .first()
            .and_then(|value| value.child_by_field_name("value"))
        else {
            return;
        };
        let operation = node_text(first, source)
            .trim()
            .strip_prefix('.')
            .unwrap_or_else(|| node_text(first, source).trim())
            .to_ascii_uppercase();
        start = 1;
        operation
    } else {
        method.to_ascii_uppercase()
    };
    let mut path_segments = Vec::new();
    let mut handler = None;
    for value in values.iter().skip(start) {
        if value
            .child_by_field_name("name")
            .is_some_and(|name| node_text(name, source).trim() == "use")
        {
            handler = value.child_by_field_name("value");
            continue;
        }
        if let Some(value_node) = value.child_by_field_name("value") {
            if value_node.kind() == "lambda_literal" {
                handler = Some(value_node);
            } else if let Some(segment) = vapor_segment(value_node, source) {
                path_segments.push(segment);
            }
        }
    }
    let raw_path = if path_segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", path_segments.join("/"))
    };
    let opaque_handler = handler.is_none() && lambda.is_some()
        || handler.is_some_and(|handler| handler.kind() == "lambda_literal");
    let anchor = anchor(path, source, node.start_byte(), node.end_byte());
    let handler_reference = if opaque_handler {
        format!("opaque_closure_at_line_{}", anchor.start_line)
    } else if let Some(handler) = handler {
        node_text(handler, source).trim().to_owned()
    } else {
        return;
    };
    if handler_reference.is_empty() {
        return;
    }
    facts.push(RawFrameworkFact::Route(RawRouteFact {
        framework: "vapor".to_owned(),
        operation,
        raw_path: raw_path.clone(),
        normalized_path: join_route_path(prefix, &raw_path),
        declaring_scope: receiver,
        anchor,
        handler_reference,
        middleware_references: Vec::new(),
        origin: RawFrameworkOrigin::Ast,
        rule: Some("vapor-route-call".to_owned()),
        detail: if opaque_handler {
            Map::from_iter([("opaque_handler".into(), Value::Bool(true))])
        } else {
            Map::new()
        },
    }));
}

fn call_parts(node: Node<'_>) -> Option<(Node<'_>, Node<'_>, Option<Node<'_>>)> {
    let mut cursor = node.walk();
    let mut named = node.named_children(&mut cursor);
    let navigation = named.find(|child| child.kind() == "navigation_expression")?;
    let suffix = named.find(|child| child.kind() == "call_suffix")?;
    let mut suffix_cursor = suffix.walk();
    let mut suffix_children = suffix.named_children(&mut suffix_cursor);
    let arguments = suffix_children.find(|child| child.kind() == "value_arguments")?;
    let lambda = suffix_children.find(|child| child.kind() == "lambda_literal");
    Some((navigation, arguments, lambda))
}

fn navigation_method(navigation: Node<'_>, source: &[u8]) -> Option<String> {
    navigation
        .child_by_field_name("suffix")
        .and_then(|suffix| suffix.child_by_field_name("suffix"))
        .map(|suffix| node_text(suffix, source).to_owned())
}

fn vapor_target(
    target: Node<'_>,
    source: &[u8],
    prefixes: &HashMap<String, String>,
) -> Option<(String, String)> {
    if target.kind() == "simple_identifier" {
        let receiver = node_text(target, source).to_owned();
        return Some((
            receiver.clone(),
            prefixes.get(&receiver).cloned().unwrap_or_default(),
        ));
    }
    if target.kind() != "call_expression" {
        return None;
    }
    let (navigation, arguments, _) = call_parts(target)?;
    let method = navigation_method(navigation, source)?;
    let base_target = navigation.child_by_field_name("target")?;
    let (receiver, mut prefix) = vapor_target(base_target, source, prefixes)?;
    if matches!(method.as_str(), "grouped" | "group")
        && let Some(segment) = first_argument_literal(arguments, source)
    {
        prefix = join_route_path(&prefix, &segment);
    }
    Some((receiver, prefix))
}

fn first_argument_literal(arguments: Node<'_>, source: &[u8]) -> Option<String> {
    value_arguments(arguments)
        .first()
        .and_then(|value| value.child_by_field_name("value"))
        .and_then(|value| literal(node_text(value, source)))
}

fn value_arguments(arguments: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = arguments.walk();
    arguments
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "value_argument")
        .collect()
}

fn vapor_segment(value: Node<'_>, source: &[u8]) -> Option<String> {
    let value = node_text(value, source).trim();
    if let Some(value) = literal(value) {
        return Some(value);
    }
    value
        .strip_prefix('.')
        .filter(|value| {
            value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
        .map(|value| format!(":{value}"))
}

fn lambda_parameter_name(lambda: Node<'_>, source: &[u8]) -> Option<String> {
    let parameters = lambda.child_by_field_name("type")?;
    let parameter = find_descendant(parameters, "lambda_parameter")?;
    parameter
        .child_by_field_name("name")
        .map(|name| node_text(name, source).to_owned())
}

fn lambda_statements(lambda: Node<'_>) -> Option<Node<'_>> {
    find_descendant(lambda, "statements")
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
    std::str::from_utf8(
        &source[node.start_byte().min(source.len())..node.end_byte().min(source.len())],
    )
    .unwrap_or_default()
}
