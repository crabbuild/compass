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
    let body = std::str::from_utf8(source).unwrap_or_default();
    let evidence = EvidenceSet::new()
        .direct_if(
            project.is_none_or(|project| project.has_dependency("@sveltejs/kit"))
                && segment_after(&portable, "src/routes/").is_some()
                && matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("+page.svelte" | "+server.ts" | "+server.js")
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
                        && (project.is_some() || body.contains("defineNuxtRouteMiddleware")))),
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
        if lower.ends_with("/+page.svelte") {
            return page_routes(
                "sveltekit",
                relative.trim_end_matches("+page.svelte"),
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
        let operation = method.as_deref().unwrap_or("ANY");
        let handler =
            exported_endpoint_handlers(std::str::from_utf8(source).unwrap_or_default(), "nuxt")
                .into_iter()
                .find(|handler| handler.operation == operation || handler.operation == "ANY");
        let Some(handler) = handler else {
            return Vec::new();
        };
        return one_route(
            "nuxt",
            operation,
            &route,
            path,
            source,
            extraction,
            "nuxt-server-api-convention",
            Some(&handler),
        );
    }
    if evidence.activates("nuxt")
        && let Some(relative) = segment_after(&portable, "middleware/")
        && lower.ends_with(".ts")
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

/// Detect Next.js App Router and Pages Router conventions. The generic
/// filesystem adapter deliberately excludes Next so a project cannot receive
/// duplicate route facts from two convention packs.
pub(super) fn detect_next(
    path: &Path,
    source: &[u8],
    project: Option<&ProjectEvidence>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    let enabled = project.is_none_or(|project| {
        project.has_dependency("next")
            || project.has_configuration("next.config.js")
            || project.has_configuration("next.config.mjs")
            || project.has_configuration("next.config.ts")
    });
    if !enabled || source.is_empty() {
        return Vec::new();
    }
    let portable = path.to_string_lossy().replace('\\', "/");
    let project_relative = project
        .and_then(|project| path.strip_prefix(project.project_root()).ok())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or(portable);
    let Some((router, relative)) = next_route_file(&project_relative) else {
        return Vec::new();
    };
    let lower = relative.to_ascii_lowercase();
    if router == "app" {
        if ["page.tsx", "page.ts", "page.jsx", "page.js"]
            .iter()
            .any(|name| next_file_is(&lower, name))
        {
            let route = relative
                .rsplit_once('/')
                .map_or("", |(parent, _)| parent)
                .trim_matches('/');
            return page_routes("next", route, path, source, extraction);
        }
        if ["route.tsx", "route.ts", "route.jsx", "route.js"]
            .iter()
            .any(|name| next_file_is(&lower, name))
        {
            let route = relative
                .rsplit_once('/')
                .map_or("", |(parent, _)| parent)
                .trim_matches('/');
            return endpoint_routes("next", route, path, source, extraction);
        }
    } else if router == "pages" {
        let page_route = trim_known_extension(relative);
        if page_route == "api" || page_route.starts_with("api/") {
            return next_pages_api_route(relative, path, source, extraction);
        }
        if matches!(page_route, "_app" | "_document" | "_error") {
            return Vec::new();
        }
        if lower.ends_with(".tsx")
            || lower.ends_with(".ts")
            || lower.ends_with(".jsx")
            || lower.ends_with(".js")
        {
            return page_routes(
                "next",
                trim_known_extension(relative),
                path,
                source,
                extraction,
            );
        }
    }
    Vec::new()
}

/// Detect Remix flat-route and nested route modules. Remix keeps page
/// components and resource/data handlers in the same `app/routes` tree, so a
/// route file can publish more than one operation (`PAGE`, `LOADER`, and
/// `ACTION`) while retaining one convention anchor.
pub(super) fn detect_remix(
    path: &Path,
    source: &[u8],
    project: Option<&ProjectEvidence>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    let enabled = project.is_none_or(|project| {
        project.has_any_dependency(&[
            "@remix-run/dev",
            "@remix-run/node",
            "@remix-run/react",
            "@remix-run/router",
            "@remix-run/serve",
        ]) || project.has_any_configuration(&[
            "remix.config.cjs",
            "remix.config.js",
            "remix.config.mjs",
            "remix.config.ts",
        ])
    });
    if !enabled || source.is_empty() {
        return Vec::new();
    }
    let portable = path.to_string_lossy().replace('\\', "/");
    let project_relative = project
        .and_then(|project| path.strip_prefix(project.project_root()).ok())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or(portable);
    let Some(relative) = remix_route_file(&project_relative) else {
        return Vec::new();
    };
    let handlers = remix_endpoint_handlers(std::str::from_utf8(source).unwrap_or_default());
    handlers
        .into_iter()
        .flat_map(|handler| {
            one_route(
                "remix",
                &handler.operation,
                &remix_route_path(relative),
                path,
                source,
                extraction,
                "remix-route-convention",
                Some(&handler),
            )
        })
        .collect()
}

fn remix_route_file(relative: &str) -> Option<&str> {
    for marker in ["app/routes/", "src/routes/"] {
        if let Some(route) = relative.strip_prefix(marker) {
            return (!route.is_empty()).then_some(route);
        }
        if let Some(index) = relative.find(marker)
            && (index == 0 || relative.as_bytes().get(index - 1) == Some(&b'/'))
        {
            let route = &relative[index + marker.len()..];
            return (!route.is_empty()).then_some(route);
        }
    }
    if let Some(route) = relative.strip_prefix("routes/") {
        return (!route.is_empty()).then_some(route);
    }
    None
}

fn remix_route_path(relative: &str) -> String {
    let mut stem = trim_known_extension(relative).trim_matches('/').to_owned();
    if stem.ends_with("/index") {
        stem.truncate(stem.len().saturating_sub("/index".len()));
    }
    let mut segments = Vec::new();
    let mut pieces = stem.split('/').filter(|piece| !piece.is_empty()).peekable();
    while let Some(piece) = pieces.next() {
        if pieces.peek().is_none() {
            for segment in piece.split('.').filter(|segment| !segment.is_empty()) {
                if segment == "_index" || segment == "index" {
                    continue;
                }
                let segment = segment.strip_suffix('_').unwrap_or(segment);
                if segment.starts_with('_') {
                    continue;
                }
                segments.push(remix_route_segment(segment));
            }
        } else if piece != "index" && piece != "_index" && !piece.starts_with('_') {
            segments.push(remix_route_segment(piece));
        }
    }
    segments.join("/")
}

fn remix_route_segment(segment: &str) -> String {
    if segment == "$" {
        "[...splat]".to_owned()
    } else if let Some(name) = segment.strip_prefix('$') {
        format!("[{name}]")
    } else {
        segment.to_owned()
    }
}

fn remix_endpoint_handlers(source: &str) -> Vec<EndpointHandler> {
    let Ok(named) =
        Regex::new(r"(?m)^\s*export\s+(?:(?:async\s+)?function|const|let|var)\s+(loader|action)\b")
    else {
        return Vec::new();
    };
    let mut handlers = named
        .captures_iter(source)
        .filter_map(|capture| capture.get(1).map(|name| name.as_str().to_owned()))
        .map(|name| EndpointHandler {
            operation: name.to_ascii_uppercase(),
            reference: name,
            module: None,
        })
        .collect::<Vec<_>>();
    if let Ok(reexports) = Regex::new(r"(?m)^\s*export\s*\{([^}]*)\}") {
        for capture in reexports.captures_iter(source) {
            let Some(names) = capture.get(1) else {
                continue;
            };
            for name in names.as_str().split(',').map(str::trim) {
                let (local, exported) = name
                    .split_once(" as ")
                    .map(|(local, exported)| (local.trim(), exported.trim()))
                    .unwrap_or((name, name));
                if matches!(exported, "loader" | "action") {
                    handlers.push(EndpointHandler {
                        operation: exported.to_ascii_uppercase(),
                        reference: local.to_owned(),
                        module: None,
                    });
                }
            }
        }
    }
    if Regex::new(r"(?m)^\s*export\s+default\b").is_ok_and(|pattern| pattern.is_match(source)) {
        let reference = Regex::new(
            r"(?m)^\s*export\s+default\s+(?:(?:async\s+)?function|class)\s+([A-Za-z_$][\w$]*)",
        )
        .ok()
        .and_then(|pattern| {
            pattern
                .captures(source)
                .and_then(|capture| capture.get(1).map(|name| name.as_str().to_owned()))
        })
        .unwrap_or_else(|| "default".to_owned());
        handlers.push(EndpointHandler {
            operation: "PAGE".to_owned(),
            reference,
            module: None,
        });
    }
    handlers.sort_by(|left, right| {
        (&left.operation, &left.reference).cmp(&(&right.operation, &right.reference))
    });
    handlers.dedup();
    handlers
}

fn next_pages_api_route(
    relative: &str,
    path: &Path,
    source: &[u8],
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    let handler = EndpointHandler {
        operation: "ANY".to_owned(),
        reference: "default".to_owned(),
        module: None,
    };
    one_route(
        "next",
        "ANY",
        trim_known_extension(relative),
        path,
        source,
        extraction,
        "next-pages-api-convention",
        Some(&handler),
    )
}

fn next_route_file(relative: &str) -> Option<(&'static str, &str)> {
    for (marker, router) in [
        ("src/app/", "app"),
        ("app/", "app"),
        ("src/pages/", "pages"),
        ("pages/", "pages"),
    ] {
        if let Some(relative) = relative.strip_prefix(marker) {
            return Some((router, relative));
        }
        if let Some(index) = relative.find(marker)
            && (index == 0 || relative.as_bytes().get(index - 1) == Some(&b'/'))
        {
            return Some((router, &relative[index + marker.len()..]));
        }
    }
    None
}

fn next_file_is(relative: &str, file_name: &str) -> bool {
    relative == file_name || relative.ends_with(&format!("/{file_name}"))
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
        None,
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
    let handlers = exported_endpoint_handlers(text, framework);
    handlers
        .into_iter()
        .flat_map(|handler| {
            one_route(
                framework,
                &handler.operation,
                relative,
                path,
                source,
                extraction,
                &format!("{framework}-endpoint-convention"),
                Some(&handler),
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
    handler: Option<&EndpointHandler>,
) -> Vec<RawFrameworkFact> {
    let original_path = convention_path(relative);
    let normalized_path = normalize_dynamic_segments(&original_path);
    let handler_reference = match handler {
        Some(handler) if handler.reference == "default" => {
            ensure_default_export_handler(path, framework, operation, source, extraction);
            "default".to_owned()
        }
        Some(handler) => handler.reference.clone(),
        None => ensure_route_component(
            path,
            framework,
            operation,
            &normalized_path,
            source,
            extraction,
        ),
    };
    vec![RawFrameworkFact::Route(RawRouteFact {
        framework: framework.to_owned(),
        operation: operation.to_owned(),
        raw_path: original_path,
        normalized_path,
        declaring_scope: path.to_string_lossy().replace('\\', "/"),
        anchor: file_anchor(path, source),
        handler_reference,
        middleware_references: Vec::new(),
        origin: RawFrameworkOrigin::Convention,
        rule: Some(rule.to_owned()),
        detail: endpoint_detail(handler, path),
    })]
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EndpointHandler {
    operation: String,
    reference: String,
    module: Option<String>,
}

fn endpoint_detail(handler: Option<&EndpointHandler>, path: &Path) -> Map<String, Value> {
    let mut detail = Map::from_iter([
        (
            "route_file".into(),
            Value::String(path.to_string_lossy().replace('\\', "/")),
        ),
        (
            "handler_source".into(),
            Value::String(path.to_string_lossy().replace('\\', "/")),
        ),
    ]);
    if let Some(module) = handler.and_then(|handler| handler.module.as_ref()) {
        detail.insert("handler_module".into(), Value::String(module.clone()));
    }
    detail
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

fn ensure_default_export_handler(
    path: &Path,
    framework: &str,
    operation: &str,
    source: &[u8],
    extraction: &mut Extraction,
) {
    let source_file = path.to_string_lossy().into_owned();
    let id = make_id(&["route-default-handler", framework, &source_file, operation]);
    if extraction.nodes.iter().any(|node| node.id == id) {
        return;
    }
    let line_end = source.iter().filter(|byte| **byte == b'\n').count() + 1;
    extraction.nodes.push(RawNodeRecord {
        id: id.clone(),
        attributes: Map::from_iter([
            ("label".into(), Value::String("default".into())),
            ("name".into(), Value::String("default".into())),
            ("qualified_name".into(), Value::String("default".into())),
            ("symbol_kind".into(), Value::String("function".into())),
            ("file_type".into(), Value::String("code".into())),
            ("framework".into(), Value::String(framework.to_owned())),
            ("source_file".into(), Value::String(source_file.clone())),
            ("source_location".into(), Value::String("L1".into())),
            ("line_start".into(), Value::from(1)),
            ("line_end".into(), Value::from(line_end)),
            ("_origin".into(), Value::String("convention".into())),
            (
                "rule".into(),
                Value::String(format!("{framework}-default-export-handler")),
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
            node.string("symbol_kind") == "file" || node.string("source_file") == source_file
        })
        .map(|node| node.id.clone())
    {
        extraction.edges.push(RawEdgeRecord {
            source: file_id,
            target: id,
            attributes: Map::from_iter([
                ("relation".into(), Value::String("contains".into())),
                ("confidence".into(), Value::String("EXTRACTED".into())),
                ("source_file".into(), Value::String(source_file)),
                ("source_location".into(), Value::String("L1".into())),
                ("_origin".into(), Value::String("convention".into())),
                (
                    "rule".into(),
                    Value::String(format!("{framework}-default-export-handler")),
                ),
                (
                    "extractor".into(),
                    Value::String(format!("compass.frameworks.{framework}")),
                ),
            ]),
        });
    }
}

fn exported_endpoint_handlers(source: &str, framework: &str) -> Vec<EndpointHandler> {
    let Ok(named) = Regex::new(
        r"(?m)^\s*export\s+(?:(?:async\s+)?function|const|let|var)\s+(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD|ALL|fallback)\b",
    ) else {
        return Vec::new();
    };
    let mut handlers = named
        .captures_iter(source)
        .filter_map(|capture| capture.get(1).map(|name| name.as_str().to_owned()))
        .map(|name| {
            let operation = match name.as_str() {
                "ALL" | "fallback" => "ANY".to_owned(),
                method => method.to_owned(),
            };
            EndpointHandler {
                operation,
                reference: name,
                module: None,
            }
        })
        .collect::<Vec<_>>();
    let Ok(reexports) =
        Regex::new(r#"(?m)^\s*export\s*\{([^}]*)\}(?:\s*from\s*[\"']([^\"']+)[\"'])?"#)
    else {
        return handlers;
    };
    for capture in reexports.captures_iter(source) {
        let Some(names) = capture.get(1) else {
            continue;
        };
        let module = capture.get(2).map(|value| value.as_str().to_owned());
        for name in names.as_str().split(',').map(str::trim) {
            let (local, exported) = name
                .split_once(" as ")
                .map(|(local, exported)| (local.trim(), exported.trim()))
                .unwrap_or((name, name));
            if matches!(
                exported,
                "GET"
                    | "POST"
                    | "PUT"
                    | "PATCH"
                    | "DELETE"
                    | "OPTIONS"
                    | "HEAD"
                    | "ALL"
                    | "fallback"
            ) {
                let operation = match exported {
                    "ALL" | "fallback" => "ANY".to_owned(),
                    method => method.to_owned(),
                };
                handlers.push(EndpointHandler {
                    operation,
                    reference: local.to_owned(),
                    module: module.clone(),
                });
            }
        }
    }
    handlers.sort_by(|left, right| {
        (&left.operation, &left.reference, &left.module).cmp(&(
            &right.operation,
            &right.reference,
            &right.module,
        ))
    });
    handlers.dedup();

    // Nuxt server handlers and Astro endpoints commonly use a default export.
    // The route operation comes from the filename for Nuxt method files; for a
    // generic endpoint it is an explicit ANY operation rather than an
    // invented route component.
    if handlers.is_empty()
        && source.contains("export default")
        && (framework == "nuxt" || framework == "astro")
    {
        handlers.push(EndpointHandler {
            operation: "ANY".to_owned(),
            reference: "default".to_owned(),
            module: None,
        });
    }
    handlers
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
    let mut segments = Vec::new();
    for segment in path.trim_matches('/').split('/') {
        if segment.is_empty() || (segment.starts_with('(') && segment.ends_with(')')) {
            continue;
        }
        let normalized = if let Some(rest) = segment
            .strip_prefix("[[...")
            .and_then(|value| value.strip_suffix("]]"))
        {
            format!("{{*{rest}}}")
        } else if let Some(rest) = segment
            .strip_prefix("[...")
            .and_then(|value| value.strip_suffix(']'))
        {
            format!("{{*{rest}}}")
        } else if let Some(rest) = segment
            .strip_prefix("[[")
            .and_then(|value| value.strip_suffix("]]"))
        {
            format!("{{{}}}", rest.split('=').next().unwrap_or(rest))
        } else if let Some(rest) = segment
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            format!("{{{}}}", rest.split('=').next().unwrap_or(rest))
        } else {
            segment.to_owned()
        };
        segments.push(normalized);
    }
    if segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", segments.join("/"))
    }
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
