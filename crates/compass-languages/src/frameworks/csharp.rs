//! ASP.NET MVC framework evidence derived from universal C# facts.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use crate::SemanticRole;

use super::text::{anchor as source_anchor, join_route_path, text};
use super::{
    DetectionContext, RawFrameworkAnchor, RawFrameworkAnnotationFact, RawFrameworkFact,
    RawFrameworkOrigin, RawRouteFact, UniversalDetectionContext,
};

const PACK_ID: &str = "aspnet-csharp";
const FRAMEWORK: &str = "aspnet";
const MVC_NAMESPACE: &str = "Microsoft.AspNetCore.Mvc";

pub(super) fn detect(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    if context.evidence.pipeline.language != "csharp" {
        return Vec::new();
    }
    let activated = context.project.is_some_and(|project| {
        project.has_any_dependency(super::pack::ASPNET_CSHARP_DESCRIPTOR.dependency_markers)
    }) || context.evidence.bindings.iter().any(|binding| {
        binding.qualified_target == MVC_NAMESPACE
            || binding
                .qualified_target
                .starts_with(&format!("{MVC_NAMESPACE}."))
    });
    if !activated {
        return Vec::new();
    }
    let declarations = context
        .evidence
        .declarations
        .iter()
        .map(|declaration| (declaration.id.as_str(), declaration))
        .collect::<HashMap<_, _>>();
    let candidates = context
        .evidence
        .candidates
        .iter()
        .filter_map(|candidate| {
            candidate
                .occurrence_id
                .as_deref()
                .map(|occurrence| (occurrence, candidate))
        })
        .collect::<HashMap<_, _>>();
    let mut attributes = BTreeMap::new();
    collect_attributes(context.root, &mut attributes);
    let unique_bindings = unique_binding_map(context);
    let mut facts = context
        .evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.role == SemanticRole::Annotation)
        .filter_map(|occurrence| {
            let declaration = declarations.get(occurrence.owner_declaration_id.as_str())?;
            let attribute = attributes.get(&occurrence.range.start_byte).copied();
            let candidate = candidates.get(occurrence.id.as_str());
            let qualified = qualified_attribute(context, &occurrence.spelling).or_else(|| {
                candidate.and_then(|candidate| candidate.constraints.qualified_name.clone())
            });
            if !is_mvc_attribute(&occurrence.spelling, qualified.as_deref()) {
                return None;
            }
            let mut detail = Map::from_iter([(
                "occurrenceId".to_owned(),
                Value::String(occurrence.id.clone()),
            )]);
            if !unique_bindings.is_empty() {
                detail.insert(
                    "bindings".to_owned(),
                    Value::Object(unique_bindings.clone()),
                );
            }
            Some(RawFrameworkFact::Annotation(RawFrameworkAnnotationFact {
                pack_id: PACK_ID.to_owned(),
                framework: FRAMEWORK.to_owned(),
                annotation_name: qualified
                    .as_deref()
                    .map(terminal_attribute)
                    .unwrap_or_else(|| terminal_attribute(&occurrence.spelling))
                    .trim_end_matches("Attribute")
                    .to_owned(),
                annotation_qualified_name: qualified,
                owner_declaration_id: declaration.id.clone(),
                owner_graph_node_id: declaration.graph_node_id.clone(),
                owner_qualified_name: declaration.qualified_name.clone(),
                owner_kind: declaration.kind.clone(),
                owner_signature: declaration.signature.clone(),
                anchor: anchor(&occurrence.range),
                arguments: attribute
                    .map(|node| attribute_arguments(node, context.source))
                    .unwrap_or_default(),
                detail,
            }))
        })
        .collect::<Vec<_>>();
    facts.sort_by(|left, right| fact_key(left).cmp(&fact_key(right)));
    facts
}

/// Preserve ASP.NET Minimal API registrations while MVC routing moves to the
/// universal evidence pack. Minimal API receivers and inline handlers do not
/// yet have a universal C# relationship contract, so they remain behind the
/// established bounded, source-anchored detector.
pub(super) fn detect_minimal(
    context: &DetectionContext<'_, '_>,
    _extraction: &mut crate::Extraction,
) -> Vec<RawFrameworkFact> {
    let mut masked = context.source.to_vec();
    mask_comments(context.root, &mut masked);
    let body = text(&masked);
    if !body.contains("WebApplication.CreateBuilder") || !body.contains(".Map") {
        return Vec::new();
    }
    collect_minimal_api_routes(context, body)
}

