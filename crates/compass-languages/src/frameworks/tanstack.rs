//! AST-backed TanStack Router/Start route evidence.

use std::path::Path;

use serde_json::{Map, Value};
use tree_sitter::Node;

use super::typescript_syntax::{StaticValue, TypeScriptSyntax};
use super::{
    RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin, RawRouteFact, RawRouteStageFact,
    RawRouteStageRole,
};

const TANSTACK_DEPENDENCIES: &[&str] = &[
    "@tanstack/react-router",
    "@tanstack/router-core",
    "@tanstack/router-generator",
];
const TANSTACK_START_DEPENDENCIES: &[&str] = &["@tanstack/react-start", "@tanstack/start"];

pub(super) fn detect(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    project: Option<&crate::ProjectEvidence>,
) -> Vec<RawFrameworkFact> {
    if source.is_empty()
        || !project.is_none_or(|project| project.has_any_dependency(TANSTACK_DEPENDENCIES))
    {
        return Vec::new();
    }
    let syntax = TypeScriptSyntax::new(root, source);
    let imported_factories = syntax_imported_factory_names(syntax, TANSTACK_DEPENDENCIES);
    let mut facts = Vec::new();
    for call in syntax
        .descendants(root)
        .into_iter()
        .filter(|node| node.kind() == "call_expression")
    {
        let Some(local_factory) = syntax.call_callee(call) else {
            continue;
        };
        let Some(factory) = imported_factories
            .iter()
            .find_map(|(local, canonical)| (local == &local_factory).then_some(canonical))
        else {
            continue;
        };
        let route_path = call
            .child_by_field_name("arguments")
            .and_then(|arguments| {
                let mut cursor = arguments.walk();
                arguments
                    .named_children(&mut cursor)
                    .find_map(|node| syntax.literal_string(node))
            })
            .unwrap_or_else(|| {
                if factory == "createRootRoute" {
                    "/".to_owned()
                } else {
                    String::new()
                }
            });
        if route_path.is_empty() && factory != "createRootRoute" {
            continue;
        }
        let config_call = call
            .parent()
            .filter(|parent| {
                parent.kind() == "call_expression"
                    && parent
                        .child_by_field_name("function")
                        .is_some_and(|function| {
                            function.start_byte() == call.start_byte()
                                && function.end_byte() == call.end_byte()
                        })
            })
            .unwrap_or(call);
        let config = config_call
            .child_by_field_name("arguments")
            .and_then(|arguments| {
                let mut cursor = arguments.walk();
                arguments
                    .named_children(&mut cursor)
                    .filter(|node| node.kind() == "object")
                    .find(|node| !syntax.is_incomplete(*node))
            });
        let anchor = syntax.range(config_call).map_or_else(
            || fallback_anchor(path, source),
            |range| range_anchor(path, range),
        );
        let mut stages = Vec::new();
        let mut handler_reference = "route".to_owned();
        if let Some(config) = config {
            let mut pairs = config.walk();
            for pair in config
                .named_children(&mut pairs)
                .filter(|node| node.kind() == "pair" && !syntax.is_incomplete(*node))
            {
                let Some(name) = syntax.property_name(pair) else {
                    continue;
                };
                let Some(value) = pair
                    .child_by_field_name("value")
                    .or_else(|| pair.named_child(1))
                else {
                    continue;
                };
                let Some(reference) = static_reference(syntax, value) else {
                    continue;
                };
                let Some(role) = stage_role(&name) else {
                    continue;
                };
                if matches!(role, RawRouteStageRole::RouteComponent) {
                    handler_reference.clone_from(&reference);
                }
                let stage_anchor = syntax
                    .range(pair)
                    .map_or_else(|| anchor.clone(), |range| range_anchor(path, range));
                stages.push(RawRouteStageFact {
                    role,
                    position: u32::try_from(stages.len()).unwrap_or(u32::MAX),
                    reference,
                    anchor: stage_anchor,
                    origin: RawFrameworkOrigin::Ast,
                    detail: Map::from_iter([(
                        "factory".to_owned(),
                        Value::String(factory.clone()),
                    )]),
                });
            }
        }
        if stages.is_empty() {
            stages.push(RawRouteStageFact {
                role: RawRouteStageRole::RouteComponent,
                position: 0,
                reference: handler_reference.clone(),
                anchor: anchor.clone(),
                origin: RawFrameworkOrigin::Ast,
                detail: Map::new(),
            });
        }
        let mut detail = Map::from_iter([
            ("factory".to_owned(), Value::String(factory.clone())),
            ("route_path".to_owned(), Value::String(route_path.clone())),
        ]);
        detail.insert(
            "source_file".to_owned(),
            Value::String(path.to_string_lossy().replace('\\', "/")),
        );
        facts.push(RawFrameworkFact::Route(RawRouteFact {
            framework: "tanstack-router".to_owned(),
            operation: if route_path == "/" {
                "ROOT".to_owned()
            } else {
                "PAGE".to_owned()
            },
            raw_path: route_path.clone(),
            normalized_path: normalize_path(&route_path),
            declaring_scope: path.to_string_lossy().replace('\\', "/"),
            anchor,
            handler_reference,
            middleware_references: Vec::new(),
            stages,
            origin: RawFrameworkOrigin::Ast,
            rule: Some("tanstack-route-factory".to_owned()),
            detail,
        }));
    }
    if !facts
        .iter()
        .any(|fact| matches!(fact, RawFrameworkFact::Route(_)))
        && let Some(route) = detect_file_route(
            path,
            source,
            syntax,
            project,
            has_react_router_import(syntax),
        )
    {
        facts.push(RawFrameworkFact::Route(route));
    }
    facts.sort_by_key(|fact| (fact.anchor().start_byte, fact.framework().to_owned()));
    facts
}

