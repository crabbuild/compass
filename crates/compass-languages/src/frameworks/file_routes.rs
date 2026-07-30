use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};

use super::evidence::{EvidenceKind, EvidenceSet};
use super::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
};
use crate::{Extraction, ProjectEvidence, RawEdgeRecord, RawNodeRecord, make_id};

pub(super) fn detect(
    path: &Path,
    source: &[u8],
    project: Option<&ProjectEvidence>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    if source.is_empty() {
        return Vec::new();
    }
    let portable = path.to_string_lossy().replace('\\', "/");
    let lower = portable.to_ascii_lowercase();
    let evidence = EvidenceSet::new()
        .direct_if(
            project.is_none_or(|project| project.has_dependency("@sveltejs/kit"))
                && segment_after(&portable, "src/routes/").is_some()
                && matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("+page.svelte" | "+page.ts" | "+server.ts" | "+server.js")
                ),
            "sveltekit",
            EvidenceKind::ConfigurationContract,
            "SvelteKit src/routes artifact",
        )
        .direct_if(
            project.is_none_or(|project| project.has_dependency("nuxt"))
                && ((segment_after(&portable, "pages/").is_some() && lower.ends_with(".vue"))
                    || segment_after(&portable, "server/api/").is_some()
                    || (segment_after(&portable, "middleware/").is_some()
                        && lower.ends_with(".ts")
                        && lower.contains("nuxt"))),
            "nuxt",
            EvidenceKind::ConfigurationContract,
            "Nuxt route artifact",
        )
        .direct_if(
            project.is_none_or(|project| project.has_dependency("astro"))
                && segment_after(&portable, "src/pages/").is_some()
                && (lower.ends_with(".astro")
                    || matches!(
                        path.extension().and_then(|extension| extension.to_str()),
                        Some("ts" | "js")
                    )),
            "astro",
            EvidenceKind::ConfigurationContract,
            "Astro src/pages artifact",
        );
    if evidence.activates("sveltekit")
        && let Some(relative) = segment_after(&portable, "src/routes/")
    {
        if lower.ends_with("/+page.svelte") || lower.ends_with("/+page.ts") {
            return page_routes(
                "sveltekit",
                relative
                    .trim_end_matches("+page.svelte")
                    .trim_end_matches("+page.ts"),
                path,
                source,
                extraction,
            );
        }
        if lower.ends_with("/+server.ts") || lower.ends_with("/+server.js") {
            let route = relative
                .trim_end_matches("+server.ts")
                .trim_end_matches("+server.js");
            return endpoint_routes("sveltekit", route, path, source, extraction);
        }
    }
    if evidence.activates("nuxt")
        && let Some(relative) = segment_after(&portable, "pages/")
        && lower.ends_with(".vue")
    {
        return page_routes(
            "nuxt",
            trim_known_extension(relative),
            path,
            source,
            extraction,
        );
    }
    if evidence.activates("nuxt")
        && let Some(relative) = segment_after(&portable, "server/api/")
        && matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("ts" | "js" | "mts" | "mjs")
        )
    {
        let (route, method) = nuxt_api_route(relative);
        return one_route(
            "nuxt",
            method.as_deref().unwrap_or("ANY"),
            &route,
            path,
            source,
            extraction,
            "nuxt-server-api-convention",
        );
    }
    if evidence.activates("nuxt")
        && let Some(relative) = segment_after(&portable, "middleware/")
        && lower.ends_with(".ts")
        && lower.contains("nuxt")
    {
        return vec![RawFrameworkFact::Domain(RawDomainFact {
            framework: "nuxt".to_owned(),
            kind: "route_middleware".to_owned(),
            name: trim_known_extension(relative).to_owned(),
            declaring_scope: portable,
            anchor: file_anchor(path, source),
            origin: RawFrameworkOrigin::Convention,
            detail: Map::new(),
        })];
    }
    if evidence.activates("astro")
        && let Some(relative) = segment_after(&portable, "src/pages/")
    {
        if lower.ends_with(".astro") {
            return page_routes(
                "astro",
                trim_known_extension(relative),
                path,
                source,
                extraction,
            );
        }
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("ts" | "js")
        ) {
            return endpoint_routes(
                "astro",
                trim_known_extension(relative),
                path,
                source,
                extraction,
            );
        }
    }
    Vec::new()
}

fn page_routes(
    framework: &str,
    relative: &str,
    path: &Path,
    source: &[u8],
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    one_route(
        framework,
        "PAGE",
        relative,
        path,
        source,
        extraction,
        &format!("{framework}-file-route-convention"),
    )
}

