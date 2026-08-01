use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Engine, Extraction, FrameworkLimits, RawFrameworkFact};
use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::{
    FrameworkResolutionError, RouteStageRole, resolve_and_publish_framework_routes, resolve_routes,
};
use compass_resolve::resolve;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/code-graph/routes/python")
        .join(name)
}

fn extract(path: &Path, fixture_name: &str) -> Result<Extraction, Box<dyn std::error::Error>> {
    let source = fs::read(fixture(fixture_name))?;
    Ok(Engine::default().extract_source(path, &source)?)
}

fn extract_resolved(
    path: &Path,
    fixture_name: &str,
) -> Result<Extraction, Box<dyn std::error::Error>> {
    let source = fs::read(fixture(fixture_name))?;
    let extraction = Engine::default().extract_source(path, &source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source)?,
    )]);
    Ok(resolve(&[extraction], &sources))
}

#[test]
fn django_flask_and_fastapi_shapes_emit_framework_specific_route_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let django = extract(Path::new("project/urls.py"), "django_urls.py")?;
    let django_routes = routes(&django);
    assert!(django_routes.iter().any(|route| {
        route.framework == "django"
            && route.normalized_path == "/users/{user_id}"
            && route.handler_reference == "views.user_detail"
    }));
    assert!(django_routes.iter().any(|route| {
        route
            .detail
            .get("include")
            .and_then(serde_json::Value::as_str)
            == Some("project.admin.urls")
    }));
    assert!(
        django_routes
            .iter()
            .any(|route| route.handler_reference == "views.AccountView.as_view()")
    );

    let flask = extract(Path::new("app.py"), "flask_app.py")?;
    let flask_routes = routes(&flask);
    assert!(flask_routes.iter().any(|route| {
        route.framework == "flask" && route.operation == "ANY" && route.normalized_path == "/health"
    }));
    let methods = flask_routes
        .iter()
        .filter(|route| route.normalized_path == "/api/users/<user_id>")
        .map(|route| route.operation.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(methods, HashSet::from(["GET", "PATCH"]));

    let fastapi = extract(Path::new("api.py"), "fastapi_app.py")?;
    let fastapi_routes = routes(&fastapi);
    let create = fastapi_routes
        .iter()
        .find(|route| route.normalized_path == "/v1/users")
        .ok_or("missing FastAPI router route")?;
    assert_eq!(create.operation, "POST");
    assert_eq!(create.middleware_references, vec!["authenticate"]);
    Ok(())
}

#[test]
fn python_routes_resolve_handlers_and_dependencies_but_not_near_matches()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fastapi = extract_resolved(Path::new("api.py"), "fastapi_app.py")?;
    let resolved = resolve_and_publish_framework_routes(&mut fastapi, FrameworkLimits::default())?;
    let create = resolved
        .iter()
        .find(|route| route.route.normalized_path == "/v1/users")
        .ok_or("missing create route")?;
    assert_eq!(create.state, ResolutionState::Exact);
    assert_eq!(
        create
            .stages
            .iter()
            .map(|stage| stage.role)
            .collect::<Vec<_>>(),
        vec![RouteStageRole::Middleware, RouteStageRole::Handler]
    );
    assert!(
        fastapi
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "routes_to")
    );

    let near = extract(Path::new("near_matches.py"), "near_matches.py")?;
    assert!(routes(&near).is_empty());
    assert!(resolve_routes(&near, FrameworkLimits::default())?.is_empty());
    Ok(())
}

#[test]
fn django_includes_compose_paths_with_cycle_safe_resolution()
-> Result<(), Box<dyn std::error::Error>> {
    let root_source = br#"
from django.urls import include, path
urlpatterns = [path("api/", include("project.users.urls"))]
"#;
    let child_source = br#"
from django.urls import path
def detail(request, user_id): return None
urlpatterns = [path("users/<int:user_id>/", detail)]
"#;
    let mut engine = Engine::default();
    let root = engine.extract_source(Path::new("project/urls.py"), root_source)?;
    let child = engine.extract_source(Path::new("project/users/urls.py"), child_source)?;
    let sources = HashMap::from([
        (
            "project/urls.py".to_owned(),
            String::from_utf8(root_source.to_vec())?,
        ),
        (
            "project/users/urls.py".to_owned(),
            String::from_utf8(child_source.to_vec())?,
        ),
    ]);
    let merged = resolve(&[root, child], &sources);
    let resolved = resolve_routes(&merged, FrameworkLimits::default())?;
    assert!(resolved.iter().any(|route| {
        route.route.normalized_path == "/api/users/{user_id}"
            && route.state == ResolutionState::Exact
            && route.route.detail.contains_key("include_anchor")
    }));
    assert!(
        !resolved
            .iter()
            .any(|route| route.route.normalized_path == "/users/{user_id}")
    );
    Ok(())
}

