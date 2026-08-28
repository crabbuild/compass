use std::error::Error;
use std::path::Path;

use compass_languages::{
    Engine, FrameworkCapability, FrameworkPackRegistry, FrameworkRelation, RawDomainFact,
    RawFrameworkFact, RawRouteFact, RawRouteStageRole, framework_pack_semantics_version,
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
        "sqlalchemy-python",
        "celery-python",
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
    assert_eq!(framework_pack_semantics_version("flask-python"), Some(2));
    assert_eq!(
        framework_pack_semantics_version("starlette-python"),
        Some(1)
    );
    assert_eq!(framework_pack_semantics_version("pydantic-python"), Some(1));
    assert_eq!(
        framework_pack_semantics_version("sqlalchemy-python"),
        Some(1)
    );
    assert_eq!(framework_pack_semantics_version("celery-python"), Some(1));
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
    let sqlalchemy = FrameworkPackRegistry::descriptors()
        .iter()
        .find(|descriptor| descriptor.id == "sqlalchemy-python")
        .ok_or("sqlalchemy descriptor")?;
    assert_eq!(
        sqlalchemy.framework_capabilities,
        &[
            FrameworkCapability::DataModeling,
            FrameworkCapability::Persistence,
        ]
    );
    assert_eq!(
        sqlalchemy.emitted_relation_families,
        &[FrameworkRelation::DependsOn, FrameworkRelation::MapsTo]
    );
    let celery = FrameworkPackRegistry::descriptors()
        .iter()
        .find(|descriptor| descriptor.id == "celery-python")
        .ok_or("celery descriptor")?;
    assert_eq!(
        celery.framework_capabilities,
        &[
            FrameworkCapability::Messaging,
            FrameworkCapability::Scheduling,
        ]
    );
    assert_eq!(
        celery.emitted_relation_families,
        &[
            FrameworkRelation::Produces,
            FrameworkRelation::Consumes,
            FrameworkRelation::Schedules,
            FrameworkRelation::Triggers,
        ]
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
fn django_class_view_arguments_and_near_matches_remain_explicit_route_references()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "django/admindocs/urls.py",
        br#"from django.contrib.admindocs import views
from django.urls import path
urlpatterns = [
    path("", views.BaseAdminDocsView.as_view(template_name="admin_doc/index.html")),
    path("near/", views.BaseAdminDocsView.as_views(template_name="admin_doc/index.html")),
    path("dynamic/", factory().as_view(template_name="admin_doc/index.html")),
]
"#,
    )?;
    let routes = routes(&extraction)
        .into_iter()
        .filter(|route| route.framework == "django")
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 3, "routes={routes:#?}");
    assert!(routes.iter().any(|route| {
        route.handler_reference
            == "views.BaseAdminDocsView.as_view(template_name=\"admin_doc/index.html\")"
    }));
    assert!(routes.iter().any(|route| {
        route.handler_reference
            == "views.BaseAdminDocsView.as_views(template_name=\"admin_doc/index.html\")"
    }));
    assert!(routes.iter().any(|route| {
        route.handler_reference == "factory().as_view(template_name=\"admin_doc/index.html\")"
    }));
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
fn django_signal_connect_requires_an_exact_imported_signal_and_local_handler()
-> Result<(), Box<dyn Error>> {
    let source = br#"from django.db import models
from django.db.models import signals as model_signals
from django.db.models.signals import post_save

class Item(models.Model): pass

def saved(sender, **kwargs): return None
def deleted(sender, **kwargs): return None

post_save.connect(saved, sender=Item)
model_signals.post_delete.connect(receiver=deleted)
"#;
    let extraction = extract("project/signals.py", source)?;
    let subscriptions = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Relation(relation)
                if relation.pack_id == "django-python" && relation.relation == "subscribes" =>
            {
                Some(relation)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        2,
        subscriptions.len(),
        "facts={:#?}",
        extraction.framework_facts
    );
    let mut targets = subscriptions
        .iter()
        .filter_map(|relation| relation.target_hint.as_deref())
        .collect::<Vec<_>>();
    targets.sort_unstable();
    assert_eq!(
        vec![
            "django.db.models.signals.post_delete",
            "django.db.models.signals.post_save",
        ],
        targets
    );
    assert!(subscriptions.iter().all(|relation| {
        relation.context.as_deref() == Some("signal")
            && relation.evidence_class == "exact"
            && relation.anchor.end_byte > relation.anchor.start_byte
    }));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Relation(relation) if relation.context.as_deref() == Some("signal_sender"))
    }));
    Ok(())
}

