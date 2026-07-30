use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::text::{join_route_path, line_anchor, normalize_route_path, split_top_level, text};
use super::{RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect(path: &Path, source: &[u8], _root: Node<'_>) -> Vec<RawFrameworkFact> {
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
    let Ok(group) = Regex::new(
        r#"^\s*([A-Za-z_]\w*)\s*:?=\s*([A-Za-z_]\w*)\.(?:Group|Route)\(\s*["']([^"']*)["']"#,
    ) else {
        return Vec::new();
    };
    let Ok(route) = Regex::new(
        r#"^\s*([A-Za-z_]\w*)\.(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD|Get|Post|Put|Patch|Delete|Options|Head|HandleFunc)\(\s*["']([^"']*)["']\s*,\s*([^)]+)\)"#,
    ) else {
        return Vec::new();
    };
    let Ok(methods) = Regex::new(r#"\.Methods\(\s*([^)]+)\)"#) else {
        return Vec::new();
    };
    let mut prefixes = HashMap::<String, String>::new();
    let mut facts = Vec::new();
    let mut offset = 0_usize;
    for line in body.split_inclusive('\n') {
        if let Some(capture) = group.captures(line)
            && let (Some(child), Some(parent), Some(prefix)) =
                (capture.get(1), capture.get(2), capture.get(3))
        {
            let parent_prefix = prefixes
                .get(parent.as_str())
                .map(String::as_str)
                .unwrap_or_default();
            prefixes.insert(
                child.as_str().to_owned(),
                join_route_path(parent_prefix, prefix.as_str()),
            );
        }
        let Some(capture) = route.captures(line) else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let (Some(receiver), Some(method), Some(raw_path), Some(stages)) = (
            capture.get(1),
            capture.get(2),
            capture.get(3),
            capture.get(4),
        ) else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let stages = split_top_level(stages.as_str())
            .into_iter()
            .map(clean_reference)
            .filter(|stage| !stage.is_empty())
            .collect::<Vec<_>>();
        let Some((handler, middleware)) = stages.split_last() else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let prefix = prefixes
            .get(receiver.as_str())
            .map(String::as_str)
            .unwrap_or_default();
        let normalized_path = if prefix.is_empty() {
            normalize_route_path(raw_path.as_str())
        } else {
            join_route_path(prefix, raw_path.as_str())
        };
        let operations = if method.as_str() == "HandleFunc" {
            methods
                .captures(line)
                .and_then(|capture| capture.get(1))
                .map(|value| {
                    split_top_level(value.as_str())
                        .into_iter()
                        .filter_map(super::text::literal)
                        .map(|value| value.to_ascii_uppercase())
                        .collect::<Vec<_>>()
                })
                .filter(|values| !values.is_empty())
                .unwrap_or_else(|| vec!["ANY".to_owned()])
        } else {
            vec![method.as_str().to_ascii_uppercase()]
        };
        for operation in operations {
            facts.push(RawFrameworkFact::Route(RawRouteFact {
                framework: framework.to_owned(),
                operation,
                raw_path: raw_path.as_str().to_owned(),
                normalized_path: normalized_path.clone(),
                declaring_scope: receiver.as_str().to_owned(),
                anchor: line_anchor(path, source, offset, line),
                handler_reference: handler.clone(),
                middleware_references: middleware.to_vec(),
                origin: RawFrameworkOrigin::Ast,
                rule: Some(format!("{framework}-router-call")),
                detail: Map::from_iter([(
                    "receiver".into(),
                    Value::String(receiver.as_str().to_owned()),
                )]),
            }));
        }
        offset = offset.saturating_add(line.len());
    }
    facts
}

fn clean_reference(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('&')
        .trim_end_matches("...")
        .trim()
        .to_owned()
}
