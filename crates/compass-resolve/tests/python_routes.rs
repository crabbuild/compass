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
        route.framework == "flask" && route.operation == "GET" && route.normalized_path == "/health"
    }));
    let methods = flask_routes
        .iter()
        .filter(|route| route.normalized_path == "/v2/api/users/<user_id>")
        .map(|route| route.operation.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(methods, HashSet::from(["GET", "PATCH"]));

    let fastapi = extract(Path::new("api.py"), "fastapi_app.py")?;
    let fastapi_routes = routes(&fastapi);
    let create = fastapi_routes
        .iter()
        .find(|route| route.normalized_path == "/api/v1/users")
        .ok_or("missing FastAPI router route")?;
    assert_eq!(create.operation, "POST");
    assert!(create.stages.iter().any(|stage| {
        stage.role == compass_languages::RawRouteStageRole::Dependency
            && stage.reference.ends_with("authenticate")
    }));
    Ok(())
}

#[test]
fn python_route_decorators_and_django_calls_accept_named_path_arguments()
-> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::default();
    let django = engine.extract_source(
        Path::new("project/urls.py"),
        br#"from django.conf.urls import url
from django.urls import path
from . import views
urlpatterns = [
    path(route="named/", view=views.named),
    url(r"^legacy/$", "project.views.named"),
]
"#,
    )?;
    assert!(routes(&django).iter().any(|route| {
        route.framework == "django"
            && route.normalized_path == "/named"
            && route.handler_reference == "views.named"
    }));
    assert!(routes(&django).iter().any(|route| {
        route.framework == "django"
            && route.raw_path == "^legacy/$"
            && route.handler_reference == "project.views.named"
    }));

    let flask = engine.extract_source(
        Path::new("app.py"),
        br#"from flask import Flask
app = Flask(__name__)
@app.route(rule="/named", methods=["POST"])
def named(): return None
"#,
    )?;
    assert!(routes(&flask).iter().any(|route| {
        route.framework == "flask" && route.operation == "POST" && route.normalized_path == "/named"
    }));

    let fastapi = engine.extract_source(
        Path::new("api.py"),
        br#"from fastapi import FastAPI
app = FastAPI()
@app.patch(path="/named")
def named(): return None
@app.get("/search/a=b")
def search(): return None
"#,
    )?;
    assert!(routes(&fastapi).iter().any(|route| {
        route.framework == "fastapi"
            && route.operation == "PATCH"
            && route.normalized_path == "/named"
    }));
    assert!(routes(&fastapi).iter().any(|route| {
        route.framework == "fastapi"
            && route.operation == "GET"
            && route.normalized_path == "/search/a=b"
    }));
    Ok(())
}

#[test]
fn python_routes_resolve_handlers_and_dependencies_but_not_near_matches()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fastapi = extract_resolved(Path::new("api.py"), "fastapi_app.py")?;
    let resolved = resolve_and_publish_framework_routes(&mut fastapi, FrameworkLimits::default())?;
    let create = resolved
        .iter()
        .find(|route| route.route.normalized_path == "/api/v1/users")
        .ok_or("missing create route")?;
    assert_eq!(create.state, ResolutionState::Exact);
    assert_eq!(
        create
            .stages
            .iter()
            .map(|stage| stage.role)
            .collect::<Vec<_>>(),
        vec![RouteStageRole::Dependency, RouteStageRole::Handler]
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
    let mut merged = resolve(&[root, child], &sources);
    let resolved = resolve_routes(&merged, FrameworkLimits::default())?;
    let route = resolved
        .iter()
        .find(|route| route.route.normalized_path == "/api/users/{user_id}")
        .ok_or("missing expanded Django include route")?;
    assert_eq!(route.state, ResolutionState::Exact);
    assert!(route.route.detail.contains_key("include_anchor"));
    let handler = route
        .stages
        .iter()
        .find(|stage| stage.role == RouteStageRole::Handler)
        .ok_or("missing expanded Django handler stage")?;
    let include_source = b"path(\"api/\", include(\"project.users.urls\"))";
    let include_start = root_source
        .windows(include_source.len())
        .position(|candidate| candidate == include_source)
        .ok_or("missing include source range")?;
    assert_eq!(handler.anchor.source_file, "project/urls.py");
    assert_eq!(handler.anchor.start_byte, u64::try_from(include_start)?);
    assert_eq!(
        handler.anchor.end_byte,
        u64::try_from(include_start.saturating_add(include_source.len()))?
    );
    assert!(
        !resolved
            .iter()
            .any(|route| route.route.normalized_path == "/users/{user_id}")
    );
    resolve_and_publish_framework_routes(&mut merged, FrameworkLimits::default())?;
    let edge = merged
        .edges
        .iter()
        .find(|edge| edge.string("relation") == "routes_to")
        .ok_or("missing published Django include edge")?;
    assert_eq!(edge.string("source_file"), "project/urls.py");
    assert_eq!(edge.string("extractor"), "compass.frameworks.django");
    assert_eq!(edge.string("_origin"), "ast");
    let published_anchor = edge
        .attributes
        .get("source_anchor")
        .and_then(serde_json::Value::as_object)
        .ok_or("missing published include source anchor")?;
    assert_eq!(
        published_anchor
            .get("startByte")
            .and_then(serde_json::Value::as_u64),
        Some(u64::try_from(include_start)?)
    );
    assert_eq!(
        published_anchor
            .get("endByte")
            .and_then(serde_json::Value::as_u64),
        Some(u64::try_from(
            include_start.saturating_add(include_source.len())
        )?)
    );
    Ok(())
}

