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


def compare_documents(
    compass_document: str, graphify_document: str
):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        compass = root / "compass.json"
        graphify = root / "graphify.json"
        compass.write_text(compass_document, encoding="utf-8")
        graphify.write_text(graphify_document, encoding="utf-8")
        database = sqlite3.connect(":memory:")
        index_graph("compass", compass, database)
        index_graph("graphify", graphify, database)
        return compare_graphs(database)


class CorrectnessTests(unittest.TestCase):
    def test_compass_superset_passes_shared_fact_comparison(self) -> None:
        database = sqlite3.connect(":memory:")
        compass = index_graph("compass", FIXTURES / "compass_graph.json", database)
        graphify = index_graph("graphify", FIXTURES / "graphify_graph.json", database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)
        self.assertGreater(compass.nodes, graphify.nodes)
        self.assertEqual(compass.digest, canonical_graph_digest(database, "compass"))
        self.assertEqual(result.digest, compare_graphs(database).digest)

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

    def test_v1_compass_nodes_match_graphify_by_source_fact_not_internal_id(self) -> None:
        database = sqlite3.connect(":memory:")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compass = root / "compass.json"
            graphify = root / "graphify.json"
            compass.write_text(
                """
                {
                  "graph":{"schema":"compass.code-graph/1","diagnostics":[]},
                  "nodes":[
                    {"id":"sha256:file","kind":"file","name":"base.py",
                     "source":{"file":"src/base.py","startLine":1}},
                    {"id":"sha256:function","kind":"function","name":"run",
                     "source":{"file":"src/base.py","startLine":12}}
                  ],
                  "edges":[
                    {"source":"sha256:file","target":"sha256:function","kind":"contains"}
                  ]
                }
                """,
                encoding="utf-8",
            )
            graphify.write_text(
                """
                {
                  "nodes":[
                    {"id":"src_base","label":"src/base.py","source_file":"src/base.py",
                     "source_location":"L1"},
                    {"id":"src_base_run","label":"run()","source_file":"src/base.py",
                     "source_location":"L12"}
                  ],
                  "links":[
                    {"source":"src_base","target":"src_base_run","relation":"contains"}
                  ]
                }
                """,
                encoding="utf-8",
            )
            index_graph("compass", compass, database)
            index_graph("graphify", graphify, database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 0)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)

    def test_shared_relation_projection_accepts_more_precise_compass_edges(self) -> None:
        database = sqlite3.connect(":memory:")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compass = root / "compass.json"
            graphify = root / "graphify.json"
            nodes = (
                '{"id":"source","label":"Source","source_file":"src/a.py","source_location":"L1"},'
                '{"id":"target","label":"Target","source_file":"src/b.py","source_location":"L2"}'
            )
            compass.write_text(
                '{"graph":{"diagnostics":[]},"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target","relation":"instantiates"}]}',
                encoding="utf-8",
            )
            graphify.write_text(
                '{"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target","relation":"calls"}]}',
                encoding="utf-8",
            )
            index_graph("compass", compass, database)
            index_graph("graphify", graphify, database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)

    def test_rationale_facts_match_by_source_anchor_across_schema_names(self) -> None:
        database = sqlite3.connect(":memory:")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compass = root / "compass.json"
            graphify = root / "graphify.json"
            compass.write_text(
                """
                {"graph":{"diagnostics":[]},"nodes":[
                  {"id":"r","kind":"resource","name":"Long rationale without ellipsis",
                   "source":{"file":"src/a.py","startLine":9},
                   "details":{"type":"resource","data":{"resourceKind":"rationale"}}},
                  {"id":"f","kind":"function","name":"run",
                   "source":{"file":"src/a.py","startLine":10}}
                ],"edges":[{"source":"r","target":"f","kind":"documents"}]}
                """,
                encoding="utf-8",
            )
            graphify.write_text(
                """
                {"nodes":[
                  {"id":"legacy_r","label":"Long rationale…","file_type":"rationale",
                   "source_file":"src/a.py","source_location":"L9"},
                  {"id":"legacy_f","label":"run()","source_file":"src/a.py","source_location":"L10"}
                ],"links":[{"source":"legacy_r","target":"legacy_f",
                            "relation":"rationale_for"}]}
                """,
                encoding="utf-8",
            )
            index_graph("compass", compass, database)
            index_graph("graphify", graphify, database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)

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

    def test_unique_generated_receiver_and_occurrence_edge_are_dominated(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"caller","label":"run()","kind":"function",
               "source_file":"pkg/call.go","source_location":"L5","language":"go"},
              {"id":"type","label":"Widget","kind":"class",
               "source_file":"pkg/schema.go","source_location":"L1","language":"go"},
              {"id":"stub","label":"Widget","kind":"type_alias","language":"go"}
            ],"links":[
              {"source":"caller","target":"stub","relation":"references",
               "source_file":"pkg/call.go","source_location":"L9"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_caller","label":"run()",
               "source_file":"pkg/call.go","source_location":"L5"},
              {"id":"generated_receiver","label":"Widget",
               "source_file":"pkg/generated.go","source_location":"L20"}
            ],"links":[
              {"source":"legacy_caller","target":"generated_receiver","relation":"uses",
               "source_file":"pkg/call.go","source_location":"L9"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_nodes"], 1)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)

    def test_same_label_cross_package_definition_is_ambiguous(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"left","label":"Agent","kind":"class",
               "source_file":"pkg/left/agent.go","source_location":"L1","language":"go"},
              {"id":"right","label":"Agent","kind":"class",
               "source_file":"pkg/right/agent.go","source_location":"L1","language":"go"}
            ],"links":[]}
            """,
            """
            {"nodes":[{"id":"receiver","label":"Agent"}],"links":[]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["ambiguous_graphify_nodes"], 1)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 0)

    def test_two_hop_containment_dominates_flat_graphify_ownership(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"a.go","kind":"file",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"owner","label":"Widget","kind":"class",
               "source_file":"pkg/a.go","source_location":"L2"},
              {"id":"method","label":"run()","kind":"method",
               "source_file":"pkg/a.go","source_location":"L3"}
            ],"links":[
              {"source":"file","target":"owner","relation":"contains"},
              {"source":"owner","target":"method","relation":"contains"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_file","label":"pkg/a.go",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"legacy_method","label":"run()",
               "source_file":"pkg/a.go","source_location":"L3"}
            ],"links":[
              {"source":"legacy_file","target":"legacy_method","relation":"contains"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:containment_path",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_three_hop_containment_and_different_call_sites_fail_closed(self) -> None:
        containment = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"a.go","kind":"file",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"outer","label":"Outer","kind":"class",
               "source_file":"pkg/a.go","source_location":"L2"},
              {"id":"inner","label":"Inner","kind":"class",
               "source_file":"pkg/a.go","source_location":"L3"},
              {"id":"method","label":"run()","kind":"method",
               "source_file":"pkg/a.go","source_location":"L4"}
            ],"links":[
              {"source":"file","target":"outer","relation":"contains"},
              {"source":"outer","target":"inner","relation":"contains"},
              {"source":"inner","target":"method","relation":"contains"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_file","label":"pkg/a.go",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"legacy_method","label":"run()",
               "source_file":"pkg/a.go","source_location":"L4"}
            ],"links":[
              {"source":"legacy_file","target":"legacy_method","relation":"contains"}
            ]}
            """,
        )
        self.assertFalse(containment.passed)
        self.assertEqual(containment.metrics["missing_graphify_edges"], 1)

        occurrence = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"caller","label":"run()","kind":"function",
               "source_file":"pkg/a.go","source_location":"L1","language":"go"},
              {"id":"type","label":"Widget","kind":"class",
               "source_file":"pkg/a.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"caller","target":"type","relation":"references",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_caller","label":"run()",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"receiver","label":"Widget",
               "source_file":"pkg/generated.go","source_location":"L5"}
            ],"links":[
              {"source":"legacy_caller","target":"receiver","relation":"uses",
               "source_file":"pkg/a.go","source_location":"L11"}
            ]}
            """,
        )
        self.assertFalse(occurrence.passed)
        self.assertEqual(occurrence.metrics["missing_graphify_edges"], 1)


if __name__ == "__main__":
    unittest.main()
