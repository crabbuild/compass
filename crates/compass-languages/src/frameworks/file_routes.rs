use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use tree_sitter::Node;

use super::evidence::{EvidenceKind, EvidenceSet};
use super::typescript_syntax::TypeScriptSyntax;
use super::{
    RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact,
    RawRouteStageFact, RawRouteStageRole,
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
    root: Node<'_>,
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
        return next_middleware_fact(&project_relative, path, source);
    };
    let lower = relative.to_ascii_lowercase();
    if router == "app" {
        if let Some(app_file) = next_app_file(relative) {
            if app_file.kind == "route" {
                let handlers =
                    exported_endpoint_handlers_ast(TypeScriptSyntax::new(root, source), "next");
                if handlers.is_empty() {
                    // A route module can re-export its handlers (`export * from
                    // ...`) without exposing a locally resolvable method name.
                    // Keep the file in the route tree as an explicit unresolved
                    // endpoint so parent/child hierarchy remains complete while
                    // the target resolver stays fail-closed.
                    let unresolved = EndpointHandler {
                        operation: "ANY".to_owned(),
                        reference: "default".to_owned(),
                        module: None,
                        synthetic_handler: false,
                    };
                    return one_route_with_options(
                        "next",
                        "ANY",
                        &app_file.raw_route,
                        path,
                        source,
                        extraction,
                        "next-app-route-unresolved-convention",
                        Some(&unresolved),
                        Some(app_file.normalized_route),
                        app_file.detail,
                        Some(RawRouteStageRole::Handler),
                        Map::new(),
                    );
                }
                return handlers
                    .into_iter()
                    .flat_map(|handler| {
                        one_route_with_options(
                            "next",
                            &handler.operation,
                            &app_file.raw_route,
                            path,
                            source,
                            extraction,
                            "next-app-route-convention",
                            Some(&handler),
                            Some(app_file.normalized_route.clone()),
                            app_file.detail.clone(),
                            Some(RawRouteStageRole::Handler),
                            Map::new(),
                        )
                    })
                    .collect();
            }
            let syntax = TypeScriptSyntax::new(root, source);
            let handler = next_default_export_handler(syntax).unwrap_or(EndpointHandler {
                operation: "PAGE".to_owned(),
                reference: "default".to_owned(),
                module: None,
                synthetic_handler: has_default_export(syntax),
            });
            let mut facts = one_route_with_options(
                "next",
                "PAGE",
                &app_file.raw_route,
                path,
                source,
                extraction,
                "next-app-router-convention",
                Some(&handler),
                Some(app_file.normalized_route),
                app_file.detail,
                Some(app_file.stage_role),
                Map::from_iter([(
                    "convention".to_owned(),
                    Value::String(app_file.file_kind.to_owned()),
                )]),
            );
            append_next_generated_stages(&mut facts, syntax, path, source);
            return facts;
        }
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
            let (stage_role, stage_name) = match page_route {
                "_app" => (RawRouteStageRole::RouteComponent, "_app"),
                "_document" => (RawRouteStageRole::Template, "_document"),
                _ => (RawRouteStageRole::ErrorBoundary, "_error"),
            };
            let syntax = TypeScriptSyntax::new(root, source);
            let handler = next_default_export_handler(syntax).unwrap_or(EndpointHandler {
                operation: "PAGE".to_owned(),
                reference: "default".to_owned(),
                module: None,
                synthetic_handler: has_default_export(syntax),
            });
            return one_route_with_options(
                "next",
                "PAGE",
                "/",
                path,
                source,
                extraction,
                "next-pages-router-special-file",
                Some(&handler),
                Some("/".to_owned()),
                Map::from_iter([(
                    "pages_special".to_owned(),
                    Value::String(stage_name.to_owned()),
                )]),
                Some(stage_role),
                Map::new(),
            );
        }
        if lower.ends_with(".tsx")
            || lower.ends_with(".ts")
            || lower.ends_with(".jsx")
            || lower.ends_with(".js")
        {
            return next_pages_page_route(
                "next",
                trim_known_extension(relative),
                path,
                source,
                root,
                extraction,
            );
        }
    }
    Vec::new()
}

