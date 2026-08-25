"""Regression tests for the source-grounded Python quality-audit provider."""

from __future__ import annotations

from collections import Counter
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

from benchmarks.performance.compass.occurrences import (
    SourceConstruct,
    independent_source_inventory,
    source_construct_inventory_sha256,
)
from scripts.build_python_quality_audit import (
    _extractor_pack,
    _source_target_is_exact,
    _target_matches,
)


class PythonQualityAuditBuilderTests(unittest.TestCase):
    @staticmethod
    def construct(
        relation: str,
        target: str,
        qualifier: str | None = None,
    ) -> SourceConstruct:
        return SourceConstruct(
            source_file="src/app.py",
            relation=relation,
            capability=relation,
            owner_qualified_name="src.app.owner",
            target_spelling=target,
            qualifier=qualifier,
            start_byte=10,
            end_byte=20,
            start_line=2,
        )

    @staticmethod
    def edge(qualified_name: str) -> dict[str, object]:
        return {"targetNode": {"qualifiedName": qualified_name}}

    def inventory(self, sources: dict[str, str]):
        temporary = tempfile.TemporaryDirectory(prefix="compass-python-audit-")
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        for relative, source in sources.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source, encoding="utf-8")
        return independent_source_inventory(
            root,
            "python-frameworks",
            include_globs=("src/**/*.py",),
        )

    def test_exact_framework_bindings_emit_route_and_dependency_anchors(self) -> None:
        source = (
            "from fastapi import Depends, FastAPI\n"
            "app = FastAPI()\n"
            "def auth():\n"
            "    return True\n"
            "@app.get('/items')\n"
            "def read_items(user=Depends(auth)):\n"
            "    return user\n"
        )
        inventory = self.inventory({"src/app.py": source})
        self.assertEqual(1, inventory.scanned_files)
        self.assertEqual(1, inventory.parsed_files)
        self.assertEqual((), inventory.rejected_files)
        self.assertEqual(
            {
                (construct.relation, construct.framework_pack, construct.target_spelling)
                for construct in inventory.constructs
            },
            {
                ("depends_on", "fastapi-python", "auth"),
                ("routes_to", "fastapi-python", "read_items"),
            },
        )
        dependency = next(
            construct
            for construct in inventory.constructs
            if construct.relation == "depends_on"
        )
        self.assertEqual(
            b"Depends(auth)",
            source.encode("utf-8")[dependency.start_byte : dependency.end_byte],
        )

    def test_starlette_route_anchors_the_unique_handler_declaration(self) -> None:
        source = (
            "from starlette.routing import Route\n"
            "def home(request):\n"
            "    return request\n"
            "routes = [Route('/', home)]\n"
        )
        inventory = self.inventory({"src/routes.py": source})
        self.assertEqual(1, len(inventory.constructs))
        route = inventory.constructs[0]
        encoded = source.encode("utf-8")
        self.assertEqual(b"home", encoded[route.start_byte : route.end_byte])
        self.assertEqual("starlette-python", route.framework_pack)

    def test_shadowed_framework_bindings_fail_closed(self) -> None:
        inventory = self.inventory(
            {
                "src/app.py": (
                    "from fastapi import FastAPI\n"
                    "FastAPI = object\n"
                    "app = FastAPI()\n"
                    "@app.get('/items')\n"
                    "def read_items():\n"
                    "    return None\n"
                )
            }
        )
        self.assertEqual((), inventory.constructs)

    def test_exact_domain_constructs_preserve_framework_source_anchors(self) -> None:
        source = (
            "from celery import Celery, shared_task\n"
            "from django.db import models\n"
            "from django.db.models.signals import post_save\n"
            "from django.dispatch import receiver\n"
            "from fastapi import FastAPI\n"
            "from pydantic import BaseModel\n"
            "from rest_framework import serializers, viewsets\n"
            "from sqlalchemy.orm import DeclarativeBase, Mapped, relationship\n"
            "\n"
            "celery_app = Celery('tasks')\n"
            "web = FastAPI()\n"
            "\n"
            "class Parent(models.Model):\n"
            "    pass\n"
            "class Child(models.Model):\n"
            "    parent = models.ForeignKey(Parent, on_delete=models.CASCADE)\n"
            "class ParentSerializer(serializers.ModelSerializer):\n"
            "    class Meta:\n"
            "        model = Parent\n"
            "class ParentViewSet(viewsets.ModelViewSet):\n"
            "    serializer_class = ParentSerializer\n"
            "class Payload(BaseModel):\n"
            "    name: str\n"
            "class Base(DeclarativeBase):\n"
            "    pass\n"
            "class User(Base):\n"
            "    __tablename__ = 'users'\n"
            "class Post(Base):\n"
            "    __tablename__ = 'posts'\n"
            "    author: Mapped[User] = relationship()\n"
            "\n"
            "@shared_task(queue='jobs')\n"
            "def refresh():\n"
            "    return None\n"
            "def dispatch():\n"
            "    refresh.delay(queue='jobs')\n"
            "@receiver(post_save, sender=Parent)\n"
            "def saved(sender, **kwargs):\n"
            "    return sender\n"
            "@web.post('/payload', response_model=Payload)\n"
            "def create(payload: Payload) -> Payload:\n"
            "    return payload\n"
        )
        inventory = self.inventory({"src/domain.py": source})
        facts = {
            (
                construct.relation,
                construct.framework_pack,
                construct.target_spelling,
                source.encode("utf-8")[construct.start_byte : construct.end_byte],
            )
            for construct in inventory.constructs
        }
        self.assertIn(
            ("depends_on", "django-python", "Parent", b"parent"),
            facts,
        )
        self.assertIn(
            (
                "depends_on",
                "django-rest-framework-python",
                "Parent",
                b"Parent",
            ),
            facts,
        )
        self.assertIn(
            (
                "depends_on",
                "sqlalchemy-python",
                "User",
                b"author: Mapped[User] = relationship()",
            ),
            facts,
        )
        self.assertIn(
            ("maps_to", "sqlalchemy-python", "users", b"'users'"),
            facts,
        )
        self.assertIn(
            (
                "schedules",
                "celery-python",
                "src.domain.refresh",
                b"@shared_task(queue='jobs')",
            ),
            facts,
        )
        self.assertIn(
            (
                "consumes",
                "celery-python",
                "jobs",
                b"@shared_task(queue='jobs')",
            ),
            facts,
        )
        self.assertIn(
            (
                "triggers",
                "celery-python",
                "refresh",
                b"refresh.delay(queue='jobs')",
            ),
            facts,
        )
        self.assertIn(
            (
                "subscribes",
                "django-python",
                "django.db.models.signals.post_save",
                b"@receiver(post_save, sender=Parent)",
            ),
            facts,
        )
        self.assertIn(
            (
                "depends_on",
                "pydantic-python",
                "Payload",
                b"@web.post('/payload', response_model=Payload)",
            ),
            facts,
        )

    def test_django_constructor_route_uses_whole_call_anchor(self) -> None:
        source = (
            "from django.urls import include, path\n"
            "class Feed:\n"
            "    pass\n"
            "urlpatterns = [\n"
            "    path('feed/', Feed()),\n"
            "    path('view/', Feed.as_view()),\n"
            "    path('nested/', include([])),\n"
            "]\n"
        )
        inventory = self.inventory({"src/urls.py": source})
        routes = [
            construct
            for construct in inventory.constructs
            if construct.relation == "routes_to"
        ]
        self.assertEqual(
            {
                ("Feed", b"path('feed/', Feed())"),
                ("Feed", b"path('view/', Feed.as_view())"),
            },
            {
                (
                    route.target_spelling,
                    source.encode("utf-8")[route.start_byte : route.end_byte],
                )
                for route in routes
            },
        )

    def test_django_include_routes_recursively_use_each_mount_anchor(self) -> None:
        leaf = (
            "from django.urls import path\n"
            "def leaf(request):\n"
            "    return request\n"
            "urlpatterns = [path('leaf/', leaf)]\n"
        )
        middle = (
            "from django.urls import include, path\n"
            "from .leaf import urlpatterns as leaf_patterns\n"
            "base_patterns = [\n"
            "    path('leaf/', include((leaf_patterns, 'leaf'), namespace='leaf')),\n"
            "]\n"
            "urlpatterns = []\n"
            "urlpatterns += base_patterns\n"
        )
        root = (
            "from django.urls import include, path\n"
            "urlpatterns = [path('middle/', include('src.middle'))]\n"
        )
        inventory = self.inventory(
            {
                "src/leaf.py": leaf,
                "src/middle.py": middle,
                "src/root.py": root,
            }
        )
        routes = [
            construct
            for construct in inventory.constructs
            if construct.framework_pack == "django-python"
            and construct.relation == "routes_to"
            and construct.target_spelling == "leaf"
        ]
        snippets = Counter()
        sources = {
            "src/leaf.py": leaf.encode("utf-8"),
            "src/middle.py": middle.encode("utf-8"),
            "src/root.py": root.encode("utf-8"),
        }
        for route in routes:
            snippets[
                sources[route.source_file][route.start_byte : route.end_byte]
            ] += 1
        self.assertEqual(
            Counter(
                {
                    b"path('leaf/', leaf)": 1,
                    b"path('leaf/', include((leaf_patterns, 'leaf'), namespace='leaf'))": 1,
                    b"path('middle/', include('src.middle'))": 1,
                }
            ),
            snippets,
        )

    def test_django_dynamic_includes_fail_closed_and_static_cycles_terminate(self) -> None:
        dynamic = (
            "from django.urls import include, path\n"
            "def choose():\n"
            "    return []\n"
            "patterns = choose()\n"
            "urlpatterns = [path('dynamic/', include(patterns))]\n"
        )
        cyclic_a = (
            "from django.urls import include, path\n"
            "urlpatterns = [path('b/', include('src.cyclic_b'))]\n"
        )
        cyclic_b = (
            "from django.urls import include, path\n"
            "def concrete(request):\n"
            "    return request\n"
            "urlpatterns = [\n"
            "    path('a/', include('src.cyclic_a')),\n"
            "    path('concrete/', concrete),\n"
            "]\n"
        )
        inventory = self.inventory(
            {
                "src/dynamic.py": dynamic,
                "src/cyclic_a.py": cyclic_a,
                "src/cyclic_b.py": cyclic_b,
            }
        )
        routes = [
            construct
            for construct in inventory.constructs
            if construct.framework_pack == "django-python"
            and construct.relation == "routes_to"
        ]
        self.assertNotIn("choose", {route.target_spelling for route in routes})
        self.assertNotIn("patterns", {route.target_spelling for route in routes})
        self.assertEqual(
            3,
            sum(route.target_spelling == "concrete" for route in routes),
        )

    def test_django_url_pattern_inventory_limit_is_explicit(self) -> None:
        source = (
            "from django.urls import path\n"
            "def one(request):\n"
            "    return request\n"
            "def two(request):\n"
            "    return request\n"
            "urlpatterns = [path('one/', one), path('two/', two)]\n"
        )
        with patch(
            "benchmarks.performance.compass.occurrences."
            "_PYTHON_FRAMEWORK_MAX_INCLUDE_TARGETS",
            1,
        ):
            with self.assertRaisesRegex(
                RuntimeError,
                "Python Django URL pattern count 2 exceeds 1",
            ):
                self.inventory({"src/urls.py": source})

    def test_drf_router_routes_require_exact_mount_and_viewset_methods(self) -> None:
        source = (
            "from rest_framework.decorators import action\n"
            "from rest_framework.routers import SimpleRouter\n"
            "from rest_framework.viewsets import ViewSet\n"
            "class Items(ViewSet):\n"
            "    def list(self, request):\n"
            "        return request\n"
            "    @action(detail=False)\n"
            "    def refresh(self, request):\n"
            "        return request\n"
            "mounted = SimpleRouter()\n"
            "mounted.register('items', Items, basename='items')\n"
            "urlpatterns = mounted.urls\n"
            "unmounted = SimpleRouter()\n"
            "unmounted.register('hidden', Items, basename='hidden')\n"
            "class CustomRouter:\n"
            "    pass\n"
            "custom = CustomRouter()\n"
            "custom.register('custom', Items)\n"
        )
        inventory = self.inventory({"src/urls.py": source})
        routes = [
            construct
            for construct in inventory.constructs
            if construct.framework_pack == "django-rest-framework-python"
            and construct.relation == "routes_to"
        ]
        self.assertEqual({"list", "refresh"}, {route.target_spelling for route in routes})
        self.assertEqual(
            {b"mounted.register('items', Items, basename='items')"},
            {
                source.encode("utf-8")[route.start_byte : route.end_byte]
                for route in routes
            },
        )

    def test_partial_files_are_explicit_and_digest_stable(self) -> None:
        inventory = self.inventory(
            {
                "src/ok.py": "def ok():\n    return 1\n",
                "src/partial.py": "def broken(:\n",
                "outside.py": "def ignored():\n    return 1\n",
            }
        )
        self.assertEqual(2, inventory.scanned_files)
        self.assertEqual(1, inventory.parsed_files)
        self.assertEqual(("src/partial.py",), inventory.rejected_files)
        self.assertEqual(
            source_construct_inventory_sha256("python-frameworks", inventory),
            source_construct_inventory_sha256("python-frameworks", inventory),
        )

    def test_call_target_matches_compass_member_qualified_name(self) -> None:
        construct = self.construct("calls", "_test_settings_get")
        self.assertTrue(
            _target_matches(
                construct,
                self.edge(
                    "django.db.backends.oracle.creation."
                    "DatabaseCreation::_test_settings_get"
                ),
            )
        )

    def test_domain_target_and_extractor_normalize_without_changing_pack(self) -> None:
        construct = self.construct("schedules", "pkg.tasks.refresh")
        self.assertTrue(
            _target_matches(
                construct,
                self.edge("celery::job::pkg.tasks.refresh"),
            )
        )
        self.assertEqual(
            "celery-python",
            _extractor_pack("compass.frameworks.celery.domain"),
        )
        self.assertIsNone(_extractor_pack("compass.frameworks.unknown.domain"))

    def test_sqlalchemy_relation_target_matches_exact_model_identity(self) -> None:
        construct = self.construct("depends_on", "Company")
        self.assertTrue(
            _target_matches(
                construct,
                self.edge("examples.inheritance.concrete.Company"),
            )
        )
        self.assertFalse(
            _target_matches(
                construct,
                self.edge("examples.inheritance.concrete.Employer"),
            )
        )

    def test_import_target_matches_source_module_reexport(self) -> None:
        construct = self.construct(
            "imports",
            "django.forms.MultiWidget",
            "django.forms",
        )
        self.assertTrue(
            _target_matches(
                construct,
                self.edge("django.forms.widgets::MultiWidget"),
            )
        )
        self.assertFalse(
            _target_matches(
                construct,
                self.edge("unrelated.forms.widgets::MultiWidget"),
            )
        )

    def test_relative_import_target_resolves_from_source_package(self) -> None:
        construct = self.construct(
            "imports",
            "widgets.MultiWidget",
            ".widgets",
        )
        construct = SourceConstruct(
            **{
                **construct.__dict__,
                "source_file": "django/forms/__init__.py",
            }
        )
        self.assertTrue(
            _target_matches(
                construct,
                self.edge("django.forms.widgets::MultiWidget"),
            )
        )
        self.assertFalse(
            _target_matches(
                construct,
                self.edge("django.forms.other::MultiWidget"),
            )
        )

    def test_dynamic_member_call_is_not_proven_by_unrelated_corpus_declaration(self) -> None:
        construct = self.construct("calls", "errors", "exc_info.value")
        declared = Counter({"errors": 1})
        local = Counter({("src/other.py", "errors"): 1})
        self.assertFalse(_source_target_is_exact(construct, declared, local))

        local_construct = self.construct("calls", "render")
        local[("src/app.py", "render")] = 1
        self.assertTrue(_source_target_is_exact(local_construct, declared, local))

    def test_terminal_matching_retains_multiple_candidates_for_ambiguity(self) -> None:
        construct = self.construct("calls", "render")
        candidates = [
            self.edge("pkg.first.View::render"),
            self.edge("pkg.second.View::render"),
        ]
        self.assertEqual(
            2,
            sum(_target_matches(construct, candidate) for candidate in candidates),
        )


if __name__ == "__main__":
    unittest.main()