fn detect_file_route(
    path: &Path,
    source: &[u8],
    syntax: TypeScriptSyntax<'_, '_>,
    project: Option<&crate::ProjectEvidence>,
    has_react_router_import: bool,
) -> Option<RawRouteFact> {
    // A workspace may intentionally install TanStack and React Router side by
    // side.  Do not let TanStack's directory convention claim a React Router
    // route module merely because it happens to live under `src/routes`.
    // Import identity is the strongest negative signal available at this
    // per-file boundary; package-level activation alone is not sufficient.
    if has_react_router_import {
        return None;
    }
    let relative = project
        .and_then(|project| path.strip_prefix(project.project_root()).ok())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"));
    let marker = ["src/routes/", "routes/"]
        .into_iter()
        .find(|marker| relative.starts_with(marker))?;
    let route_file = relative.strip_prefix(marker)?;
    if route_file.is_empty()
        || route_file.contains("node_modules/")
        || route_file.contains("/.tanstack/")
        || route_file.ends_with("routeTree.gen.ts")
        || route_file.ends_with("routeTree.gen.tsx")
        || route_file.ends_with(".d.ts")
    {
        return None;
    }
    let stem = route_file
        .rsplit_once('/')
        .map_or(route_file, |(_, file)| file);
    let stem = trim_typescript_extension(stem);
    let mut segments = route_file
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(trim_typescript_extension)
        .collect::<Vec<_>>();
    let last = segments.last_mut()?;
    *last = stem;
    let route_segments = segments
        .into_iter()
        .filter_map(|segment| {
            if segment == "index" || segment == "__root" {
                None
            } else if let Some(name) = segment.strip_prefix('$') {
                (!name.is_empty()).then(|| format!("{{{name}}}"))
            } else {
                Some(segment.to_owned())
            }
        })
        .collect::<Vec<_>>();
    let normalized_path = if route_segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", route_segments.join("/"))
    };
    let has_route_export = syntax.descendants(syntax.root()).into_iter().any(|node| {
        node.kind() == "export_statement"
            && syntax.text(node).is_some_and(|text| text.contains("Route"))
    });
    if !has_route_export {
        return None;
    }
    let anchor = syntax.range(syntax.root()).map_or_else(
        || fallback_anchor(path, source),
        |range| range_anchor(path, range),
    );
    Some(RawRouteFact {
        framework: "tanstack-router".to_owned(),
        operation: if normalized_path == "/" {
            "ROOT".to_owned()
        } else {
            "PAGE".to_owned()
        },
        raw_path: normalized_path.clone(),
        normalized_path,
        declaring_scope: relative.clone(),
        anchor: anchor.clone(),
        handler_reference: "Route".to_owned(),
        middleware_references: Vec::new(),
        stages: vec![RawRouteStageFact {
            role: RawRouteStageRole::RouteComponent,
            position: 0,
            reference: "Route".to_owned(),
            anchor,
            origin: RawFrameworkOrigin::Convention,
            detail: Map::from_iter([("generated_tree".to_owned(), Value::Bool(false))]),
        }],
        origin: RawFrameworkOrigin::Convention,
        rule: Some("tanstack-file-route-convention".to_owned()),
        detail: Map::from_iter([
            ("file_route".to_owned(), Value::Bool(true)),
            ("route_file".to_owned(), Value::String(relative)),
        ]),
    })
}

fn has_react_router_import(syntax: TypeScriptSyntax<'_, '_>) -> bool {
    ["react-router", "react-router-dom"]
        .into_iter()
        .any(|module| {
            ["*", "default"]
                .into_iter()
                .any(|imported| !syntax.imported_local_names(module, imported).is_empty())
                || [
                    "Route",
                    "createBrowserRouter",
                    "createHashRouter",
                    "createMemoryRouter",
                ]
                .into_iter()
                .any(|imported| !syntax.imported_local_names(module, imported).is_empty())
        })
}