#[test]
fn django_relative_module_import_resolves_class_based_view_route()
-> Result<(), Box<dyn std::error::Error>> {
    let urls_source = br#"
from django.urls import path
from . import views

urlpatterns = [
    path("profile/", views.ProfileView.as_view()),
]
"#;
    let views_source = br#"
class ProfileView:
    pass
"#;
    let mut engine = Engine::default();
    let urls = engine.extract_source(Path::new("accounts/urls.py"), urls_source)?;
    let views = engine.extract_source(Path::new("accounts/views.py"), views_source)?;
    let sources = HashMap::from([
        (
            "accounts/urls.py".to_owned(),
            String::from_utf8(urls_source.to_vec())?,
        ),
        (
            "accounts/views.py".to_owned(),
            String::from_utf8(views_source.to_vec())?,
        ),
    ]);

    let extraction = resolve(&[urls, views], &sources);
    let route = resolve_routes(&extraction, FrameworkLimits::default())?
        .into_iter()
        .find(|route| route.route.handler_reference.contains("ProfileView"))
        .ok_or("missing profile route")?;

    assert_eq!(route.state, ResolutionState::Exact, "{route:#?}");
    let target = route
        .candidates
        .first()
        .and_then(|candidate| {
            extraction
                .nodes
                .iter()
                .find(|node| node.id == candidate.node_id)
        })
        .ok_or("missing route target")?;
    assert_eq!(target.string("symbol_kind"), "class");
    assert_eq!(target.label(), "ProfileView");
    Ok(())
}

#[test]
fn django_relative_symbol_import_resolves_class_based_view_route()
-> Result<(), Box<dyn std::error::Error>> {
    let urls_source = br#"
from django.urls import path
from .views import ProfileView

urlpatterns = [
    path("profile/", ProfileView.as_view()),
]
"#;
    let views_source = br#"
class ProfileView:
    pass
"#;
    let mut engine = Engine::default();
    let urls = engine.extract_source(Path::new("accounts/urls.py"), urls_source)?;
    let views = engine.extract_source(Path::new("accounts/views.py"), views_source)?;
    let sources = HashMap::from([
        (
            "accounts/urls.py".to_owned(),
            String::from_utf8(urls_source.to_vec())?,
        ),
        (
            "accounts/views.py".to_owned(),
            String::from_utf8(views_source.to_vec())?,
        ),
    ]);

    let extraction = resolve(&[urls, views], &sources);
    let route = resolve_routes(&extraction, FrameworkLimits::default())?
        .into_iter()
        .find(|route| route.route.handler_reference.contains("ProfileView"))
        .ok_or("missing profile route")?;

    assert_eq!(route.state, ResolutionState::Exact, "{route:#?}");
    assert_eq!(route.candidates.len(), 1, "{route:#?}");
    Ok(())
}

#[test]
fn django_include_cycles_stop_and_depth_overflow_fails_explicitly()
-> Result<(), Box<dyn std::error::Error>> {
    let source = |target: &str| {
        format!(
            "from django.urls import include, path\nurlpatterns = [path(\"x/\", include(\"{target}\"))]\n"
        )
    };
    let mut engine = Engine::default();
    let a = engine.extract_source(
        Path::new("project/a/urls.py"),
        source("project.b.urls").as_bytes(),
    )?;
    let b = engine.extract_source(
        Path::new("project/b/urls.py"),
        source("project.a.urls").as_bytes(),
    )?;
    let mut cycle = Extraction::default();
    for mut extraction in [a, b] {
        cycle.nodes.append(&mut extraction.nodes);
        cycle.edges.append(&mut extraction.edges);
        cycle
            .framework_facts
            .append(&mut extraction.framework_facts);
    }
    assert!(
        resolve_routes(&cycle, FrameworkLimits::default())?.is_empty(),
        "an include cycle must not invent a binding"
    );

    let a = engine.extract_source(
        Path::new("project/a/urls.py"),
        source("project.b.urls").as_bytes(),
    )?;
    let b = engine.extract_source(
        Path::new("project/b/urls.py"),
        source("project.c.urls").as_bytes(),
    )?;
    let c = engine.extract_source(
        Path::new("project/c/urls.py"),
        b"from django.urls import path\ndef final(request): return None\nurlpatterns = [path(\"final/\", final)]\n",
    )?;
    let mut chain = Extraction::default();
    for mut extraction in [a, b, c] {
        chain.nodes.append(&mut extraction.nodes);
        chain.edges.append(&mut extraction.edges);
        chain
            .framework_facts
            .append(&mut extraction.framework_facts);
    }
    let limits = FrameworkLimits {
        max_include_depth: 1,
        ..FrameworkLimits::default()
    };
    assert!(matches!(
        resolve_routes(&chain, limits),
        Err(FrameworkResolutionError::Limit(error))
            if error.limit == "max_include_depth"
    ));
    Ok(())
}

#[test]
fn collection_resolution_publishes_python_routes_into_v1() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = fs::read(fixture("fastapi_app.py"))?;
    fs::write(root.join("api.py"), &source)?;
    let extraction = Engine::default().extract_source(Path::new("api.py"), &source)?;
    let sources = HashMap::from([(
        "api.py".to_owned(),
        String::from_utf8(source).map_err(|error| error.utf8_error())?,
    )]);
    let mut extraction = resolve(&[extraction], &sources);
    let ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    extraction
        .edges
        .retain(|edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str()));
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test")?;
    let graph = normalize_v1(extraction, evidence)?;
    assert!(graph.nodes.iter().any(|node| {
        node.kind == NodeKind::Route && node.framework.as_deref() == Some("fastapi")
    }));
    assert!(
        graph
            .links
            .iter()
            .any(|edge| edge.kind == EdgeKind::RoutesTo)
    );
    Ok(())
}

fn routes(extraction: &Extraction) -> Vec<&compass_languages::RawRouteFact> {
    extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) => None,
        })
        .collect()
}
