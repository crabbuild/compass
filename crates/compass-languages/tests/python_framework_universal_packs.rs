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
        "django-rest-framework-python",
        "fastapi-python",
        "flask-python",
        "pydantic-python",
        "starlette-python",
    ] {
        assert!(ids.contains(&id), "missing universal descriptor {id}");
    }
    assert_eq!(framework_pack_semantics_version("django-python"), Some(2));
    assert_eq!(
        framework_pack_semantics_version("django-rest-framework-python"),
        Some(1)
    );
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
fn django_static_pattern_collections_i18n_namespaces_and_converters_are_exact()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "project/urls.py",
        br#"from django.conf.urls.i18n import i18n_patterns
from django.urls import include, path, register_converter

class YearConverter: pass
register_converter(YearConverter, "year")

def health(request): return None
def item(request, year): return None
def localized(request): return None
def extra(request): return None

base_patterns = [path("health/", health)]
api_patterns = (path("items/<year:year>/", item),)
localized_patterns = i18n_patterns(path("localized/", localized))
urlpatterns = base_patterns + localized_patterns + [
    path("v1/", include((api_patterns, "api"), namespace="v1")),
]
urlpatterns += [path("extra/", extra)]
"#,
    )?;
    let django_routes = routes(&extraction)
        .into_iter()
        .filter(|route| route.framework == "django")
        .collect::<Vec<_>>();
    assert_eq!(django_routes.len(), 5, "routes={django_routes:#?}");
    assert!(
        django_routes
            .iter()
            .any(|route| route.normalized_path == "/extra")
    );
    assert!(
        django_routes
            .iter()
            .any(|route| route.normalized_path == "/items/{year}")
    );
    let include = django_routes
        .iter()
        .find(|route| route.normalized_path == "/v1")
        .ok_or("missing local collection include")?;
    assert_eq!(
        include
            .detail
            .get("include_collection")
            .and_then(serde_json::Value::as_str),
        Some("api_patterns")
    );
    assert_eq!(
        include
            .detail
            .get("namespace")
            .and_then(serde_json::Value::as_str),
        Some("v1")
    );
    assert!(django_routes.iter().any(|route| {
        route.normalized_path == "/localized"
            && route
                .detail
                .get("i18n")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    }));
    Ok(())
}

#[test]
fn django_orphan_dynamic_and_same_named_i18n_collections_fail_closed() -> Result<(), Box<dyn Error>>
{
    let extraction = extract(
        "project/near_matches.py",
        br#"from django.urls import path

def i18n_patterns(*patterns): return patterns
def endpoint(request): return None

orphan = [path("orphan/", endpoint)]
dynamic = build_patterns()
urlpatterns = i18n_patterns(path("wrong/", endpoint)) + dynamic
"#,
    )?;
    assert!(routes(&extraction).is_empty());
    Ok(())
}

#[test]
fn drf_router_viewset_action_and_serializer_facts_require_exact_evidence()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "api/urls.py",
        br#"from django.db import models
from django.db.models.signals import post_save
from django.dispatch import receiver
from django.urls import include, path
from rest_framework.authentication import TokenAuthentication
from rest_framework.decorators import action
from rest_framework.filters import SearchFilter
from rest_framework.permissions import IsAuthenticated
from rest_framework.routers import DefaultRouter
from rest_framework.serializers import ModelSerializer
from rest_framework.throttling import UserRateThrottle
from rest_framework.viewsets import ModelViewSet

class ItemManager(models.Manager):
    pass

class Owner(models.Model):
    name = models.CharField(max_length=100)

class Item(models.Model):
    name = models.CharField(max_length=100)
    owner = models.ForeignKey(Owner, on_delete=models.CASCADE)
    reviewers = models.ManyToManyField(Owner, related_name="reviewed_items")
    primary_owner = models.OneToOneField(Owner, on_delete=models.CASCADE, related_name="primary_item")
    objects = ItemManager()

class ItemSerializer(ModelSerializer):
    class Meta:
        model = Item
        fields = ["name"]

class ItemViewSet(ModelViewSet):
    lookup_field = "slug"
    serializer_class = ItemSerializer
    permission_classes = [IsAuthenticated]
    authentication_classes = [TokenAuthentication]
    filter_backends = [SearchFilter]
    throttle_classes = [UserRateThrottle]

    def list(self, request): return None
    def retrieve(self, request, pk=None): return None

    @action(detail=True, methods=["post"], url_path="publish")
    def publish(self, request, pk=None): return None

@receiver(post_save, sender=Item)
def item_saved(sender, instance, **kwargs): return None

