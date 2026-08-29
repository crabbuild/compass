use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use compass_languages::{
    Engine, Extraction, FrameworkLimits, ProjectEvidenceIndex, RawFrameworkFact, RawNodeRecord,
    make_id,
};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::{RouteStageRole, resolve_and_publish_framework_routes};
use compass_resolve::resolve;
use tempfile::tempdir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/code-graph/routes/typescript")
        .join(name)
}

fn source_extract(
    logical_path: &Path,
    fixture_name: &str,
) -> Result<Extraction, Box<dyn std::error::Error>> {
    let source = fs::read(fixture(fixture_name))?;
    Ok(Engine::default().extract_source(logical_path, &source)?)
}

fn routes(extraction: &Extraction) -> impl Iterator<Item = &compass_languages::RawRouteFact> {
    extraction.framework_facts.iter().filter_map(|fact| {
        if let RawFrameworkFact::Route(route) = fact {
            Some(route)
        } else {
            None
        }
    })
}

#[test]
fn express_routes_preserve_ordered_middleware_and_reject_computed_paths()
-> Result<(), Box<dyn std::error::Error>> {
    let extraction = source_extract(Path::new("src/server.ts"), "express.ts")?;
    let user = routes(&extraction)
        .find(|route| route.normalized_path == "/users/{userId}")
        .ok_or("missing Express user route")?;
    assert_eq!(user.operation, "GET");
    assert_eq!(user.middleware_references, ["authenticate", "audit"]);
    assert_eq!(user.handler_reference, "showUser");
    let inline = routes(&extraction)
        .find(|route| route.normalized_path == "/inline")
        .ok_or("missing Express inline route")?;
    assert_eq!(inline.middleware_references, ["authenticate"]);
    assert!(
        inline
            .handler_reference
            .starts_with("opaque_inline_handler_at_")
    );
    assert_eq!(
        inline.detail.get("opaque_handler"),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(routes(&extraction).count(), 3);

    let commonjs = Engine::default().extract_source(
        Path::new("src/server.ts"),
        br#"const express = require("express");
const app = express();
const api = express.Router();
app.use("/api", api);
function show() {}
api.route("/users").get(show);
"#,
    )?;
    let mounted = routes(&commonjs)
        .find(|route| route.normalized_path == "/api/users")
        .ok_or("missing mounted CommonJS Express route")?;
    assert_eq!(mounted.handler_reference, "show");

    let nested = Engine::default().extract_source(
        Path::new("src/nest.ts"),
        br#"const express = require("express");
const app = express();
const api = express.Router();
app.use("/v1", api);
function list() {}
api.route("/items").get(list);
"#,
    )?;
    let nested_route = routes(&nested)
        .find(|route| route.normalized_path == "/v1/items")
        .ok_or("missing nested Express route")?;
    assert_eq!(nested_route.handler_reference, "list");
    Ok(())
}

#[test]
fn fastify_routes_reuse_literal_stages_and_route_objects() -> Result<(), Box<dyn std::error::Error>>
{
    let extraction = Engine::default().extract_source(
        Path::new("src/server.ts"),
        br#"import fastify from "fastify";
const app = fastify();
function authenticate() {}
function listUsers() {}
function createUser() {}
app.get("/users/:id", { preHandler: [authenticate] }, listUsers);
app.route({ method: ["POST", "PUT"], url: "/users", preHandler: authenticate, handler: createUser });
app.get(dynamicPath, ignored);
        "#,
    )?;
    let users = routes(&extraction)
        .find(|route| route.normalized_path == "/users/{id}")
        .ok_or("missing Fastify route")?;
    assert_eq!(users.framework, "fastify");
    assert_eq!(users.operation, "GET");
    assert_eq!(users.handler_reference, "listUsers");
    assert_eq!(users.middleware_references, ["authenticate"]);
    let object_routes = routes(&extraction)
        .filter(|route| route.normalized_path == "/users")
        .map(|route| route.operation.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(object_routes, HashSet::from(["POST", "PUT"]));
    assert_eq!(routes(&extraction).count(), 3);

    let commonjs = Engine::default().extract_source(
        Path::new("src/commonjs.ts"),
        br#"const createFastify = require("fastify");
const app = createFastify();
function list() {}
app.route({ method: ["GET", "POST"] as const, url: "/items", handler: list });
"#,
    )?;
    let commonjs_operations = routes(&commonjs)
        .filter(|route| route.normalized_path == "/items")
        .map(|route| route.operation.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(commonjs_operations, HashSet::from(["GET", "POST"]));
    Ok(())
}

#[test]
fn hono_routes_preserve_mounts_method_arrays_and_base_paths()
-> Result<(), Box<dyn std::error::Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/routes.ts"),
        br#"import { Hono } from "hono";
const app = new Hono();
const api = new Hono();
function authenticate() {}
function listUsers() {}
app.route("/api", api);
app.get("/health", listUsers);
api.on(["GET", "POST"], "/users/:id", authenticate, listUsers);
app.basePath("/v1").get("/status", listUsers);
app.basePath("/v2").on(["GET", "POST"], "/batch", listUsers);
unknown.get("/not-a-route", listUsers);
"#,
    )?;
    assert!(routes(&extraction).any(|route| {
        route.framework == "hono" && route.operation == "GET" && route.normalized_path == "/health"
    }));
    assert!(routes(&extraction).any(|route| {
        route.framework == "hono"
            && route.operation == "GET"
            && route.normalized_path == "/api/users/{id}"
            && route.middleware_references == ["authenticate"]
    }));
    assert!(routes(&extraction).any(|route| {
        route.framework == "hono"
            && route.operation == "POST"
            && route.normalized_path == "/api/users/{id}"
    }));
    assert!(routes(&extraction).any(|route| {
        route.framework == "hono"
            && route.operation == "GET"
            && route.normalized_path == "/v1/status"
    }));
    assert!(routes(&extraction).any(|route| {
        route.framework == "hono"
            && route.operation == "POST"
            && route.normalized_path == "/v2/batch"
    }));
    assert!(!routes(&extraction).any(|route| route.normalized_path == "/not-a-route"));
    Ok(())
}

#[test]
fn angular_router_extracts_typed_named_configs_and_nested_lazy_routes()
-> Result<(), Box<dyn std::error::Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/app.routes.ts"),
        br#"import { provideRouter, Routes } from "@angular/router";
import { AdminPage } from "./admin.page";
const routes: Routes = [
  {
    path: "admin",
    component: AdminPage,
    children: [
      { path: "users/:id", loadComponent: () => import("./user.page").then(m => m.UserPage) },
    ],
  },
];
export const providers = [provideRouter(routes)];
"#,
    )?;
    let admin = routes(&extraction)
        .find(|route| route.framework == "angular-router" && route.normalized_path == "/admin")
        .ok_or("missing Angular parent route")?;
    assert_eq!(admin.handler_reference, "AdminPage");

    let lazy = routes(&extraction)
        .find(|route| {
            route.framework == "angular-router" && route.normalized_path == "/admin/users/{id}"
        })
        .ok_or("missing Angular lazy child route")?;
    assert!(
        lazy.handler_reference
            .starts_with("opaque_route_handler_at_")
    );
    assert_eq!(
        lazy.detail.get("opaque_handler"),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(
        routes(&extraction)
            .filter(|route| route.framework == "angular-router")
            .count(),
        2,
        "typed config and provideRouter must not duplicate the same routes"
    );

    let near_match = Engine::default().extract_source(
        Path::new("src/not-router.ts"),
        br#"const routes = [{ path: "admin", component: AdminPage }];"#,
    )?;
    assert!(!routes(&near_match).any(|route| route.framework == "angular-router"));
    Ok(())
}

