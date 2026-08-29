use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use compass_languages::{Engine, Extraction, FrameworkLimits, RawFrameworkFact};
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

    for (framework, source, expected_path) in [
        (
            "echo",
            br#"package web
import "github.com/labstack/echo/v4"
func show(c echo.Context) error { return nil }
func routes() {
  e := echo.New()
  api := e.Group("/api")
  api.GET("/users/:id", show)
}
"#
            .as_slice(),
            "/api/users/{id}",
        ),
        (
            "fiber",
            br#"package web
import "github.com/gofiber/fiber/v3"
func auth(c fiber.Ctx) error { return c.Next() }
func show(c fiber.Ctx) error { return nil }
func routes() {
  app := fiber.New()
  api := app.Group("/api")
  api.Get("/users/:id", auth, show)
}
"#
            .as_slice(),
            "/api/users/{id}",
        ),
    ] {
        let extraction = Engine::default().extract_source(Path::new("routes.go"), source)?;
        let route = extraction
            .framework_facts
            .iter()
            .find_map(|fact| match fact {
                RawFrameworkFact::Route(route)
                    if route.framework == framework && route.normalized_path == expected_path =>
                {
                    Some(route)
                }
                RawFrameworkFact::Route(_)
                | RawFrameworkFact::Domain(_)
                | RawFrameworkFact::Annotation(_) => None,
            })
            .ok_or("missing Echo/Fiber route")?;
        assert_eq!(route.handler_reference, "show");
        if framework == "fiber" {
            assert_eq!(route.middleware_references, ["auth"]);
        }
    }

    let near_match = Engine::default().extract_source(
        Path::new("routes.go"),
        br#"package web
func routes() {
  e := echo.New()
  e.GET("/invented", show)
}
"#,
    )?;
    assert!(near_match.framework_facts.is_empty());
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
    let multiline = Engine::default().extract_source(
        Path::new("routes.rs"),
        br#"use rocket::get;
#[get(
    "/multiline"
)]
fn multiline() {}
"#,
    )?;
    assert!(multiline.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            RawFrameworkFact::Route(route)
                if route.framework == "rocket"
                    && route.operation == "GET"
                    && route.normalized_path == "/multiline"
                    && route.handler_reference == "multiline"
        )
    }));
    let commented = Engine::default().extract_source(
        Path::new("routes.rs"),
        br#"use rocket::get;
// #[get("/commented")]
// fn commented() {}
"#,
    )?;
    assert!(commented.framework_facts.is_empty());
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
    assert!(routes.iter().any(|route| {
        route.route.operation == "GET"
            && route.route.normalized_path == "/status"
            && route.route.handler_reference == "UsersController.Status"
            && route.state == ResolutionState::Exact
    }));
    assert!(routes.iter().any(|route| {
        route.route.operation == "GET"
            && route.route.normalized_path == "/api/Users/health"
            && route.route.handler_reference == "UsersController.Health"
            && route.state == ResolutionState::Exact
    }));
    assert!(extract("csharp/NearMatches.cs")?.framework_facts.is_empty());

    let minimal = Engine::default().extract_source(
        Path::new("Program.cs"),
        br#"var builder = WebApplication.CreateBuilder(args);
var app = builder.Build();
var api = app.MapGroup("/api");
var users = api.MapGroup("/users");
users.MapGet("/{id:int}", UserHandlers.Show);
app.MapPost("/users", (User user) => Results.Created($"/users/{user.Id}", user));
app.Run();
"#,
    )?;
    let minimal_routes = minimal
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) if route.framework == "aspnet" => Some(route),
            RawFrameworkFact::Route(_)
            | RawFrameworkFact::Domain(_)
            | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    let exact = minimal_routes
        .iter()
        .find(|route| route.normalized_path == "/api/users/{id:int}")
        .ok_or("missing ASP.NET Minimal API route group")?;
    assert_eq!(exact.operation, "GET");
    assert_eq!(exact.handler_reference, "UserHandlers.Show");
    let inline = minimal_routes
        .iter()
        .find(|route| route.normalized_path == "/users")
        .ok_or("missing ASP.NET Minimal API inline route")?;
    assert_eq!(inline.operation, "POST");
    assert!(
        inline
            .handler_reference
            .starts_with("opaque_minimal_handler_at_")
    );
    assert_eq!(
        inline.detail.get("opaque_handler"),
        Some(&serde_json::Value::Bool(true))
    );

    let near_match = Engine::default().extract_source(
        Path::new("Program.cs"),
        br#"var builder = WebApplication.CreateBuilder(args);
var app = builder.Build();
// app.MapGet("/invented", Handler);
/* app.MapPost("/also-invented", Handler); */
"#,
    )?;
    assert!(near_match.framework_facts.is_empty());
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