router = DefaultRouter()
router.register("items", ItemViewSet, basename="item")
route_patterns = [path("api/", include((router.urls, "api"), namespace="v1"), name="api")]
urlpatterns = route_patterns
"#,
    )?;
    let drf_domains = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain) if domain.framework == "django-rest-framework" => {
                Some(domain)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        drf_domains
            .iter()
            .any(|fact| fact.kind == "drf_router_registration")
    );
    assert!(drf_domains.iter().any(|fact| {
        fact.kind == "drf_router_registration"
            && fact
                .detail
                .get("lookup_parameter")
                .and_then(serde_json::Value::as_str)
                == Some("slug")
    }));
    assert!(
        drf_domains
            .iter()
            .any(|fact| fact.kind == "drf_router_mount")
    );
    assert!(drf_domains.iter().any(|fact| {
        fact.kind == "drf_router_mount"
            && fact
                .detail
                .get("namespace")
                .and_then(serde_json::Value::as_str)
                == Some("v1")
    }));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Role(role) if role.pack_id == "django-rest-framework-python" && role.role == "controller")
    }));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Role(role) if role.pack_id == "django-python" && role.role == "model")
    }));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Role(role) if role.pack_id == "django-python" && role.role == "service" && role.context.as_deref() == Some("model_manager"))
    }));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Role(role) if role.pack_id == "django-python" && role.role == "subscriber" && role.context.as_deref() == Some("django.db.models.signals.post_save"))
    }));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Relation(relation) if relation.pack_id == "django-python" && relation.relation == "subscribes" && relation.target_hint.as_deref() == Some("django.db.models.signals.post_save"))
    }));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Role(role) if role.pack_id == "django-python" && role.role == "model" && role.detail.get("fields").and_then(serde_json::Value::as_array).is_some_and(|fields| fields.len() >= 4))
    }), "facts={:#?}", extraction.framework_facts);
    let relation_contexts = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Relation(relation)
                if matches!(
                    relation.pack_id.as_str(),
                    "django-python" | "django-rest-framework-python"
                ) =>
            {
                relation.context.as_deref()
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for expected in [
        "serializer_class",
        "permission_classes",
        "authentication_classes",
        "filter_backends",
        "throttle_classes",
        "serializer_model",
        "ForeignKey",
        "ManyToManyField",
        "OneToOneField",
        "manager:objects",
        "signal_sender",
    ] {
        assert!(
            relation_contexts.contains(&expected),
            "missing {expected}; facts={:#?}",
            extraction.framework_facts
        );
    }
    Ok(())
}

#[test]
fn drf_custom_router_dynamic_registration_and_wrong_imports_fail_closed()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "api/near_matches.py",
        br#"from django.urls import include, path
from other import DefaultRouter, ModelViewSet, action

class ItemViewSet(ModelViewSet):
    @action(detail=True, methods=get_methods(), url_path=dynamic_path())
    def publish(self, request): return None

router = DefaultRouter()
prefix = "items"
router.register(prefix, ItemViewSet)
urlpatterns = [path("api/", include(router.urls))]
"#,
    )?;
    assert!(!extraction.framework_facts.iter().any(|fact| match fact {
        RawFrameworkFact::Domain(domain) => domain.framework == "django-rest-framework",
        RawFrameworkFact::Role(role) => role.pack_id == "django-rest-framework-python",
        RawFrameworkFact::Relation(relation) => {
            relation.pack_id == "django-rest-framework-python"
        }
        _ => false,
    }));
    Ok(())
}

#[test]
fn drf_router_registration_requires_a_literal_basename() -> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "api/dynamic_basename.py",
        br#"from django.urls import include, path
from rest_framework.routers import DefaultRouter
from rest_framework.viewsets import ModelViewSet

class ItemViewSet(ModelViewSet):
    def list(self, request): return None

router = DefaultRouter()
router.register("missing", ItemViewSet)
router.register("dynamic", ItemViewSet, basename=choose_basename())
urlpatterns = [path("api/", include(router.urls))]
"#,
    )?;
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "drf_router_registration")
    }));
    Ok(())
}

#[test]
fn django_dynamic_registrations_and_ambiguous_signal_sender_fail_closed()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "project/registrations.py",
        br#"from django.contrib import admin
from django.db import models
from django.db.models.signals import post_save
from django.dispatch import receiver

class Item(models.Model): pass
class ItemAdmin: pass

Item = choose_model()

@receiver(post_save, sender=Item)
def ambiguous_sender(sender, **kwargs): return None

admin.site.register(Item, ItemAdmin)
MIDDLEWARE = build_middleware()
INSTALLED_APPS = [dynamic_app]
"#,
    )?;
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Relation(relation) if matches!(relation.context.as_deref(), Some("signal_sender")) || relation.relation == "registers")
    }));
    Ok(())
}

#[test]
fn drf_dynamic_lookup_serializer_and_custom_router_templates_fail_closed()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "api/dynamic.py",
        br#"from rest_framework.routers import DefaultRouter
from rest_framework.viewsets import ModelViewSet

class CustomRouter(DefaultRouter): pass

class ItemViewSet(ModelViewSet):
    lookup_field = choose_lookup()
    serializer_class = make_serializer()
    def retrieve(self, request, pk=None): return None

router = DefaultRouter()
router.register("items", ItemViewSet, basename="item")
custom = CustomRouter()
custom.register("custom", ItemViewSet, basename="custom")
urlpatterns = router.urls
"#,
    )?;
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "drf_router_mount")
    }));
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "drf_router_registration")
    }));
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Relation(relation) if relation.context.as_deref() == Some("serializer_class"))
    }));
    Ok(())
}

#[test]
fn drf_router_mount_tuple_requires_a_literal_application_name() -> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "api/tuple_near_match.py",
        br#"from django.urls import include, path
from rest_framework.routers import DefaultRouter
from rest_framework.viewsets import ViewSet

class ItemViewSet(ViewSet):
    def list(self, request): return None

router = DefaultRouter()
router.register("items", ItemViewSet, basename="item")
application_name = choose_name()
urlpatterns = [path("api/", include((router.urls, application_name), namespace="v1"))]
"#,
    )?;
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "drf_router_registration")
    }));
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "drf_router_mount")
    }));
    Ok(())
}

#[test]
fn drf_external_inherited_viewset_methods_are_not_synthesized() -> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "api/inherited.py",
        br#"from rest_framework.routers import SimpleRouter
from rest_framework.viewsets import ModelViewSet

class ItemViewSet(ModelViewSet):
    pass

router = SimpleRouter()
router.register("items", ItemViewSet, basename="item")
urlpatterns = router.urls
"#,
    )?;
    let registration = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Domain(domain) if domain.kind == "drf_router_registration" => {
                Some(domain)
            }
            _ => None,
        })
        .ok_or("missing exact router registration")?;
    assert_eq!(
        registration
            .detail
            .get("methods")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(0)
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
