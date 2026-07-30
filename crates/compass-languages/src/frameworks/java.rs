use std::path::Path;

use regex::Regex;
use serde_json::Map;
use tree_sitter::Node;

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
    if !body.contains("org.springframework.web.bind.annotation")
        && !body.contains("@RestController")
    {
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
        r"\b(?:public|protected|private|static|final|synchronized|abstract|native|\s)+[A-Za-z0-9_<>,.?\[\]\s]+\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
    ) else {
        return Vec::new();
    };
    let Ok(kotlin_method) = Regex::new(r"\bfun\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(") else {
        return Vec::new();
    };

    let mut facts = Vec::new();
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
        let Some(method_name) = java_method
            .captures(line)
            .or_else(|| kotlin_method.captures(line))
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
                        detail: Map::new(),
                    }));
                }
            }
        }
        offset = offset.saturating_add(line.len());
    }
    facts
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
