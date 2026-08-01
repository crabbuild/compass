use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{join_route_path, line_anchor, normalize_route_path, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

#[derive(Clone)]
struct Mapping {
    name: String,
    arguments: String,
    offset: usize,
    line: String,
}

pub(super) fn detect(path: &Path, source: &[u8], _root: Node<'_>) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let evidence = EvidenceSet::new()
        .direct_if(
            body.contains("org.springframework.web.bind.annotation"),
            "spring",
            EvidenceKind::Import,
            "org.springframework.web.bind.annotation",
        )
        .supporting_if(
            body.contains("@RestController"),
            "spring",
            EvidenceKind::DecoratorOrAttribute,
            "@RestController",
        );
    if !evidence.activates("spring") {
        return Vec::new();
    }
    let Ok(annotation) = Regex::new(
        r"@(GetMapping|PostMapping|PutMapping|PatchMapping|DeleteMapping|RequestMapping)\s*(?:\((.*)\))?",
    ) else {
        return Vec::new();
    };
    let Ok(class) = Regex::new(r"\b(?:class|interface)\s+([A-Za-z_][A-Za-z0-9_]*)") else {
        return Vec::new();
    };
    let Ok(java_method) = Regex::new(
        r"\b(?:public|protected|private|static|final|synchronized|abstract|native|\s)+[A-Za-z0-9_<>,.?\[\]\s]+\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)",
    ) else {
        return Vec::new();
    };
    let Ok(kotlin_method) = Regex::new(r"\bfun\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(") else {
        return Vec::new();
    };

    let mut facts = Vec::new();
    let package = java_package_name(body);
    let mut pending = Vec::<Mapping>::new();
    let mut class_name = None::<String>;
    let mut class_prefix = String::new();
    let mut offset = 0_usize;
    for line in body.split_inclusive('\n') {
        for capture in annotation.captures_iter(line) {
            let Some(name) = capture.get(1) else {
                continue;
            };
            pending.push(Mapping {
                name: name.as_str().to_owned(),
                arguments: capture
                    .get(2)
                    .map(|value| value.as_str().to_owned())
                    .unwrap_or_default(),
                offset,
                line: line.to_owned(),
            });
        }
        if let Some(capture) = class.captures(line) {
            class_name = capture.get(1).map(|value| value.as_str().to_owned());
            class_prefix = pending
                .iter()
                .rev()
                .find(|mapping| mapping.name == "RequestMapping")
                .and_then(|mapping| mapping_paths(&mapping.arguments).into_iter().next())
                .unwrap_or_default();
            pending.clear();
            offset = offset.saturating_add(line.len());
            continue;
        }
        let Some(class_name) = class_name.as_deref() else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let java_capture = java_method.captures(line);
        let kotlin_capture = java_capture
            .is_none()
            .then(|| kotlin_method.captures(line))
            .flatten();
        let Some(method_name) = java_capture
            .as_ref()
            .or(kotlin_capture.as_ref())
            .and_then(|capture| capture.get(1))
            .map(|value| value.as_str())
        else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        for mapping in pending.drain(..) {
            let paths = {
                let values = mapping_paths(&mapping.arguments);
                if values.is_empty() {
                    vec![String::new()]
                } else {
                    values
                }
            };
            let operations = mapping_operations(&mapping.name, &mapping.arguments);
            for method_path in &paths {
                let normalized_path = if class_prefix.is_empty() {
                    normalize_route_path(method_path)
                } else {
                    join_route_path(&class_prefix, method_path)
                };
                for operation in &operations {
                    let mut detail = Map::new();
                    if let Some(capture) = java_capture.as_ref() {
                        let parameters = capture.get(2).map_or("", |value| value.as_str());
                        let (qualified, signature) =
                            java_callable_target(&package, class_name, method_name, parameters);
                        detail.insert("target_qualified_name".to_owned(), Value::String(qualified));
                        detail.insert(
                            "target_signature_qualified".to_owned(),
                            Value::String(signature),
                        );
                    }
                    facts.push(RawFrameworkFact::Route(RawRouteFact {
                        framework: "spring".to_owned(),
                        operation: operation.clone(),
                        raw_path: method_path.clone(),
                        normalized_path: normalized_path.clone(),
                        declaring_scope: class_name.to_owned(),
                        anchor: line_anchor(path, source, mapping.offset, &mapping.line),
                        handler_reference: format!("{class_name}.{method_name}"),
                        middleware_references: Vec::new(),
                        origin: RawFrameworkOrigin::Ast,
                        rule: Some("spring-request-mapping".to_owned()),
                        detail,
                    }));
                }
            }
        }
        offset = offset.saturating_add(line.len());
    }
    facts
}

pub(super) fn java_package_name(body: &str) -> String {
    Regex::new(r"(?m)^\s*package\s+([A-Za-z_$][A-Za-z0-9_$.]*)\s*;")
        .ok()
        .and_then(|pattern| pattern.captures(body))
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
        .unwrap_or_else(|| "<default>".to_owned())
}

pub(super) fn java_callable_target(
    package: &str,
    owner: &str,
    name: &str,
    parameters: &str,
) -> (String, String) {
    let owner = if package.is_empty() {
        owner.to_owned()
    } else {
        format!("{package}.{owner}")
    };
    let qualified = format!("{owner}.{name}");
    let parameters = split_java_parameters(parameters)
        .into_iter()
        .filter_map(|parameter| java_parameter_type(&parameter))
        .collect::<Vec<_>>()
        .join(",");
    let signature = format!("{qualified}({parameters})");
    (qualified, signature)
}

fn split_java_parameters(parameters: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    for (offset, character) in parameters.char_indices() {
        match character {
            '<' | '(' | '[' => depth = depth.saturating_add(1),
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                output.push(parameters[start..offset].trim().to_owned());
                start = offset.saturating_add(1);
            }
            _ => {}
        }
    }
    let tail = parameters[start..].trim();
    if !tail.is_empty() {
        output.push(tail.to_owned());
    }
    output
}