#[test]
fn django_included_class_view_with_arguments_resolves_only_exact_dotted_receiver()
-> Result<(), Box<dyn std::error::Error>> {
    let root_source = br#"
from django.urls import include, path
urlpatterns = [path("admin/doc/", include("django.contrib.admindocs.urls"))]
"#;
    let child_source = br#"
from django.contrib.admindocs import views
from django.urls import path
urlpatterns = [
    path("", views.BaseAdminDocsView.as_view(template_name="admin_doc/index.html")),
    path("near/", views.BaseAdminDocsView.as_views(template_name="admin_doc/index.html")),
    path("dynamic/", factory().as_view(template_name="admin_doc/index.html")),
]
"#;
    let views_source = br#"
class BaseAdminDocsView:
    pass
"#;
    let mut engine = Engine::default();
    let root = engine.extract_source(Path::new("project/urls.py"), root_source)?;
    let child =
        engine.extract_source(Path::new("django/contrib/admindocs/urls.py"), child_source)?;
    let views =
        engine.extract_source(Path::new("django/contrib/admindocs/views.py"), views_source)?;
    let sources = HashMap::from([
        (
            "project/urls.py".to_owned(),
            String::from_utf8(root_source.to_vec())?,
        ),
        (
            "django/contrib/admindocs/urls.py".to_owned(),
            String::from_utf8(child_source.to_vec())?,
        ),
        (
            "django/contrib/admindocs/views.py".to_owned(),
            String::from_utf8(views_source.to_vec())?,
        ),
    ]);
    let extraction = resolve(&[root, child, views], &sources);
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    let exact = routes
        .iter()
        .find(|route| route.route.normalized_path == "/admin/doc")
        .ok_or("missing exact class-based route")?;
    assert_eq!(exact.state, ResolutionState::Exact, "{exact:#?}");
    assert_eq!(exact.candidates.len(), 1, "{exact:#?}");
    let include_source = b"path(\"admin/doc/\", include(\"django.contrib.admindocs.urls\"))";
    let include_start = root_source
        .windows(include_source.len())
        .position(|candidate| candidate == include_source)
        .ok_or("missing include source range")?;
    let handler = exact
        .stages
        .iter()
        .find(|stage| stage.role == RouteStageRole::Handler)
        .ok_or("missing exact handler stage")?;
    assert_eq!(handler.anchor.source_file, "project/urls.py");
    assert_eq!(handler.anchor.start_byte, u64::try_from(include_start)?);
    assert_eq!(
        handler.anchor.end_byte,
        u64::try_from(include_start.saturating_add(include_source.len()))?
    );
    for path in ["/admin/doc/near", "/admin/doc/dynamic"] {
        let unresolved = routes
            .iter()
            .find(|route| route.route.normalized_path == path)
            .ok_or("missing unresolved near-match route")?;
        assert_eq!(
            unresolved.state,
            ResolutionState::Unresolved,
            "{unresolved:#?}"
        );
        assert!(unresolved.candidates.is_empty(), "{unresolved:#?}");
    }
    Ok(())
}

#[test]
fn django_ambiguous_module_and_urls_include_remains_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let root_source = br#"from django.urls import include, path
urlpatterns = [path("mount/", include("pkg.feature"))]
"#;
    let flat_source = br#"from django.urls import path
def flat(request): return None
urlpatterns = [path("flat/", flat)]
"#;
    let nested_source = br#"from django.urls import path
def nested(request): return None
urlpatterns = [path("nested/", nested)]
"#;
    let mut engine = Engine::default();
    let root = engine.extract_source(Path::new("pkg/urls.py"), root_source)?;
    let flat = engine.extract_source(Path::new("pkg/feature.py"), flat_source)?;
    let nested = engine.extract_source(Path::new("pkg/feature/urls.py"), nested_source)?;
    let sources = HashMap::from([
        (
            "pkg/urls.py".to_owned(),
            String::from_utf8(root_source.to_vec())?,
        ),
        (
            "pkg/feature.py".to_owned(),
            String::from_utf8(flat_source.to_vec())?,
        ),
        (
            "pkg/feature/urls.py".to_owned(),
            String::from_utf8(nested_source.to_vec())?,
        ),
    ]);
    let extraction = resolve(&[root, flat, nested], &sources);
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    assert_eq!(routes.len(), 1, "routes={routes:#?}");
    assert_eq!(routes[0].route.normalized_path, "/mount");
    assert_eq!(routes[0].state, ResolutionState::Unresolved);
    assert_eq!(routes[0].route.handler_reference, "@include:pkg.feature");
    Ok(())
}