#[test]
fn gorilla_http_method_constants_and_comments_remain_precise() -> Result<(), Box<dyn Error>> {
    let mut engine = Engine::default();
    let extraction = engine.extract_source(
        Path::new("routes.go"),
        br#"package routes
import (
  "net/http"
  "github.com/gorilla/mux"
)
func show(w http.ResponseWriter, r *http.Request) {}
func configure(r *mux.Router) {
  // r.HandleFunc("/comment", show).Methods("GET")
  r.HandleFunc("/users", show).Methods(http.MethodGet)
}
"#,
    )?;
    let routes = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].operation, "GET");
    assert_eq!(routes[0].normalized_path, "/users");
    Ok(())
}

#[test]
fn native_composition_and_multiline_registrations_keep_each_method_and_prefix()
-> Result<(), Box<dyn Error>> {
    let axum = Engine::default().extract_source(
        Path::new("routes.rs"),
        br#"use axum::{Router, routing::{get, post}};
async fn show() {}
async fn create() {}
fn router() -> Router {
  Router::new()
    .route("/users/:id", get(show))
    .route("/users", post(create))
    .nest("/api", Router::new().route("/users", get(show).post(create)))
}
"#,
    )?;
    let axum_routes = axum
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    assert!(
        axum_routes
            .iter()
            .any(|route| { route.operation == "GET" && route.normalized_path == "/api/users" })
    );
    assert!(
        axum_routes
            .iter()
            .any(|route| { route.operation == "POST" && route.normalized_path == "/api/users" })
    );
    assert!(
        axum_routes
            .iter()
            .any(|route| { route.operation == "GET" && route.normalized_path == "/users/:id" })
    );
    assert!(
        axum_routes
            .iter()
            .any(|route| { route.operation == "POST" && route.normalized_path == "/users" })
    );

    let go = Engine::default().extract_source(
        Path::new("routes.go"),
        br#"package routes
import "github.com/gin-gonic/gin"
func auth() {}
func show() {}
func configure(r *gin.Engine) {
  r.GET(
    "/users",
    auth(),
    show,
  )
}
"#,
    )?;
    let go_route = go
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .ok_or("missing multiline Go route")?;
    assert_eq!(go_route.normalized_path, "/users");
    assert_eq!(go_route.handler_reference, "show");
    assert_eq!(go_route.middleware_references, ["auth()"]);

    let chi = Engine::default().extract_source(
        Path::new("routes.go"),
        br#"package routes
import "github.com/go-chi/chi/v5"
func list(w http.ResponseWriter, r *http.Request) {}
func configure(r chi.Router) {
  r.Route("/api", func(r chi.Router) {
    r.Get("/items", list)
  })
}
"#,
    )?;
    let chi_route = chi
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .ok_or("missing chi closure route")?;
    assert_eq!(chi_route.framework, "chi");
    assert_eq!(chi_route.normalized_path, "/api/items");

    let gorilla = Engine::default().extract_source(
        Path::new("routes.go"),
        br#"package routes
import "github.com/gorilla/mux"
func show(w http.ResponseWriter, r *http.Request) {}
func configure(r *mux.Router) {
  r.PathPrefix("/api").Subrouter().HandleFunc("/items", show).Methods("GET")
}
"#,
    )?;
    let gorilla_route = gorilla
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .ok_or("missing gorilla subrouter route")?;
    assert_eq!(gorilla_route.framework, "gorilla");
    assert_eq!(gorilla_route.normalized_path, "/api/items");
    assert_eq!(gorilla_route.operation, "GET");

    let actix = Engine::default().extract_source(
        Path::new("routes.rs"),
        br#"use actix_web::{web, App};
async fn show() {}
async fn create() {}
fn configure() -> App {
  App::new().service(web::scope("/api").service(
    web::resource("/items")
      .route(web::get().to(show))
      .route(web::post().to(create))
  ))
}
"#,
    )?;
    let actix_routes = actix
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    assert!(actix_routes.iter().any(|route| {
        route.framework == "actix"
            && route.operation == "GET"
            && route.normalized_path == "/api/items"
    }));
    assert!(actix_routes.iter().any(|route| {
        route.framework == "actix"
            && route.operation == "POST"
            && route.normalized_path == "/api/items"
    }));
    Ok(())
}