#[test]
fn django_signal_connect_shadowed_dynamic_and_unrelated_receivers_fail_closed()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "project/near_signal.py",
        br#"from django.db.models.signals import post_save
from vendor import signals

def handler(sender, **kwargs): return None

post_save = build_signal()
post_save.connect(handler)
signals.post_save.connect(handler)
database.connect(handler)
get_signal().connect(handler)
"#,
    )?;
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Relation(relation) if relation.relation == "subscribes")
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
fn fastapi_websocket_and_api_route_decorators_preserve_exact_handlers() -> Result<(), Box<dyn Error>>
{
    let extraction = extract(
        "api/routes.py",
        br#"from fastapi import FastAPI

app = FastAPI()

@app.websocket("/events")
async def events():
    return None

@app.api_route("/default")
def default_route():
    return None

@app.api_route("/explicit", methods=["POST", "PATCH"])
def explicit_route():
    return None
"#,
    )?;
    let routes = routes(&extraction);
    assert!(routes.iter().any(|route| {
        route.operation == "WEBSOCKET"
            && route.normalized_path == "/events"
            && route.handler_reference == "events"
    }));
    assert!(routes.iter().any(|route| {
        route.operation == "GET"
            && route.normalized_path == "/default"
            && route.handler_reference == "default_route"
    }));
    let explicit = routes
        .iter()
        .filter(|route| route.normalized_path == "/explicit")
        .map(|route| route.operation.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        explicit,
        std::collections::BTreeSet::from(["PATCH", "POST"])
    );
    assert!(routes.iter().all(|route| {
        route
            .stages
            .last()
            .is_some_and(|stage| stage.detail.contains_key("declaration_id"))
    }));
    Ok(())
}

#[test]
fn flask_factories_shortcuts_url_rules_method_views_nested_blueprints_and_hooks_are_exact()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "web/factory.py",
        br#"from flask import Blueprint, Flask
from flask.views import MethodView

root = Blueprint("root", __name__, url_prefix="/root")
nested = Blueprint("nested", __name__, url_prefix="/nested")

@nested.before_request
def authorize(): return None

@nested.errorhandler(404)
def not_found(error): return None

@nested.get("/items")
def items(): return None

root.register_blueprint(nested, url_prefix="/v2")

class ItemView(MethodView):
    def get(self): return None

def health(): return None

def create_app():
    app = Flask(__name__)
    app.register_blueprint(root, url_prefix="/api")
    app.add_url_rule("/health", view_func=health)
    app.add_url_rule("/item", view_func=ItemView.as_view("item"))
    return app
"#,
    )?;
    let flask_routes = routes(&extraction)
        .into_iter()
        .filter(|route| route.framework == "flask")
        .collect::<Vec<_>>();
    assert_eq!(
        flask_routes.len(),
        3,
        "facts={:#?}",
        extraction.framework_facts
    );
    for path in ["/v2/nested/items", "/health", "/item"] {
        assert!(
            flask_routes
                .iter()
                .any(|route| route.normalized_path == path && route.operation == "GET"),
            "routes={flask_routes:#?}"
        );
    }
    let nested = flask_routes
        .iter()
        .find(|route| route.normalized_path == "/v2/nested/items")
        .ok_or("nested route")?;
    assert_eq!(
        nested.detail.get("implicit_methods"),
        Some(&serde_json::json!(["HEAD", "OPTIONS"]))
    );
    assert!(
        nested
            .stages
            .iter()
            .any(|stage| stage.role == RawRouteStageRole::Middleware)
    );
    assert!(
        nested
            .stages
            .iter()
            .any(|stage| stage.role == RawRouteStageRole::ErrorBoundary)
    );
    assert_eq!(
        extraction
            .framework_facts
            .iter()
            .filter(|fact| {
                matches!(fact, RawFrameworkFact::Role(role)
                    if role.pack_id == "flask-python" && role.role == "hook")
            })
            .count(),
        2
    );
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Role(role)
            if role.pack_id == "flask-python"
                && role.role == "service"
                && role.context.as_deref() == Some("application_factory"))
    }));
    Ok(())
}

