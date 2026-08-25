"""Contract tests for the Python framework qualification skeleton."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import qualify_python_frameworks as qualification  # noqa: E402


class PythonFrameworkQualificationTests(unittest.TestCase):
    def test_fixture_report_is_deterministic_and_explicitly_unqualified(self) -> None:
        first = qualification.fixture_report()
        second = qualification.fixture_report()
        self.assertEqual(qualification.canonical_bytes(first), qualification.canonical_bytes(second))
        self.assertEqual(first["schema"], qualification.SCHEMA)
        self.assertEqual(first["status"], "established-unqualified")
        self.assertFalse(first["productionQualified"])
        self.assertEqual(first["pythonProducer"]["version"], 11)
        self.assertEqual(first["expectations"], 7)
        self.assertEqual(len(first["expectedGaps"]), 9)

    def test_fixture_ledgers_have_exact_source_ranges(self) -> None:
        expectations = qualification.load_json(qualification.EXPECTATIONS)
        frameworks = qualification.validate_expectations(expectations, qualification.FIXTURE_ROOT)
        self.assertEqual(frameworks, {"django": 2, "fastapi": 3, "flask": 2})
        for record in expectations["records"]:
            source = (qualification.FIXTURE_ROOT / record["sourceFile"]).read_bytes()
            self.assertTrue(source[record["startByte"] : record["endByte"]].strip())

    def test_baseline_cannot_be_mistaken_for_a_quality_claim(self) -> None:
        baseline = qualification.load_json(qualification.BASELINE)
        qualification.validate_baseline(baseline, 7, 9)
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
