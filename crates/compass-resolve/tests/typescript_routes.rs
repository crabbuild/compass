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
    assert_eq!(routes(&extraction).count(), 2);
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
    assert!(route_shapes.contains(&("nestjs-graphql", "QUERY", "/graphql/user")));
    assert!(route_shapes.contains(&("nestjs-graphql", "MUTATION", "/graphql/createUser")));

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
        assert!(resolved.iter().all(|route| {
            route.state == ResolutionState::Exact
                && route
                    .stages
                    .last()
                    .is_some_and(|stage| stage.role == RouteStageRole::Handler)
        }));
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
        assert!(extraction.edges.iter().any(|edge| {
            edge.string("relation") == "routes_to"
                && edge.string("_origin") == "convention"
                && !edge.string("rule").is_empty()
        }));
    }
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