fn mask_comments(node: Node<'_>, source: &mut [u8]) {
    if node.kind() == "comment" {
        for byte in source
            .get_mut(node.start_byte()..node.end_byte())
            .into_iter()
            .flatten()
            .filter(|byte| **byte != b'\n' && **byte != b'\r')
        {
            *byte = b' ';
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        mask_comments(child, source);
    }
}

fn collect_minimal_api_routes(
    context: &DetectionContext<'_, '_>,
    body: &str,
) -> Vec<RawFrameworkFact> {
    let Ok(group_pattern) = Regex::new(
        r#"(?m)\b(?:var|RouteGroupBuilder)\s+([A-Za-z_]\w*)\s*=\s*([A-Za-z_]\w*)\.MapGroup\(\s*"([^"]*)"\s*\)"#,
    ) else {
        return Vec::new();
    };
    let mut prefixes = HashMap::<String, String>::new();
    for capture in group_pattern.captures_iter(body) {
        let (Some(child), Some(parent), Some(prefix)) =
            (capture.get(1), capture.get(2), capture.get(3))
        else {
            continue;
        };
        let parent_prefix = prefixes
            .get(parent.as_str())
            .map(String::as_str)
            .unwrap_or_default();
        prefixes.insert(
            child.as_str().to_owned(),
            join_route_path(parent_prefix, prefix.as_str()),
        );
    }

    let Ok(route_pattern) =
        Regex::new(r"\b([A-Za-z_]\w*)\.Map(Get|Post|Put|Patch|Delete|Options|Head|Methods)\s*\(")
    else {
        return Vec::new();
    };
    let Ok(reference_pattern) = Regex::new(r"^[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*$") else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    for capture in route_pattern.captures_iter(body) {
        let (Some(start), Some(receiver), Some(operation)) =
            (capture.get(0), capture.get(1), capture.get(2))
        else {
            continue;
        };
        let Some(end) = balanced_call_end(body, start.end().saturating_sub(1)) else {
            continue;
        };
        let arguments = split_top_level_arguments(&body[start.end()..end.saturating_sub(1)]);
        let Some(raw_path) = arguments
            .first()
            .and_then(|argument| string_literal(argument))
        else {
            continue;
        };
        let (operations, handler_index) = if operation.as_str() == "Methods" {
            let Some(methods) = arguments.get(1) else {
                continue;
            };
            let methods = quoted_literals(methods);
            if methods.is_empty() {
                continue;
            }
            (methods, 2)
        } else {
            (vec![operation.as_str().to_ascii_uppercase()], 1)
        };
        let Some(handler) = arguments.get(handler_index).map(|handler| handler.trim()) else {
            continue;
        };
        let (handler_reference, detail) = if reference_pattern.is_match(handler) {
            (handler.to_owned(), Map::new())
        } else if handler.contains("=>") {
            (
                format!("lambda_handler_at_{}", start.start()),
                Map::from_iter([("handler_kind".into(), Value::String("lambda".into()))]),
            )
        } else {
            (
                format!("opaque_minimal_handler_at_{}", start.start()),
                Map::from_iter([("opaque_handler".into(), Value::Bool(true))]),
            )
        };
        let receiver_prefix = prefixes
            .get(receiver.as_str())
            .map(String::as_str)
            .unwrap_or_default();
        for operation in operations {
            facts.push(RawFrameworkFact::Route(RawRouteFact {
                framework: FRAMEWORK.to_owned(),
                operation,
                raw_path: raw_path.clone(),
                normalized_path: join_route_path(receiver_prefix, &raw_path),
                declaring_scope: receiver.as_str().to_owned(),
                anchor: source_anchor(context.path, context.source, start.start(), end),
                handler_reference: handler_reference.clone(),
                middleware_references: Vec::new(),
                origin: RawFrameworkOrigin::Ast,
                rule: None,
                detail: detail.clone(),
            }));
        }
    }
    facts
}

fn balanced_call_end(body: &str, open: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    (bytes.get(open) == Some(&b'(')).then_some(())?;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (offset, byte) in bytes.get(open..)?.iter().copied().enumerate() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'(' => depth = depth.saturating_add(1),
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open.saturating_add(offset).saturating_add(1));
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_arguments(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut output = Vec::new();
    let mut start = 0_usize;
    let mut depths = [0_u32; 4];
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'(' => depths[0] = depths[0].saturating_add(1),
            b')' => depths[0] = depths[0].saturating_sub(1),
            b'[' => depths[1] = depths[1].saturating_add(1),
            b']' => depths[1] = depths[1].saturating_sub(1),
            b'{' => depths[2] = depths[2].saturating_add(1),
            b'}' => depths[2] = depths[2].saturating_sub(1),
            b'<' => depths[3] = depths[3].saturating_add(1),
            b'>' => depths[3] = depths[3].saturating_sub(1),
            b',' if depths.iter().all(|depth| *depth == 0) => {
                output.push(body[start..index].trim());
                start = index.saturating_add(1);
            }
            _ => {}
        }
    }
    output.push(body[start..].trim());
    output
}

fn quoted_literals(value: &str) -> Vec<String> {
    let Ok(pattern) = Regex::new(r#""([^"]+)""#) else {
        return Vec::new();
    };
    pattern
        .captures_iter(value)
        .filter_map(|capture| {
            capture
                .get(1)
                .map(|value| value.as_str().to_ascii_uppercase())
        })
        .collect()
}