#[test]
fn axum_composes_nested_router_factories_across_rust_modules() -> Result<(), Box<dyn Error>> {
    let sources = [
        (
            "src/http/server.rs",
            r#"use crate::auth::{auth, unused};
use axum::{Router, middleware, routing};
async fn health() {}
async fn public() {}
async fn global_trace() {}
fn create_router() -> Router {
    let repository_router = repositories::create_router();
    let authenticated_router = Router::new()
        .nest("/repository", repository_router)
        .route_layer(middleware::from_fn_with_state((), auth))
        .route("/public", routing::get(public));
    let unauthenticated_router = Router::new()
        .route("/health_check", routing::get(health).with_state(()));
    let mut router = Router::new()
        .merge(unauthenticated_router)
        .nest("/v1", authenticated_router);
    router = router.nest("/v1/presigned", presigned::create_router());
    router.layer(middleware::from_fn(global_trace))
}
"#,
        ),
        (
            "src/auth.rs",
            r#"async fn auth() {}
"#,
        ),
        (
            "src/http/repositories.rs",
            r#"use axum::Router;
fn create_router() -> Router {
    let repository_router = repository::create_router();
    Router::new().nest("/{repository_id}", repository_router)
}
"#,
        ),
        (
            "src/http/repositories/repository.rs",
            r#"use axum::Router;
fn create_router() -> Router {
    let contents_router = contents::create_router();
    Router::new().nest("/content", contents_router)
}
"#,
        ),
        (
            "src/http/repositories/repository/contents.rs",
            r#"use axum::{Router, routing};
async fn put_handler() {}
fn create_router() -> Router {
    let content_router = content::create_router();
    Router::new()
        .route("/", routing::put(put_handler))
        .nest("/{address}", content_router)
}
"#,
        ),
        (
            "src/http/repositories/repository/contents/content.rs",
            r#"use axum::{Router, middleware, routing};
async fn get_handler() {}
async fn presign_handler() {}
async fn trace() {}
fn create_router() -> Router {
    Router::new()
        .route("/", routing::get(get_handler))
        .route("/presign", routing::post(presign_handler))
        .layer(middleware::from_fn(trace))
}
"#,
        ),
        (
            "src/http/presigned.rs",
            r#"use axum::Router;
fn create_router() -> Router {
    repository::create_router()
}
"#,
        ),
        (
            "src/http/presigned/repository.rs",
            r#"use axum::Router;
fn create_router() -> Router {
    Router::new().nest("/{repository_id}", redeem::create_router())
}
"#,
        ),
        (
            "src/http/presigned/redeem.rs",
            r#"use axum::{Router, routing};
async fn handler() {}
fn create_router() -> Router {
    Router::new().route("/{address}", routing::get(handler))
}
"#,
        ),
    ];
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut source_map = HashMap::new();
    for (path, source) in sources {
        extractions.push(engine.extract_source(Path::new(path), source.as_bytes())?);
        source_map.insert(path.to_owned(), source.to_owned());
    }
    let mut extraction = compass_resolve::resolve(&extractions, &source_map);
    let routes = resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    let expected = [
        ("GET", "/health_check", "health"),
        ("GET", "/v1/public", "public"),
        (
            "PUT",
            "/v1/repository/{repository_id}/content",
            "put_handler",
        ),
        (
            "GET",
            "/v1/repository/{repository_id}/content/{address}",
            "get_handler",
        ),
        (
            "POST",
            "/v1/repository/{repository_id}/content/{address}/presign",
            "presign_handler",
        ),
        ("GET", "/v1/presigned/{repository_id}/{address}", "handler"),
    ];
    for (operation, path, handler) in expected {
        assert!(
            routes.iter().any(|route| {
                route.route.operation == operation
                    && route.route.normalized_path == path
                    && route.route.handler_reference == handler
                    && route.state == ResolutionState::Exact
            }),
            "missing exact Axum route {operation} {path} -> {handler}"
        );
    }
    let get_content = routes
        .iter()
        .find(|route| {
            route.route.operation == "GET"
                && route.route.normalized_path == "/v1/repository/{repository_id}/content/{address}"
        })
        .ok_or("missing composed content route")?;
    assert_eq!(
        get_content.route.middleware_references,
        ["server.global_trace", "auth.auth", "content.trace"]
    );
    assert!(
        get_content
            .stages
            .iter()
            .all(|stage| { stage.state == ResolutionState::Exact && stage.target.is_some() }),
        "unexpected Axum stages: {:#?}",
        get_content.stages
    );
    let public = routes
        .iter()
        .find(|route| route.route.normalized_path == "/v1/public")
        .ok_or("missing public route registered after route middleware")?;
    assert_eq!(public.route.middleware_references, ["server.global_trace"]);
    Ok(())
}

