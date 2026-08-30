"""Contract tests for the Python framework qualification skeleton."""

from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import qualify_python_frameworks as qualification  # noqa: E402


class PythonFrameworkQualificationTests(unittest.TestCase):
    def test_checkout_root_defaults_to_mounted_github_root(self) -> None:
        environment = dict(os.environ)
        environment.pop(qualification.CHECKOUT_ROOT_ENV, None)
        self.assertEqual(
            qualification.checkout_root(environment),
            Path("/Volumes/Workspace/Github"),
        )

    def test_checkout_root_accepts_contained_absolute_override(self) -> None:
        root = Path("/Volumes/Workspace/Github/qualification-clean")
        with mock.patch.dict(os.environ, {qualification.CHECKOUT_ROOT_ENV: str(root)}):
            self.assertEqual(qualification.checkout_root(), root)
            self.assertEqual(
                qualification.checkout_for("https://github.com/fastapi/fastapi.git", root),
                root / "fastapi" / "fastapi",
            )

    def test_checkout_root_rejects_relative_and_escaping_overrides(self) -> None:
        for override in (
            "qualification-clean",
            "/Volumes/Workspace/Github/../outside",
            "/tmp/qualification-clean",
        ):
            with self.subTest(override=override):
                with self.assertRaises(qualification.QualificationError):
                    qualification.checkout_root({qualification.CHECKOUT_ROOT_ENV: override})

    def test_fixture_report_is_deterministic_and_explicitly_unqualified(self) -> None:
        first = qualification.fixture_report()
        second = qualification.fixture_report()
        self.assertEqual(qualification.canonical_bytes(first), qualification.canonical_bytes(second))
        self.assertEqual(first["schema"], qualification.SCHEMA)
        self.assertEqual(first["status"], "established-unqualified")
        self.assertFalse(first["productionQualified"])
        self.assertEqual(first["pythonProducer"]["version"], 1)
        self.assertEqual(first["expectations"], 17)
        self.assertEqual(len(first["expectedGaps"]), 3)

    def test_fixture_ledgers_have_exact_source_ranges(self) -> None:
        expectations = qualification.load_json(qualification.EXPECTATIONS)
        frameworks = qualification.validate_expectations(expectations, qualification.FIXTURE_ROOT)
        self.assertEqual(
            frameworks,
            {
                "django": 4,
                "django-rest-framework": 2,
                "fastapi": 4,
                "celery": 1,
                "flask": 3,
                "pydantic": 1,
                "sqlalchemy": 1,
                "starlette": 1,
            },
        )
        for record in expectations["records"]:
            source = (qualification.FIXTURE_ROOT / record["sourceFile"]).read_bytes()
            self.assertTrue(source[record["startByte"] : record["endByte"]].strip())

    def test_baseline_cannot_be_mistaken_for_a_quality_claim(self) -> None:
        baseline = qualification.load_json(qualification.BASELINE)
        qualification.validate_baseline(baseline, 17, 3)
        self.assertEqual(baseline["qualification"]["acceptedRelationships"], 0)
        self.assertFalse(baseline["qualification"]["eligibleForProductionClaim"])

    def test_cli_report_is_canonical_machine_json(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-python-framework-report-") as directory:
            output = Path(directory) / "report.json"
            self.assertEqual(
                qualification.main(["--fixtures-only", "--output", str(output)]),
                0,
            )
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(output.read_bytes(), qualification.canonical_bytes(report))


if __name__ == "__main__":
    unittest.main()