fn qualified_attribute(
    context: &UniversalDetectionContext<'_, '_>,
    spelling: &str,
) -> Option<String> {
    let terminal = terminal_attribute(spelling);
    if spelling.contains('.') {
        return Some(ensure_attribute_suffix(spelling));
    }
    for binding in &context.evidence.bindings {
        if binding.spelling == terminal && binding.qualified_target != MVC_NAMESPACE {
            return Some(ensure_attribute_suffix(&binding.qualified_target));
        }
    }
    let namespaces = context
        .evidence
        .bindings
        .iter()
        .filter(|binding| binding.qualified_target == MVC_NAMESPACE)
        .map(|binding| binding.qualified_target.as_str())
        .collect::<BTreeSet<_>>();
    (namespaces.len() == 1).then(|| {
        format!(
            "{MVC_NAMESPACE}.{}Attribute",
            terminal.trim_end_matches("Attribute")
        )
    })
}

fn is_mvc_attribute(spelling: &str, qualified: Option<&str>) -> bool {
    let terminal = qualified
        .map(terminal_attribute)
        .unwrap_or_else(|| terminal_attribute(spelling))
        .trim_end_matches("Attribute");
    let supported = matches!(
        terminal,
        "AcceptVerbs"
            | "ApiController"
            | "Controller"
            | "HttpDelete"
            | "HttpGet"
            | "HttpHead"
            | "HttpOptions"
            | "HttpPatch"
            | "HttpPost"
            | "HttpPut"
            | "NonAction"
            | "Route"
    );
    supported
        && qualified.is_some_and(|qualified| qualified.starts_with(&format!("{MVC_NAMESPACE}.")))
}

fn terminal_attribute(value: &str) -> &str {
    value
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(value)
}

fn ensure_attribute_suffix(value: &str) -> String {
    if value.ends_with("Attribute") {
        value.to_owned()
    } else {
        format!("{value}Attribute")
    }
}

fn collect_attributes<'tree>(node: Node<'tree>, output: &mut BTreeMap<u64, Node<'tree>>) {
    if node.kind() == "attribute" {
        output.insert(u64::try_from(node.start_byte()).unwrap_or(u64::MAX), node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_attributes(child, output);
    }
}

fn attribute_arguments(node: Node<'_>, source: &[u8]) -> Map<String, Value> {
    let Some(arguments) = node
        .child_by_field_name("arguments")
        .or_else(|| first_child(node, "attribute_argument_list"))
    else {
        return Map::new();
    };
    let mut output = Map::new();
    let mut position = 0_u32;
    let mut cursor = arguments.walk();
    for argument in arguments
        .children(&mut cursor)
        .filter(|child| child.is_named())
    {
        let raw = argument.utf8_text(source).unwrap_or_default().trim();
        let (key, expression) = raw.split_once('=').map_or_else(
            || (position.to_string(), raw),
            |(key, value)| (key.trim().to_owned(), value.trim()),
        );
        if let Some(value) = string_literal(expression) {
            output.insert(key, Value::String(value));
        } else if !expression.is_empty() {
            output.insert(key, Value::String(expression.to_owned()));
        }
        position = position.saturating_add(1);
    }
    output
}

fn string_literal(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value.strip_prefix('@').unwrap_or(value);
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    Some(value.replace("\"\"", "\"").replace("\\\"", "\""))
}

fn unique_binding_map(context: &UniversalDetectionContext<'_, '_>) -> Map<String, Value> {
    let mut grouped = BTreeMap::<&str, Vec<&str>>::new();
    for binding in &context.evidence.bindings {
        grouped
            .entry(&binding.spelling)
            .or_default()
            .push(&binding.qualified_target);
    }
    grouped
        .into_iter()
        .filter_map(|(spelling, mut targets)| {
            targets.sort_unstable();
            targets.dedup();
            (targets.len() == 1)
                .then(|| (spelling.to_owned(), Value::String(targets[0].to_owned())))
        })
        .collect()
}

fn anchor(range: &crate::EvidenceRange) -> RawFrameworkAnchor {
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

fn fact_key(fact: &RawFrameworkFact) -> (&str, u64, &str) {
    match fact {
        RawFrameworkFact::Annotation(annotation) => (
            annotation.anchor.source_file.as_str(),
            annotation.anchor.start_byte,
            annotation.annotation_name.as_str(),
        ),
        RawFrameworkFact::Route(route) => (
            route.anchor.source_file.as_str(),
            route.anchor.start_byte,
            route.operation.as_str(),
        ),
        RawFrameworkFact::Domain(domain) => (
            domain.anchor.source_file.as_str(),
            domain.anchor.start_byte,
            domain.kind.as_str(),
        ),
    }
}

fn first_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}
