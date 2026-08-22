//! Bounded Dart framework convention facts.
//!
//! These facts are intentionally separate from universal language evidence.
//! They preserve the established Flutter/BLoC/Riverpod/navigation and local
//! resource-export behavior without publishing a second declaration or call
//! graph. The scanner only emits anchored convention edges and is never used
//! for language identity or target resolution.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::facts::stamp_source_range;
use crate::{Extraction, RawEdgeRecord, RawNodeRecord, file_stem, make_id};

static CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^[ \t]*(?:(?:abstract|sealed|base|interface|final|mixin)\s+)*(?:class|mixin|enum|extension\s+type)\s+(\w+)",
    )
    .unwrap_or_else(|_| Regex::new("$^").unwrap_or_else(|_| unreachable!()))
});
static FUNCTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^[ \t]{0,2}(?:factory\s+|static\s+|async\s+|external\s+|abstract\s+)?(?:\([^)]+\)|[A-Za-z0-9_<>,.?]+)(?:\s+[A-Za-z0-9_<>,.?]+){0,3}\s+(\w+(?:\.\w+)?)\s*\(",
    )
    .unwrap_or_else(|_| Regex::new("$^").unwrap_or_else(|_| unreachable!()))
});
static IMPORT_EXPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^\s*(import|export)\s+['"]([^'"]+)['"]"#)
        .unwrap_or_else(|_| Regex::new("$^").unwrap_or_else(|_| unreachable!()))
});

#[derive(Clone)]
struct Owner {
    id: String,
    start: usize,
    end: usize,
}

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let text = String::from_utf8_lossy(source).into_owned();
    let mut output = Extraction {
        raw_calls: None,
        ..Extraction::default()
    };
    let mut nodes = HashMap::<String, RawNodeRecord>::new();
    let mut owners = Vec::new();
    add_node(
        &mut nodes,
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        "code",
        Some(&source_file),
    );
    for captures in CLASS.captures_iter(&text) {
        let Some(full) = captures.get(0) else {
            continue;
        };
        let Some(name_match) = captures.get(1) else {
            continue;
        };
        let name = name_match.as_str();
        let id = make_id(&[&stem, name]);
        let end = body_end(&text, full.start()).unwrap_or(full.end());
        add_node(&mut nodes, id.clone(), name, "code", Some(&source_file));
        stamp_node(&mut nodes, &id, source, full.start(), end, "class");
        owners.push(Owner {
            id,
            start: full.start(),
            end,
        });
    }
    for captures in FUNCTION.captures_iter(&text) {
        let Some(full) = captures.get(0) else {
            continue;
        };
        let Some(name_match) = captures.get(1) else {
            continue;
        };
        let name = name_match.as_str().rsplit('.').next().unwrap_or_default();
        if matches!(
            name,
            "if" | "for" | "while" | "switch" | "catch" | "return" | "void"
        ) || name.starts_with(|character: char| character.is_ascii_uppercase())
        {
            continue;
        }
        let id = make_id(&[&stem, name]);
        let end = body_end(&text, full.start()).unwrap_or(full.end());
        add_node(&mut nodes, id.clone(), name, "code", Some(&source_file));
        stamp_node(&mut nodes, &id, source, full.start(), end, "function");
        owners.push(Owner {
            id,
            start: full.start(),
            end,
        });
    }
    owners.sort_unstable_by(|left, right| {
        left.start.cmp(&right.start).then(left.end.cmp(&right.end))
    });

    for owner in &owners {
        let Some(body) = text.get(owner.start..owner.end) else {
            continue;
        };
        emit_framework_patterns(&mut output, &mut nodes, owner, body, owner.start, source);
    }
    for captures in IMPORT_EXPORT.captures_iter(&text) {
        let Some(keyword) = captures.get(1) else {
            continue;
        };
        let Some(target) = captures.get(2) else {
            continue;
        };
        let package = target.as_str();
        let target_id = make_id(&[package]);
        add_node(&mut nodes, target_id.clone(), package, "code", None);
        if let Some(node) = nodes.get_mut(&target_id) {
            node.attributes.insert(
                "symbol_kind".to_owned(),
                Value::String("resource".to_owned()),
            );
        }
        let relation = if keyword.as_str() == "export" {
            "exports"
        } else {
            "imports"
        };
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        if !package.contains(':') && !Path::new(package).is_absolute() {
            let target_file = Path::new(&source_file)
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(package);
            attributes.insert(
                "target_file".to_owned(),
                Value::String(target_file.to_string_lossy().replace('\\', "/")),
            );
        }
        output.edges.push(RawEdgeRecord {
            source: file_id.clone(),
            target: target_id,
            attributes,
        });
        if let Some(edge) = output.edges.last_mut() {
            stamp_source_range(
                &mut edge.attributes,
                source,
                captures.get(0).map_or(0, |value| value.start()),
                captures.get(0).map_or(0, |value| value.end()),
            );
        }
    }
    output.nodes = nodes.into_values().collect();
    output
        .nodes
        .sort_unstable_by(|left, right| left.id.cmp(&right.id));
    output.edges.sort_unstable_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| left.string("relation").cmp(&right.string("relation")))
            .then_with(|| {
                left.attributes
                    .get("start_byte")
                    .and_then(Value::as_u64)
                    .cmp(&right.attributes.get("start_byte").and_then(Value::as_u64))
            })
    });
    output
}

