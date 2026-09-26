from __future__ import annotations

import copy
from pathlib import Path
import tempfile
import unittest

from benchmarks.agent_query.path_audit import audit_path, parse_path


class PathAuditTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "code.rs").write_text("fn start() {}\nfn finish() {}\nfinish();\n")
        self.witness = {
            "category": "call",
            "nodes": [
                {"file": "code.rs", "line": 1, "text": "fn start()", "labels": ["start()"]},
                {"file": "code.rs", "line": 2, "text": "fn finish()", "labels": ["finish()"]},
            ],
            "steps": [{"relations": ["calls"], "direction": "forward",
                       "site": {"file": "code.rs", "line": 3, "text": "finish();"}}],
        }
        self.compass = {
            "nodes": [
                {"id": "s", "name": "start()", "source": {"file": "code.rs", "startLine": 1}},
                {"id": "t", "name": "finish()", "source": {"file": "code.rs", "startLine": 2}},
            ],
            "edges": [{"source": "s", "target": "t", "kind": "calls",
                       "relationshipSite": {"file": "code.rs", "startLine": 3}}],
        }
        self.graphify = {
            "nodes": [
                {"id": "s", "label": "start()", "source_file": "code.rs", "source_location": "L1"},
                {"id": "t", "label": "finish()", "source_file": "code.rs", "source_location": "L2"},
            ],
            "links": [{"source": "t", "target": "s", "_src": "s", "_tgt": "t",
                       "relation": "calls", "source_file": "code.rs", "source_location": "L3"}],
        }
        self.output = "Shortest path (1 hops):\n  start() --calls [EXTRACTED]--> finish()\n"

    def audit(self, graph=None, output=None, tool="compass"):
        return audit_path(self.witness, tool, graph or self.compass,
                          self.output if output is None else output, self.root)

    def test_both_formats_and_semantic_direction_are_supported(self) -> None:
        self.assertTrue(self.audit()["matched"])
        result = self.audit(self.graphify, tool="graphify")
        self.assertTrue(result["matched"])
        self.assertTrue(result["steps"][0]["reviewedSiteSupported"])
        compass = self.output.replace("Shortest path (1 hops)", "Best path (weighted, 1 hops, weight 1)")
        self.assertTrue(self.audit(output=compass)["matched"])

    def test_endpoint_mentions_and_false_headers_do_not_prove_a_path(self) -> None:
        for text in ("start() finish()", "No path found between start() and finish()",
                     self.output.replace("1 hops", "2 hops"), self.output + self.output):
            with self.subTest(text=text):
                self.assertFalse(self.audit(output=text)["matched"])

    def test_wrong_printed_direction_and_relation_fail(self) -> None:
        for text in (self.output.replace("--calls [EXTRACTED]-->", "<--calls [EXTRACTED]--"),
                     self.output.replace("--calls", "--references")):
            self.assertFalse(self.audit(output=text)["matched"])

    def test_missing_or_reversed_graph_edge_fails(self) -> None:
        graph = copy.deepcopy(self.compass)
        graph["edges"] = []
        self.assertFalse(self.audit(graph)["matched"])
        graph["edges"] = [{"source": "t", "target": "s", "kind": "calls"}]
        self.assertFalse(self.audit(graph)["matched"])

    def test_wrong_occurrence_and_wrong_declaration_fail(self) -> None:
        for field in ("site", "declaration"):
            graph = copy.deepcopy(self.compass)
            if field == "site":
                graph["edges"][0]["relationshipSite"]["startLine"] = 2
            else:
                graph["nodes"][0]["source"]["startLine"] = 2
            self.assertFalse(self.audit(graph)["matched"])

    def test_ambiguous_labels_are_not_resolved_using_the_expected_answer(self) -> None:
        graph = copy.deepcopy(self.compass)
        rival = copy.deepcopy(graph["nodes"][0])
        rival["id"] = "other"
        rival["source"]["file"] = "elsewhere.rs"
        graph["nodes"].append(rival)
        result = self.audit(graph)
        self.assertFalse(result["matched"])
        self.assertIn("unverified identity", result["failures"][0])

    def test_source_witness_drift_fails_instead_of_scoring(self) -> None:
        (self.root / "code.rs").write_text("changed\n")
        with self.assertRaisesRegex(ValueError, "source witness changed"):
            self.audit()

    def test_coarse_relation_uses_its_own_reviewed_source_site(self) -> None:
        step = self.witness["steps"][0]
        step["relations"].append("references")
        step["sitesByRelation"] = {
            "calls": step.pop("site"),
            "references": {"file": "code.rs", "line": 1, "text": "fn start()"},
        }
        graph = copy.deepcopy(self.compass)
        graph["edges"][0]["kind"] = "references"
        output = self.output.replace("--calls", "--references")
        self.assertFalse(self.audit(graph, output)["matched"])
        graph["edges"][0]["relationshipSite"]["startLine"] = 1
        self.assertTrue(self.audit(graph, output)["matched"])
        graph["edges"][0]["relationshipSite"]["startLine"] = True
        self.assertFalse(self.audit(graph, output)["matched"])

    def test_invalid_or_duplicate_ids_fail_instead_of_scoring(self) -> None:
        for identity in (None, "", 1, "s"):
            graph = copy.deepcopy(self.compass)
            graph["nodes"][1]["id"] = identity
            with self.subTest(identity=identity), self.assertRaisesRegex(ValueError, "graph node IDs"):
                self.audit(graph)

    def test_hop_count_is_bounded(self) -> None:
        with self.assertRaises(ValueError):
            parse_path("Shortest path (1000000 hops):\n  start()\n")


if __name__ == "__main__":
    unittest.main()
