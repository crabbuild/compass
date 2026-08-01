use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use compass_languages::{Engine, Extraction, FrameworkLimits};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::resolve_and_publish_framework_routes;

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/code-graph/routes")
        .join(relative)
}

fn extract(relative: &str) -> Result<Extraction, Box<dyn Error>> {
    let path = fixture(relative);
    let extraction = Engine::default().extract(&path)?;
    if let Some(error) = &extraction.error {
        return Err(error.clone().into());
    }
    Ok(extraction)
}

fn resolved(
    relative: &str,
) -> Result<Vec<compass_resolve::frameworks::ResolvedRoute>, Box<dyn Error>> {
    let path = fixture(relative);
    let source = fs::read_to_string(&path)?;
    let extracted = extract(relative)?;
    let sources = HashMap::from([(path.to_string_lossy().into_owned(), source)]);
    let mut extraction = compass_resolve::resolve(&[extracted], &sources);
    Ok(resolve_and_publish_framework_routes(
        &mut extraction,
        FrameworkLimits::default(),
    )?)
}

#[test]
fn go_frameworks_require_imports_and_resolve_handlers() -> Result<(), Box<dyn Error>> {
    let gin = resolved("go/gin.go")?;
    assert!(gin.iter().any(|route| {
        route.route.operation == "GET"
            && route.route.normalized_path == "/api/users"
            && route.route.middleware_references == ["auth"]
            && route.state == ResolutionState::Exact
    }));

    let chi = resolved("go/chi.go")?;
    assert!(chi.iter().any(|route| {
        route.route.framework == "chi"
            && route.route.normalized_path == "/users/{id}"
            && route.state == ResolutionState::Exact
    }));

    let gorilla = resolved("go/gorilla.go")?;
    assert_eq!(gorilla.len(), 2);
    assert!(
        gorilla
            .iter()
            .all(|route| route.state == ResolutionState::Exact)
    );
    assert!(extract("go/near_matches.go")?.framework_facts.is_empty());
    Ok(())
}

#[test]
fn rust_calls_and_attributes_resolve_with_framework_guards() -> Result<(), Box<dyn Error>> {
    for (fixture, expected_framework) in [
        ("rust/axum.rs", "axum"),
        ("rust/actix.rs", "actix"),
        ("rust/rocket.rs", "rocket"),
    ] {
        let routes = resolved(fixture)?;
        assert!(!routes.is_empty(), "missing {expected_framework} route");
        assert!(routes.iter().all(|route| {
            route.route.framework == expected_framework && route.state == ResolutionState::Exact
        }));
    }
    assert!(extract("rust/near_matches.rs")?.framework_facts.is_empty());
    Ok(())
}

#[test]
fn aspnet_composes_controller_and_action_templates() -> Result<(), Box<dyn Error>> {
    let routes = resolved("csharp/AspNetController.cs")?;
    assert!(routes.iter().any(|route| {
        route.route.operation == "GET"
            && route.route.normalized_path == "/api/Users/{id}"
            && route.route.handler_reference == "UsersController.Show"
            && route.state == ResolutionState::Exact
    }));
    assert!(routes.iter().any(|route| {
        route.route.operation == "POST"
            && route.route.normalized_path == "/api/Users"
            && route.state == ResolutionState::Exact
    }));
    assert!(extract("csharp/NearMatches.cs")?.framework_facts.is_empty());
    Ok(())
}

#[test]
fn vapor_segmented_routes_resolve_explicit_handlers() -> Result<(), Box<dyn Error>> {
    let routes = resolved("swift/VaporRoutes.swift")?;
    assert_eq!(routes.len(), 3);
    assert!(
        routes
            .iter()
            .filter(|route| { route.route.normalized_path == "/api/users" })
            .all(|route| route.state == ResolutionState::Exact)
    );
    assert!(routes.iter().any(|route| {
        route.route.normalized_path == "/api/health"
            && route.state == ResolutionState::Unresolved
            && route
                .route
                .detail
                .get("opaque_handler")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    }));
    assert!(
        extract("swift/NearMatches.swift")?
            .framework_facts
            .is_empty()
    );
    Ok(())
}