#[derive(Clone, Debug)]
struct NextAppFile {
    raw_route: String,
    normalized_route: String,
    file_kind: &'static str,
    stage_role: RawRouteStageRole,
    detail: Map<String, Value>,
    kind: &'static str,
}

fn next_app_file(relative: &str) -> Option<NextAppFile> {
    let relative = relative.trim_matches('/');
    let mut pieces = relative.split('/').filter(|piece| !piece.is_empty());
    let file = pieces.next_back()?;
    let stem = trim_known_extension(file);
    let (file_kind, stage_role, kind) = match stem {
        "page" => ("page", RawRouteStageRole::RouteComponent, "page"),
        "layout" => ("layout", RawRouteStageRole::Layout, "layout"),
        "template" => ("template", RawRouteStageRole::Template, "template"),
        "loading" => ("loading", RawRouteStageRole::Loading, "loading"),
        "error" => ("error", RawRouteStageRole::ErrorBoundary, "error"),
        "global-error" => (
            "global-error",
            RawRouteStageRole::ErrorBoundary,
            "global-error",
        ),
        "not-found" => ("not-found", RawRouteStageRole::NotFound, "not-found"),
        "default" => ("default", RawRouteStageRole::Default, "default"),
        "route" => ("route", RawRouteStageRole::Handler, "route"),
        _ => return None,
    };
    let mut url_segments = Vec::new();
    let mut raw_segments = Vec::new();
    let mut groups = Vec::new();
    let mut slots = Vec::new();
    let mut intercepts = Vec::new();
    for segment in pieces {
        if segment.starts_with('_') {
            return None;
        }
        raw_segments.push(segment.to_owned());
        if let Some(intercept) = segment
            .strip_prefix("(...)")
            .or_else(|| segment.strip_prefix("(..)"))
            .or_else(|| segment.strip_prefix("(.)"))
        {
            intercepts.push(Value::String(segment.to_owned()));
            if intercept.is_empty() {
                return None;
            }
            url_segments.push(intercept.to_owned());
            continue;
        }
        if segment.starts_with('(') && segment.ends_with(')') {
            groups.push(Value::String(segment.to_owned()));
            continue;
        }
        if let Some(slot) = segment.strip_prefix('@') {
            if slot.is_empty() {
                return None;
            }
            slots.push(Value::String(slot.to_owned()));
            continue;
        }
        url_segments.push(segment.to_owned());
    }
    let raw_route = format!("/{}", raw_segments.join("/"))
        .trim_end_matches('/')
        .to_owned();
    let raw_route = if raw_route == "/" {
        "/".to_owned()
    } else {
        raw_route
    };
    let normalized_route = normalize_dynamic_segments(&format!("/{}", url_segments.join("/")));
    let mut detail = Map::from_iter([
        ("router".to_owned(), Value::String("app".to_owned())),
        ("file_kind".to_owned(), Value::String(file_kind.to_owned())),
        (
            "route_segments".to_owned(),
            Value::Array(url_segments.into_iter().map(Value::String).collect()),
        ),
    ]);
    if !groups.is_empty() {
        detail.insert("route_groups".to_owned(), Value::Array(groups));
    }
    if !slots.is_empty() {
        detail.insert("parallel_slots".to_owned(), Value::Array(slots));
    }
    if !intercepts.is_empty() {
        detail.insert("intercepting_segments".to_owned(), Value::Array(intercepts));
    }
    Some(NextAppFile {
        raw_route,
        normalized_route,
        file_kind,
        stage_role,
        detail,
        kind,
    })
}

