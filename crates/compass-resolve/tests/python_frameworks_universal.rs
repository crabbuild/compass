use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Engine, Extraction, FrameworkLimits};
use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::{FrameworkResolutionError, RouteStageRole, resolve_routes};
use compass_resolve::{resolve, resolve_with_root};

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
fn nested_fastapi_routes_and_dependencies_use_exact_lexical_identities()
-> Result<(), Box<dyn Error>> {
    let dependencies = br#"def dependency_c():
    return "c"
"#;
    let routes = br#"from fastapi import Depends, FastAPI
import pkg.dependencies as module

class Checker:
    def __call__(self):
        return True

checker = Checker()

def first_app():
    app = FastAPI()

    @app.websocket("/first")
    async def endpoint(value=Depends(checker)):
        return value

    return app

def second_app():
    app = FastAPI()

    @app.api_route("/second")
    def endpoint(value=Depends(module.dependency_c)):
        return value

    return app
"#;
    let extraction = resolved_project(&[
        ("pkg/dependencies.py", dependencies),
        ("pkg/routes.py", routes),
    ])?;
    let resolved = resolve_routes(&extraction, FrameworkLimits::default())?;
    assert_eq!(resolved.len(), 2, "routes={resolved:#?}");
    assert!(
        resolved
            .iter()
            .all(|route| route.state == ResolutionState::Exact)
    );
    let handler_targets = resolved
        .iter()
        .filter_map(|route| {
            route
                .stages
                .iter()
                .find(|stage| stage.role == RouteStageRole::Handler)
                .and_then(|stage| stage.target.as_deref())
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(handler_targets.len(), 2);

    let checker = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.routes.checker")
        .ok_or("missing checker instance")?;
    let dependency_c = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.dependencies.dependency_c")
        .ok_or("missing qualified dependency")?;
    assert_eq!(checker.string("symbol_kind"), "variable");
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| { edge.string("relation") == "depends_on" && edge.target == checker.id })
    );
    assert!(
        extraction.edges.iter().any(|edge| {
            edge.string("relation") == "depends_on" && edge.target == dependency_c.id
        })
    );
    Ok(())
}