#[test]
fn node_router_mounts_compose_across_modules_and_fail_closed_on_ambiguity()
-> Result<(), Box<dyn std::error::Error>> {
    for (framework, parent, child, expected_path) in [
        (
            "express",
            br#"import express from "express";
import api from "./api";
const app = express();
app.use("/api", api);
"#
            .as_slice(),
            br#"import express from "express";
const router = express.Router();
function listUsers() {}
router.get("/users", listUsers);
export default router;
"#
            .as_slice(),
            "/api/users",
        ),
        (
            "hono",
            br#"import { Hono } from "hono";
import api from "./api";
const app = new Hono();
app.route("/v1", api);
"#
            .as_slice(),
            br#"import { Hono } from "hono";
const router = new Hono();
function listUsers() {}
router.get("/users", listUsers);
export default router;
"#
            .as_slice(),
            "/v1/users",
        ),
        (
            "fastify",
            br#"import fastify from "fastify";
import api from "./api";
const app = fastify();
app.register(api, { prefix: "/internal" });
"#
            .as_slice(),
            br#"import type { FastifyInstance } from "fastify";
function listUsers() {}
export default async function api(router: FastifyInstance) {
  router.get("/users", listUsers);
}
"#
            .as_slice(),
            "/internal/users",
        ),
    ] {
        let mut engine = Engine::default();
        let parent_path = "src/server.ts";
        let child_path = "src/api.ts";
        let files = [
            engine.extract_source(Path::new(parent_path), parent)?,
            engine.extract_source(Path::new(child_path), child)?,
        ];
        let sources = HashMap::from([
            (parent_path.to_owned(), String::from_utf8(parent.to_vec())?),
            (child_path.to_owned(), String::from_utf8(child.to_vec())?),
        ]);
        let mut extraction = resolve(&files, &sources);
        if framework == "express" {
            let original_nodes = extraction.nodes.clone();
            let original_edges = extraction.edges.clone();
            let error = resolve_and_publish_framework_routes(
                &mut extraction,
                FrameworkLimits {
                    max_include_depth: 0,
                    ..FrameworkLimits::default()
                },
            )
            .err()
            .ok_or("expected an explicit cross-module mount depth error")?;
            assert!(error.to_string().contains("max_include_depth"));
            assert_eq!(extraction.nodes, original_nodes);
            assert_eq!(extraction.edges, original_edges);
        }
        let resolved =
            resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
        assert!(
            resolved.iter().any(|route| {
                route.route.framework == framework && route.route.normalized_path == expected_path
            }),
            "missing {framework} cross-module route {expected_path}"
        );

        let forward_shapes = resolved
            .iter()
            .map(|route| {
                (
                    route.route.framework.clone(),
                    route.route.operation.clone(),
                    route.route.normalized_path.clone(),
                    route.route.handler_reference.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        let reverse_files = [
            engine.extract_source(Path::new(child_path), child)?,
            engine.extract_source(Path::new(parent_path), parent)?,
        ];
        let mut reverse_extraction = resolve(&reverse_files, &sources);
        let reverse = resolve_and_publish_framework_routes(
            &mut reverse_extraction,
            FrameworkLimits::default(),
        )?;
        let reverse_shapes = reverse
            .iter()
            .map(|route| {
                (
                    route.route.framework.clone(),
                    route.route.operation.clone(),
                    route.route.normalized_path.clone(),
                    route.route.handler_reference.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(forward_shapes, reverse_shapes, "{framework} input order");
    }

    let mut engine = Engine::default();
    let parent_path = "src/server.ts";
    let child_path = "src/api.ts";
    let parent = br#"import express from "express";
import api from "./api";
const app = express();
app.use("/api", api);
"#;
    let child = br#"import express from "express";
const users = express.Router();
const admin = express.Router();
function listUsers() {}
users.get("/users", listUsers);
admin.get("/admin", listUsers);
export default users;
"#;
    let files = [
        engine.extract_source(Path::new(parent_path), parent)?,
        engine.extract_source(Path::new(child_path), child)?,
    ];
    let sources = HashMap::from([
        (parent_path.to_owned(), String::from_utf8(parent.to_vec())?),
        (child_path.to_owned(), String::from_utf8(child.to_vec())?),
    ]);
    let mut extraction = resolve(&files, &sources);
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert!(
        resolved
            .iter()
            .all(|route| !route.route.normalized_path.starts_with("/api/")),
        "an ambiguous imported router must not receive an invented mount"
    );
    Ok(())
}

#[test]
fn nest_http_graphql_and_messaging_shapes_are_distinct() -> Result<(), Box<dyn std::error::Error>> {
    let extraction = source_extract(Path::new("src/users.ts"), "nest.ts")?;
    let route_shapes = routes(&extraction)
        .map(|route| {
            (
                route.framework.as_str(),
                route.operation.as_str(),
                route.normalized_path.as_str(),
            )
        })
        .collect::<HashSet<_>>();
    assert!(route_shapes.contains(&("nestjs", "GET", "/users/{userId}")));
    assert!(route_shapes.contains(&("nestjs", "POST", "/users")));
    assert!(route_shapes.contains(&("nestjs-graphql", "QUERY", "/graphql")));
    assert!(route_shapes.contains(&("nestjs-graphql", "MUTATION", "/graphql")));

    let domain_shapes = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| {
            if let RawFrameworkFact::Domain(domain) = fact {
                Some((domain.kind.as_str(), domain.name.as_str()))
            } else {
                None
            }
        })
        .collect::<HashSet<_>>();
    assert!(domain_shapes.contains(&("message", "users.lookup")));
    assert!(domain_shapes.contains(&("event", "users.created")));
    assert!(domain_shapes.contains(&("message", "users.watch")));
    Ok(())
}

#[test]
fn nest_dynamic_decorators_are_not_promoted_to_root_routes()
-> Result<(), Box<dyn std::error::Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("src/nest-dynamic.ts"),
        br#"import { Controller, Get } from "@nestjs/common";
const PATH = "/dynamic";
@Controller({ path: "/users" })
class UsersController {
  @Get(PATH)
  dynamic() {}
  @Get()
  root() {}
}
"#,
    )?;
    let routes = routes(&extraction).collect::<Vec<_>>();
    assert!(routes.iter().any(|route| route.normalized_path == "/users"));
    assert!(
        !routes
            .iter()
            .any(|route| route.normalized_path == "/users/dynamic")
    );
    Ok(())
}

#[test]
fn react_and_vue_router_configs_require_literal_paths_and_known_shapes()
-> Result<(), Box<dyn std::error::Error>> {
    let react = source_extract(Path::new("src/routes.tsx"), "react-router.tsx")?;
    assert!(routes(&react).any(|route| {
        route.framework == "react-router"
            && route.normalized_path == "/accounts/{accountId}"
            && route.handler_reference == "AccountAlias"
    }));
    assert!(routes(&react).any(|route| {
        route.framework == "react-router"
            && route.normalized_path == "/account-settings"
            && route.handler_reference == "AccountAlias"
    }));
    assert!(routes(&react).any(|route| {
        route.framework == "react-router"
            && route.normalized_path == "/users/{userId}"
            && route.handler_reference == "UserPage"
            && route.middleware_references == ["loadUser"]
    }));

    let vue = source_extract(Path::new("src/router.ts"), "vue-router.ts")?;
    assert_eq!(
        routes(&vue)
            .filter(|route| {
                route.framework == "vue-router"
                    && route.normalized_path == "/users/{userId}"
                    && route.handler_reference == "UserPage"
            })
            .count(),
        1,
        "nested route configuration objects must not duplicate their child route"
    );

    let nested = Engine::default().extract_source(
        Path::new("src/nested-routes.tsx"),
        br#"import { createBrowserRouter } from "react-router-dom";
const router = createBrowserRouter([
  { path: "/admin", children: [{ path: "users/:id", Component: UserPage }] },
]);
const guard = <RouteGuard path="/not-a-route" component={UserPage} />;
"#,
    )?;
    assert!(routes(&nested).any(|route| {
        route.framework == "react-router"
            && route.normalized_path == "/admin/users/{id}"
            && route.handler_reference == "UserPage"
    }));
    assert!(!routes(&nested).any(|route| route.normalized_path == "/not-a-route"));

    let object_routes = Engine::default().extract_source(
        Path::new("src/object-routes.tsx"),
        br#"import { createBrowserRouter } from "react-router-dom";
function UserPage() { return null; }
export const router = createBrowserRouter([
  { path: "/users", element: <UserPage /> },
  { path: "/lazy", component: () => import("./LazyPage") },
]);
const unrelated = { path: "/not-a-router", component: UserPage };
"#,
    )?;
    assert!(routes(&object_routes).any(|route| {
        route.normalized_path == "/users" && route.handler_reference == "UserPage"
    }));
    let lazy = routes(&object_routes)
        .find(|route| route.normalized_path == "/lazy")
        .ok_or("missing lazy route")?;
    assert!(
        lazy.handler_reference
            .starts_with("opaque_route_handler_at_")
    );
    assert_eq!(
        lazy.detail.get("opaque_handler"),
        Some(&serde_json::Value::Bool(true))
    );
    assert!(!routes(&object_routes).any(|route| route.normalized_path == "/not-a-router"));
    Ok(())
}

#[test]
fn project_react_router_named_imports_resolve_to_source_callables()
-> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::default();
    let mut files = Vec::new();
    let mut sources = HashMap::new();
    for (path, fixture_name) in [
        ("src/routes.tsx", "react-router.tsx"),
        ("src/AccountPage.tsx", "AccountPage.tsx"),
        ("src/UserPage.tsx", "UserPage.tsx"),
    ] {
        let source = fs::read(fixture(fixture_name))?;
        files.push(engine.extract_source(Path::new(path), &source)?);
        sources.insert(path.to_owned(), String::from_utf8(source)?);
    }
    let mut extraction = resolve(&files, &sources);
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;

    for (path, expected_source) in [
        ("/accounts/{accountId}", "src/AccountPage.tsx"),
        ("/account-settings", "src/AccountPage.tsx"),
        ("/users/{userId}", "src/UserPage.tsx"),
    ] {
        let route = resolved
            .iter()
            .find(|route| route.route.normalized_path == path)
            .ok_or("missing React Router route")?;
        assert_eq!(route.state, ResolutionState::Exact, "{path}");
        let target = route
            .stages
            .last()
            .and_then(|stage| stage.target.as_deref())
            .ok_or("missing React Router handler target")?;
        assert_eq!(
            extraction
                .nodes
                .iter()
                .find(|node| node.id == target)
                .map(|node| node.string("source_file")),
            Some(expected_source.to_owned()),
            "{path}"
        );
    }
    Ok(())
}

#[test]
fn import_alias_metadata_and_near_matches_remain_conservative()
-> Result<(), Box<dyn std::error::Error>> {
    let mut react = source_extract(Path::new("src/routes.tsx"), "react-router.tsx")?;
    assert!(react.nodes.iter().any(|node| {
        node.string("symbol_kind") == "import"
            && node
                .attributes
                .get("local_name")
                .and_then(serde_json::Value::as_str)
                == Some("AccountAlias")
            && node
                .attributes
                .get("imported_name")
                .and_then(serde_json::Value::as_str)
                == Some("AccountPage")
    }));
    react.nodes.push(RawNodeRecord {
        id: make_id(&["src/AccountPage.tsx", "AccountPage"]),
        attributes: serde_json::Map::from_iter([
            (
                "label".into(),
                serde_json::Value::String("AccountPage".into()),
            ),
            (
                "name".into(),
                serde_json::Value::String("AccountPage".into()),
            ),
            (
                "qualified_name".into(),
                serde_json::Value::String("AccountPage".into()),
            ),
            (
                "symbol_kind".into(),
                serde_json::Value::String("component".into()),
            ),
            (
                "source_file".into(),
                serde_json::Value::String("src/AccountPage.tsx".into()),
            ),
            (
                "source_location".into(),
                serde_json::Value::String("L1".into()),
            ),
        ]),
    });
    let resolved = resolve_and_publish_framework_routes(&mut react, FrameworkLimits::default())?;
    assert!(resolved.iter().any(|route| {
        route.route.handler_reference == "AccountAlias"
            && route.state == ResolutionState::Exact
            && route.stages.last().is_some_and(|stage| {
                stage
                    .target
                    .as_deref()
                    .is_some_and(|target| target.ends_with("accountpage"))
            })
    }));

    let near = source_extract(Path::new("src/not-routes.ts"), "near-matches.ts")?;
    assert_eq!(routes(&near).count(), 0);
    Ok(())
}

#[test]
fn default_import_aliases_are_scoped_by_declaring_module_and_follow_default_exports()
-> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::default();
    let mut files = Vec::new();
    let mut sources = HashMap::new();
    for (path, source) in [
        (
            "src/admin/routes.tsx",
            r#"import { createBrowserRouter } from "react-router-dom";
import Screen from "./AdminPage";
export const router = createBrowserRouter([{ path: "/admin", Component: Screen }]);
"#,
        ),
        (
            "src/admin/AdminPage.tsx",
            "export default function AdminPage() { return null; }\n",
        ),
        (
            "src/public/routes.tsx",
            r#"import { createBrowserRouter } from "react-router-dom";
import Screen from "./PublicPage";
export const router = createBrowserRouter([{ path: "/public", Component: Screen }]);
"#,
        ),
        (
            "src/public/PublicPage.tsx",
            "export default function PublicPage() { return null; }\n",
        ),
    ] {
        files.push(engine.extract_source(Path::new(path), source.as_bytes())?);
        sources.insert(path.to_owned(), source.to_owned());
    }
    let mut extraction = resolve(&files, &sources);

    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    let target_source = |path: &str| {
        let target = resolved
            .iter()
            .find(|route| route.route.normalized_path == path)
            .and_then(|route| route.stages.last())
            .and_then(|stage| stage.target.as_deref())
            .unwrap_or_default();
        extraction
            .nodes
            .iter()
            .find(|node| node.id == target)
            .map(|node| node.string("source_file"))
            .unwrap_or_default()
    };
    assert_eq!(target_source("/admin"), "src/admin/AdminPage.tsx");
    assert_eq!(target_source("/public"), "src/public/PublicPage.tsx");
    assert!(
        resolved
            .iter()
            .all(|route| route.state == ResolutionState::Exact)
    );
    Ok(())
}

#[test]
fn file_routes_emit_convention_components_and_exact_bindings()
-> Result<(), Box<dyn std::error::Error>> {
    for (name, expected_framework, expected_path, expected_operations) in [
        (
            "sveltekit/src/routes/users/[id]/+page.svelte",
            "sveltekit",
            "/users/{id}",
            vec!["PAGE"],
        ),
        (
            "sveltekit/src/routes/api/users/[id]/+server.ts",
            "sveltekit",
            "/api/users/{id}",
            vec!["GET", "PATCH"],
        ),
        (
            "sveltekit/src/routes/files/[...rest]/+page.svelte",
            "sveltekit",
            "/files/{*rest}",
            vec!["PAGE"],
        ),
        (
            "nuxt/pages/users/[id].vue",
            "nuxt",
            "/users/{id}",
            vec!["PAGE"],
        ),
        (
            "nuxt/server/api/users/[id].post.ts",
            "nuxt",
            "/users/{id}",
            vec!["POST"],
        ),
        (
            "astro/src/pages/blog/[slug].astro",
            "astro",
            "/blog/{slug}",
            vec!["PAGE"],
        ),
        (
            "astro/src/pages/files/[...rest].astro",
            "astro",
            "/files/{*rest}",
            vec!["PAGE"],
        ),
        (
            "astro/src/pages/api/items/[id].ts",
            "astro",
            "/api/items/{id}",
            vec!["DELETE", "GET"],
        ),
    ] {
        let mut extraction = Engine::default().extract(&fixture(name))?;
        let operations = routes(&extraction)
            .filter(|route| {
                route.framework == expected_framework && route.normalized_path == expected_path
            })
            .map(|route| route.operation.clone())
            .collect::<HashSet<_>>();
        assert_eq!(
            operations,
            expected_operations.into_iter().map(str::to_owned).collect(),
            "{name}"
        );
        let resolved =
            resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
        assert!(
            resolved.iter().all(|route| {
                route.state == ResolutionState::Exact
                    && route
                        .stages
                        .last()
                        .is_some_and(|stage| stage.role == RouteStageRole::Handler)
            }),
            "{name}: {:?}",
            resolved
                .iter()
                .map(|route| (
                    &route.route.operation,
                    &route.route.handler_reference,
                    route.state,
                    route.stages.last().map(|stage| (
                        &stage.reference,
                        &stage.state,
                        &stage.target
                    ))
                ))
                .collect::<Vec<_>>()
        );
        let endpoint =
            name.contains("+server") || (name.contains("/api/") && name.ends_with(".ts"));
        if endpoint {
            assert!(
                routes(&extraction)
                    .filter(|route| {
                        route.framework == expected_framework
                            && route.normalized_path == expected_path
                    })
                    .all(|route| !route
                        .handler_reference
                        .starts_with("sveltekit::route-component::")
                        && !route
                            .handler_reference
                            .starts_with("nuxt::route-component::")
                        && !route
                            .handler_reference
                            .starts_with("astro::route-component::"))
            );
        } else {
            for operation in &operations {
                assert!(extraction.nodes.iter().any(|node| {
                    node.string("symbol_kind") == "component"
                        && node.string("_origin") == "convention"
                        && !node.string("rule").is_empty()
                        && node.string("qualified_name")
                            == format!(
                                "{expected_framework}::route-component::{operation}::{expected_path}"
                            )
                }));
            }
        }
        assert!(extraction.edges.iter().any(|edge| {
            edge.string("relation") == "routes_to"
                && edge.string("_origin") == "convention"
                && !edge.string("rule").is_empty()
        }));
    }
    let page_load_only = Engine::default().extract_source(
        Path::new("src/routes/users/+page.ts"),
        b"export const load = async () => ({ data: true });\n",
    )?;
    assert!(routes(&page_load_only).next().is_none());

    let directory = tempfile::tempdir()?;
    let grouped_path = directory
        .path()
        .join("src/routes/(app)/[[lang]]/[id=integer]/+page.svelte");
    fs::create_dir_all(grouped_path.parent().ok_or("missing route parent")?)?;
    fs::write(&grouped_path, "<h1>User</h1>")?;
    let grouped = Engine::default().extract(&grouped_path)?;
    assert!(routes(&grouped).any(|route| route.normalized_path == "/{lang}/{id}"));
    Ok(())
}

#[test]
fn file_endpoint_reexports_bind_to_the_exported_handler_module()
-> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::default();
    let mut files = Vec::new();
    let mut sources = HashMap::new();
    for (path, source) in [
        (
            "src/pages/api/users.ts",
            r#"export { GET } from "./handlers";
"#,
        ),
        (
            "src/pages/api/handlers.ts",
            "export async function GET() { return new Response(); }\n",
        ),
    ] {
        files.push(engine.extract_source(Path::new(path), source.as_bytes())?);
        sources.insert(path.to_owned(), source.to_owned());
    }
    let mut extraction = resolve(&files, &sources);

    let route = routes(&extraction)
        .find(|route| route.framework == "astro" && route.normalized_path == "/api/users")
        .ok_or("missing Astro endpoint route")?;
    assert_eq!(route.handler_reference, "GET");
    assert_eq!(
        route.detail.get("handler_module"),
        Some(&serde_json::Value::String("./handlers".into()))
    );

    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    let resolved = resolved
        .iter()
        .find(|route| route.route.normalized_path == "/api/users")
        .ok_or("missing resolved Astro endpoint")?;
    assert_eq!(resolved.state, ResolutionState::Exact);
    let target = resolved
        .stages
        .last()
        .and_then(|stage| stage.target.as_deref())
        .ok_or("missing re-export target")?;
    assert_eq!(
        extraction
            .nodes
            .iter()
            .find(|node| node.id == target)
            .map(|node| node.string("source_file")),
        Some("src/pages/api/handlers.ts".to_owned())
    );
    Ok(())
}

