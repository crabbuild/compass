use std::error::Error;
use std::path::Path;

use compass_languages::{
    Engine, FrameworkCapability, FrameworkPackRegistry, RawDomainFact, RawFrameworkFact,
    RawRouteFact, RawRouteStageRole, framework_pack_semantics_version,
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
fn python_web_cutover_has_independent_versioned_universal_owners() -> Result<(), Box<dyn Error>> {
    let ids = FrameworkPackRegistry::descriptors()
        .iter()
        .map(|descriptor| descriptor.id)
        .collect::<Vec<_>>();
    for id in [
        "django-python",
        "fastapi-python",
        "flask-python",
        "pydantic-python",
        "starlette-python",
    ] {
        assert!(ids.contains(&id), "missing universal descriptor {id}");
    }
    assert_eq!(framework_pack_semantics_version("fastapi-python"), Some(2));
    assert_eq!(
        framework_pack_semantics_version("starlette-python"),
        Some(1)
    );
    assert_eq!(framework_pack_semantics_version("pydantic-python"), Some(1));
    let fastapi = FrameworkPackRegistry::descriptors()
        .iter()
        .find(|descriptor| descriptor.id == "fastapi-python")
        .ok_or("fastapi descriptor")?;
    assert!(
        fastapi
            .framework_capabilities
            .contains(&FrameworkCapability::DependencyInjection)
    );
    assert!(
        fastapi
            .framework_capabilities
            .contains(&FrameworkCapability::Security)
    );
    assert!(!ids.contains(&"python-web"));
    assert_eq!(framework_pack_semantics_version("python-web"), None);
    assert_eq!(FrameworkPackRegistry::validate(), Ok(()));
    Ok::<(), Box<dyn Error>>(())
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
    assert!(
        fastapi_routes[0]
            .stages
            .iter()
            .any(|stage| stage.role == RawRouteStageRole::Dependency)
    );
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

#[test]
fn starlette_routes_cover_decorated_imperative_constructor_websocket_and_mount_forms()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "starlette_app.py",
        br#"from starlette.applications import Starlette
from starlette.routing import Mount, Route, Router, WebSocketRoute

def homepage(request): return None
def created(request): return None
def socket(websocket): return None

child = Router(routes=[Route("/child", homepage)])
app = Starlette(routes=[
    Route("/home", homepage, methods=["GET", "POST"]),
    WebSocketRoute("/ws", socket),
    Mount("/nested", routes=[Route("/inside", homepage)]),
    Mount("/mounted", app=child),
])
app.add_route("/created", created, methods=["PUT"])
app.add_websocket_route("/events", socket)

@app.route("/decorated")
def decorated(request): return None
"#,
    )?;
    let routes = routes(&extraction);
    let operations = routes
        .iter()
        .map(|route| (route.normalized_path.as_str(), route.operation.as_str()))
        .collect::<Vec<_>>();
    for expected in [
        ("/home", "GET"),
        ("/home", "POST"),
        ("/ws", "WEBSOCKET"),
        ("/nested/inside", "GET"),
        ("/created", "PUT"),
        ("/events", "WEBSOCKET"),
        ("/decorated", "GET"),
    ] {
        assert!(operations.contains(&expected), "routes={routes:#?}");
    }
    assert!(mounts(&extraction).iter().any(|mount| {
        mount
            .detail
            .get("mount_prefix")
            .and_then(serde_json::Value::as_str)
            == Some("/mounted")
    }));
    Ok(())
}

#[test]
fn fastapi_dependencies_security_and_pydantic_schemas_are_exact_facts() -> Result<(), Box<dyn Error>>
{
    let extraction = extract(
        "api.py",
        br#"from typing import Annotated
from fastapi import APIRouter, Depends, FastAPI, Security
from pydantic import BaseModel, computed_field, field_serializer, field_validator

class Item(BaseModel):
    name: str

    @field_validator("name")
    @classmethod
    def valid_name(cls, value): return value

    @field_serializer("name")
    def serialize_name(self, value): return value

    @computed_field
    @property
    def slug(self) -> str: return self.name

class ChildItem(Item):
    count: int

def database():
    yield object()

def authorize(database=Depends(database)): return True

app = FastAPI(dependencies=[Depends(database)])
router = APIRouter(dependencies=[Security(authorize, scopes=["items:write"])])

@router.post("/items", dependencies=[Depends(authorize)], response_model=Item)
def create_item(item: Item, database: Annotated[object, Depends(database)]) -> Item:
    return item

app.include_router(router, prefix="/api", dependencies=[Depends(database)])
"#,
    )?;
    let routes = routes(&extraction);
    assert_eq!(routes.len(), 1, "facts={:#?}", extraction.framework_facts);
    let roles = routes[0]
        .stages
        .iter()
        .map(|stage| stage.role)
        .collect::<Vec<_>>();
    assert_eq!(roles.last(), Some(&RawRouteStageRole::Handler));
    assert!(roles.contains(&RawRouteStageRole::Dependency));
    assert!(roles.contains(&RawRouteStageRole::Security));
    assert!(routes[0].stages.iter().any(|stage| {
        stage
            .detail
            .get("lifecycle")
            .and_then(serde_json::Value::as_str)
            == Some("yield")
    }));
    let model_roles = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Role(role)
                if role.pack_id == "pydantic-python" && role.role == "model" =>
            {
                Some(role)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(model_roles.len(), 2);
    let item_role = model_roles
        .iter()
        .find(|role| {
            role.subject_reference
                .as_deref()
                .is_some_and(|reference| reference.ends_with("_item"))
        })
        .ok_or("missing Item model role")?;
    assert!(
        item_role
            .detail
            .get("fields")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|fields| !fields.is_empty())
    );
    assert!(
        item_role
            .detail
            .get("members")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|members| members.len() >= 3)
    );
    let schema_contexts = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Relation(relation) if relation.pack_id == "pydantic-python" => {
                relation.context.as_deref()
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(schema_contexts.contains(&"request_model"));
    assert!(schema_contexts.contains(&"response_model"));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Relation(relation) if relation.pack_id == "fastapi-python" && relation.context.as_deref() == Some("subdependency"))
    }));
    Ok(())
}

#[test]
fn starlette_pydantic_and_dependency_near_matches_fail_closed() -> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "near_matches.py",
        br#"from other import BaseModel, Depends, Route, Starlette

class NotPydantic(BaseModel): pass
def provider(): return None
def handler(request): return None

app = Starlette(routes=[Route("/wrong", handler)])
path = "/dynamic"
app.add_route(path, handler)

def endpoint(value=Depends(provider)) -> NotPydantic: return value
"#,
    )?;
    assert!(routes(&extraction).is_empty());
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Role(role) if role.pack_id == "pydantic-python")
            || matches!(fact, RawFrameworkFact::Relation(relation) if matches!(relation.pack_id.as_str(), "pydantic-python" | "fastapi-python"))
    }));

    let shadowed = extract(
        "shadowed_dependency.py",
        br#"from fastapi import Depends, FastAPI

def provider(): return None
provider = object()
app = FastAPI()

@app.get("/shadowed", dependencies=[Depends(provider)])
def endpoint(): return None
"#,
    )?;
    let shadowed_routes = routes(&shadowed);
    assert_eq!(shadowed_routes.len(), 1);
    assert_eq!(shadowed_routes[0].stages.len(), 1);
    assert_eq!(
        shadowed_routes[0].stages[0].role,
        RawRouteStageRole::Handler
    );
    assert!(!shadowed.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Relation(relation) if relation.pack_id == "fastapi-python")
    }));
    Ok(())
}
