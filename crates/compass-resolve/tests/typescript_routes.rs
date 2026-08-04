use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use compass_languages::{
    Engine, Extraction, FrameworkLimits, ProjectEvidenceIndex, RawFrameworkFact, RawNodeRecord,
    make_id,
};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::{RouteStageRole, resolve_and_publish_framework_routes};

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
    let mut extraction = Extraction::default();
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
        let mut source = engine.extract_source(Path::new(path), source.as_bytes())?;
        extraction.nodes.append(&mut source.nodes);
        extraction.edges.append(&mut source.edges);
        extraction
            .framework_facts
            .append(&mut source.framework_facts);
    }

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
    let mut extraction = Extraction::default();
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
        let mut source = engine.extract_source(Path::new(path), source.as_bytes())?;
        extraction.nodes.append(&mut source.nodes);
        extraction.edges.append(&mut source.edges);
        extraction
            .framework_facts
            .append(&mut source.framework_facts);
    }

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
