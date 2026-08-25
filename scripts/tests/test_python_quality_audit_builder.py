"""Regression tests for the source-grounded Python quality-audit provider."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from benchmarks.performance.compass.occurrences import (
    independent_source_inventory,
    source_construct_inventory_sha256,
)


class PythonQualityAuditBuilderTests(unittest.TestCase):
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
        inventory = self.inventory(
            {
                "src/app.py": (
                    "from fastapi import Depends, FastAPI\n"
                    "app = FastAPI()\n"
                    "def auth():\n"
                    "    return True\n"
                    "@app.get('/items')\n"
                    "def read_items(user=Depends(auth)):\n"
                    "    return user\n"
                )
            }
        )
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


if __name__ == "__main__":
    unittest.main()