#[test]
fn django_static_local_and_imported_pattern_collections_compose_exactly()
-> Result<(), Box<dyn std::error::Error>> {
    let local_source = br#"from django.conf.urls.i18n import i18n_patterns
from django.urls import include, path

def health(request): return None
def item(request, item_id): return None
def localized(request): return None

api_patterns = [path("items/<int:item_id>/", item)]
base_patterns = [path("health/", health)]
urlpatterns = base_patterns + i18n_patterns(path("localized/", localized)) + [
    path("v1/", include((api_patterns, "api"), namespace="v1")),
]
"#;
    let child_source = br#"from django.urls import path
def child(request): return None
child_patterns = [path("child/", child)]
urlpatterns = child_patterns
"#;
    let root_source = br#"from django.urls import include, path
from .child_urls import urlpatterns as child_patterns
urlpatterns = child_patterns + [path("nested/", include(child_patterns))]
"#;
    let mut engine = Engine::default();
    let local = engine.extract_source(Path::new("pkg/local_urls.py"), local_source)?;
    let child = engine.extract_source(Path::new("pkg/child_urls.py"), child_source)?;
    let root = engine.extract_source(Path::new("pkg/root_urls.py"), root_source)?;
    let sources = HashMap::from([
        (
            "pkg/local_urls.py".to_owned(),
            String::from_utf8(local_source.to_vec())?,
        ),
        (
            "pkg/child_urls.py".to_owned(),
            String::from_utf8(child_source.to_vec())?,
        ),
        (
            "pkg/root_urls.py".to_owned(),
            String::from_utf8(root_source.to_vec())?,
        ),
    ]);
    let extraction = resolve(&[local, child, root], &sources);
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    let shapes = routes
        .iter()
        .filter(|route| route.route.framework == "django")
        .map(|route| route.route.normalized_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        shapes,
        [
            "/child",
            "/nested/child",
            "/health",
            "/localized",
            "/v1/items/{item_id}",
        ]
    );
    assert!(
        routes
            .iter()
            .all(|route| route.state == ResolutionState::Exact)
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
fn python_router_mounts_resolve_across_imported_modules() -> Result<(), Box<dyn std::error::Error>>
{
    let mut engine = Engine::default();
    let router_path = Path::new("src/api.py");
    let app_path = Path::new("src/app.py");
    let router_source = br#"from fastapi import APIRouter
router = APIRouter(prefix="/v1")
@router.get("/users")
def users(): return None
"#;
    let app_source = br#"from fastapi import FastAPI
from .api import router
app = FastAPI()
app.include_router(router, prefix="/api")
"#;
    let other_path = Path::new("src/other.py");
    let other_source = br#"from fastapi import APIRouter
router = APIRouter()
@router.get("/other")
def other(): return None
"#;
    let router_extraction = engine.extract_source(router_path, router_source)?;
    let app_extraction = engine.extract_source(app_path, app_source)?;
    let other_extraction = engine.extract_source(other_path, other_source)?;
    let sources = HashMap::from([
        (
            router_path.to_string_lossy().into_owned(),
            String::from_utf8(router_source.to_vec())?,
        ),
        (
            app_path.to_string_lossy().into_owned(),
            String::from_utf8(app_source.to_vec())?,
        ),
        (
            other_path.to_string_lossy().into_owned(),
            String::from_utf8(other_source.to_vec())?,
        ),
    ]);
    let mut extraction = resolve(
        &[router_extraction, app_extraction, other_extraction],
        &sources,
    );
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    let route = resolved
        .iter()
        .find(|route| route.route.framework == "fastapi")
        .ok_or("missing mounted cross-module FastAPI route")?;
    assert_eq!(route.route.normalized_path, "/api/v1/users");
    assert_eq!(route.state, ResolutionState::Exact);
    assert!(
        !resolved
            .iter()
            .any(|route| route.route.normalized_path == "/api/other")
    );
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
            RawFrameworkFact::Domain(_)
            | RawFrameworkFact::Annotation(_)
            | RawFrameworkFact::Role(_)
            | RawFrameworkFact::Relation(_)
            | RawFrameworkFact::Configuration(_)
            | RawFrameworkFact::FileSet(_) => None,
        })
        .collect()
}
