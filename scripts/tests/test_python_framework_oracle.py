"""Tests for the bounded Python framework source oracle."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import python_framework_oracle as oracle  # noqa: E402


class PythonFrameworkOracleTests(unittest.TestCase):
    def test_fixture_inventory_is_byte_deterministic_and_broad(self) -> None:
        root = ROOT / "fixtures" / "code-graph" / "routes" / "python"
        first = oracle.build_inventory(root)
        second = oracle.build_inventory(root)
        self.assertEqual(oracle.canonical_bytes(first), oracle.canonical_bytes(second))
        self.assertEqual(first["schema"], oracle.SCHEMA)
        kinds = first["summary"]["constructKinds"]
        for kind in ("declaration", "import", "decorator", "call", "route_registration", "mount_registration"):
            self.assertGreater(kinds.get(kind, 0), 0, kind)
        self.assertEqual(first["summary"]["partialFiles"], 0)

    def test_utf8_ranges_are_exact_source_byte_ranges(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-python-oracle-utf8-") as directory:
            root = Path(directory)
            source = "class Café(BaseModel):\n    @router.get('/élève')\n    def lire(self):\n        return self\n"
            (root / "models.py").write_text(source, encoding="utf-8")
            inventory = oracle.build_inventory(root)
            records = inventory["files"][0]["constructs"]
            route = next(record for record in records if record["kind"] == "route_registration")
            raw = source.encode("utf-8")
            anchor = route["anchor"]
            self.assertIn(b"router.get", raw[anchor["startByte"] : anchor["endByte"]])

    def test_malformed_invalid_utf8_and_oversized_files_are_not_success(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-python-oracle-partial-") as directory:
            root = Path(directory)
            (root / "malformed.py").write_text("def broken(:\n", encoding="utf-8")
            (root / "invalid.py").write_bytes(b"value = \xff\n")
            statuses = {
                item["path"]: item["status"]
                for item in oracle.build_inventory(root)["files"]
            }
            self.assertEqual(statuses, {"invalid.py": "partial", "malformed.py": "partial"})

    def test_cli_writes_canonical_json(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-python-oracle-cli-") as directory:
            output = Path(directory) / "oracle.json"
            self.assertEqual(
                oracle.main(
                    [
                        "--root",
                        str(ROOT / "fixtures" / "code-graph" / "routes" / "python"),
                        "--output",
                        str(output),
                    ]
                ),
                0,
            )
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(output.read_bytes(), oracle.canonical_bytes(report))


if __name__ == "__main__":
    unittest.main()
