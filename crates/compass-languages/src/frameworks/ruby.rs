use serde_json::Map;
use tree_sitter::Node;

use crate::SemanticRole;

use super::text::{join_route_path, literal, normalize_route_path};
use super::{
    RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
    UniversalDetectionContext,
};

/// Universal Rails routing detector.  It consumes only AST calls that have a
/// matching Ruby universal occurrence; source text is used for literal
/// argument values and never as a line-oriented semantic authority.
pub(super) fn detect_universal(
    context: &UniversalDetectionContext<'_, '_>,
) -> Vec<RawFrameworkFact> {
    if context.evidence.pipeline.language != "ruby" {
        return Vec::new();
    }
    let source_file = context
        .evidence
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "file")
        .map(|declaration| declaration.range.source_file.clone())
        .unwrap_or_default();
    if source_file.is_empty() {
        return Vec::new();
    }
    let call_occurrences = context
        .evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == SemanticRole::Call)
        .map(|occurrence| (occurrence.range.start_byte, occurrence.spelling.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut calls = Vec::new();
    collect_call_nodes(context.root, &mut calls);
    let mut routes = Vec::new();
    for call in calls {
        let Some(method_node) = call.child_by_field_name("method") else {
            continue;
        };
        if method_name(call, context.source).as_deref() != Some("draw")
            || !call_occurrences.contains(&(method_node.start_byte() as u64, "draw"))
            || receiver_text(call, context.source).as_deref() != Some("Rails.application.routes")
        {
            continue;
        }
        let Some(block) = call.child_by_field_name("block") else {
            continue;
        };
        collect_routes(
            block,
            context,
            &source_file,
            String::new(),
            Vec::new(),
            &call_occurrences,
            &mut routes,
        );
    }
    routes.sort_by_key(route_key);
    routes
}

fn collect_call_nodes<'tree>(node: Node<'tree>, calls: &mut Vec<Node<'tree>>) {
    if node.kind() == "call" {
        calls.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_call_nodes(child, calls);
    }
}

fn collect_routes(
    block: Node<'_>,
    context: &UniversalDetectionContext<'_, '_>,
    source_file: &str,
    prefix: String,
    namespaces: Vec<String>,
    call_occurrences: &std::collections::BTreeSet<(u64, &str)>,
    routes: &mut Vec<RawFrameworkFact>,
) {
    let nested_calls = direct_body_calls(block);
    for call in nested_calls {
        let Some(method_node) = call.child_by_field_name("method") else {
            continue;
        };
        let Some(method) = method_name(call, context.source) else {
            continue;
        };
        if !call_occurrences.contains(&(method_node.start_byte() as u64, method.as_str())) {
            continue;
        }
        if matches!(method.as_str(), "namespace" | "scope")
            && let Some(nested_block) = call.child_by_field_name("block")
            && let Some((nested_prefix, nested_namespaces)) =
                route_scope_arguments(call, context.source, &method, &prefix, &namespaces)
        {
            collect_routes(
                nested_block,
                context,
                source_file,
                nested_prefix,
                nested_namespaces,
                call_occurrences,
                routes,
            );
            continue;
        }
        if !matches!(
            method.as_str(),
            "get" | "post" | "put" | "patch" | "delete" | "options" | "head" | "match"
        ) {
            continue;
        }
        let Some((raw_path, handler, operations)) = route_arguments(call, context.source, &method)
        else {
            continue;
        };
        let normalized_path = if prefix.is_empty() {
            normalize_route_path(&raw_path)
        } else {
            join_route_path(&prefix, &raw_path)
        };
        let handler_reference = rails_handler(&handler, &namespaces);
        let occurrence = context.evidence.occurrences.iter().find(|occurrence| {
            occurrence.role == SemanticRole::Call
                && occurrence.range.start_byte == method_node.start_byte() as u64
                && occurrence.spelling == method
        });
        let Some(occurrence) = occurrence else {
            continue;
        };
        let anchor = evidence_anchor(&occurrence.range);
        for operation in operations {
            routes.push(RawFrameworkFact::Route(RawRouteFact {
                framework: "rails".to_owned(),
                operation,
                raw_path: raw_path.clone(),
                normalized_path: normalized_path.clone(),
                declaring_scope: source_file.to_owned(),
                anchor: anchor.clone(),
                handler_reference: handler_reference.clone(),
                middleware_references: Vec::new(),
                stages: Vec::new(),
                origin: RawFrameworkOrigin::Ast,
                rule: Some("rails-routes-dsl".to_owned()),
                detail: Map::from_iter([(
                    "frameworkPack".to_owned(),
                    serde_json::Value::String("rails-ruby".to_owned()),
                )]),
            }));
        }
    }
}

fn direct_body_calls<'tree>(block: Node<'tree>) -> Vec<Node<'tree>> {
    let Some(body) = block.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut cursor = body.walk();
    body.children(&mut cursor)
        .filter(Node::is_named)
        .filter(|node| node.kind() == "call")
        .collect()
}