#[test]
fn flask_dynamic_prefixes_and_method_view_lookalikes_fail_closed() -> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "web/near_match_flask.py",
        br#"from flask import Blueprint, Flask
from other import MethodView

prefix = get_prefix()
app = Flask(__name__)
blueprint = Blueprint("dynamic", __name__, url_prefix=prefix)

@blueprint.get("/wrong")
def wrong(): return None

class WrongView(MethodView):
    def get(self): return None

app.add_url_rule("/wrong-view", view_func=WrongView.as_view("wrong"))
"#,
    )?;
    assert!(
        routes(&extraction).is_empty(),
        "facts={:#?}",
        extraction.framework_facts
    );
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

#[test]
fn sqlalchemy_models_fields_relationships_and_table_mappings_are_exact()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "models.py",
        br#"from sqlalchemy import ForeignKey, String
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship

class Base(DeclarativeBase):
    pass

class User(Base):
    __tablename__ = "users"
    id: Mapped[int] = mapped_column(primary_key=True)
    name: Mapped[str] = mapped_column(String(120))

class Post(Base):
    __tablename__ = "posts"
    __table_args__ = {"schema": "content"}
    id: Mapped[int] = mapped_column(primary_key=True)
    author_id: Mapped[int] = mapped_column(ForeignKey("users.id"))
    author: Mapped[User] = relationship(back_populates="posts")
"#,
    )?;
    let model_roles = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Role(role)
                if role.pack_id == "sqlalchemy-python" && role.role == "model" =>
            {
                Some(role)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        model_roles.len(),
        2,
        "facts={:#?}",
        extraction.framework_facts
    );
    assert!(model_roles.iter().all(|role| {
        role.detail
            .get("fields")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|fields| !fields.is_empty())
    }));
    let mappings = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain)
                if domain.framework == "sqlalchemy"
                    && domain.kind == "orm_mapping"
                    && domain
                        .detail
                        .get("pack_id")
                        .and_then(serde_json::Value::as_str)
                        == Some("sqlalchemy-python") =>
            {
                Some(domain)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(mappings.len(), 2);
    assert!(mappings.iter().any(|mapping| {
        mapping
            .detail
            .get("database_schema")
            .and_then(serde_json::Value::as_str)
            == Some("content")
    }));
    assert!(mappings.iter().all(|mapping| {
        mapping
            .detail
            .get("model_reference")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|reference| {
                model_roles
                    .iter()
                    .any(|role| role.subject_reference.as_deref() == Some(reference))
            })
    }));
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Relation(relation)
            if relation.pack_id == "sqlalchemy-python"
                && relation.relation == "depends_on"
                && relation.context.as_deref() == Some("relationship:author"))
    }));
    Ok(())
}

#[test]
fn sqlalchemy_dynamic_shadowed_and_lookalike_forms_fail_closed() -> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "near_match_models.py",
        br#"from other import DeclarativeBase, mapped_column, relationship

class Base(DeclarativeBase): pass
class Wrong(Base):
    __tablename__ = "wrong"
    id = mapped_column()

from sqlalchemy.orm import DeclarativeBase as RealBase, Mapped, mapped_column as real_column

class ExactBase(RealBase): pass
class DynamicTable(ExactBase):
    __tablename__ = table_name()
    id: Mapped[int] = real_column()

real_column = object()
class ShadowedColumn(ExactBase):
    __tablename__ = table_name()
    id: Mapped[int] = real_column()
"#,
    )?;
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain)
            if domain.framework == "sqlalchemy"
                && domain.detail.get("pack_id").and_then(serde_json::Value::as_str)
                    == Some("sqlalchemy-python"))
    }));
    let roles = extraction
        .framework_facts
        .iter()
        .filter(|fact| {
            matches!(fact, RawFrameworkFact::Role(role) if role.pack_id == "sqlalchemy-python")
        })
        .count();
    assert_eq!(roles, 2, "facts={:#?}", extraction.framework_facts);
    Ok(())
}

