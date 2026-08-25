use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::{Engine, Extraction, FrameworkLimits};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::{FrameworkResolutionError, RouteStageRole, resolve_routes};
use compass_resolve::resolve;

fn extract(path: &str, source: &[u8]) -> Result<Extraction, Box<dyn Error>> {
    Ok(Engine::default().extract_source(Path::new(path), source)?)
}

fn resolved_project(sources: &[(&str, &[u8])]) -> Result<Extraction, Box<dyn Error>> {
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut source_map = HashMap::new();
    for (path, source) in sources {
        extractions.push(engine.extract_source(Path::new(path), source)?);
        source_map.insert((*path).to_owned(), String::from_utf8((*source).to_vec())?);
    }
    Ok(resolve(&extractions, &source_map))
}

#[test]
fn nested_and_repeated_fastapi_mounts_preserve_receiver_identity_and_multiplicity()
-> Result<(), Box<dyn Error>> {
    let leaf = br#"from fastapi import APIRouter
leaf = APIRouter()

@leaf.get("/leaf")
def handler():
    return None
"#;
    let middle = br#"from fastapi import APIRouter
from .leaf import leaf
middle = APIRouter()
middle.include_router(leaf, prefix="/middle")
"#;
    let app = br#"from fastapi import FastAPI
from .middle import middle
app = FastAPI()
app.include_router(middle, prefix="/v1")
app.include_router(middle, prefix="/v2")
"#;
    let extraction = resolved_project(&[
        ("pkg/leaf.py", leaf),
        ("pkg/middle.py", middle),
        ("pkg/app.py", app),
    ])?;
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    let paths = routes
        .iter()
        .map(|route| route.route.normalized_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["/v1/middle/leaf", "/v2/middle/leaf"]);
    assert!(
        routes
            .iter()
            .all(|route| route.state == ResolutionState::Exact)
    );
    assert!(routes.iter().all(|route| {
        route
            .route
            .detail
            .get("mount_anchors")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|anchors| anchors.len() == 2)
    }));
    Ok(())
}

#[test]
fn receiver_mount_cycles_emit_no_fabricated_route_and_depth_overflow_is_explicit()
-> Result<(), Box<dyn Error>> {
    let a = br#"from fastapi import APIRouter
from .b import b
a = APIRouter()
a.include_router(b, prefix="/a")

@a.get("/route")
def handler():
    return None
"#;
    let b = br#"from fastapi import APIRouter
from .a import a
b = APIRouter()
b.include_router(a, prefix="/b")
"#;
    let cycle = resolved_project(&[("pkg/a.py", a), ("pkg/b.py", b)])?;
    assert!(resolve_routes(&cycle, FrameworkLimits::default())?.is_empty());

    let leaf = br#"from fastapi import APIRouter
leaf = APIRouter()
@leaf.get("/leaf")
def handler(): return None
"#;
    let middle = br#"from fastapi import APIRouter
from .leaf import leaf
middle = APIRouter()
middle.include_router(leaf, prefix="/middle")
"#;
    let app = br#"from fastapi import FastAPI
from .middle import middle
app = FastAPI()
app.include_router(middle, prefix="/api")
"#;
    let chain = resolved_project(&[
        ("pkg/leaf.py", leaf),
        ("pkg/middle.py", middle),
        ("pkg/app.py", app),
    ])?;
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
fn ambiguous_receiver_identity_retains_only_the_unmounted_route() -> Result<(), Box<dyn Error>> {
    let first = br#"from fastapi import APIRouter
router = APIRouter()
@router.get("/first")
def first(): return None
"#;
    let second = br#"from fastapi import APIRouter

router = APIRouter()
@router.get("/second")
def second(): return None
"#;
    let app = br#"from fastapi import FastAPI
from .routes import router
app = FastAPI()
app.include_router(router, prefix="/api")
"#;
    let mut ambiguous = Extraction::default();
    for mut extraction in [
        extract("pkg/routes.py", first)?,
        extract("pkg/routes.py", second)?,
        extract("pkg/app.py", app)?,
    ] {
        ambiguous.nodes.append(&mut extraction.nodes);
        ambiguous.edges.append(&mut extraction.edges);
        ambiguous
            .framework_facts
            .append(&mut extraction.framework_facts);
    }
    let routes = resolve_routes(&ambiguous, FrameworkLimits::default())?;
    assert!(
        routes
            .iter()
            .all(|route| !route.route.normalized_path.starts_with("/api"))
    );
    assert_eq!(
        routes
            .iter()
            .map(|route| route.route.normalized_path.as_str())
            .collect::<Vec<_>>(),
        ["/first", "/second"]
    );
    Ok(())
}

#[test]
fn starlette_constructor_and_imperative_routes_compose_repeated_nested_mounts()
-> Result<(), Box<dyn Error>> {
    let leaf = br#"from starlette.routing import Route, Router
def endpoint(request): return None
leaf = Router(routes=[Route("/leaf", endpoint)])
"#;
    let middle = br#"from starlette.applications import Starlette
from .leaf import leaf
middle = Starlette()
middle.mount("/middle", leaf)
"#;
    let app = br#"from starlette.applications import Starlette
from .middle import middle
app = Starlette()
app.mount("/v1", middle)
app.mount("/v2", middle)
"#;
    let extraction = resolved_project(&[
        ("pkg/leaf.py", leaf),
        ("pkg/middle.py", middle),
        ("pkg/app.py", app),
    ])?;
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    assert_eq!(
        routes
            .iter()
            .map(|route| route.route.normalized_path.as_str())
            .collect::<Vec<_>>(),
        ["/v1/middle/leaf", "/v2/middle/leaf"]
    );
    assert!(
        routes
            .iter()
            .all(|route| route.state == ResolutionState::Exact)
    );
    Ok(())
}

#[test]
fn fastapi_inherited_stages_and_pydantic_schema_relations_publish_exactly()
-> Result<(), Box<dyn Error>> {
    let source = br#"from typing import Annotated
from fastapi import APIRouter, Depends, FastAPI, Security
from pydantic import BaseModel

class Item(BaseModel):
    name: str

def database():
    yield object()

def authorize(database=Depends(database)): return True

app = FastAPI(dependencies=[Depends(database)])
router = APIRouter(dependencies=[Security(authorize)])

@router.post("/items", dependencies=[Depends(authorize)], response_model=Item)
def create_item(item: Item, database: Annotated[object, Depends(database)]) -> Item:
    return item

app.include_router(router, prefix="/api", dependencies=[Depends(database)])
"#;
    let extraction = resolved_project(&[("pkg/api.py", source)])?;
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].route.normalized_path, "/api/items");
    let roles = routes[0]
        .stages
        .iter()
        .map(|stage| stage.role)
        .collect::<Vec<_>>();
    assert_eq!(roles.last(), Some(&RouteStageRole::Handler));
    assert!(
        roles
            .iter()
            .filter(|role| **role == RouteStageRole::Dependency)
            .count()
            >= 3,
        "roles={roles:?} stages={:?}",
        routes[0]
            .route
            .stages
            .iter()
            .map(|stage| (&stage.role, &stage.reference))
            .collect::<Vec<_>>()
    );
    assert!(roles.contains(&RouteStageRole::Security));
    assert!(extraction.edges.iter().any(|edge| {
        edge.string("relation") == "depends_on"
            && edge.source.contains("create_item")
            && edge.target.contains("item")
    }));
    assert!(
        extraction
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "depends_on")
            .all(|edge| edge.string("extractor")
                == format!("compass.frameworks.{}", edge.string("framework")))
    );
    assert!(extraction.nodes.iter().any(|node| {
        node.id.contains("item")
            && node
                .attributes
                .get("roles")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|roles| roles.iter().any(|role| role.as_str() == Some("model")))
    }));
    Ok(())
}

