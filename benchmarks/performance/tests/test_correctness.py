from __future__ import annotations

from pathlib import Path
import sqlite3
import tempfile
import unittest

from benchmarks.performance.compass_perf.correctness import (
    canonical_graph_digest,
    compare_graphs,
    index_graph,
)


FIXTURES = Path(__file__).parent / "fixtures"


class CorrectnessTests(unittest.TestCase):
    def test_compass_superset_passes_shared_fact_comparison(self) -> None:
        database = sqlite3.connect(":memory:")
        compass = index_graph("compass", FIXTURES / "compass_graph.json", database)
        graphify = index_graph("graphify", FIXTURES / "graphify_graph.json", database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)
        self.assertGreater(compass.nodes, graphify.nodes)
        self.assertEqual(compass.digest, canonical_graph_digest(database, "compass"))

    def test_storage_order_does_not_change_digest(self) -> None:
        database = sqlite3.connect(":memory:")
        first = index_graph("compass", FIXTURES / "compass_graph.json", database)
        with tempfile.TemporaryDirectory() as directory:
            reordered = Path(directory) / "graph.json"
            reordered.write_text(
                """
                {"links":[
                  {"relation":"routes_to","target":"a","source":"c","confidence":"EXTRACTED"},
                  {"relation":"calls","target":"b","source":"a","confidence":"EXTRACTED"}
                ],"nodes":[
                  {"source_location":"L3","source_file":"src/c.py","kind":"route","label":"CompassOnly","id":"c"},
                  {"source_location":"L2","source_file":"src/b.py","kind":"function","label":"Beta","id":"b"},
                  {"source_location":"L1","source_file":"src/a.py","kind":"function","label":"Alpha","id":"a"}
                ],"graph":{"schema":"compass.graph/1","diagnostics":[]}}
                """,
                encoding="utf-8",
            )
            second = index_graph("compass", reordered, database)
        self.assertEqual(first.digest, second.digest)

    def test_missing_shared_node_fails(self) -> None:
        database = sqlite3.connect(":memory:")
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "compass.json"
            graph.write_text(
                '{"graph":{},"nodes":[{"id":"a","label":"Alpha","kind":"function",'
                '"source_file":"src/a.py","source_location":"L1"}],"links":[]}',
                encoding="utf-8",
            )
            index_graph("compass", graph, database)
        index_graph("graphify", FIXTURES / "graphify_graph.json", database)
        result = compare_graphs(database)
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 1)

    def test_dangling_edge_is_rejected(self) -> None:
        database = sqlite3.connect(":memory:")
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "graph.json"
            graph.write_text(
                '{"graph":{},"nodes":[{"id":"a"}],'
                '"links":[{"source":"a","target":"missing","relation":"calls"}]}',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "dangling"):
                index_graph("compass", graph, database)

    def test_conflicting_duplicate_id_is_rejected(self) -> None:
        database = sqlite3.connect(":memory:")
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "graph.json"
            graph.write_text(
                '{"graph":{},"nodes":[{"id":"a","label":"One"},{"id":"a","label":"Two"}],'
                '"links":[]}',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "conflicting"):
                index_graph("compass", graph, database)


if __name__ == "__main__":
    unittest.main()

