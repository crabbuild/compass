use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use compass_languages::{
    Engine, Extraction, FrameworkLimits, RawFrameworkFact, RawNodeRecord, make_id,
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
    assert!(routes(&vue).any(|route| {
        route.framework == "vue-router"
            && route.normalized_path == "/users/{userId}"
            && route.handler_reference == "UserPage"
    }));
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
            && route
                .stages
                .last()
                .is_some_and(|stage| stage.target.ends_with("accountpage"))
    }));

    let near = source_extract(Path::new("src/not-routes.ts"), "near-matches.ts")?;
    assert_eq!(routes(&near).count(), 0);
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
            .map(|route| route.operation.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(
            operations,
            expected_operations.into_iter().collect(),
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
        assert!(extraction.nodes.iter().any(|node| {
            node.string("symbol_kind") == "component"
                && node.string("_origin") == "convention"
                && !node.string("rule").is_empty()
        }));
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