#[test]
fn drf_default_router_expands_only_exact_viewset_methods_and_actions() -> Result<(), Box<dyn Error>>
{
    let source = br#"from django.db import models
from django.urls import include, path
from rest_framework.decorators import action
from rest_framework.permissions import IsAuthenticated
from rest_framework.routers import DefaultRouter
from rest_framework.serializers import ModelSerializer
from rest_framework.viewsets import ModelViewSet

class Item(models.Model):
    name = models.CharField(max_length=100)

class ItemSerializer(ModelSerializer):
    class Meta:
        model = Item
        fields = ["name"]

class ItemViewSet(ModelViewSet):
    lookup_field = "slug"
    lookup_url_kwarg = "item_slug"
    serializer_class = ItemSerializer
    permission_classes = [IsAuthenticated]

    def list(self, request): return None
    def retrieve(self, request, pk=None): return None

    @action(detail=True, methods=["post"], url_path="publish")
    def publish(self, request, pk=None): return None

router = DefaultRouter()
router.register("items", ItemViewSet, basename="item")
urlpatterns = [path("api/", include((router.urls, "api"), namespace="v1"))]
"#;
    let extraction = resolved_project(&[("api/urls.py", source)])?;
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    let route_shapes = routes
        .iter()
        .filter(|route| route.route.framework == "django-rest-framework")
        .map(|route| {
            (
                route.route.operation.as_str(),
                route.route.normalized_path.as_str(),
                route.state,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        route_shapes,
        [
            ("GET", "/api/items", ResolutionState::Exact),
            ("GET", "/api/items/{item_slug}", ResolutionState::Exact),
            (
                "POST",
                "/api/items/{item_slug}/publish",
                ResolutionState::Exact,
            ),
        ]
    );
    assert!(!routes.iter().any(|route| {
        route.route.framework == "django" && route.route.handler_reference.contains("router.urls")
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.string("relation") == "depends_on"
            && edge.source.contains("itemviewset")
            && edge.target.contains("itemserializer")
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.string("relation") == "depends_on"
            && edge.source.contains("itemserializer")
            && edge.target.contains("item")
    }));
    Ok(())
}