#[test]
fn celery_tasks_invocations_canvas_queues_retry_and_schedules_are_exact()
-> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "tasks.py",
        br#"from celery import Celery, chain, chord, group, shared_task

app = Celery("tasks")

@app.task(name="orders.cleanup", queue="maintenance", bind=True)
def cleanup(self, order_id):
    self.retry(countdown=5)

@shared_task(queue="inventory")
def refresh_inventory():
    return None

@app.task(bind=True)
def rebound_retry(self):
    self = object()
    self.retry()

def dispatch():
    cleanup.delay(1)
    refresh_inventory.apply_async(queue="priority")
    chain(cleanup.s(2), refresh_inventory.s())()
    group(cleanup.s(3), refresh_inventory.s())()
    chord([cleanup.s(4)], refresh_inventory.s())()
    app.send_task("external.rebuild", queue="external")

def dynamic_canvas(task):
    chain(task.s())()

app.conf.beat_schedule = {
    "refresh-every-minute": {
        "task": "inventory.refresh",
        "schedule": 60.0,
    },
}
"#,
    )?;
    let celery_domains = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain)
                if domain
                    .detail
                    .get("pack_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("celery-python") =>
            {
                Some(domain)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        celery_domains
            .iter()
            .filter(|domain| domain.kind == "job" && domain.detail.get("scheduled_task").is_none())
            .count(),
        3,
        "facts={:#?}",
        extraction.framework_facts
    );
    assert!(celery_domains.iter().any(|domain| {
        domain.kind == "job"
            && domain.name == "orders.cleanup"
            && domain
                .detail
                .get("queue")
                .and_then(serde_json::Value::as_str)
                == Some("maintenance")
    }));
    assert!(celery_domains.iter().any(|domain| {
        domain.kind == "message"
            && domain.name == "external.rebuild"
            && domain
                .detail
                .get("relationship")
                .and_then(serde_json::Value::as_str)
                == Some("produces")
    }));
    assert!(celery_domains.iter().any(|domain| {
        domain.kind == "job"
            && domain.name == "refresh-every-minute"
            && domain
                .detail
                .get("scheduled_task")
                .and_then(serde_json::Value::as_str)
                == Some("inventory.refresh")
    }));
    let trigger_contexts = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Relation(relation)
                if relation.pack_id == "celery-python" && relation.relation == "triggers" =>
            {
                relation.context.as_deref()
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for context in [
        "invocation:delay",
        "invocation:apply_async",
        "canvas:chain",
        "canvas:group",
        "canvas:chord",
        "retry",
    ] {
        assert!(
            trigger_contexts.contains(&context),
            "facts={:#?}",
            extraction.framework_facts
        );
    }
    assert_eq!(
        trigger_contexts
            .iter()
            .filter(|context| **context == "retry")
            .count(),
        1,
        "a rebound bound-task receiver must not create retry evidence"
    );
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Relation(relation)
            if relation.pack_id == "celery-python"
                && relation.context.as_deref() == Some("send_task"))
    }));
    assert_eq!(
        extraction
            .framework_facts
            .iter()
            .filter(|fact| {
                matches!(fact, RawFrameworkFact::Role(role)
                    if role.pack_id == "celery-python" && role.role == "consumer")
            })
            .count(),
        3
    );
    Ok(())
}

#[test]
fn celery_lookalike_rebound_and_dynamic_metadata_forms_fail_closed() -> Result<(), Box<dyn Error>> {
    let extraction = extract(
        "near_match_tasks.py",
        br#"from other import Celery, shared_task

fake = Celery("fake")
@fake.task(name="wrong")
def wrong(): pass

from celery import Celery as ExactCelery, shared_task as exact_task
app = ExactCelery("tasks")
app = object()

@app.task(name="rebound")
def rebound(): pass

exact_task = lambda function: function
@exact_task
def shadowed(): pass
"#,
    )?;
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain)
            if domain.detail.get("pack_id").and_then(serde_json::Value::as_str)
                == Some("celery-python"))
            || matches!(fact, RawFrameworkFact::Role(role) if role.pack_id == "celery-python")
            || matches!(fact, RawFrameworkFact::Relation(relation) if relation.pack_id == "celery-python")
    }));
    Ok(())
}