fn next_middleware_fact(relative: &str, path: &Path, source: &[u8]) -> Vec<RawFrameworkFact> {
    let name = Path::new(relative)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !matches!(name, "middleware" | "proxy") {
        return Vec::new();
    }
    vec![RawFrameworkFact::Domain(RawDomainFact {
        framework: "next".to_owned(),
        kind: "route_middleware".to_owned(),
        name: name.to_owned(),
        declaring_scope: relative.to_owned(),
        anchor: file_anchor(path, source),
        origin: RawFrameworkOrigin::Convention,
        detail: Map::from_iter([("convention".to_owned(), Value::String(name.to_owned()))]),
    })]
}

fn has_default_export(syntax: TypeScriptSyntax<'_, '_>) -> bool {
    syntax
        .descendants(syntax.root())
        .into_iter()
        .filter(|node| node.kind() == "export_statement")
        .any(|statement| syntax.is_default_export_statement(statement))
}

fn next_default_export_handler(syntax: TypeScriptSyntax<'_, '_>) -> Option<EndpointHandler> {
    for statement in syntax
        .descendants(syntax.root())
        .into_iter()
        .filter(|node| node.kind() == "export_statement")
    {
        let is_default = syntax.is_default_export_statement(statement);
        if !is_default {
            continue;
        }
        let mut cursor = statement.walk();
        for child in statement.named_children(&mut cursor) {
            if let Some(name) = child
                .child_by_field_name("name")
                .and_then(|name| syntax.text(name))
                && (child.kind().contains("function") || child.kind().contains("class"))
            {
                return Some(EndpointHandler {
                    operation: "PAGE".to_owned(),
                    reference: name.to_owned(),
                    module: None,
                    synthetic_handler: false,
                });
            }
            if matches!(child.kind(), "identifier" | "member_expression") {
                return syntax.text(child).map(|reference| EndpointHandler {
                    operation: "PAGE".to_owned(),
                    reference: reference.to_owned(),
                    module: None,
                    synthetic_handler: false,
                });
            }
        }
        let Some(module) = statement
            .child_by_field_name("source")
            .and_then(|node| syntax.literal_string(node))
        else {
            continue;
        };
        for specifier in syntax
            .descendants(statement)
            .into_iter()
            .filter(|node| node.kind() == "export_specifier")
        {
            let Some(name) = specifier
                .child_by_field_name("name")
                .and_then(|node| syntax.text(node))
            else {
                continue;
            };
            let exported = specifier
                .child_by_field_name("alias")
                .and_then(|node| syntax.text(node))
                .unwrap_or(name);
            if exported == "default" {
                return Some(EndpointHandler {
                    operation: "PAGE".to_owned(),
                    reference: name.to_owned(),
                    module: Some(module.to_owned()),
                    synthetic_handler: false,
                });
            }
        }
    }
    None
}

