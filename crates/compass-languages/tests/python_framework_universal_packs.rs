use std::error::Error;
use std::path::Path;

use compass_languages::{
    Engine, FrameworkPackRegistry, RawDomainFact, RawFrameworkFact, RawRouteFact,
    framework_pack_semantics_version,
};

fn extract(path: &str, source: &[u8]) -> Result<compass_languages::Extraction, Box<dyn Error>> {
    Ok(Engine::default().extract_source(Path::new(path), source)?)
}

fn routes(extraction: &compass_languages::Extraction) -> Vec<&RawRouteFact> {
    extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            _ => None,
        })
        .collect()
}

fn mounts(extraction: &compass_languages::Extraction) -> Vec<&RawDomainFact> {
    extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain) if domain.kind == "router_mount" => Some(domain),
            _ => None,
        })
        .collect()
}

#[test]
fn python_web_cutover_has_three_versioned_universal_owners() {
    let ids = FrameworkPackRegistry::descriptors()
        .iter()
        .map(|descriptor| descriptor.id)
        .collect::<Vec<_>>();
    for id in ["django-python", "fastapi-python", "flask-python"] {
        assert!(ids.contains(&id), "missing universal descriptor {id}");
        assert_eq!(framework_pack_semantics_version(id), Some(1));
    }
    assert!(!ids.contains(&"python-web"));
    assert_eq!(framework_pack_semantics_version("python-web"), None);
    assert_eq!(FrameworkPackRegistry::validate(), Ok(()));
}

#[test]
fn django_routes_require_exact_imported_calls_and_urlpatterns_flow() -> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "project/urls.py",
        br#"from django.urls import include, path as django_path

def path(route, view):
    return None

def handler(request):
    return None

urlpatterns = [
    django_path("ok/", handler),
    path("shadowed/", handler),
    django_path("api/", include("project.api.urls")),
]
not_patterns = [django_path("outside/", handler)]
"#,
    )?;
    let routes = routes(&extraction);
    assert_eq!(routes.len(), 2, "facts={:#?}", extraction.framework_facts);
    assert!(routes.iter().any(|route| route.normalized_path == "/ok"));
    assert!(routes.iter().any(|route| {
        route.normalized_path == "/api"
            && route
                .detail
                .get("include")
                .and_then(serde_json::Value::as_str)
                == Some("project.api.urls")
    }));
    assert!(
        !routes
            .iter()
            .any(|route| { matches!(route.normalized_path.as_str(), "/shadowed" | "/outside") })
    );
    Ok(())
}

#[test]
fn fastapi_and_flask_routes_join_exact_receiver_declarations() -> Result<(), Box<dyn Error>> {
    let fastapi = extract(
        "api/routes.py",
        br#"from fastapi import APIRouter, Depends, FastAPI

app = FastAPI()
router = APIRouter(prefix="/v1")

def authorize():
    return True

@router.post("/items", dependencies=[Depends(authorize)])
def create_item():
    return None

app.include_router(router, prefix="/api")
"#,
    )?;
    let fastapi_routes = routes(&fastapi);
    assert_eq!(
        fastapi_routes.len(),
        1,
        "facts={:#?}",
        fastapi.framework_facts
    );
    assert_eq!(fastapi_routes[0].framework, "fastapi");
    assert_eq!(fastapi_routes[0].normalized_path, "/api/v1/items");
    assert_eq!(fastapi_routes[0].middleware_references, ["authorize"]);
    assert!(fastapi_routes[0].detail.contains_key("receiver_id"));
    let fastapi_mounts = mounts(&fastapi);
    assert_eq!(fastapi_mounts.len(), 1);
    assert!(
        fastapi_mounts[0]
            .detail
            .contains_key("target_receiver_qualified_name")
    );

    let flask = extract(
        "web/app.py",
        br#"from flask import Blueprint, Flask

app = Flask(__name__)
api = Blueprint("api", __name__, url_prefix="/v1")

@api.route("/health")
def health():
    return None

app.register_blueprint(api, url_prefix="/api")
"#,
    )?;
    let flask_routes = routes(&flask);
    assert_eq!(flask_routes.len(), 1, "facts={:#?}", flask.framework_facts);
    assert_eq!(flask_routes[0].framework, "flask");
    assert_eq!(flask_routes[0].operation, "GET");
    assert_eq!(flask_routes[0].normalized_path, "/api/v1/health");
    assert_eq!(mounts(&flask).len(), 1);
    Ok(())
}

#[test]
fn wrong_framework_dynamic_and_rebound_receivers_fail_closed() -> Result<(), Box<dyn Error>> {
    let wrong = extract(
        "wrong.py",
        br#"from other import FastAPI
app = FastAPI()
@app.get("/wrong")
def wrong():
    return None
"#,
    )?;
    assert!(routes(&wrong).is_empty());

    let dynamic = extract(
        "dynamic.py",
        br#"from fastapi import FastAPI
app = FastAPI()
path = "/dynamic"
@app.get(path)
def dynamic():
    return None
"#,
    )?;
    assert!(routes(&dynamic).is_empty());

    let rebound = extract(
        "rebound.py",
        br#"from flask import Flask
app = Flask(__name__)
app = object()
@app.route("/rebound")
def rebound():
    return None
"#,
    )?;
    assert!(routes(&rebound).is_empty());
    Ok(())
}