fn emit_framework_patterns(
    output: &mut Extraction,
    nodes: &mut HashMap<String, RawNodeRecord>,
    owner: &Owner,
    body: &str,
    body_start: usize,
    source: &[u8],
) {
    for (pattern, relation, context, uppercase) in [
        (r"\bon<(\w+)>\s*\(", "calls", "bloc_event", false),
        (
            r"\b(?:emit|yield)\s*\(?\s*(?:const\s+)?([A-Z]\w*)\b",
            "calls",
            "emit_state",
            true,
        ),
        (
            r"\b(?:\w*[Bb]loc\w*|context\.read<\w+>\(\))\.add\(\s*(?:const\s+)?([A-Z]\w*)\b",
            "calls",
            "bloc_add_event",
            true,
        ),
        (
            r"\bref\.(?:watch|read|listen)\s*\(\s*(\w+)\b",
            "references",
            "riverpod_reference",
            false,
        ),
        (
            r"\bBloc(?:Builder|Listener|Consumer|Provider|Selector)\s*<\s*([A-Za-z0-9_]+)\b",
            "references",
            "bloc_widget_binding",
            true,
        ),
        (
            r"\b(?:read|watch|select|of)\s*<([A-Za-z0-9_]+)>",
            "references",
            "bloc_lookup",
            true,
        ),
    ] {
        let Ok(pattern) = Regex::new(pattern) else {
            continue;
        };
        for captures in pattern.captures_iter(body) {
            let Some(value) = captures.get(1) else {
                continue;
            };
            if uppercase
                && value
                    .as_str()
                    .starts_with(|character: char| character.is_ascii_lowercase())
            {
                continue;
            }
            let target = value.as_str();
            let target_id = make_id(&[target]);
            add_node(nodes, target_id.clone(), target, "code", None);
            add_context_edge(
                output,
                &owner.id,
                &target_id,
                relation,
                context,
                source,
                body_start + captures.get(0).map_or(0, |value| value.start()),
                body_start + captures.get(0).map_or(0, |value| value.end()),
            );
        }
    }
    for (pattern, context, route_object) in [
        (
            r#"\b(?:go|push|goNamed|pushNamed|replace|replaceNamed)\s*\(\s*(?:context\s*,\s*)?['"]([A-Za-z0-9_/?=&%-]+)['"]"#,
            "route_path",
            false,
        ),
        (
            r"\b(?:go|push|goNamed|pushNamed|replace|replaceNamed)\s*\(\s*(?:context\s*,\s*)?([A-Z][A-Za-z0-9_]*\.[A-Za-z0-9_]+)",
            "route_const",
            false,
        ),
        (
            r"\b(?:push|replace)\s*\(\s*(?:context\s*,\s*)?.*?\b([A-Z]\w*(?:Route|Screen|Page))\b",
            "route_object",
            true,
        ),
    ] {
        let Ok(pattern) = Regex::new(pattern) else {
            continue;
        };
        for captures in pattern.captures_iter(body) {
            let Some(value) = captures.get(1) else {
                continue;
            };
            let raw = value.as_str();
            let label = if route_object || context != "route_path" {
                raw.to_owned()
            } else {
                format!("Route {raw}")
            };
            let normalized = raw.replace(['/', '?', '=', '&', '.'], "_");
            let target_id = make_id(&["route", &normalized]);
            add_node(nodes, target_id.clone(), &label, "concept", None);
            add_context_edge(
                output,
                &owner.id,
                &target_id,
                "navigates",
                context,
                source,
                body_start + captures.get(0).map_or(0, |value| value.start()),
                body_start + captures.get(0).map_or(0, |value| value.end()),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_context_edge(
    output: &mut Extraction,
    source_id: &str,
    target_id: &str,
    relation: &str,
    context: &str,
    source: &[u8],
    start: usize,
    end: usize,
) {
    let mut attributes = Map::new();
    attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
    attributes.insert("context".to_owned(), Value::String(context.to_owned()));
    output.edges.push(RawEdgeRecord {
        source: source_id.to_owned(),
        target: target_id.to_owned(),
        attributes,
    });
    if let Some(edge) = output.edges.last_mut() {
        stamp_source_range(&mut edge.attributes, source, start, end);
    }
}

fn add_node(
    nodes: &mut HashMap<String, RawNodeRecord>,
    id: String,
    label: &str,
    file_type: &str,
    source_file: Option<&str>,
) {
    nodes.entry(id.clone()).or_insert_with(|| {
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String(file_type.to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            source_file.map_or(Value::Null, |value| Value::String(value.to_owned())),
        );
        RawNodeRecord { id, attributes }
    });
}

fn stamp_node(
    nodes: &mut HashMap<String, RawNodeRecord>,
    id: &str,
    source: &[u8],
    start: usize,
    end: usize,
    kind: &str,
) {
    let Some(node) = nodes.get_mut(id) else {
        return;
    };
    stamp_source_range(&mut node.attributes, source, start, end);
    node.attributes
        .insert("symbol_kind".to_owned(), Value::String(kind.to_owned()));
    node.attributes
        .insert("language".to_owned(), Value::String("dart".to_owned()));
}

fn body_end(text: &str, start: usize) -> Option<usize> {
    let open = text.get(start..)?.find('{')?.saturating_add(start);
    let mut depth = 0_i32;
    for (relative, character) in text[open..].char_indices() {
        let offset = open.saturating_add(relative);
        if character == '{' {
            depth = depth.saturating_add(1);
        } else if character == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(offset.saturating_add(character.len_utf8()));
            }
        }
    }
    None
}
