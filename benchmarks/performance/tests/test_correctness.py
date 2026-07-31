from __future__ import annotations

from contextlib import closing
from pathlib import Path
import sqlite3
import tempfile
import unittest

from benchmarks.performance.compass.correctness import (
    canonical_graph_digest,
    compare_graphs,
    index_graph,
)
from benchmarks.performance.compass.occurrences import (
    independent_source_constructs,
    independent_source_inventory,
)


FIXTURES = Path(__file__).parent / "fixtures"


def compare_documents(
    compass_document: str,
    graphify_document: str,
    source_documents: dict[str, str] | None = None,
):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        compass = root / "compass.json"
        graphify = root / "graphify.json"
        compass.write_text(compass_document, encoding="utf-8")
        graphify.write_text(graphify_document, encoding="utf-8")
        for source_file, document in (source_documents or {}).items():
            destination = root / source_file
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(document, encoding="utf-8")
        with closing(sqlite3.connect(":memory:")) as database:
            index_graph("compass", compass, database)
            index_graph("graphify", graphify, database)
            return compare_graphs(database, root if source_documents is not None else None)


class CorrectnessTests(unittest.TestCase):
    def database(self) -> sqlite3.Connection:
        database = sqlite3.connect(":memory:")
        self.addCleanup(database.close)
        return database

    def test_independent_python_source_oracle_preserves_exact_bytes_and_owners(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "pkg" / "module.py"
            source.parent.mkdir()
            document = (
                "@decorate(factory())\n"
                "def outer():\n"
                "    café = 1\n"
                "    run()\n"
                "    service.run(\n"
                "        café,\n"
                "    )\n"
                "    def inner():\n"
                "        return finish()\n"
            )
            source.write_text(document, encoding="utf-8")

            constructs = independent_source_constructs(root, "python")
            calls = [construct for construct in constructs if construct.relation == "calls"]
            self.assertEqual(
                [(call.owner_qualified_name, call.target_spelling) for call in calls],
                [
                    ("pkg.module.outer", "run"),
                    ("pkg.module.outer", "run"),
                    ("pkg.module.outer.inner", "finish"),
                ],
            )
            raw = source.read_bytes()
            self.assertEqual(
                [raw[call.start_byte : call.end_byte].decode("utf-8") for call in calls],
                ["run", "service.run", "finish"],
            )
            (root / "bad.py").write_text("def broken(:\n", encoding="utf-8")
            inventory = independent_source_inventory(root, "python")
            self.assertEqual((inventory.scanned_files, inventory.parsed_files), (2, 1))
            self.assertEqual(inventory.rejected_files, ("bad.py",))
            self.assertEqual(independent_source_constructs(root, "go"), ())

    def test_compass_superset_passes_shared_fact_comparison(self) -> None:
        database = self.database()
        compass = index_graph("compass", FIXTURES / "compass_graph.json", database)
        graphify = index_graph("graphify", FIXTURES / "graphify_graph.json", database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)
        self.assertGreater(compass.nodes, graphify.nodes)
        self.assertEqual(compass.digest, canonical_graph_digest(database, "compass"))
        self.assertEqual(result.digest, compare_graphs(database).digest)

    def test_storage_order_does_not_change_digest(self) -> None:
        database = self.database()
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
        database = self.database()
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
        database = self.database()
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
        database = self.database()
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

    def test_multiline_python_imports_match_only_with_same_statement_evidence(self) -> None:
        nodes = """
          {"id":"source","label":"module.py","kind":"file",
           "source_file":"pkg/module.py","source_location":"L1","language":"python"},
          {"id":"target","label":"Target","kind":"type_alias","language":"python"}
        """
        compass = (
            '{"graph":{"diagnostics":[]},"nodes":['
            + nodes
            + '],"links":[{"source":"source","target":"target","relation":"imports",'
            '"source_file":"pkg/module.py","source_location":"L2"}]}'
        )
        graphify = (
            '{"nodes":['
            + nodes
            + '],"links":[{"source":"source","target":"target","relation":"imports",'
            '"source_file":"pkg/module.py","source_location":"L1"}]}'
        )
        source = {"pkg/module.py": "from package import (\n    Target,\n)\n"}

        strict = compare_documents(compass, graphify)
        self.assertEqual(strict.metrics["missing_graphify_edges"], 1)

        proven = compare_documents(compass, graphify, source)
        self.assertTrue(proven.passed, proven.failures)
        self.assertEqual(proven.metrics["exact_graphify_edges"], 0)
        self.assertEqual(proven.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            '"dominated:source_statement_occurrence":1',
            proven.metrics["graphify_edges_coverage_reasons"],
        )

    def test_occurrence_oracle_rejects_different_statements_and_invalid_sources(self) -> None:
        def result_for(source_file: str, compass_line: int, source: str):
            nodes = (
                f'{{"id":"source","label":"module","kind":"file",'
                f'"source_file":"{source_file}","source_location":"L1",'
                '"language":"python"},'
                '{"id":"target","label":"Target","kind":"type_alias",'
                '"language":"python"}'
            )
            compass = (
                '{"graph":{"diagnostics":[]},"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target",'
                f'"relation":"imports","source_file":"{source_file}",'
                f'"source_location":"L{compass_line}"}}]}}'
            )
            graphify = (
                '{"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target",'
                f'"relation":"imports","source_file":"{source_file}",'
                '"source_location":"L1"}]}'
            )
            return compare_documents(compass, graphify, {source_file: source})

        different = result_for(
            "pkg/module.py",
            2,
            "from package import Target\nfrom package import Target\n",
        )
        self.assertEqual(different.metrics["missing_graphify_edges"], 1)

        malformed = result_for(
            "pkg/module.py",
            2,
            "from package import (\n    Target,\n",
        )
        self.assertEqual(malformed.metrics["missing_graphify_edges"], 1)

        unsupported = result_for(
            "pkg/module.java",
            2,
            "import package.\n    Target;\n",
        )
        self.assertEqual(unsupported.metrics["missing_graphify_edges"], 1)

    def test_occurrence_oracle_does_not_merge_nested_calls(self) -> None:
        nodes = """
          {"id":"source","label":"run","kind":"function",
           "source_file":"pkg/module.py","source_location":"L1","language":"python"},
          {"id":"target","label":"target","kind":"function",
           "source_file":"pkg/target.py","source_location":"L1","language":"python"}
        """
        compass = (
            '{"graph":{"diagnostics":[]},"nodes":['
            + nodes
            + '],"links":[{"source":"source","target":"target","relation":"calls",'
            '"source_file":"pkg/module.py","source_location":"L2"}]}'
        )
        graphify = (
            '{"nodes":['
            + nodes
            + '],"links":[{"source":"source","target":"target","relation":"calls",'
            '"source_file":"pkg/module.py","source_location":"L1"}]}'
        )
        result = compare_documents(
            compass,
            graphify,
            {"pkg/module.py": "target(\n    target()\n)\n"},
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 1)

    def test_rationale_facts_match_by_source_anchor_across_schema_names(self) -> None:
        database = self.database()
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
        database = self.database()
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
        database = self.database()
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

    def test_case_exact_generated_owner_disambiguates_case_distinct_types(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"exported","label":"EphemeralStore","kind":"interface",
               "source_file":"pkg/checkpoint/api.go","source_location":"L10",
               "language":"go"},
              {"id":"private","label":"ephemeralStore","kind":"struct",
               "source_file":"pkg/checkpoint/store.go","source_location":"L20",
               "language":"go"}
            ],"links":[]}
            """,
            """
            {"nodes":[
              {"id":"pkg_checkpoint_generated_ephemeralstore",
               "label":"ephemeralStore",
               "source_file":"pkg/checkpoint/write.go","source_location":"L30"}
            ],"links":[]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_nodes"], 1)
        self.assertIn(
            "dominated:case_exact_owner",
            result.metrics["graphify_nodes_coverage_reasons"],
        )

    def test_embedding_relation_requires_first_class_compass_embedding(self) -> None:
        graphify = """
            {"nodes":[
              {"id":"owner","label":"Owner","source_file":"pkg/a.go","source_location":"L1"},
              {"id":"target","label":"Target","source_file":"pkg/a.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"embeds",
               "source_file":"pkg/a.go","source_location":"L3"}
            ]}
        """
        collapsed = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Owner","kind":"struct",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"target","label":"Target","kind":"interface",
               "source_file":"pkg/a.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"contains",
               "source_file":"pkg/a.go","source_location":"L3"}
            ]}
            """,
            graphify,
        )
        self.assertFalse(collapsed.passed)
        self.assertEqual(collapsed.metrics["missing_graphify_edges"], 1)

        preserved = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Owner","kind":"struct",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"target","label":"Target","kind":"interface",
               "source_file":"pkg/a.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"embeds",
               "source_file":"pkg/a.go","source_location":"L3"}
            ]}
            """,
            graphify,
        )
        self.assertTrue(preserved.passed, preserved.failures)

    def test_generated_receiver_id_disambiguates_an_exact_module(self) -> None:
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
            {"nodes":[
              {"id":"pkg_left_generated_go_agent","label":"Agent"}
            ],"links":[]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_nodes"], 1)
        self.assertIn(
            "dominated:qualified_generated_owner",
            result.metrics["graphify_nodes_coverage_reasons"],
        )

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

    def test_module_import_projection_is_rejected_but_real_use_is_required(self) -> None:
        graphify = """
            {"nodes":[
              {"id":"module","label":"app.py",
               "source_file":"app.py","source_location":"L1"},
              {"id":"owner","label":"Owner",
               "source_file":"app.py","source_location":"L20"},
              {"id":"symbol","label":"Widget",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"symbol","relation":"uses",
               "source_file":"app.py","source_location":"L3"},
              {"source":"owner","target":"symbol","relation":"uses",
               "source_file":"app.py","source_location":"L21"}
            ]}
        """
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"module","label":"app.py","kind":"file",
               "source_file":"app.py","source_location":"L1"},
              {"id":"owner","label":"Owner","kind":"class",
               "source_file":"app.py","source_location":"L20"},
              {"id":"symbol","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"module","target":"symbol","relation":"imports",
               "source_file":"app.py","source_location":"L3"}
            ]}
            """,
            graphify,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 1)
        self.assertIn(
            "rejected:module_import_projected_to_symbol",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_occurrence_with_more_precise_owner_dominates_baseline(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"outer","label":"run","kind":"function",
               "source_file":"app.py","source_location":"L10"},
              {"id":"inner","label":"run_inner","kind":"function",
               "source_file":"app.py","source_location":"L20"},
              {"id":"target","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"inner","target":"target","relation":"calls",
               "source_file":"app.py","source_location":"L21"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"outer","label":"run()",
               "source_file":"app.py","source_location":"L10"},
              {"id":"target","label":"Widget",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"outer","target":"target","relation":"calls",
               "source_file":"app.py","source_location":"L21"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:precise_occurrence_owner",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_unique_source_occurrence_recovers_only_a_compatible_target(self) -> None:
        compass = """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"caller","label":"run","kind":"function",
               "source_file":"app.py","source_location":"L10","language":"python"},
              {"id":"target","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L2","language":"python"}
            ],"links":[
              {"source":"caller","target":"target","relation":"calls",
               "source_file":"app.py","source_location":"L11"}
            ]}
        """
        compatible = compare_documents(
            compass,
            """
            {"nodes":[
              {"id":"caller","label":"run()",
               "source_file":"app.py","source_location":"L10"},
              {"id":"generated_target","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L20"}
            ],"links":[
              {"source":"caller","target":"generated_target","relation":"calls",
               "source_file":"app.py","source_location":"L11"}
            ]}
            """,
        )
        self.assertFalse(compatible.passed)
        self.assertEqual(compatible.metrics["missing_graphify_nodes"], 1)
        self.assertEqual(compatible.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:precise_occurrence_target",
            compatible.metrics["graphify_edges_coverage_reasons"],
        )

        incompatible = compare_documents(
            compass,
            """
            {"nodes":[
              {"id":"caller","label":"run()",
               "source_file":"app.py","source_location":"L10"},
              {"id":"wrong_target","label":"Different","kind":"class",
               "source_file":"lib.py","source_location":"L20"}
            ],"links":[
              {"source":"caller","target":"wrong_target","relation":"calls",
               "source_file":"app.py","source_location":"L11"}
            ]}
            """,
        )
        self.assertEqual(incompatible.metrics["missing_graphify_edges"], 1)
        self.assertNotIn(
            "dominated:precise_occurrence_target",
            incompatible.metrics["graphify_edges_coverage_reasons"],
        )

    def test_qualified_external_target_rejects_same_named_local_rebinding(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Run","kind":"function",
               "source_file":"app.go","source_location":"L10","language":"go"},
              {"id":"external","label":"Context","kind":"type_alias",
               "source_file":"","source_location":"","language":"go",
               "qualified_name":"context.context"},
              {"id":"local","label":"Context","kind":"struct",
               "source_file":"internal/contexts.go","source_location":"L2",
               "language":"go"}
            ],"links":[
              {"source":"owner","target":"external","relation":"references",
               "source_file":"app.go","source_location":"L11"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"Run()",
               "source_file":"app.go","source_location":"L10"},
              {"id":"local","label":"Context",
               "source_file":"internal/contexts.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"local","relation":"uses",
               "source_file":"app.go","source_location":"L11"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:qualified_external_target_rebound_to_local",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_import_binding_dominates_a_sourceless_external_placeholder(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Run","kind":"function",
               "source_file":"app.go","source_location":"L10","language":"go"},
              {"id":"external","label":"RawMessage","kind":"import",
               "source_file":"app.go","source_location":"L2","language":"go",
               "qualified_name":"encoding/json.rawmessage"}
            ],"links":[
              {"source":"owner","target":"external","relation":"references",
               "source_file":"app.go","source_location":"L11"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"Run()",
               "source_file":"app.go","source_location":"L10"},
              {"id":"external","label":"RawMessage"}
            ],"links":[
              {"source":"owner","target":"external","relation":"uses",
               "source_file":"app.go","source_location":"L11"}
            ]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:qualified_external_binding",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_imported_symbol_dominates_a_module_level_import(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"module","label":"app.py","kind":"file",
               "source_file":"app.py","source_location":"L1"},
              {"id":"symbol","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L20"}
            ],"links":[
              {"source":"module","target":"symbol","relation":"imports",
               "source_file":"app.py","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"module","label":"app.py",
               "source_file":"app.py","source_location":"L1"},
              {"id":"library","label":"lib.py",
               "source_file":"lib.py","source_location":"L1"}
            ],"links":[
              {"source":"module","target":"library","relation":"imports",
               "source_file":"app.py","source_location":"L2"}
            ]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:imported_symbol_definition",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_external_import_rejects_terminal_name_local_rebinding(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"module","label":"app.py","kind":"file",
               "source_file":"app.py","source_location":"L1"},
              {"id":"external","label":"inspect","kind":"import",
               "source_file":"app.py","source_location":"L2",
               "qualified_name":"inspect"}
            ],"links":[
              {"source":"module","target":"external","relation":"imports",
               "source_file":"app.py","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"module","label":"app.py",
               "source_file":"app.py","source_location":"L1"},
              {"id":"wrong","label":"inspect.py",
               "source_file":"project/inspect.py","source_location":"L1"}
            ],"links":[
              {"source":"module","target":"wrong","relation":"imports",
               "source_file":"app.py","source_location":"L2"}
            ]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:qualified_external_import_rebound_to_local",
            result.metrics["graphify_edges_coverage_reasons"],
        )


if __name__ == "__main__":
    unittest.main()
