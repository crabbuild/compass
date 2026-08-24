use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};

use super::{RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin};
use crate::{Extraction, ProjectEvidence};

const MAX_CONFIG_ITEMS: usize = 256;

/// Vite is a build/configuration framework rather than an HTTP router. This
/// adapter publishes a bounded configuration node so aliases and plugins are
/// visible to graph consumers without pretending that a build plugin is a
/// route or callable target.
pub(super) fn detect(
    path: &Path,
    source: &[u8],
    project: Option<&ProjectEvidence>,
    _extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    if !is_config(path) || source.is_empty() {
        return Vec::new();
    }
    let body = std::str::from_utf8(source).unwrap_or_default();
    let project_activates = project.is_none_or(|project| {
        project.has_dependency("vite")
            || project.has_configuration("vite.config.js")
            || project.has_configuration("vite.config.mjs")
            || project.has_configuration("vite.config.ts")
            || project.has_configuration("vite.config.cjs")
    });
    let source_activates = body.contains("defineConfig")
        || body.contains("from 'vite'")
        || body.contains("from \"vite\"");
    if !project_activates || !source_activates {
        return Vec::new();
    }

    let mut detail = Map::new();
    let mut keys = Vec::new();
    if body.contains("resolve") && body.contains("alias") {
        keys.push(Value::String("resolve.alias".to_owned()));
    }
    if body.contains("plugins") {
        keys.push(Value::String("plugins".to_owned()));
    }
    if !keys.is_empty() {
        detail.insert("configuration_keys".to_owned(), Value::Array(keys));
    }
    let aliases = quoted_pairs(body);
    if !aliases.is_empty() {
        detail.insert("aliases".to_owned(), Value::Object(aliases));
    }
    let plugins = imported_plugins(body);
    if !plugins.is_empty() {
        detail.insert("plugins".to_owned(), Value::Array(plugins));
    }
    let portable = path.to_string_lossy().replace('\\', "/");
    vec![RawFrameworkFact::Domain(RawDomainFact {
        framework: "vite".to_owned(),
        kind: "framework_configuration".to_owned(),
        name: portable.clone(),
        declaring_scope: portable.clone(),
        anchor: anchor(path, source),
        origin: RawFrameworkOrigin::Config,
        detail,
    })]
}

pub(super) fn is_config(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            lower.starts_with("vite.config.")
                && matches!(
                    lower.rsplit_once('.').map(|(_, extension)| extension),
                    Some("js" | "mjs" | "cjs" | "ts" | "mts" | "cts")
                )
        })
}

fn quoted_pairs(source: &str) -> Map<String, Value> {
    let Ok(pattern) =
        Regex::new(r#"[\"']([^\"']+)[\"']\s*:\s*(?:path\.resolve\([^,]+,\s*)?[\"']([^\"']+)[\"']"#)
    else {
        return Map::new();
    };
    let mut aliases = Map::new();
    for capture in pattern.captures_iter(source).take(MAX_CONFIG_ITEMS) {
        let (Some(alias), Some(target)) = (capture.get(1), capture.get(2)) else {
            continue;
        };
        aliases.insert(
            alias.as_str().trim().to_owned(),
            Value::String(
                target
                    .as_str()
                    .trim()
                    .trim_start_matches("./")
                    .replace('\\', "/"),
            ),
        );
    }
    aliases
}

fn imported_plugins(source: &str) -> Vec<Value> {
    let Ok(pattern) = Regex::new(r#"(?:from|require\s*\()\s*[\"']([^\"']+)[\"']"#) else {
        return Vec::new();
    };
    let mut plugins = pattern
        .captures_iter(source)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str()))
        .filter(|value| {
            value.contains("plugin")
                || value.starts_with("@vitejs/")
                || value.starts_with("vite-plugin-")
        })
        .map(|value| Value::String(value.to_owned()))
        .take(MAX_CONFIG_ITEMS)
        .collect::<Vec<_>>();
    plugins.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    plugins.dedup();
    plugins
}

fn anchor(path: &Path, source: &[u8]) -> RawFrameworkAnchor {
    let end_line = source.iter().filter(|byte| **byte == b'\n').count() + 1;
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().replace('\\', "/"),
        start_byte: 0,
        end_byte: source.len() as u64,
        start_line: 1,
        start_column: 0,
        end_line: u32::try_from(end_line).unwrap_or(u32::MAX),
        end_column: 0,
    }
}
