from __future__ import annotations

import copy
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

from benchmarks.performance.compass.audit import (
    AUDIT_RESULT_SCHEMA,
    AuditError,
    audit_result_json_value,
    export_comparison_candidates,
    load_manifest,
    run_audit,
    wilson_interval,
)
from benchmarks.performance.compass.correctness import index_graph
from benchmarks.performance.compass.occurrences import (
    independent_source_inventory,
    source_construct_inventory_sha256,
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
        self.assertEqual(payload["schema"], AUDIT_RESULT_SCHEMA)
        self.assertFalse(payload["eligibleForQualityClaim"])
        self.assertEqual(6, payload["auditedAcceptedEdges"])
        self.assertEqual(2, payload["precision"]["numerator"])
        self.assertEqual(6, payload["precision"]["denominator"])
        self.assertEqual(0.4, payload["f1"])
        self.assertEqual(1, payload["ambiguity"]["numerator"])
        self.assertEqual(3, payload["ambiguity"]["denominator"])
        self.assertEqual(
            {
                "python": {
                    "scannedFiles": 1,
                    "parsedFiles": 1,
                    "unsupportedFiles": 0,
                    "coverage": 1.0,
                }
            },
            payload["sourceCoverage"],
        )
        self.assertEqual(0.4, payload["strata"]["language"]["python"]["f1"])
        self.assertEqual(
            1 / 3,
            payload["strata"]["language"]["python"]["ambiguityRate"],
        )
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

    def test_partial_source_population_is_reportable_but_not_complete(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            partial = copy.deepcopy(self.base)
            partial["sourceOracles"][0]["scannedFiles"] = 2
            partial["sourceOracles"][0]["parsedFiles"] = 1
            manifest = load_manifest(self.write_manifest(partial, Path(temporary)))

        self.assertEqual(2, manifest.source_oracles[0].scanned_files)
        self.assertEqual(1, manifest.source_oracles[0].parsed_files)

    def test_partial_source_population_fails_qualification_with_coverage(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / ".compass-audit-commit").write_text("0" * 40, encoding="utf-8")
            (root / "main.py").write_bytes((BASE_CORPUS / "main.py").read_bytes())
            (root / "broken.py").write_text("def broken(:\n", encoding="utf-8")
            inventory = independent_source_inventory(root, "python")
            partial = copy.deepcopy(self.base)
            partial["mode"] = "qualification"
            partial["sourceOracles"][0].update(
                {
                    "scannedFiles": inventory.scanned_files,
                    "parsedFiles": inventory.parsed_files,
                    "rejectedFiles": list(inventory.rejected_files),
                    "inventorySha256": source_construct_inventory_sha256(
                        "python", inventory
                    ),
                }
            )
            result = run_audit(
                self.write_manifest(partial, root),
                BASE_GRAPH,
                root,
            )
            loaded = load_manifest(self.write_manifest(partial, root))

        self.assertEqual(1, result.source_coverage["python"]["unsupportedFiles"])
        self.assertEqual(("broken.py",), loaded.source_oracles[0].rejected_files)
        self.assertTrue(
            any("complete source coverage is required" in failure for failure in result.failures),
            result.failures,
        )

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

    def test_represented_elsewhere_exact_edge_pointer_preserves_source_occurrence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            graph = json.loads(BASE_GRAPH.read_text(encoding="utf-8"))
            represented_edge = next(
                edge
                for edge in graph["links"]
                if edge["source"] == "alpha"
                and edge["target"] == "beta"
                and edge["relationshipSite"]["startByte"] == 4
            )
            represented_edge.update(
                {
                    "id": "sha256:" + "a" * 64,
                    "occurrenceRule": "framework-route-stage:handler:0",
                    "evidence": [
                        {
                            "origin": "ast",
                            "extractor": "compass.frameworks.django",
                            "confidence": "exact",
                            "rule": "framework-route-stage:handler:0",
                        }
                    ],
                }
            )
            graph_path = root / "graph.json"
            graph_path.write_text(
                json.dumps(graph, sort_keys=True, separators=(",", ":")),
                encoding="utf-8",
            )
            manifest = copy.deepcopy(self.base)
            manifest["corpora"][0]["graphSha256"] = hashlib.sha256(
                graph_path.read_bytes()
            ).hexdigest()
            record = next(
                record
                for record in manifest["records"]
                if record["judgment"] == "represented_elsewhere"
            )
            record["source"]["nodeId"] = "oracle-child-route"
            record["target"]["nodeId"] = "oracle-child-handler"
            record["representation"] = {
                "source": "alpha",
                "target": "beta",
                "relation": "calls",
                "edgeId": represented_edge["id"],
                "extractor": "compass.frameworks.django",
                "rule": "framework-route-stage:handler:0",
            }
            manifest_path = self.write_manifest(manifest, root)
            result = run_audit(manifest_path, graph_path, BASE_CORPUS)
            self.assertEqual(1, result.judgments["represented_elsewhere"])

            stale = copy.deepcopy(manifest)
            stale_record = next(
                item
                for item in stale["records"]
                if item["judgment"] == "represented_elsewhere"
            )
            stale_record["representation"]["rule"] = "framework-route-stage:handler:1"
            with self.assertRaisesRegex(AuditError, "representation edge is absent or stale"):
                run_audit(self.write_manifest(stale, root), graph_path, BASE_CORPUS)

            incomplete = copy.deepcopy(manifest)
            incomplete_record = next(
                item
                for item in incomplete["records"]
                if item["judgment"] == "represented_elsewhere"
            )
            del incomplete_record["representation"]["extractor"]
            with self.assertRaisesRegex(AuditError, "requires edgeId, extractor, and rule"):
                load_manifest(self.write_manifest(incomplete, root))

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

    def test_source_oracle_inventory_is_complete_and_recomputed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            incomplete = copy.deepcopy(self.base)
            incomplete["sourceOracles"][0]["parsedFiles"] = 0
            incomplete_manifest = self.write_manifest(incomplete, root)
            loaded = load_manifest(incomplete_manifest)
            self.assertEqual(0, loaded.source_oracles[0].parsed_files)
            with self.assertRaisesRegex(AuditError, "coverage mismatch"):
                run_audit(incomplete_manifest, BASE_GRAPH, BASE_CORPUS)

            unpinned = copy.deepcopy(self.base)
            unpinned["sourceOracles"] = []
            with self.assertRaisesRegex(AuditError, "no pinned source-oracle"):
                load_manifest(self.write_manifest(unpinned, root))

            stale = copy.deepcopy(self.base)
            stale["sourceOracles"][0]["inventorySha256"] = "f" * 64
            manifest = self.write_manifest(stale, root)
            with self.assertRaisesRegex(AuditError, "inventory digest mismatch"):
                run_audit(manifest, BASE_GRAPH, BASE_CORPUS)

    def test_comparison_candidate_export_is_pinned_deterministic_and_unjudged(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            corpus = root / "corpus"
            corpus.mkdir()
            (corpus / ".compass-audit-commit").write_text(
                "1" * 40, encoding="utf-8"
            )
            (corpus / "main.py").write_text("target(); target()\n", encoding="utf-8")
            nodes = (
                '{"id":"source","label":"run()","kind":"function",'
                '"source_file":"main.py","source_location":"L1","language":"python"},'
                '{"id":"target","label":"target()","kind":"function",'
                '"source_file":"main.py","source_location":"L1","language":"python"}'
            )
            compass = root / "compass.json"
            compass.write_text(
                '{"graph":{"diagnostics":[]},"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target",'
                '"relation":"calls","relationshipSite":{"file":"main.py",'
                '"startLine":1,"startByte":0,"endByte":8}},'
                '{"source":"source","target":"target","relation":"calls",'
                '"relationshipSite":{"file":"main.py","startLine":1,'
                '"startByte":10,"endByte":18}}]}',
                encoding="utf-8",
            )
            graphify = root / "graphify.json"
            graphify.write_text(
                '{"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target",'
                '"relation":"calls","source_file":"main.py",'
                '"source_location":"L1"}]}',
                encoding="utf-8",
            )
            database_path = root / "comparison.sqlite"
            with closing(sqlite3.connect(database_path)) as database:
                index_graph("compass", compass, database)
                index_graph("graphify", graphify, database)

            first = root / "first.json"
            second = root / "second.json"
            export_comparison_candidates(
                database_path, compass, corpus, "fixture", "python", first
            )
            export_comparison_candidates(
                database_path, compass, corpus, "fixture", "python", second
            )
            self.assertEqual(first.read_bytes(), second.read_bytes())
            payload = json.loads(first.read_text(encoding="utf-8"))
            self.assertEqual(payload["schema"], "compass.quality-audit-candidates/2")
            self.assertEqual(payload["producer"], "python")
            self.assertTrue(payload["recordsAreUnjudged"])
            self.assertEqual(payload["corpus"]["commit"], "1" * 40)
            self.assertEqual(
                payload["populations"],
                {
                    "compassAccepted": 2,
                    "graphifyHypotheses": 1,
                    "sourceOracle": 2,
                },
            )
            self.assertEqual(
                payload["sourceOracleCoverage"],
                {
                    "provider": "python_ast",
                    "providerAvailable": True,
                    "scannedFiles": 1,
                    "parsedFiles": 1,
                    "rejectedFiles": [],
                    "inventorySha256": (
                        "cbaa05744dcb8ecb2254ee981b53b2d51b630e2f8934875af4e848326e15fbc4"
                    ),
                    "complete": True,
                },
            )
            self.assertEqual(len(payload["candidates"]), 5)
            self.assertTrue(
                all(candidate["judgment"] is None for candidate in payload["candidates"])
            )
            self.assertTrue(
                all(candidate["producer"] == "python" for candidate in payload["candidates"])
            )
            accepted = [
                candidate
                for candidate in payload["candidates"]
                if candidate["candidateSource"] == "compass_graph"
            ]
            self.assertEqual(len(accepted), 2)
            self.assertEqual(
                {candidate["occurrence"]["startByte"] for candidate in accepted},
                {0, 10},
            )
            self.assertTrue(
                all(
                    not candidate["occurrence"]["requiresExactGraphRange"]
                    for candidate in accepted
                )
            )
            hypothesis = next(
                candidate
                for candidate in payload["candidates"]
                if candidate["candidateSource"] == "graphify_comparison"
            )
            self.assertEqual(hypothesis["comparison"]["status"], "exact")
            self.assertTrue(hypothesis["occurrence"]["requiresExactGraphRange"])
            source_oracle = [
                candidate
                for candidate in payload["candidates"]
                if candidate["candidateSource"] == "independent_source"
            ]
            self.assertEqual(len(source_oracle), 2)
            self.assertEqual(
                {candidate["target"]["spelling"] for candidate in source_oracle},
                {"target"},
            )
            self.assertTrue(
                all(
                    candidate["suggestedPool"] == "source_oracle"
                    for candidate in source_oracle
                )
            )

            unrelated = root / "unrelated.json"
            unrelated.write_text(
                '{"graph":{"diagnostics":[]},"nodes":[],"links":[]}',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AuditError, "does not match"):
                export_comparison_candidates(
                    database_path,
                    unrelated,
                    corpus,
                    "fixture",
                    "python",
                    root / "unrelated-candidates.json",
                )


if __name__ == "__main__":
    unittest.main()
