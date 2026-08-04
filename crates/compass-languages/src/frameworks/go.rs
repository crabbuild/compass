use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{anchor, join_route_path, normalize_route_path, split_top_level, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

#[derive(Clone, Default)]
struct GoContext {
    prefixes: HashMap<String, String>,
    frameworks: HashMap<String, &'static str>,
}

pub(super) fn detect(path: &Path, source: &[u8], root: Node<'_>) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let evidence = EvidenceSet::new()
        .direct_if(
            body.contains("github.com/gin-gonic/gin"),
            "gin",
            EvidenceKind::Import,
            "github.com/gin-gonic/gin",
        )
        .direct_if(
            body.contains("github.com/go-chi/chi"),
            "chi",
            EvidenceKind::Import,
            "github.com/go-chi/chi",
        )
        .direct_if(
            body.contains("github.com/gorilla/mux"),
            "gorilla",
            EvidenceKind::Import,
            "github.com/gorilla/mux",
        );
    let framework = if evidence.activates("gin") {
        "gin"
    } else if evidence.activates("chi") {
        "chi"
    } else if evidence.activates("gorilla") {
        "gorilla"
    } else {
        return Vec::new();
    };
    let Ok(receiver_types) = Regex::new(
        r#"\b([A-Za-z_]\w*)\s+(?:\*\s*)?(gin\.Engine|chi\.Router|mux\.Router)\b|\b([A-Za-z_]\w*)\s*:?=\s*(?:gin\.(?:New|Default)|chi\.NewRouter|mux\.NewRouter)\s*\("#,
    ) else {
        return Vec::new();
    };
    let prefixes = HashMap::<String, String>::new();
    let mut receiver_frameworks = HashMap::<String, &'static str>::new();
    for capture in receiver_types.captures_iter(body) {
        let receiver = capture
            .get(1)
            .or_else(|| capture.get(3))
            .map(|value| value.as_str());
        let framework = capture
            .get(2)
            .and_then(|value| match value.as_str() {
                "gin.Engine" => Some("gin"),
                "chi.Router" => Some("chi"),
                "mux.Router" => Some("gorilla"),
                _ => None,
            })
            .or_else(|| {
                capture
                    .get(3)
                    .and_then(|_| body.get(capture.get(0)?.start()..capture.get(0)?.end()))
                    .and_then(|value| {
                        if value.contains("gin.") {
                            Some("gin")
                        } else if value.contains("chi.") {
                            Some("chi")
                        } else if value.contains("mux.") {
                            Some("gorilla")
                        } else {
                            None
                        }
                    })
            });
        if let (Some(receiver), Some(framework)) = (receiver, framework) {
            receiver_frameworks.insert(receiver.to_owned(), framework);
        }
    }
    let mut context = GoContext {
        prefixes,
        frameworks: receiver_frameworks,
    };
    collect_go_group_prefixes(root, source, &mut context);
    let mut facts = Vec::new();
    collect_go_route_calls(root, source, path, framework, &context, &mut facts);
    facts
}

fn collect_go_route_calls(
    node: Node<'_>,
    source: &[u8],
    path: &Path,
    default_framework: &str,
    context: &GoContext,
    facts: &mut Vec<RawFrameworkFact>,
) {
    let mut skip_children = HashSet::new();
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && let Some((_, method)) = selector_parts(function, source)
        && matches!(method.as_str(), "Route" | "Group")
        && let Some(arguments) = node.child_by_field_name("arguments")
    {
        let mut cursor = arguments.walk();
        let named = arguments.named_children(&mut cursor).collect::<Vec<_>>();
        let prefix = named
            .first()
            .and_then(|value| super::text::literal(node_text(*value, source)))
            .unwrap_or_default();
        if !prefix.is_empty()
            && method == "Route"
            && let Some(base) = function
                .child_by_field_name("operand")
                .and_then(|operand| expression_receiver_info(operand, source, context))
        {
            for closure in named
                .iter()
                .skip(1)
                .copied()
                .filter(|child| child.kind() == "func_literal")
            {
                let Some(parameter) = closure_parameter_name(closure, source) else {
                    continue;
                };
                let mut local = context.clone();
                local
                    .prefixes
                    .insert(parameter.clone(), join_route_path(&base.1, &prefix));
                local.frameworks.insert(parameter, "chi");
                if let Some(body) = closure.child_by_field_name("body") {
                    collect_go_route_calls(body, source, path, "chi", &local, facts);
                }
                skip_children.insert(closure.id());
            }
        }
    }
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && let Some((receiver, method, route_prefix)) =
            route_receiver_info(function, source, context)
        && matches_method(&method)
        && let Some(arguments) = node.child_by_field_name("arguments")
    {
        let mut cursor = arguments.walk();
        let arguments = arguments
            .named_children(&mut cursor)
            .map(|argument| node_text(argument, source).trim().to_owned())
            .collect::<Vec<_>>();
        let raw_path = arguments
            .first()
            .and_then(|value| super::text::literal(value));
        if let Some(raw_path) = raw_path
            && let Some((handler, middleware)) = arguments
                .iter()
                .skip(1)
                .map(|value| clean_reference(value))
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .split_last()
                .map(|(handler, middleware)| (handler.clone(), middleware.to_vec()))
        {
            let normalized_path = if route_prefix.is_empty() {
                normalize_route_path(&raw_path)
            } else {
                join_route_path(&route_prefix, &raw_path)
            };
            let operations = if method.eq_ignore_ascii_case("handlefunc") {
                method_operations(node, source).unwrap_or_else(|| {
                    if has_methods_parent(node, source) {
                        Vec::new()
                    } else {
                        vec!["ANY".to_owned()]
                    }
                })
            } else {
                vec![method.to_ascii_uppercase()]
            };
            let framework = context
                .frameworks
                .get(&receiver)
                .copied()
                .unwrap_or(default_framework);
            for operation in operations {
                facts.push(RawFrameworkFact::Route(RawRouteFact {
                    framework: framework.to_owned(),
                    operation,
                    raw_path: raw_path.clone(),
                    normalized_path: normalized_path.clone(),
                    declaring_scope: receiver.clone(),
                    anchor: anchor(path, source, node.start_byte(), node.end_byte()),
                    handler_reference: handler.clone(),
                    middleware_references: middleware.clone(),
                    origin: RawFrameworkOrigin::Ast,
                    rule: Some(format!("{framework}-router-call")),
                    detail: Map::from_iter([("receiver".into(), Value::String(receiver.clone()))]),
                }));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node
        .children(&mut cursor)
        .filter(|child| child.is_named() && !skip_children.contains(&child.id()))
    {
        collect_go_route_calls(child, source, path, default_framework, context, facts);
    }
}

fn collect_go_group_prefixes(node: Node<'_>, source: &[u8], context: &mut GoContext) {
    if matches!(
        node.kind(),
        "short_var_declaration" | "var_spec" | "assignment"
    ) && let Some(left) = node
        .child_by_field_name("left")
        .or_else(|| node.child_by_field_name("name"))
        && let Some(right) = node.child_by_field_name("right")
        && let Some(child) = first_named_child(left)
        && let Some(expression) = first_named_child(right)
        && let Some(call) = (expression.kind() == "call_expression").then_some(expression)
        && let Some(function) = call.child_by_field_name("function")
        && let Some((operand, method)) = selector_parts(function, source)
        && matches!(
            method.as_str(),
            "Group" | "Route" | "PathPrefix" | "Subrouter"
        )
        && let Some((parent, parent_prefix)) = expression_receiver_info(operand, source, context)
    {
        let mut arguments = call.child_by_field_name("arguments").into_iter();
        let prefix = arguments
            .next()
            .and_then(|arguments| {
                let mut cursor = arguments.walk();
                arguments
                    .named_children(&mut cursor)
                    .next()
                    .and_then(|value| super::text::literal(node_text(value, source)))
            })
            .unwrap_or_default();
        let child_name = node_text(child, source).trim();
        if !child_name.is_empty() {
            let child_prefix = if method == "Subrouter" || prefix.is_empty() {
                parent_prefix
            } else {
                join_route_path(&parent_prefix, &prefix)
            };
            context.prefixes.insert(child_name.to_owned(), child_prefix);
            if let Some(framework) = context.frameworks.get(&parent).copied() {
                context.frameworks.insert(child_name.to_owned(), framework);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_go_group_prefixes(child, source, context);
    }
}

fn first_named_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

fn selector_parts<'tree>(node: Node<'tree>, source: &[u8]) -> Option<(Node<'tree>, String)> {
    (node.kind() == "selector_expression").then(|| {
        let operand = node.child_by_field_name("operand")?;
        let field = node.child_by_field_name("field")?;
        Some((operand, node_text(field, source).to_owned()))
    })?
}

fn route_receiver_info(
    function: Node<'_>,
    source: &[u8],
    context: &GoContext,
) -> Option<(String, String, String)> {
    let (operand, method) = selector_parts(function, source)?;
    if !matches_method(&method) {
        return None;
    }
    let (receiver, prefix) = expression_receiver_info(operand, source, context)?;
    Some((receiver, method, prefix))
}

fn expression_receiver_info(
    expression: Node<'_>,
    source: &[u8],
    context: &GoContext,
) -> Option<(String, String)> {
    if expression.kind() == "identifier" {
        let receiver = node_text(expression, source).to_owned();
        return context.frameworks.contains_key(&receiver).then(|| {
            (
                receiver.clone(),
                context.prefixes.get(&receiver).cloned().unwrap_or_default(),
            )
        });
    }
    if expression.kind() != "call_expression" {
        return None;
    }
    let function = expression.child_by_field_name("function")?;
    let (operand, method) = selector_parts(function, source)?;
    let mut base = expression_receiver_info(operand, source, context)?;
    let arguments = expression.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let first = arguments
        .named_children(&mut cursor)
        .next()
        .and_then(|value| super::text::literal(node_text(value, source)));
    if matches!(method.as_str(), "PathPrefix" | "Group" | "Route")
        && let Some(prefix) = first
    {
        base.1 = join_route_path(&base.1, &prefix);
    }
    Some(base)
}

fn closure_parameter_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let parameters = node.child_by_field_name("parameters")?;
    let mut cursor = parameters.walk();
    parameters.named_children(&mut cursor).find_map(|child| {
        child
            .child_by_field_name("name")
            .or_else(|| (child.kind() == "identifier").then_some(child))
            .map(|name| node_text(name, source).to_owned())
    })
}

fn matches_method(method: &str) -> bool {
    matches!(
        method,
        "GET"
            | "POST"
            | "PUT"
            | "PATCH"
            | "DELETE"
            | "OPTIONS"
            | "HEAD"
            | "Get"
            | "Post"
            | "Put"
            | "Patch"
            | "Delete"
            | "Options"
            | "Head"
            | "HandleFunc"
    )
}

fn method_operations(node: Node<'_>, source: &[u8]) -> Option<Vec<String>> {
    let mut parent = node.parent()?;
    let parent = loop {
        if parent.kind() == "call_expression"
            && parent
                .child_by_field_name("function")
                .is_some_and(|function| node_text(function, source).trim().ends_with(".Methods"))
        {
            break parent;
        }
        parent = parent.parent()?;
    };
    let arguments = parent.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let operations = arguments
        .named_children(&mut cursor)
        .flat_map(|argument| split_top_level(node_text(argument, source)))
        .filter_map(|value| {
            super::text::literal(value)
                .or_else(|| value.trim().strip_prefix("http.Method").map(str::to_owned))
        })
        .map(|value| value.to_ascii_uppercase())
        .collect::<Vec<_>>();
    (!operations.is_empty()).then_some(operations)
}

fn has_methods_parent(node: Node<'_>, source: &[u8]) -> bool {
    let mut parent = node.parent();
    while let Some(candidate) = parent {
        if candidate.kind() == "call_expression"
            && candidate
                .child_by_field_name("function")
                .is_some_and(|function| node_text(function, source).trim().ends_with(".Methods"))
        {
            return true;
        }
        parent = candidate.parent();
    }
    false
}

fn clean_reference(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('&')
        .trim_end_matches("...")
        .trim()
        .to_owned()
}

fn node_text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(
        &source[node.start_byte().min(source.len())..node.end_byte().min(source.len())],
    )
    .unwrap_or_default()
}