fn java_parameter_type(parameter: &str) -> Option<String> {
    let stripped = strip_java_parameter_annotations(parameter);
    let stripped = stripped
        .split_whitespace()
        .filter(|token| !matches!(*token, "final" | "volatile" | "transient"))
        .collect::<Vec<_>>();
    if stripped.len() < 2 {
        return None;
    }
    let raw_type = stripped[..stripped.len().saturating_sub(1)].join("");
    let mut normalized = String::with_capacity(raw_type.len());
    let mut generic_depth = 0_u32;
    for character in raw_type.chars() {
        match character {
            '<' => generic_depth = generic_depth.saturating_add(1),
            '>' => generic_depth = generic_depth.saturating_sub(1),
            _ if generic_depth > 0 => {}
            character if character.is_whitespace() => {}
            character
                if character.is_alphanumeric()
                    || matches!(character, '_' | '$' | '.' | '[' | ']') =>
            {
                normalized.push(character);
            }
            _ => {}
        }
    }
    parameter
        .contains("...")
        .then(|| normalized.push_str("..."));
    (!normalized.is_empty()).then_some(normalized)
}

fn strip_java_parameter_annotations(parameter: &str) -> String {
    let mut output = String::with_capacity(parameter.len());
    let chars = parameter.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '@' {
            output.push(chars[index]);
            index += 1;
            continue;
        }
        index += 1;
        while index < chars.len()
            && (chars[index].is_alphanumeric() || matches!(chars[index], '_' | '$' | '.'))
        {
            index += 1;
        }
        if index < chars.len() && chars[index] == '(' {
            let mut depth = 1_u32;
            index += 1;
            while index < chars.len() && depth > 0 {
                match chars[index] {
                    '(' => depth = depth.saturating_add(1),
                    ')' => depth = depth.saturating_sub(1),
                    _ => {}
                }
                index += 1;
            }
        }
        output.push(' ');
    }
    output
}

fn mapping_paths(arguments: &str) -> Vec<String> {
    let Ok(literal) = Regex::new(r#""([^"]*)"|'([^']*)'"#) else {
        return Vec::new();
    };
    literal
        .captures_iter(arguments)
        .filter_map(|capture| {
            capture
                .get(1)
                .or_else(|| capture.get(2))
                .map(|value| value.as_str().to_owned())
        })
        .collect()
}

fn mapping_operations(name: &str, arguments: &str) -> Vec<String> {
    let composed = match name {
        "GetMapping" => Some("GET"),
        "PostMapping" => Some("POST"),
        "PutMapping" => Some("PUT"),
        "PatchMapping" => Some("PATCH"),
        "DeleteMapping" => Some("DELETE"),
        _ => None,
    };
    if let Some(operation) = composed {
        return vec![operation.to_owned()];
    }
    let Ok(method) = Regex::new(r"RequestMethod\.([A-Z]+)") else {
        return vec!["ANY".to_owned()];
    };
    let methods = method
        .captures_iter(arguments)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_owned()))
        .collect::<Vec<_>>();
    if methods.is_empty() {
        vec!["ANY".to_owned()]
    } else {
        methods
    }
}