#[test]
fn nuxt_route_middleware_is_a_separate_domain_fact() -> Result<(), Box<dyn std::error::Error>> {
    let extraction = Engine::default().extract(&fixture("nuxt/middleware/auth.ts"))?;
    assert!(routes(&extraction).next().is_none());
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            RawFrameworkFact::Domain(domain)
                if domain.framework == "nuxt"
                    && domain.kind == "route_middleware"
                    && domain.name == "auth"
                    && domain.origin == compass_languages::RawFrameworkOrigin::Convention
        )
    }));
    Ok(())
}

#[test]
fn repository_file_routes_require_matching_project_dependencies()
-> Result<(), Box<dyn std::error::Error>> {
    for (dependency, relative_path, source, expected_framework) in [
        (
            "@sveltejs/kit",
            "src/routes/users/[id]/+page.svelte",
            "<h1>User</h1>",
            "sveltekit",
        ),
        (
            "nuxt",
            "pages/users/[id].vue",
            "<template><h1>User</h1></template>",
            "nuxt",
        ),
        (
            "astro",
            "src/pages/users/[id].astro",
            "<h1>User</h1>",
            "astro",
        ),
    ] {
        let directory = tempfile::tempdir()?;
        let route = directory.path().join(relative_path);
        fs::create_dir_all(route.parent().ok_or("route has no parent")?)?;
        fs::write(&route, source)?;
        fs::write(
            directory.path().join("package.json"),
            format!(r#"{{"dependencies":{{"{dependency}":"1.0.0"}}}}"#),
        )?;

        let evidence = ProjectEvidenceIndex::build(directory.path(), std::slice::from_ref(&route));
        let extraction = Engine::with_project_evidence(Arc::new(evidence)).extract(&route)?;
        assert!(
            routes(&extraction).any(|route| route.framework == expected_framework),
            "{relative_path} should activate {expected_framework}"
        );
    }

    let directory = tempfile::tempdir()?;
    let route = directory.path().join("src/routes/users/[id]/+page.svelte");
    fs::create_dir_all(route.parent().ok_or("route has no parent")?)?;
    fs::write(&route, "<h1>User</h1>")?;
    fs::write(
        directory.path().join("package.json"),
        r#"{"dependencies":{"nuxt":"1.0.0"}}"#,
    )?;
    let evidence = ProjectEvidenceIndex::build(directory.path(), std::slice::from_ref(&route));
    let extraction = Engine::with_project_evidence(Arc::new(evidence)).extract(&route)?;
    assert_eq!(
        routes(&extraction).count(),
        0,
        "an unrelated framework dependency must not activate a SvelteKit file route"
    );
    Ok(())
}

#[test]
fn next_app_and_pages_routes_use_project_evidence_and_vite_publishes_config_fact()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let root = directory.path();
    let app_page = root.join("src/app/users/[id]/page.tsx");
    let app_route = root.join("src/app/api/health/route.ts");
    let pages_api = root.join("pages/api/users.ts");
    let vite_config = root.join("vite.config.ts");
    for path in [&app_page, &app_route, &pages_api, &vite_config] {
        fs::create_dir_all(path.parent().ok_or("route has no parent")?)?;
    }
    fs::write(&app_page, "export default function User() { return null }")?;
    fs::write(
        &app_route,
        "export async function GET() { return Response.json({}) }",
    )?;
    fs::write(
        &pages_api,
        "export default function handler(req, res) { res.end() }",
    )?;
    fs::write(
        &vite_config,
        "import react from '@vitejs/plugin-react'; import { defineConfig } from 'vite'; export default defineConfig({ plugins: [react()], resolve: { alias: { '~': './src' } } });",
    )?;
    fs::write(
        root.join("package.json"),
        r#"{"dependencies":{"next":"15.0.0","vite":"7.0.0"},"devDependencies":{"@vitejs/plugin-react":"4.0.0"}}"#,
    )?;

    let sources = vec![
        app_page.clone(),
        app_route.clone(),
        pages_api.clone(),
        vite_config.clone(),
    ];
    let evidence = ProjectEvidenceIndex::build(root, &sources);
    assert!(
        evidence
            .evidence_for(&app_page)
            .has_route_root("next", "src/app")
    );
    let mut engine = Engine::with_project_evidence(Arc::new(evidence));
    let page = engine.extract(&app_page)?;
    let route = engine.extract(&app_route)?;
    let api = engine.extract(&pages_api)?;
    let vite = engine.extract(&vite_config)?;
    assert!(routes(&page).any(|route| {
        route.framework == "next"
            && route.operation == "PAGE"
            && route.normalized_path == "/users/{id}"
    }));
    assert!(routes(&route).any(|route| {
        route.framework == "next"
            && route.operation == "GET"
            && route.normalized_path == "/api/health"
    }));
    assert!(routes(&api).any(|route| {
        route.framework == "next"
            && route.operation == "ANY"
            && route.normalized_path == "/api/users"
    }));
    assert!(vite.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            RawFrameworkFact::Domain(domain)
                if domain.framework == "vite" && domain.kind == "framework_configuration"
        )
    }));
    let vite_source = fs::read_to_string(&vite_config)?;
    let mut sources = HashMap::new();
    sources.insert(vite_config.to_string_lossy().into_owned(), vite_source);
    let graph = compass_resolve::resolve(&[vite], &sources);
    assert!(
        graph.error.is_none(),
        "Vite configuration resolution failed: {:?}",
        graph.error
    );
    assert!(graph.nodes.iter().any(|node| {
        node.string("symbol_kind") == "config_key"
            && node.string("framework") == "vite"
            && node.string("component_type") == "framework_configuration"
    }));

    let config_only = tempdir()?;
    let config_page = config_only.path().join("src/app/page.tsx");
    fs::create_dir_all(
        config_page
            .parent()
            .ok_or("config-only page has no parent")?,
    )?;
    fs::write(
        &config_page,
        "export default function Home() { return null }",
    )?;
    fs::write(
        config_only.path().join("next.config.mjs"),
        "export default { rewrites() { return [] } }",
    )?;
    let config_evidence =
        ProjectEvidenceIndex::build(config_only.path(), std::slice::from_ref(&config_page));
    let config_extraction =
        Engine::with_project_evidence(Arc::new(config_evidence)).extract(&config_page)?;
    assert!(routes(&config_extraction).any(|route| {
        route.framework == "next" && route.operation == "PAGE" && route.normalized_path == "/"
    }));
    Ok(())
}