#[test]
fn module_scoped_fastapi_dependencies_preserve_exact_file_identity() -> Result<(), Box<dyn Error>> {
    let source = br#"from typing import Annotated
from fastapi import Depends

def module_dependency():
    return True

DependencyAlias = Annotated[str, Depends(module_dependency)]
"#;
    let extraction = resolved_project(&[("pkg/dependencies.py", source)])?;
    let file = extraction
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "file"
                && node.string("source_file") == "pkg/dependencies.py"
        })
        .ok_or("missing file node")?;
    let dependency = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.dependencies.module_dependency")
        .ok_or("missing module dependency")?;
    assert!(
        extraction.edges.iter().any(|edge| {
            edge.string("relation") == "depends_on"
                && edge.source == file.id
                && edge.target == dependency.id
        }),
        "edges={:#?} diagnostics={:#?}",
        extraction.edges,
        extraction.extensions
    );
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
fn flask_factory_and_nested_blueprint_routes_preserve_exact_stages_and_mount_depth()
-> Result<(), Box<dyn Error>> {
    let source = br#"from flask import Blueprint, Flask

root = Blueprint("root", __name__, url_prefix="/root")
nested = Blueprint("nested", __name__, url_prefix="/nested")

@nested.before_request
def authorize(): return None

@nested.get("/items")
def items(): return None

root.register_blueprint(nested, url_prefix="/v2")

def create_app():
    app = Flask(__name__)
    app.register_blueprint(root, url_prefix="/api")
    return app
"#;
    let extraction = resolved_project(&[("pkg/app.py", source)])?;
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    let route = routes
        .iter()
        .find(|route| route.route.framework == "flask")
        .ok_or("resolved Flask route")?;
    assert_eq!(route.route.normalized_path, "/api/v2/nested/items");
    assert_eq!(route.state, ResolutionState::Exact);
    assert!(route.stages.iter().any(|stage| {
        stage.role == RouteStageRole::Middleware && stage.reference.contains("authorize")
    }));
    assert_eq!(
        route
            .route
            .detail
            .get("mount_anchors")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    Ok(())
}

#[test]
fn sqlalchemy_and_celery_relations_publish_only_exact_declaration_identities()
-> Result<(), Box<dyn Error>> {
    let source = br#"from celery import shared_task
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship

class Base(DeclarativeBase): pass
class User(Base):
    __tablename__ = "users"
    id: Mapped[int] = mapped_column(primary_key=True)

class Post(Base):
    __tablename__ = "posts"
    author: Mapped[User] = relationship(User)

@shared_task
def refresh(): return None

def dispatch():
    refresh.delay()
"#;
    let extraction = resolved_project(&[("pkg/domain.py", source)])?;
    assert!(extraction.edges.iter().any(|edge| {
        edge.string("relation") == "depends_on"
            && edge.source.contains("post")
            && edge.target.contains("user")
            && edge.string("extractor") == "compass.frameworks.sqlalchemy"
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.string("relation") == "triggers"
            && edge.source.contains("dispatch")
            && edge.target.contains("refresh")
            && edge.string("extractor") == "compass.frameworks.celery"
    }));
    assert!(extraction.framework_facts.iter().all(|fact| match fact {
        compass_languages::RawFrameworkFact::Relation(relation)
            if matches!(
                relation.pack_id.as_str(),
                "sqlalchemy-python" | "celery-python"
            ) =>
        {
            relation.ambiguity_policy == "require_exact"
                && relation.source_reference.is_some()
                && relation.target_anchor.is_some()
        }
        _ => true,
    }));
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

#[test]
fn django_signal_subscriptions_publish_event_endpoints() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = br#"from django.db import models
from django.db.models.signals import post_save

class Item(models.Model):
    pass

def saved(sender, **kwargs):
    return None

post_save.connect(saved, sender=Item)
"#;
    let path = root.path().join("signals.py");
    fs::write(&path, source)?;
    let signal_module_path = root.path().join("django/db/models/signals.py");
    fs::create_dir_all(
        signal_module_path
            .parent()
            .ok_or("missing signal module parent")?,
    )?;
    fs::write(root.path().join("django/__init__.py"), b"")?;
    fs::write(root.path().join("django/db/__init__.py"), b"")?;
    fs::write(root.path().join("django/db/models/__init__.py"), b"")?;
    let signal_module_source = b"post_save = object()\n";
    fs::write(&signal_module_path, signal_module_source)?;
    let mut engine = Engine::default();
    let extraction = engine.extract_source(Path::new("signals.py"), source)?;
    let signal_module_extraction = engine.extract_source(
        Path::new("django/db/models/signals.py"),
        signal_module_source,
    )?;
    let source_map = HashMap::from([
        ("signals.py".to_owned(), String::from_utf8(source.to_vec())?),
        (
            "django/db/models/signals.py".to_owned(),
            String::from_utf8(signal_module_source.to_vec())?,
        ),
    ]);
    let resolved = resolve_with_root(
        &[extraction, signal_module_extraction],
        &source_map,
        root.path(),
    );
    assert!(resolved.nodes.iter().any(|node| {
        node.string("qualified_name") == "django.db.models.signals.post_save"
            && node.string("symbol_kind") == "event"
    }));
    let subscription_count = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "subscribes")
        .count();
    assert_eq!(subscription_count, 1, "edges={:#?}", resolved.edges);

    let evidence = BuildEvidence::from_extraction(root.path(), &resolved, "sha256:django-signal")?;
    let graph = normalize_v1(resolved, evidence)?;
    let event_ids = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Event)
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(graph.links.iter().any(|edge| {
        edge.kind == EdgeKind::Subscribes && event_ids.contains(edge.target.as_str())
    }));
    Ok(())
}

#[test]
fn django_external_signal_subscriptions_publish_event_endpoints() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = br#"from django.db.models.signals import post_save

def saved(sender, **kwargs):
    return None

class Lookalike:
    def post_save(self):
        return None

post_save.connect(saved)
"#;
    let path = root.path().join("signals.py");
    fs::write(&path, source)?;
    let mut engine = Engine::default();
    let extraction = engine.extract_source(Path::new("signals.py"), source)?;
    let source_map =
        HashMap::from([("signals.py".to_owned(), String::from_utf8(source.to_vec())?)]);
    let resolved = resolve_with_root(&[extraction], &source_map, root.path());
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "subscribes")
            .count(),
        1,
        "edges={:#?} diagnostics={:#?}",
        resolved.edges,
        resolved.extensions
    );

    let evidence =
        BuildEvidence::from_extraction(root.path(), &resolved, "sha256:django-signal-external")?;
    let graph = normalize_v1(resolved, evidence)?;
    let event_ids = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Event)
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(graph.links.iter().any(|edge| {
        edge.kind == EdgeKind::Subscribes && event_ids.contains(edge.target.as_str())
    }));
    Ok(())
}