#[test]
fn axum_does_not_compose_ambiguous_router_module_references() -> Result<(), Box<dyn Error>> {
    let sources = [
        (
            "src/foo.rs",
            r#"use axum::Router;
fn create_router() -> Router {
    Router::new().nest("/invented", bar::create_router())
}
"#,
        ),
        (
            "src/bar.rs",
            r#"use axum::{Router, routing};
async fn sibling_handler() {}
fn create_router() -> Router {
    Router::new().route("/child", routing::get(sibling_handler))
}
"#,
        ),
        (
            "src/foo/bar.rs",
            r#"use axum::{Router, routing};
async fn child_handler() {}
fn create_router() -> Router {
    Router::new().route("/child", routing::get(child_handler))
}
"#,
        ),
    ];
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut source_map = HashMap::new();
    for (path, source) in sources {
        extractions.push(engine.extract_source(Path::new(path), source.as_bytes())?);
        source_map.insert(path.to_owned(), source.to_owned());
    }
    let mut extraction = compass_resolve::resolve(&extractions, &source_map);
    let routes = resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;

    assert!(
        routes
            .iter()
            .all(|route| route.route.normalized_path != "/invented/child"),
        "ambiguous Rust module candidates must not invent a composed route"
    );
    assert_eq!(
        routes
            .iter()
            .filter(|route| route.route.normalized_path == "/child")
            .count(),
        2
    );
    Ok(())
}

#[test]
fn axum_router_cycles_remain_local_and_bounded() -> Result<(), Box<dyn Error>> {
    let source = r#"use axum::{Router, routing};
async fn handler() {}
fn create_router() -> Router {
    Router::new()
        .route("/loop", routing::get(handler))
        .nest("/cycle", create_router())
}
"#;
    let extracted =
        Engine::default().extract_source(Path::new("src/routes.rs"), source.as_bytes())?;
    let mut extraction = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([("src/routes.rs".to_owned(), source.to_owned())]),
    );
    let routes = resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;

    assert!(
        routes
            .iter()
            .any(|route| route.route.normalized_path == "/loop")
    );
    assert!(
        routes
            .iter()
            .all(|route| route.route.normalized_path != "/cycle/loop")
    );
    Ok(())
}

#[test]
fn vapor_root_and_direct_grouped_routes_are_visible() -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("VaporRoutes.swift"),
        br#"import Vapor
func index(_ request: Request) async throws -> String { "index" }
func users(_ request: Request) async throws -> String { "users" }
func routes(_ app: Application) throws {
  app.get(use: index)
  app.grouped("api").get("users", use: users)
  app.on(.OPTIONS, "health", use: users)
  app.group("v1") { grouped in
    grouped.get("items", use: users)
  }
}
"#,
    )?;
    let routes = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    assert!(routes.iter().any(|route| route.normalized_path == "/"));
    assert!(
        routes
            .iter()
            .any(|route| route.normalized_path == "/api/users")
    );
    assert!(
        routes
            .iter()
            .any(|route| { route.operation == "OPTIONS" && route.normalized_path == "/health" })
    );
    assert!(routes.iter().any(|route| {
        route.normalized_path == "/v1/items" && route.handler_reference == "users"
    }));
    Ok(())
}