fn append_next_generated_stages(
    facts: &mut [RawFrameworkFact],
    syntax: TypeScriptSyntax<'_, '_>,
    path: &Path,
    source: &[u8],
) {
    let Some(RawFrameworkFact::Route(route)) = facts.first_mut() else {
        return;
    };
    let mut exports = Vec::new();
    for statement in syntax
        .descendants(syntax.root())
        .into_iter()
        .filter(|node| node.kind() == "export_statement")
    {
        let mut cursor = statement.walk();
        for child in statement.named_children(&mut cursor) {
            let Some(name) = child
                .child_by_field_name("name")
                .and_then(|name| syntax.text(name))
            else {
                continue;
            };
            let role = match name {
                "generateStaticParams" | "generateMetadata" => RawRouteStageRole::DataLoader,
                _ => continue,
            };
            let Some(range) = syntax.range(child) else {
                continue;
            };
            exports.push(RawRouteStageFact {
                role,
                position: u32::try_from(route.stages.len() + exports.len()).unwrap_or(u32::MAX),
                reference: name.to_owned(),
                anchor: range_anchor(path, range),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::from_iter([("export".to_owned(), Value::String(name.to_owned()))]),
            });
        }
    }
    // Tree-sitter represents `export const generateMetadata = ...` as an
    // export statement containing a lexical declaration, so the declaration
    // name is not exposed as a direct child of the export node.  Recover that
    // parser-backed form explicitly; do not fall back to source matching.
    for declaration in syntax
        .descendants(syntax.root())
        .into_iter()
        .filter(|node| node.kind() == "variable_declarator")
    {
        let exported = declaration
            .parent()
            .filter(|parent| parent.kind() == "lexical_declaration")
            .and_then(|parent| parent.parent())
            .is_some_and(|parent| parent.kind() == "export_statement");
        if !exported {
            continue;
        }
        let Some(name) = declaration
            .child_by_field_name("name")
            .and_then(|name| syntax.text(name))
        else {
            continue;
        };
        if !matches!(name, "generateStaticParams" | "generateMetadata") {
            continue;
        }
        let Some(node) = declaration.child_by_field_name("name") else {
            continue;
        };
        let Some(range) = syntax.range(node) else {
            continue;
        };
        let duplicate = exports.iter().any(|existing: &RawRouteStageFact| {
            existing.reference == name
                && existing.anchor.start_byte == u64::try_from(range.start_byte).unwrap_or(u64::MAX)
        });
        if duplicate {
            continue;
        }
        exports.push(RawRouteStageFact {
            role: RawRouteStageRole::DataLoader,
            position: u32::try_from(route.stages.len() + exports.len()).unwrap_or(u32::MAX),
            reference: name.to_owned(),
            anchor: range_anchor(path, range),
            origin: RawFrameworkOrigin::Ast,
            detail: Map::from_iter([("export".to_owned(), Value::String(name.to_owned()))]),
        });
    }
    route.stages.extend(exports);
    let _ = source;
}