#[test]
fn remix_flat_routes_publish_nested_page_loader_and_action_operations()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let root = directory.path();
    let index = root.join("app/routes/_index.tsx");
    let user = root.join("app/routes/users.$id.tsx");
    let resource = root.join("app/routes/api.users.ts");
    let nested = root.join("app/routes/admin/users/route.tsx");
    let reexport = root.join("app/routes/settings.tsx");
    for path in [&index, &user, &resource, &nested, &reexport] {
        fs::create_dir_all(path.parent().ok_or("route has no parent")?)?;
    }
    fs::write(&index, "export default function Home() { return null }")?;
    fs::write(
        &user,
        "export async function loader() { return null }\nexport default function User() { return null }",
    )?;
    fs::write(
        &resource,
        "export async function loader() { return null }\nexport async function action() { return null }",
    )?;
    fs::write(
        &nested,
        "export default function AdminUser() { return null }",
    )?;
    fs::write(
        &reexport,
        "export { loader, action } from './settings.server';",
    )?;
    fs::write(
        root.join("package.json"),
        r#"{"dependencies":{"@remix-run/dev":"2.10.0","@remix-run/react":"2.10.0"}}"#,
    )?;
    let sources = vec![
        index.clone(),
        user.clone(),
        resource.clone(),
        nested.clone(),
        reexport.clone(),
    ];
    let evidence = ProjectEvidenceIndex::build(root, &sources);
    assert!(
        evidence
            .evidence_for(&user)
            .has_route_root("remix", "app/routes")
    );
    let mut engine = Engine::with_project_evidence(Arc::new(evidence));
    let index_extraction = engine.extract(&index)?;
    let user_extraction = engine.extract(&user)?;
    let resource_extraction = engine.extract(&resource)?;
    let nested_extraction = engine.extract(&nested)?;
    let reexport_extraction = engine.extract(&reexport)?;
    assert!(routes(&index_extraction).any(|route| {
        route.framework == "remix"
            && route.operation == "PAGE"
            && route.normalized_path == "/"
            && route.handler_reference == "Home"
    }));
    assert!(routes(&user_extraction).any(|route| {
        route.framework == "remix"
            && route.operation == "LOADER"
            && route.normalized_path == "/users/{id}"
            && route.handler_reference == "loader"
    }));
    assert!(routes(&user_extraction).any(|route| {
        route.framework == "remix"
            && route.operation == "PAGE"
            && route.normalized_path == "/users/{id}"
    }));
    let resource_operations = routes(&resource_extraction)
        .map(|route| route.operation.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(resource_operations, HashSet::from(["ACTION", "LOADER"]));
    assert!(routes(&nested_extraction).any(|route| {
        route.framework == "remix"
            && route.operation == "PAGE"
            && route.normalized_path == "/admin/users"
            && route.handler_reference == "AdminUser"
    }));
    assert!(routes(&reexport_extraction).any(|route| {
        route.framework == "remix"
            && route.operation == "LOADER"
            && route.detail.get("handler_module")
                == Some(&serde_json::Value::String("./settings.server".to_owned()))
    }));

    let unrelated = tempdir()?;
    let unrelated_route = unrelated.path().join("app/routes/home.tsx");
    fs::create_dir_all(
        unrelated_route
            .parent()
            .ok_or("unrelated route has no parent")?,
    )?;
    fs::write(
        &unrelated_route,
        "export default function Home() { return null }",
    )?;
    fs::write(
        unrelated.path().join("package.json"),
        r#"{"dependencies":{"react-router-dom":"7.0.0"}}"#,
    )?;
    let unrelated_evidence =
        ProjectEvidenceIndex::build(unrelated.path(), std::slice::from_ref(&unrelated_route));
    let unrelated_extraction =
        Engine::with_project_evidence(Arc::new(unrelated_evidence)).extract(&unrelated_route)?;
    assert!(routes(&unrelated_extraction).all(|route| route.framework != "remix"));
    Ok(())
}
