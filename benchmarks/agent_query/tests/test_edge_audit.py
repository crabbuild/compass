from __future__ import annotations

import copy
from pathlib import Path
import tempfile
import unittest

from benchmarks.agent_query.edge_audit import audit_edges


class EdgeAuditTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "code.rs").write_text("fn caller() {}\nfn target() {}\ntarget();\ntarget();\n")
        self.witness = {"id": "calls", "expected": "present", "relation": "calls",
                        "source": {"file": "code.rs", "line": 1, "symbol": "caller", "text": "fn caller"},
                        "target": {"file": "code.rs", "line": 2, "symbol": "target", "text": "fn target"},
                        "occurrences": [{"file": "code.rs", "line": line, "text": "target();"}
                                        for line in (3, 4)]}
        self.graph = {"nodes": [
            {"id": symbol, "name": symbol, "source": {"file": "code.rs", "startLine": line}}
            for line, symbol in enumerate(("caller", "target"), 1)],
            "edges": [{"source": "caller", "target": "target", "kind": "calls",
                       "relationshipSite": {"file": "code.rs", "startLine": line}}
                      for line in (3, 4)]}

    def audit(self, graph=None, tool="compass"):
        return audit_edges({"witnesses": [self.witness]}, tool,
                           self.graph if graph is None else graph, self.root)[0]

    def test_complete_occurrences_match(self) -> None:
        result = self.audit()
        self.assertTrue(result["matched"])
        self.assertEqual(result["matchedOccurrences"], 2)

    def test_graphify_semantic_direction_matches_the_same_policy(self) -> None:
        graph = {"nodes": [{"id": symbol, "label": symbol + "()", "source_file": "code.rs",
                            "source_location": f"L{line}"}
                           for line, symbol in enumerate(("caller", "target"), 1)],
                 "links": [{"source": "target", "target": "caller", "_src": "caller", "_tgt": "target",
                            "relation": "calls", "source_file": "code.rs", "source_location": f"L{line}"}
                           for line in (3, 4)]}
        self.assertTrue(self.audit(graph, "graphify")["matched"])

    def test_one_edge_does_not_prove_all_occurrences(self) -> None:
        self.graph["edges"].pop()
        result = self.audit()
        self.assertTrue(result["relationshipMatched"])
        self.assertFalse(result["matched"])
        self.assertEqual(result["matchedOccurrences"], 1)
        self.assertEqual(result["missingOccurrences"], [{"file": "code.rs", "line": 4, "count": 1}])

    def test_duplicate_site_does_not_recover_a_missing_site(self) -> None:
        self.graph["edges"][1] = copy.deepcopy(self.graph["edges"][0])
        result = self.audit()
        self.assertFalse(result["matched"])
        self.assertEqual(result["matchedOccurrences"], 1)
        self.assertEqual(result["unexpectedOccurrences"], [{"file": "code.rs", "line": 3, "count": 1}])

    def test_negative_requires_both_endpoint_identities(self) -> None:
        self.witness.update(expected="absent", occurrences=[])
        self.assertFalse(self.audit()["matched"])
        self.graph["edges"] = []
        self.assertTrue(self.audit()["matched"])
        self.graph["nodes"].pop()
        self.assertFalse(self.audit()["matched"])

    def test_wrong_owner_or_direction_never_matches(self) -> None:
        for reverse in (False, True):
            graph = copy.deepcopy(self.graph)
            other = copy.deepcopy(graph["nodes"][1])
            other["id"] = "other-owner"
            other["source"]["startLine"] = 99
            other["source"]["file"] = "elsewhere.rs"
            graph["nodes"].append(other)
            for edge in graph["edges"]:
                if reverse:
                    edge["source"], edge["target"] = edge["target"], edge["source"]
                else:
                    edge["target"] = "other-owner"
            self.assertFalse(self.audit(graph)["relationshipMatched"])

    def test_ambiguous_anchors_and_invalid_ids_cannot_pass(self) -> None:
        rival = copy.deepcopy(self.graph["nodes"][0])
        rival["id"] = "another-caller"
        self.graph["nodes"].append(rival)
        result = self.audit()
        self.assertFalse(result["endpointIdentityVerified"])
        self.assertEqual(result["expectedOccurrences"], 2)
        rival["id"] = "caller"
        with self.assertRaisesRegex(ValueError, "unique nonempty"):
            self.audit()

    def test_source_drift_or_malformed_expectation_fails_before_scoring(self) -> None:
        self.witness["expected"] = "maybe"
        with self.assertRaisesRegex(ValueError, "unsupported edge expectation"):
            self.audit()
        self.witness["expected"] = "present"
        (self.root / "code.rs").write_text("changed\n")
        with self.assertRaisesRegex(ValueError, "source witness changed"):
            self.audit()


if __name__ == "__main__":
    unittest.main()