fn endpoint_routes(
    framework: &str,
    relative: &str,
    path: &Path,
    source: &[u8],
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    let text = std::str::from_utf8(source).unwrap_or_default();
    let operations = exported_http_methods(text);
    let operations = if operations.is_empty() {
        vec!["ANY".to_owned()]
    } else {
        operations
    };
    operations
        .into_iter()
        .flat_map(|operation| {
            one_route(
                framework,
                &operation,
                relative,
                path,
                source,
                extraction,
                &format!("{framework}-endpoint-convention"),
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn one_route(
    framework: &str,
    operation: &str,
    relative: &str,
    path: &Path,
    source: &[u8],
    extraction: &mut Extraction,
    rule: &str,
) -> Vec<RawFrameworkFact> {
    let original_path = convention_path(relative);
    let normalized_path = normalize_dynamic_segments(&original_path);
    let handler = ensure_route_component(
        path,
        framework,
        operation,
        &normalized_path,
        source,
        extraction,
    );
    vec![RawFrameworkFact::Route(RawRouteFact {
        framework: framework.to_owned(),
        operation: operation.to_owned(),
        raw_path: original_path,
        normalized_path,
        declaring_scope: path.to_string_lossy().replace('\\', "/"),
        anchor: file_anchor(path, source),
        handler_reference: handler,
        middleware_references: Vec::new(),
        origin: RawFrameworkOrigin::Convention,
        rule: Some(rule.to_owned()),
        detail: Map::from_iter([(
            "route_file".into(),
            Value::String(path.to_string_lossy().replace('\\', "/")),
        )]),
    })]
}

fn ensure_route_component(
    path: &Path,
    framework: &str,
    operation: &str,
    route_path: &str,
    source: &[u8],
    extraction: &mut Extraction,
) -> String {
    let source_file = path.to_string_lossy().into_owned();
    let id = make_id(&["route-component", framework, &source_file, operation]);
    let qualified_name = format!("{framework}::route-component::{operation}::{route_path}");
    if extraction.nodes.iter().any(|node| node.id == id) {
        return qualified_name;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("route");
    let line_end = source.iter().filter(|byte| **byte == b'\n').count() + 1;
    extraction.nodes.push(RawNodeRecord {
        id: id.clone(),
        attributes: Map::from_iter([
            ("label".into(), Value::String(name.to_owned())),
            ("name".into(), Value::String(name.to_owned())),
            (
                "qualified_name".into(),
                Value::String(qualified_name.clone()),
            ),
            ("symbol_kind".into(), Value::String("component".into())),
            ("component_type".into(), Value::String("route".into())),
            ("file_type".into(), Value::String("code".into())),
            ("framework".into(), Value::String(framework.to_owned())),
            ("source_file".into(), Value::String(source_file.clone())),
            ("source_location".into(), Value::String("L1".into())),
            ("line_start".into(), Value::from(1)),
            ("line_end".into(), Value::from(line_end)),
            ("_origin".into(), Value::String("convention".into())),
            (
                "rule".into(),
                Value::String(format!("{framework}-file-route-component")),
            ),
            (
                "extractor".into(),
                Value::String(format!("compass.frameworks.{framework}")),
            ),
        ]),
    });
    if let Some(file_id) = extraction
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "file"
                || (node.string("source_file") == source_file
                    && node.label()
                        == path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default())
        })
        .map(|node| node.id.clone())
    {
        extraction.edges.push(RawEdgeRecord {
            source: file_id,
            target: id.clone(),
            attributes: Map::from_iter([
                ("relation".into(), Value::String("contains".into())),
                ("confidence".into(), Value::String("EXTRACTED".into())),
                ("source_file".into(), Value::String(source_file)),
                ("source_location".into(), Value::String("L1".into())),
                ("_origin".into(), Value::String("convention".into())),
                (
                    "rule".into(),
                    Value::String(format!("{framework}-file-route-component")),
                ),
                (
                    "extractor".into(),
                    Value::String(format!("compass.frameworks.{framework}")),
                ),
            ]),
        });
    }
    qualified_name
}

fn exported_http_methods(source: &str) -> Vec<String> {
    let Ok(regex) = Regex::new(
        r"(?m)\bexport\s+(?:(?:async\s+)?function|const|let|var)\s+(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD)\b",
    ) else {
        return Vec::new();
    };
    let mut methods = regex
        .captures_iter(source)
        .filter_map(|capture| capture.get(1))
        .map(|method| method.as_str().to_owned())
        .collect::<Vec<_>>();
    methods.sort();
    methods.dedup();
    methods
}

fn nuxt_api_route(relative: &str) -> (String, Option<String>) {
    let without_extension = trim_known_extension(relative);
    let methods = ["get", "post", "put", "patch", "delete", "options", "head"];
    for method in methods {
        if let Some(route) = without_extension.strip_suffix(&format!(".{method}")) {
            return (route.to_owned(), Some(method.to_ascii_uppercase()));
        }
    }
    (without_extension.to_owned(), None)
}

fn convention_path(relative: &str) -> String {
    let mut route = relative.trim_matches('/').trim_end_matches('/').to_owned();
    if route == "index" {
        route.clear();
    } else if let Some(stripped) = route.strip_suffix("/index") {
        route = stripped.to_owned();
    }
    format!("/{route}")
}

fn normalize_dynamic_segments(path: &str) -> String {
    let Ok(rest) = Regex::new(r"\[\.\.\.([^\]]+)\]") else {
        return path.to_owned();
    };
    let path = rest.replace_all(path, "{*$1}");
    let Ok(parameter) = Regex::new(r"\[([^\]]+)\]") else {
        return path.into_owned();
    };
    parameter.replace_all(&path, "{$1}").into_owned()
}

fn trim_known_extension(value: &str) -> &str {
    [
        ".astro", ".svelte", ".vue", ".tsx", ".ts", ".jsx", ".js", ".mts", ".mjs",
    ]
    .iter()
    .find_map(|extension| value.strip_suffix(extension))
    .unwrap_or(value)
}

fn segment_after<'a>(path: &'a str, marker: &str) -> Option<&'a str> {
    path.find(marker).map(|index| &path[index + marker.len()..])
}

fn file_anchor(path: &Path, source: &[u8]) -> RawFrameworkAnchor {
    let end_line = source.iter().filter(|byte| **byte == b'\n').count() + 1;
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().into_owned(),
        start_byte: 0,
        end_byte: source.len() as u64,
        start_line: 1,
        start_column: 0,
        end_line: u32::try_from(end_line).unwrap_or(u32::MAX),
        end_column: 0,
    }
}
