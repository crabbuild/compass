use std::path::Path;

use regex::Regex;
use serde_json::Map;

use super::text::{line_anchor_at, normalize_route_path, text};
use super::{FrameworkLimits, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact};

pub(super) fn detect(path: &Path, source: &[u8]) -> Vec<RawFrameworkFact> {
    let body = text(source);
    let Ok(route) = Regex::new(
        r"^\s*(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD)\s+(\S+)\s+(@?[A-Za-z_$][A-Za-z0-9_$.]*(?:\([^)]*\))?)",
    ) else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    let mut offset = 0_usize;
    let maximum = FrameworkLimits::default().max_facts_per_file;
    for (line_index, line) in body.split_inclusive('\n').enumerate() {
        let Some(capture) = route.captures(line) else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let Some(operation) = capture.get(1).map(|value| value.as_str().to_owned()) else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let Some(raw_path) = capture.get(2).map(|value| value.as_str().to_owned()) else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        let Some(handler) = capture.get(3).map(|value| {
            value
                .as_str()
                .trim_start_matches('@')
                .trim_start_matches("controllers.")
                .split('(')
                .next()
                .unwrap_or_default()
                .to_owned()
        }) else {
            offset = offset.saturating_add(line.len());
            continue;
        };
        facts.push(RawFrameworkFact::Route(RawRouteFact {
            framework: "play".to_owned(),
            operation,
            raw_path: raw_path.clone(),
            normalized_path: normalize_route_path(&raw_path),
            declaring_scope: path.to_string_lossy().replace('\\', "/"),
            anchor: line_anchor_at(path, source, offset, line, line_index.saturating_add(1)),
            handler_reference: handler,
            middleware_references: Vec::new(),
            origin: RawFrameworkOrigin::Config,
            rule: Some("play-conf-routes".to_owned()),
            detail: Map::new(),
        }));
        offset = offset.saturating_add(line.len());
        if facts.len() > maximum {
            break;
        }
    }
    facts
}