/// Detect Remix flat-route and nested route modules. Remix keeps page
/// components and resource/data handlers in the same `app/routes` tree, so a
/// route file can publish more than one operation (`PAGE`, `LOADER`, and
/// `ACTION`) while retaining one convention anchor.
pub(super) fn detect_remix(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
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
            "remix",
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
    if is_remix_route_config(&project_relative) {
        return detect_remix_route_config(path, source, root, extraction);
    }
    let Some(relative) = remix_route_file(&project_relative) else {
        return Vec::new();
    };
    let handlers = exported_endpoint_handlers_ast(TypeScriptSyntax::new(root, source), "remix");
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

fn is_remix_route_config(relative: &str) -> bool {
    [
        "app/routes.ts",
        "app/routes.tsx",
        "app/routes.js",
        "app/routes.jsx",
        "app/routes.mts",
        "app/routes.mjs",
    ]
    .iter()
    .any(|suffix| relative == *suffix || relative.ends_with(&format!("/{suffix}")))
}

/// Extract Remix's object route DSL (`remix/routes`) without executing the
/// project.  The DSL deliberately supports nested objects and static factory
/// calls only; computed keys, dynamic paths, and arbitrary expressions remain
/// unresolved rather than becoming invented route relationships.
fn detect_remix_route_config(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    let syntax = TypeScriptSyntax::new(root, source);
    let factories = [
        ("route", "route"),
        ("get", "get"),
        ("post", "post"),
        ("put", "put"),
        ("del", "del"),
        ("form", "form"),
        ("resources", "resources"),
    ]
    .into_iter()
    .filter_map(|(canonical, _)| {
        syntax
            .imported_local_names("remix/routes", canonical)
            .into_iter()
            .next()
            .map(|local| (local, canonical))
    })
    .collect::<std::collections::BTreeMap<_, _>>();
    if factories.is_empty() {
        return Vec::new();
    }
    let mut facts = Vec::new();
    for call in syntax
        .descendants(root)
        .into_iter()
        .filter(|node| node.kind() == "call_expression")
    {
        let Some(callee) = syntax.call_callee(call) else {
            continue;
        };
        if factories.get(&callee).copied() != Some("route")
            || call
                .parent()
                .is_some_and(|parent| parent.kind() == "call_expression")
        {
            continue;
        }
        let Some(arguments) = call.child_by_field_name("arguments") else {
            continue;
        };
        let mut cursor = arguments.walk();
        let Some(first) = arguments.named_children(&mut cursor).next() else {
            continue;
        };
        if first.kind() != "object" || syntax.is_incomplete(first) {
            continue;
        }
        collect_remix_route_config_object(
            first, "", syntax, path, source, extraction, &factories, &mut facts,
        );
    }
    facts.sort_by_key(|fact| (fact.anchor().start_byte, fact.framework().to_owned()));
    facts
}

#[allow(clippy::too_many_arguments)]
fn collect_remix_route_config_object(
    object: Node<'_>,
    parent_path: &str,
    syntax: TypeScriptSyntax<'_, '_>,
    path: &Path,
    source: &[u8],
    extraction: &mut Extraction,
    factories: &std::collections::BTreeMap<String, &str>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if object.kind() != "object" || syntax.is_incomplete(object) {
        return;
    }
    let mut cursor = object.walk();
    for property in object
        .named_children(&mut cursor)
        .filter(|node| node.kind() == "pair")
    {
        let Some(key) = syntax.property_name(property) else {
            continue;
        };
        let Some(value) = property
            .child_by_field_name("value")
            .or_else(|| property.named_child(1))
        else {
            continue;
        };
        collect_remix_route_config_value(
            value,
            parent_path,
            &key,
            syntax,
            path,
            source,
            extraction,
            factories,
            facts,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_remix_route_config_value(
    value: Node<'_>,
    parent_path: &str,
    key: &str,
    syntax: TypeScriptSyntax<'_, '_>,
    path: &Path,
    source: &[u8],
    extraction: &mut Extraction,
    factories: &std::collections::BTreeMap<String, &str>,
    facts: &mut Vec<RawFrameworkFact>,
) {
    if syntax.is_incomplete(value) {
        return;
    }
    if value.kind() == "object" {
        collect_remix_route_config_object(
            value,
            &join_remix_config_path(parent_path, key),
            syntax,
            path,
            source,
            extraction,
            factories,
            facts,
        );
        return;
    }
    if let Some(raw_path) = syntax.literal_string(value) {
        facts.extend(build_remix_config_route(
            "PAGE",
            join_remix_config_path(parent_path, &raw_path),
            value,
            syntax,
            path,
            source,
            extraction,
        ));
        return;
    }
    if value.kind() != "call_expression" {
        return;
    }
    let Some(callee) = syntax.call_callee(value) else {
        return;
    };
    let Some(factory) = factories.get(&callee).copied() else {
        return;
    };
    let Some(arguments) = value.child_by_field_name("arguments") else {
        return;
    };
    let mut cursor = arguments.walk();
    let arguments = arguments.named_children(&mut cursor).collect::<Vec<_>>();
    let first = arguments.first().copied();
    let second = arguments.get(1).copied();
    match factory {
        "route" => {
            let (segment, children) = match (first, second) {
                (Some(first), Some(second)) => (
                    syntax.literal_string(first),
                    (second.kind() == "object").then_some(second),
                ),
                (Some(first), None) if first.kind() == "object" => {
                    (Some(String::new()), Some(first))
                }
                _ => (None, None),
            };
            let Some(children) = children else {
                return;
            };
            let route_parent = segment.as_deref().map_or_else(
                || parent_path.to_owned(),
                |segment| join_remix_config_path(parent_path, segment),
            );
            collect_remix_route_config_object(
                children,
                &route_parent,
                syntax,
                path,
                source,
                extraction,
                factories,
                facts,
            );
        }
        "resources" => {
            let Some(segment) = first.and_then(|node| syntax.literal_string(node)) else {
                return;
            };
            facts.extend(build_remix_config_route(
                "RESOURCE",
                join_remix_config_path(parent_path, &segment),
                value,
                syntax,
                path,
                source,
                extraction,
            ));
        }
        "get" | "form" | "post" | "put" | "del" => {
            let Some(segment) = first.and_then(|node| syntax.literal_string(node)) else {
                return;
            };
            let operation = match factory {
                "get" => "GET",
                "post" => "POST",
                "put" => "PUT",
                "del" => "DELETE",
                // `form` describes a page/form route; its action metadata is
                // retained in the route DSL but does not change the page
                // relationship's component stage.
                "form" => "PAGE",
                _ => unreachable!(),
            };
            facts.extend(build_remix_config_route(
                operation,
                join_remix_config_path(parent_path, &segment),
                value,
                syntax,
                path,
                source,
                extraction,
            ));
        }
        _ => {}
    }
}

fn join_remix_config_path(parent: &str, child: &str) -> String {
    let child = child.trim_matches('/');
    if child.is_empty() {
        return if parent.is_empty() {
            "/".to_owned()
        } else {
            parent.to_owned()
        };
    }
    let parent = parent.trim_end_matches('/');
    if parent.is_empty() {
        format!("/{child}")
    } else {
        format!("{parent}/{child}")
    }
}

fn build_remix_config_route(
    operation: &str,
    route_path: String,
    anchor_node: Node<'_>,
    syntax: TypeScriptSyntax<'_, '_>,
    path: &Path,
    source: &[u8],
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    let relative = path.to_string_lossy().replace('\\', "/");
    let normalized = normalize_dynamic_segments(&route_path);
    let mut generated = one_route_with_options(
        "remix",
        operation,
        &relative,
        path,
        source,
        extraction,
        "remix-route-config",
        None,
        Some(normalized),
        Map::from_iter([
            ("route_config".to_owned(), Value::Bool(true)),
            ("config_path".to_owned(), Value::String(relative.clone())),
        ]),
        None,
        Map::new(),
    );
    if let Some(RawFrameworkFact::Route(route)) = generated.first_mut() {
        if let Some(range) = syntax.range(anchor_node) {
            route.anchor = range_anchor(path, range);
        }
        route.origin = RawFrameworkOrigin::Config;
        if let Some(stage) = route.stages.first_mut() {
            stage.origin = RawFrameworkOrigin::Config;
            if let Some(range) = syntax.range(anchor_node) {
                stage.anchor = range_anchor(path, range);
            }
        }
    }
    generated
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
    if stem.ends_with("/index") || stem.ends_with("/route") {
        let suffix = if stem.ends_with("/index") {
            "/index"
        } else {
            "/route"
        };
        stem.truncate(stem.len().saturating_sub(suffix.len()));
    }
    let mut segments = Vec::new();
    let pieces = stem
        .split('/')
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>();
    for (piece_index, piece) in pieces.iter().enumerate() {
        let final_piece = piece_index + 1 == pieces.len();
        for segment in piece.split('.').filter(|segment| !segment.is_empty()) {
            if final_piece && matches!(segment, "_index" | "index" | "route") {
                continue;
            }
            let segment = segment.strip_suffix('_').unwrap_or(segment);
            if segment.starts_with('_') {
                continue;
            }
            segments.push(remix_route_segment(segment));
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

fn exported_endpoint_handlers_ast(
    syntax: TypeScriptSyntax<'_, '_>,
    framework: &str,
) -> Vec<EndpointHandler> {
    let accepted = if framework == "remix" {
        ["loader", "action", "default"].as_slice()
    } else {
        [
            "GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD", "ALL", "fallback",
        ]
        .as_slice()
    };
    let mut handlers = Vec::new();
    for statement in syntax
        .descendants(syntax.root())
        .into_iter()
        .filter(|node| node.kind() == "export_statement")
    {
        let mut children = statement.walk();
        let has_default = statement
            .children(&mut children)
            .any(|child| child.kind() == "default");
        let module = statement
            .child_by_field_name("source")
            .and_then(|node| syntax.literal_string(node));
        let mut statement_cursor = statement.walk();
        let mut named = statement
            .named_children(&mut statement_cursor)
            .filter(|node| {
                matches!(
                    node.kind(),
                    "function_declaration"
                        | "class_declaration"
                        | "lexical_declaration"
                        | "variable_declaration"
                )
            });
        if let Some(declaration) = named.next() {
            if has_default {
                handlers.push(EndpointHandler {
                    operation: "PAGE".to_owned(),
                    reference: declaration_name(syntax, declaration)
                        .unwrap_or_else(|| "default".to_owned()),
                    module: module.clone(),
                    synthetic_handler: false,
                });
            } else if let Some(name) = declaration_name(syntax, declaration)
                && accepted.iter().any(|candidate| *candidate == name)
            {
                handlers.push(EndpointHandler {
                    operation: normalize_http_operation(&name),
                    reference: name,
                    module: module.clone(),
                    synthetic_handler: false,
                });
            }
            continue;
        }
        for specifier in syntax
            .descendants(statement)
            .into_iter()
            .filter(|node| node.kind() == "export_specifier")
        {
            let Some(name_node) = specifier.child_by_field_name("name") else {
                continue;
            };
            let Some(local) = syntax.text(name_node).map(str::to_owned) else {
                continue;
            };
            let exported = specifier
                .child_by_field_name("alias")
                .and_then(|node| syntax.text(node))
                .unwrap_or(&local);
            if accepted.contains(&exported) {
                handlers.push(EndpointHandler {
                    operation: if exported == "default" {
                        "PAGE".to_owned()
                    } else {
                        normalize_http_operation(exported)
                    },
                    reference: local,
                    module: module.clone(),
                    synthetic_handler: false,
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
    handlers
}

fn declaration_name(syntax: TypeScriptSyntax<'_, '_>, node: Node<'_>) -> Option<String> {
    if node.kind() == "lexical_declaration" || node.kind() == "variable_declaration" {
        return syntax.descendants(node).into_iter().find_map(|child| {
            (child.kind() == "variable_declarator")
                .then(|| child.child_by_field_name("name"))
                .flatten()
                .and_then(|name| syntax.text(name).map(str::to_owned))
        });
    }
    node.child_by_field_name("name")
        .and_then(|name| syntax.text(name).map(str::to_owned))
}

fn normalize_http_operation(value: &str) -> String {
    match value {
        "ALL" | "fallback" => "ANY".to_owned(),
        value => value.to_ascii_uppercase(),
    }
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
        synthetic_handler: false,
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
        let mut offset = 0;
        while let Some(found) = relative.get(offset..)?.find(marker) {
            let index = offset + found;
            if index == 0 || relative.as_bytes().get(index - 1) == Some(&b'/') {
                return Some((router, &relative[index + marker.len()..]));
            }
            // A directory such as `next-after-app` contains the text
            // `app/` but is not an App Router root. Continue searching for
            // the next segment boundary instead of stopping at that false
            // textual match.
            offset = index.saturating_add(1);
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

/// Next Pages route modules use the default export as their page component.
/// Preserve that parser-backed binding when it is available; falling back to
/// the convention-owned component keeps anonymous/default-only pages visible
/// without inventing a target.
fn next_pages_page_route(
    framework: &str,
    relative: &str,
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    extraction: &mut Extraction,
) -> Vec<RawFrameworkFact> {
    let syntax = TypeScriptSyntax::new(root, source);
    let handler = next_default_export_handler(syntax).unwrap_or(EndpointHandler {
        operation: "PAGE".to_owned(),
        reference: "default".to_owned(),
        module: None,
        synthetic_handler: has_default_export(syntax),
    });
    one_route(
        framework,
        "PAGE",
        relative,
        path,
        source,
        extraction,
        &format!("{framework}-file-route-convention"),
        Some(&handler),
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
    one_route_with_options(
        framework,
        operation,
        relative,
        path,
        source,
        extraction,
        rule,
        handler,
        None,
        Map::new(),
        None,
        Map::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn one_route_with_options(
    framework: &str,
    operation: &str,
    relative: &str,
    path: &Path,
    source: &[u8],
    extraction: &mut Extraction,
    rule: &str,
    handler: Option<&EndpointHandler>,
    normalized_override: Option<String>,
    mut route_detail: Map<String, Value>,
    stage_role_override: Option<RawRouteStageRole>,
    stage_detail: Map<String, Value>,
) -> Vec<RawFrameworkFact> {
    let original_path = convention_path(relative);
    let normalized_path =
        normalized_override.unwrap_or_else(|| normalize_dynamic_segments(&original_path));
    let handler_reference = match handler {
        Some(handler) if handler.reference == "default" => {
            ensure_default_export_handler(
                path,
                framework,
                operation,
                source,
                handler.synthetic_handler,
                extraction,
            );
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
    let stage_role = match operation.to_ascii_uppercase().as_str() {
        "PAGE" | "ROOT" => RawRouteStageRole::RouteComponent,
        "LAYOUT" => RawRouteStageRole::Layout,
        "TEMPLATE" => RawRouteStageRole::Template,
        "LOADING" => RawRouteStageRole::Loading,
        "DEFAULT" => RawRouteStageRole::Default,
        "ERROR" | "GLOBAL_ERROR" => RawRouteStageRole::ErrorBoundary,
        "NOT_FOUND" => RawRouteStageRole::NotFound,
        "LOADER" | "BEFORE_LOAD" => RawRouteStageRole::Loader,
        "ACTION" => RawRouteStageRole::Action,
        _ => RawRouteStageRole::Handler,
    };
    let stage_reference = handler_reference.clone();
    let route_anchor = file_anchor(path, source);
    route_detail.extend(endpoint_detail(handler, path));
    vec![RawFrameworkFact::Route(RawRouteFact {
        framework: framework.to_owned(),
        operation: operation.to_owned(),
        raw_path: original_path,
        normalized_path,
        declaring_scope: path.to_string_lossy().replace('\\', "/"),
        anchor: route_anchor.clone(),
        handler_reference,
        middleware_references: Vec::new(),
        stages: vec![RawRouteStageFact {
            role: stage_role_override.unwrap_or(stage_role),
            position: 0,
            reference: stage_reference,
            anchor: route_anchor,
            origin: RawFrameworkOrigin::Convention,
            detail: stage_detail,
        }],
        origin: RawFrameworkOrigin::Convention,
        rule: Some(rule.to_owned()),
        detail: route_detail,
    })]
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EndpointHandler {
    operation: String,
    reference: String,
    module: Option<String>,
    synthetic_handler: bool,
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
    synthetic_handler: bool,
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
            // A default-only file-route endpoint has a real convention-owned
            // target even when the parser cannot name the expression.  Keep
            // this marker distinct from parser-backed declarations so the
            // resolver can prefer the latter when both identities exist.
            (
                "synthetic_handler".into(),
                // This marker is computed from parser-backed export evidence
                // by the caller.  Never re-scan source text here: a string or
                // comment must not turn an unresolved route into a callable.
                Value::Bool(synthetic_handler),
            ),
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
                synthetic_handler: false,
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
                    synthetic_handler: false,
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
            synthetic_handler: true,
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

fn range_anchor(path: &Path, range: super::typescript_syntax::SyntaxRange) -> RawFrameworkAnchor {
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().replace('\\', "/"),
        start_byte: u64::try_from(range.start_byte).unwrap_or(u64::MAX),
        end_byte: u64::try_from(range.end_byte).unwrap_or(u64::MAX),
        start_line: range.start_line,
        start_column: range.start_column,
        end_line: range.end_line,
        end_column: range.end_column,
    }
}
