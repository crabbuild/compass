from __future__ import annotations

import copy
import json
from pathlib import Path
import tempfile
import unittest

from benchmarks.performance.compass.audit import (
    AuditError,
    audit_result_json_value,
    load_manifest,
    run_audit,
    wilson_interval,
)


FIXTURES = Path(__file__).parent / "fixtures"
BASE_MANIFEST = Path(__file__).parents[1] / "audits" / "universal-core.json"
BASE_GRAPH = FIXTURES / "audit_graph.json"
BASE_CORPUS = FIXTURES / "audit_corpus"


class AuditTests(unittest.TestCase):
    def setUp(self) -> None:
        self.base = json.loads(BASE_MANIFEST.read_text(encoding="utf-8"))

    def write_manifest(self, value: dict[str, object], root: Path) -> Path:
        path = root / "audit.json"
        path.write_text(
            json.dumps(value, sort_keys=True, separators=(",", ":")),
            encoding="utf-8",
        )
        return path

    def test_wilson_interval_uses_two_sided_95_percent_bounds(self) -> None:
        interval = wilson_interval(5, 10)
        self.assertIsNotNone(interval)
        assert interval is not None
        self.assertAlmostEqual(0.236593090512564, interval.lower, places=14)
        self.assertAlmostEqual(0.763406909487436, interval.upper, places=14)
        self.assertIsNone(wilson_interval(0, 0))
        with self.assertRaisesRegex(AuditError, "0 <= successes <= total"):
            wilson_interval(2, 1)

    def test_conformance_result_is_deterministic_and_ineligible(self) -> None:
        first = run_audit(BASE_MANIFEST, BASE_GRAPH, BASE_CORPUS)
        second = run_audit(BASE_MANIFEST, BASE_GRAPH, BASE_CORPUS)
        self.assertEqual(first, second)
        payload = audit_result_json_value(first)
        self.assertFalse(payload["eligibleForQualityClaim"])
        self.assertEqual(6, payload["auditedAcceptedEdges"])
        self.assertEqual(2, payload["precision"]["numerator"])
        self.assertEqual(6, payload["precision"]["denominator"])
        self.assertEqual(
            {
                "cross_language_match": 1,
                "fabricated_occurrence": 1,
                "unsafe_local_substitution": 1,
            },
            payload["criticalViolations"],
        )
        encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"))
        self.assertEqual(
            encoded,
            json.dumps(
                audit_result_json_value(second),
                sort_keys=True,
                separators=(",", ":"),
            ),
        )

    def test_invalid_records_remain_in_precision_denominator(self) -> None:
        result = run_audit(BASE_MANIFEST, BASE_GRAPH, BASE_CORPUS)
        self.assertEqual(6, result.precision.denominator)
        self.assertEqual(2, result.precision.numerator)
        self.assertEqual(1, result.judgments["invalid"])

    def test_stale_snippet_and_graph_hashes_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stale_snippet = copy.deepcopy(self.base)
            stale_snippet["records"][0]["occurrence"]["snippetSha256"] = "f" * 64
            with self.assertRaisesRegex(AuditError, "stale snippet hash"):
                run_audit(
                    self.write_manifest(stale_snippet, root),
                    BASE_GRAPH,
                    BASE_CORPUS,
                )

            stale_graph = copy.deepcopy(self.base)
            stale_graph["corpora"][0]["graphSha256"] = "f" * 64
            with self.assertRaisesRegex(AuditError, "graph digest mismatch"):
                run_audit(
                    self.write_manifest(stale_graph, root),
                    BASE_GRAPH,
                    BASE_CORPUS,
                )

    def test_corpus_revision_mismatch_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / ".compass-audit-commit").write_text("1" * 40, encoding="utf-8")
            (root / "main.py").write_bytes((BASE_CORPUS / "main.py").read_bytes())
            with self.assertRaisesRegex(AuditError, "commit mismatch"):
                run_audit(BASE_MANIFEST, BASE_GRAPH, root)

    def test_duplicate_ids_and_unsafe_paths_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            duplicate = copy.deepcopy(self.base)
            duplicate["records"][1]["id"] = duplicate["records"][0]["id"]
            with self.assertRaisesRegex(AuditError, "duplicate record IDs"):
                load_manifest(self.write_manifest(duplicate, root))

            unsafe = copy.deepcopy(self.base)
            unsafe["records"][0]["occurrence"]["file"] = "../main.py"
            with self.assertRaisesRegex(AuditError, "safe relative path"):
                load_manifest(self.write_manifest(unsafe, root))

    def test_absent_required_graph_fact_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            missing = copy.deepcopy(self.base)
            missing["records"][0]["target"]["nodeId"] = "not-in-graph"
            with self.assertRaisesRegex(AuditError, "absent graph fact"):
                run_audit(
                    self.write_manifest(missing, root),
                    BASE_GRAPH,
                    BASE_CORPUS,
                )

    def test_occurrence_ranges_must_be_positive_and_non_empty(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            invalid = copy.deepcopy(self.base)
            invalid["records"][0]["occurrence"]["endByte"] = 17
            with self.assertRaisesRegex(AuditError, "positive non-empty byte range"):
                load_manifest(self.write_manifest(invalid, root))

    def test_qualification_minimums_are_fixed_and_cannot_be_overridden(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            undersized = copy.deepcopy(self.base)
            undersized["mode"] = "qualification"
            result = run_audit(
                self.write_manifest(undersized, root),
                BASE_GRAPH,
                BASE_CORPUS,
            )
            self.assertFalse(result.eligible_for_quality_claim)
            self.assertTrue(
                any("2000 required" in failure for failure in result.failures),
                result.failures,
            )
            self.assertTrue(
                any("400 required" in failure for failure in result.failures),
                result.failures,
            )
            self.assertTrue(
                any("100 required" in failure for failure in result.failures),
                result.failures,
            )

    def test_manifest_requires_declared_capability_and_corpus_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            undeclared = copy.deepcopy(self.base)
            undeclared["records"][0]["capability"] = "ownership"
            with self.assertRaisesRegex(AuditError, "unadvertised capability"):
                load_manifest(self.write_manifest(undeclared, root))

            wrong_corpus = copy.deepcopy(self.base)
            wrong_corpus["records"][0]["corpus"] = "different"
            with self.assertRaisesRegex(AuditError, "unknown corpus"):
                load_manifest(self.write_manifest(wrong_corpus, root))


if __name__ == "__main__":
    unittest.main()
