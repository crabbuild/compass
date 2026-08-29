from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from benchmarks.performance.compass.multiplicity import audit_multiplicity


def edge(edge_id: str, start: int) -> dict[str, object]:
    return {
        "id": edge_id,
        "source": "source",
        "target": "target",
        "kind": "calls",
        "relationshipSite": {
            "file": "src/main.py",
            "startByte": start,
            "endByte": start + 4,
        },
    }


class MultiplicityTests(unittest.TestCase):
    def write_graph(self, root: Path, links: list[dict[str, object]]) -> Path:
        graph = root / "graph.json"
        graph.write_text(
            json.dumps({"directed": True, "nodes": [], "links": links}),
            encoding="utf-8",
        )
        return graph

    def test_distinct_parallel_occurrences_are_preserved_and_measured(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result = audit_multiplicity(
                self.write_graph(Path(temporary), [edge("one", 10), edge("two", 20)])
            )

        self.assertTrue(result["passed"])
        self.assertEqual(2, result["relationships"])
        self.assertEqual(1, result["semanticPairs"])
        self.assertEqual(1, result["parallelPairs"])
        self.assertEqual(1, result["repeatedOccurrences"])
        self.assertEqual(0, result["duplicatePairSites"])
        self.assertGreater(result["relationshipSerializedBytes"], 0)

    def test_duplicate_identity_or_site_fails_occurrence_integrity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result = audit_multiplicity(
                self.write_graph(Path(temporary), [edge("same", 10), edge("same", 10)])
            )

        self.assertFalse(result["passed"])
        self.assertEqual(1, result["duplicateEdgeIds"])
        self.assertEqual(1, result["duplicatePairSites"])

    def test_relationship_limit_fails_explicitly(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            graph = self.write_graph(
                Path(temporary), [edge("one", 10), edge("two", 20)]
            )
            with self.assertRaisesRegex(ValueError, "exceeds max_relationships=1"):
                audit_multiplicity(graph, max_relationships=1)