fn trim_typescript_extension(value: &str) -> &str {
    [".tsx", ".jsx", ".ts", ".js", ".mts", ".mjs"]
        .iter()
        .find_map(|extension| value.strip_suffix(extension))
        .unwrap_or(value)
}

pub(super) fn detect_start(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    project: Option<&crate::ProjectEvidence>,
) -> Vec<RawFrameworkFact> {
    if source.is_empty()
        || !project.is_none_or(|project| project.has_any_dependency(TANSTACK_START_DEPENDENCIES))
    {
        return Vec::new();
    }
    let syntax = TypeScriptSyntax::new(root, source);
    let imports = syntax_imported_factory_names(syntax, TANSTACK_START_DEPENDENCIES);
    let mut facts = Vec::new();
    for call in syntax
        .descendants(root)
        .into_iter()
        .filter(|node| node.kind() == "call_expression")
    {
        let Some(local_factory) = syntax.call_callee(call) else {
            continue;
        };
        let Some(factory) = imports
            .iter()
            .find_map(|(local, canonical)| (local == &local_factory).then_some(canonical))
        else {
            continue;
        };
        let Some(range) = syntax.range(call) else {
            continue;
        };
        let role = if factory == "createMiddleware" {
            "middleware"
        } else {
            "server_function"
        };
        facts.push(RawFrameworkFact::Role(super::RawFrameworkRoleFact {
            pack_id: "tanstack-start".to_owned(),
            framework: "tanstack-start".to_owned(),
            role: role.to_owned(),
            subject_reference: None,
            context: Some(path.to_string_lossy().replace('\\', "/")),
            anchor: range_anchor(path, range),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            detail: Map::from_iter([("factory".to_owned(), Value::String(factory.clone()))]),
        }));
    }
    facts.sort_by_key(|fact| (fact.anchor().start_byte, fact.framework().to_owned()));
    facts
}

fn syntax_imported_factory_names(
    syntax: TypeScriptSyntax<'_, '_>,
    modules: &[&str],
) -> Vec<(String, String)> {
    let factories = [
        "createFileRoute",
        "createLazyFileRoute",
        "createRoute",
        "createRootRoute",
        "createRootRouteWithContext",
        "createServerFn",
        "createServerOnlyFn",
        "createMiddleware",
    ];
    let mut names = modules
        .iter()
        .flat_map(|module| {
            let direct = factories.iter().flat_map(|factory| {
                syntax
                    .imported_local_names(module, factory)
                    .into_iter()
                    .map(move |local| (local, (*factory).to_owned()))
            });
            let namespace = syntax
                .imported_local_names(module, "*")
                .into_iter()
                .flat_map(|name| {
                    factories
                        .iter()
                        .map(move |factory| (format!("{name}.{factory}"), (*factory).to_owned()))
                });
            direct.chain(namespace)
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn stage_role(name: &str) -> Option<RawRouteStageRole> {
    match name {
        "component" | "pendingComponent" | "errorComponent" | "notFoundComponent" => {
            Some(if name == "component" {
                RawRouteStageRole::RouteComponent
            } else {
                RawRouteStageRole::Boundary
            })
        }
        "loader" | "beforeLoad" => Some(RawRouteStageRole::Loader),
        "action" => Some(RawRouteStageRole::Action),
        _ => None,
    }
}

fn static_reference(syntax: TypeScriptSyntax<'_, '_>, node: Node<'_>) -> Option<String> {
    if syntax.is_incomplete(node) {
        return None;
    }
    match syntax.static_value(node) {
        StaticValue::String(value) => Some(value),
        StaticValue::Incomplete => {
            let text = syntax.text(node)?.trim();
            (text
                .chars()
                .next()
                .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
                && text.chars().all(|character| {
                    character == '_' || character == '$' || character.is_ascii_alphanumeric()
                }))
            .then(|| text.to_owned())
        }
        _ => None,
    }
}

fn normalize_path(path: &str) -> String {
    let mut output = String::from("/");
    output.push_str(path.trim_matches('/'));
    output
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

fn fallback_anchor(path: &Path, source: &[u8]) -> RawFrameworkAnchor {
    RawFrameworkAnchor {
        source_file: path.to_string_lossy().replace('\\', "/"),
        start_byte: 0,
        end_byte: u64::try_from(source.len()).unwrap_or(u64::MAX),
        start_line: 1,
        start_column: 0,
        end_line: u32::try_from(source.iter().filter(|byte| **byte == b'\n').count() + 1)
            .unwrap_or(u32::MAX),
        end_column: 0,
    }
}