fn route_scope_arguments(
    call: Node<'_>,
    source: &[u8],
    method: &str,
    prefix: &str,
    namespaces: &[String],
) -> Option<(String, Vec<String>)> {
    let argument = first_positional_argument(call)?;
    let value = literal_node(argument, source)?;
    let mut next_prefix = prefix.to_owned();
    let mut next_namespaces = namespaces.to_vec();
    if !next_prefix.is_empty() {
        next_prefix.push('/');
    }
    if method == "namespace" {
        next_prefix.push_str(&value);
        next_namespaces.push(camelize(&value));
    } else {
        next_prefix.push_str(value.trim_matches('/'));
    }
    Some((next_prefix, next_namespaces))
}

fn route_arguments(
    call: Node<'_>,
    source: &[u8],
    method: &str,
) -> Option<(String, String, Vec<String>)> {
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let values = arguments
        .children(&mut cursor)
        .filter(Node::is_named)
        .collect::<Vec<_>>();
    let path = values
        .iter()
        .find(|node| node.kind() != "pair")
        .and_then(|node| literal_node(*node, source))?;
    let handler = values
        .iter()
        .find_map(|node| {
            (*node).child_by_field_name("key").and_then(|key| {
                (key_text(key, source).as_deref() == Some("to"))
                    .then(|| (*node).child_by_field_name("value"))
                    .flatten()
                    .and_then(|value| literal_node(value, source))
            })
        })
        .or_else(|| {
            values
                .iter()
                .filter(|node| node.kind() != "pair")
                .nth(1)
                .and_then(|node| literal_node(*node, source))
        })?;
    let operations = if method == "match" {
        values
            .iter()
            .find_map(|node| {
                (*node).child_by_field_name("key").and_then(|key| {
                    (key_text(key, source).as_deref() == Some("via"))
                        .then(|| (*node).child_by_field_name("value"))
                        .flatten()
                })
            })
            .map(|node| literal_array(node, source))
            .filter(|operations| !operations.is_empty())
            .unwrap_or_else(|| vec!["ANY".to_owned()])
    } else {
        vec![method.to_ascii_uppercase()]
    };
    Some((path, handler, operations))
}

fn first_positional_argument(call: Node<'_>) -> Option<Node<'_>> {
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    arguments
        .children(&mut cursor)
        .filter(Node::is_named)
        .find(|node| node.kind() != "pair")
}

fn literal_node(node: Node<'_>, source: &[u8]) -> Option<String> {
    let raw = source.get(node.start_byte()..node.end_byte())?;
    let raw = std::str::from_utf8(raw).ok()?.trim();
    if node.kind() == "simple_symbol" {
        return raw.strip_prefix(':').map(str::to_owned);
    }
    literal(raw)
}

fn literal_array(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(Node::is_named)
        .filter_map(|child| literal_node(child, source))
        .map(|value| value.to_ascii_uppercase())
        .collect()
}

fn key_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    let raw = source.get(node.start_byte()..node.end_byte())?;
    let raw = std::str::from_utf8(raw).ok()?.trim();
    Some(
        raw.trim_start_matches(':')
            .trim_matches(['"', '\''])
            .to_owned(),
    )
}

fn method_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let method = node.child_by_field_name("method")?;
    let value = source.get(method.start_byte()..method.end_byte())?;
    std::str::from_utf8(value).ok().map(str::to_owned)
}

fn receiver_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let value = source.get(receiver.start_byte()..receiver.end_byte())?;
    std::str::from_utf8(value).ok().map(str::to_owned)
}

fn evidence_anchor(range: &crate::EvidenceRange) -> RawFrameworkAnchor {
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

fn route_key(fact: &RawFrameworkFact) -> (String, String, u64, String) {
    let RawFrameworkFact::Route(route) = fact else {
        return (String::new(), String::new(), 0, String::new());
    };
    (
        route.anchor.source_file.clone(),
        route.normalized_path.clone(),
        route.anchor.start_byte,
        route.operation.clone(),
    )
}

fn rails_handler(value: &str, namespaces: &[String]) -> String {
    let Some((controller, action)) = value.split_once('#') else {
        return value.to_owned();
    };
    let mut controller_parts = controller.split('/').collect::<Vec<_>>();
    let controller = controller_parts.pop().unwrap_or(controller);
    let owners = if controller_parts.is_empty() {
        namespaces
            .iter()
            .map(String::as_str)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
    } else {
        controller_parts
    };
    owners
        .into_iter()
        .map(camelize)
        .chain(std::iter::once(format!(
            "{}Controller",
            camelize(controller)
        )))
        .chain(std::iter::once(action.to_owned()))
        .collect::<Vec<_>>()
        .join(".")
}

fn camelize(value: &str) -> String {
    value
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + characters.as_str()
            })
        })
        .collect()
}
