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
        if source_documents is not None:
            for relative_path, source in source_documents.items():
                source_path = root / relative_path
                source_path.parent.mkdir(parents=True, exist_ok=True)
                source_path.write_text(source, encoding="utf-8")
        database = sqlite3.connect(":memory:")
        try:
            index_graph("compass", compass, database)
            index_graph("graphify", graphify, database)
            return compare_graphs(database, root if source_documents is not None else None)
        finally:
            database.close()


class CorrectnessTests(unittest.TestCase):
    def database(self) -> sqlite3.Connection:
        database = sqlite3.connect(":memory:")
        self.addCleanup(database.close)
        return database

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

    def test_graphify_case_of_matches_canonical_compass_containment(self) -> None:
        nodes = (
            '{"id":"enum","label":"Policy","kind":"enum",'
            '"source_file":"Policy.java","source_location":"L1"},'
            '{"id":"member","label":"ALLOW","kind":"enum_member",'
            '"source_file":"Policy.java","source_location":"L2"}'
        )
        result = compare_documents(
            '{"graph":{"diagnostics":[]},"nodes":['
            + nodes
            + '],"links":[{"source":"enum","target":"member",'
            '"relation":"contains","source_file":"Policy.java",'
            '"source_location":"L2"}]}',
            '{"nodes":['
            + nodes
            + '],"links":[{"source":"enum","target":"member",'
            '"relation":"case_of","source_file":"Policy.java",'
            '"source_location":"L2"}]}',
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["exact_graphify_edges"], 1)

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

    def test_python_source_inventory_includes_definition_time_calls(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "sample.py"
            source.write_text(
                """
@class_decorator(class_option())
class Widget(base_factory(), metaclass=meta_factory()):
    @method_decorator(method_option())
    def run(
        self,
        value: annotation_factory() = default_factory(),
    ) -> return_factory():
        body_call()
""".lstrip(),
                encoding="utf-8",
            )

            constructs = independent_source_constructs(root, "python")

        calls = {
            (construct.owner_qualified_name, construct.target_spelling)
            for construct in constructs
            if construct.relation == "calls"
        }
        self.assertEqual(
            calls,
            {
                ("sample", "base_factory"),
                ("sample", "class_decorator"),
                ("sample", "class_option"),
                ("sample", "meta_factory"),
                ("sample.Widget", "annotation_factory"),
                ("sample.Widget", "default_factory"),
                ("sample.Widget", "method_decorator"),
                ("sample.Widget", "method_option"),
                ("sample.Widget", "return_factory"),
                ("sample.Widget.run", "body_call"),
            },
        )

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

    def test_unqualified_placeholder_cannot_choose_a_cross_package_definition(self) -> None:
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
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
        self.assertEqual(result.metrics["ambiguous_graphify_nodes"], 0)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 0)
        self.assertIn(
            "rejected:unverifiable_placeholder",
            result.metrics["graphify_nodes_coverage_reasons"],
        )

    def test_unqualified_placeholder_cannot_bind_to_a_value_or_module(self) -> None:
        for kind in ("field", "module"):
            with self.subTest(kind=kind):
                result = compare_documents(
                    f"""
                    {{"graph":{{"diagnostics":[]}},"nodes":[
                      {{"id":"result","label":"result","kind":"{kind}",
                       "source_file":"src/lib.rs","source_location":"L8",
                       "language":"rust"}}
                    ],"links":[]}}
                    """,
                    """
                    {"nodes":[
                      {"id":"src_build_rs_result","label":"Result"}
                    ],"links":[]}
                    """,
                )
                self.assertTrue(result.passed, result.failures)
                self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
                self.assertEqual(result.metrics["dominated_graphify_nodes"], 0)
                self.assertIn(
                    "rejected:unverifiable_placeholder",
                    result.metrics["graphify_nodes_coverage_reasons"],
                )

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

    def test_rust_generic_parameter_remains_missing_instead_of_binding_an_unrelated_alias(
        self,
    ) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"field","label":"i","kind":"field",
               "source_file":"src/iter/chunks.rs","source_location":"L12",
               "language":"rust"},
              {"id":"alias","label":"I","kind":"type_alias",
               "source_file":"src/iter/test.rs","source_location":"L1918",
               "language":"rust"}
            ],"links":[]}
            """,
            """
            {"nodes":[
              {"id":"src_iter_mod_i","label":"I",
               "source_file":"src/iter/mod.rs","source_location":"L290",
               "language":"rust"}
            ],"links":[]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 1)
        self.assertEqual(result.metrics["dominated_graphify_nodes"], 0)
        self.assertIn(
            "missing:no_compatible_anchored_definition",
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

    def test_exact_containment_owner_rejects_cross_type_graphify_ownership(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"flakey.go","kind":"file",
               "source_file":"pkg/flakey.go","source_location":"L1","language":"go"},
              {"id":"interface","label":"Flakey","kind":"interface",
               "qualified_name":"pkg.Flakey",
               "source_file":"pkg/flakey.go","source_location":"L2","language":"go"},
              {"id":"implementation","label":"flakey","kind":"struct",
               "qualified_name":"pkg.flakey",
               "source_file":"pkg/flakey.go","source_location":"L8","language":"go"},
              {"id":"interface_method","label":".Close()","kind":"method",
               "qualified_name":"pkg.Flakey::Close",
               "source_file":"pkg/flakey.go","source_location":"L3","language":"go"},
              {"id":"implementation_method","label":".Close()","kind":"method",
               "qualified_name":"pkg.flakey::Close",
               "source_file":"pkg/flakey.go","source_location":"L9","language":"go"}
            ],"links":[
              {"source":"file","target":"interface","relation":"contains"},
              {"source":"file","target":"implementation","relation":"contains"},
              {"source":"file","target":"implementation_method","relation":"contains"},
              {"source":"interface","target":"interface_method","relation":"contains"},
              {"source":"implementation","target":"implementation_method","relation":"contains"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_interface","label":"Flakey","kind":"interface",
               "qualified_name":"pkg.Flakey",
               "source_file":"pkg/flakey.go","source_location":"L2","language":"go"},
              {"id":"legacy_method","label":".Close()","kind":"method",
               "qualified_name":"pkg.flakey::Close",
               "source_file":"pkg/flakey.go","source_location":"L9","language":"go"}
            ],"links":[
              {"source":"legacy_interface","target":"legacy_method","relation":"contains"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:exact_containment_owner_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_go_type_conversion_rejects_graphify_call_classification(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"convert","label":"convert()","kind":"function",
               "source_file":"pkg/convert.go","source_location":"L3","language":"go"},
              {"id":"pgid","label":"Pgid","kind":"type_alias",
               "qualified_name":"common.Pgid",
               "source_file":"common/types.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"convert","target":"pgid","relation":"references",
               "source_file":"pkg/convert.go","source_location":"L4"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_convert","label":"convert()","kind":"function",
               "source_file":"pkg/convert.go","source_location":"L3","language":"go"},
              {"id":"legacy_pgid","label":"Pgid","kind":"type_alias",
               "qualified_name":"common.Pgid",
               "source_file":"common/types.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"legacy_convert","target":"legacy_pgid","relation":"calls",
               "source_file":"pkg/convert.go","source_location":"L4"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:go_type_conversion_not_call",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_argument_reference_rejects_wrong_indirect_call_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"fp/add/index.ts","kind":"file",
               "source_file":"fp/add/index.ts","source_location":"L1","language":"typescript"},
              {"id":"correct","label":"add","kind":"function",
               "source_file":"add/index.ts","source_location":"L73","language":"typescript"},
              {"id":"wrong","label":"fn","kind":"function",
               "source_file":"convert/test.ts","source_location":"L11","language":"typescript"}
            ],"links":[
              {"source":"owner","target":"correct","relation":"references",
               "context":"argument","source_file":"fp/add/index.ts","source_location":"L6"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"fp/add/index.ts",
               "source_file":"fp/add/index.ts","source_location":"L1"},
              {"id":"wrong","label":"fn()",
               "source_file":"convert/test.ts","source_location":"L11"}
            ],"links":[
              {"source":"owner","target":"wrong","relation":"indirect_call",
               "context":"argument","source_file":"fp/add/index.ts","source_location":"L6"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:argument_reference_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_collection_reference_is_not_an_indirect_call(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"locale/index.ts","kind":"file",
               "source_file":"locale/index.ts","source_location":"L1","language":"typescript"},
              {"id":"format","label":"formatDistance","kind":"function",
               "source_file":"locale/format.ts","source_location":"L12","language":"typescript"}
            ],"links":[
              {"source":"owner","target":"format","relation":"references",
               "context":"collection","source_file":"locale/index.ts","source_location":"L17"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"locale/index.ts",
               "source_file":"locale/index.ts","source_location":"L1"},
              {"id":"format","label":"formatDistance()",
               "source_file":"locale/format.ts","source_location":"L12"}
            ],"links":[
              {"source":"owner","target":"format","relation":"indirect_call",
               "context":"collection","source_file":"locale/index.ts","source_location":"L17"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:value_reference_not_indirect_call",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_anchored_cross_language_type_reference_is_rejected(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"rust_owner","label":"build()","kind":"function",
               "source_file":"src/build.rs","source_location":"L2","language":"rust"},
              {"id":"python_result","label":"Result","kind":"class",
               "source_file":"tools/bench","source_location":"L10","language":"python"}
            ],"links":[]}
            """,
            """
            {"nodes":[
              {"id":"legacy_owner","label":"build()",
               "source_file":"src/build.rs","source_location":"L2"},
              {"id":"legacy_result","label":"Result",
               "source_file":"tools/bench","source_location":"L10"}
            ],"links":[
              {"source":"legacy_owner","target":"legacy_result","relation":"references",
               "context":"return_type","source_file":"src/build.rs","source_location":"L2"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:cross_language_target",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_rust_generic_impl_owner_is_dominated_by_exact_type_ownership(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"StandardImpl","kind":"struct",
               "source_file":"src/printer.rs","source_location":"L1","language":"rust"},
              {"id":"method","label":".write()","kind":"method",
               "source_file":"src/printer.rs","source_location":"L4","language":"rust"}
            ],"links":[
              {"source":"owner","target":"method","relation":"contains",
               "source_file":"src/printer.rs","source_location":"L4"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_impl","label":"StandardImpl<'a, M, W>",
               "source_file":"src/printer.rs","source_location":"L3"},
              {"id":"legacy_method","label":".write()",
               "source_file":"src/printer.rs","source_location":"L4"}
            ],"links":[
              {"source":"legacy_impl","target":"legacy_method","relation":"method",
               "source_file":"src/printer.rs","source_location":"L4"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:canonical_rust_generic_owner",
            result.metrics["graphify_nodes_coverage_reasons"],
        )

    def test_exact_field_type_dominates_flat_owner_reference(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Config","kind":"struct",
               "source_file":"src/config.rs","source_location":"L1","language":"rust"},
              {"id":"field","label":"matcher","kind":"field",
               "source_file":"src/config.rs","source_location":"L2","language":"rust"},
              {"id":"target","label":"Matcher","kind":"struct",
               "source_file":"src/matcher.rs","source_location":"L1","language":"rust"}
            ],"links":[
              {"source":"owner","target":"field","relation":"contains",
               "source_file":"src/config.rs","source_location":"L2"},
              {"source":"field","target":"target","relation":"type_of",
               "source_file":"src/config.rs","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_owner","label":"Config",
               "source_file":"src/config.rs","source_location":"L1"},
              {"id":"legacy_target","label":"Matcher",
               "source_file":"src/matcher.rs","source_location":"L1"}
            ],"links":[
              {"source":"legacy_owner","target":"legacy_target","relation":"references",
               "context":"field","source_file":"src/config.rs","source_location":"L2"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:precise_field_type",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_field_generic_argument_dominates_flat_owner_reference(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"JobFifo","kind":"struct",
               "source_file":"src/job.rs","source_location":"L1","language":"rust"},
              {"id":"field","label":"inner","kind":"field",
               "source_file":"src/job.rs","source_location":"L2","language":"rust"},
              {"id":"target","label":"JobRef","kind":"struct",
               "source_file":"src/job.rs","source_location":"L10","language":"rust"}
            ],"links":[
              {"source":"owner","target":"field","relation":"contains",
               "source_file":"src/job.rs","source_location":"L2"},
              {"source":"field","target":"target","relation":"type_of",
               "source_file":"src/job.rs","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_owner","label":"JobFifo",
               "source_file":"src/job.rs","source_location":"L1"},
              {"id":"legacy_target","label":"JobRef",
               "source_file":"src/job.rs","source_location":"L10"}
            ],"links":[
              {"source":"legacy_owner","target":"legacy_target","relation":"references",
               "context":"generic_arg","source_file":"src/job.rs","source_location":"L2"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:precise_field_type",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_unique_three_hop_containment_dominates_flat_graphify_ownership(self) -> None:
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
        self.assertTrue(containment.passed, containment.failures)
        self.assertEqual(containment.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:containment_path",
            containment.metrics["graphify_edges_coverage_reasons"],
        )

    def test_multiple_bounded_containment_paths_fail_closed(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"a.go","kind":"file",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"left","label":"Left","kind":"class",
               "source_file":"pkg/a.go","source_location":"L2"},
              {"id":"right","label":"Right","kind":"class",
               "source_file":"pkg/a.go","source_location":"L3"},
              {"id":"method","label":"run()","kind":"method",
               "source_file":"pkg/a.go","source_location":"L4"}
            ],"links":[
              {"source":"file","target":"left","relation":"contains"},
              {"source":"file","target":"right","relation":"contains"},
              {"source":"left","target":"method","relation":"contains"},
              {"source":"right","target":"method","relation":"contains"}
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
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["ambiguous_graphify_edges"], 1)
        self.assertIn(
            "ambiguous:multiple_containment_paths",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_different_call_sites_still_fail_closed(self) -> None:
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

    def test_precise_reference_site_dominates_a_declaration_line_projection(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Run","kind":"function",
               "source_file":"app.go","source_location":"L10","language":"go"},
              {"id":"target","label":"Widget","kind":"struct",
               "source_file":"lib.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"owner","target":"target","relation":"references",
               "source_file":"app.go","source_location":"L12"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"Run()",
               "source_file":"app.go","source_location":"L10"},
              {"id":"target","label":"Widget",
               "source_file":"lib.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"uses",
               "source_file":"app.go","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:precise_declaration_reference_occurrence",
            result.metrics["graphify_edges_coverage_reasons"],
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

    def test_exact_occurrence_rejects_same_named_receiver_misbinding(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"MarshalJSON","kind":"method",
               "source_file":"pkg/a.go","source_location":"L8","language":"go"},
              {"id":"correct","label":"A::Encode","kind":"method",
               "source_file":"pkg/a.go","source_location":"L2","language":"go",
               "qualified_name":"pkg.A::Encode"},
              {"id":"wrong","label":".Encode()","kind":"method",
               "source_file":"pkg/b.go","source_location":"L2","language":"go",
               "qualified_name":"pkg.B::Encode"}
            ],"links":[
              {"source":"owner","target":"correct","relation":"calls",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"MarshalJSON()",
               "source_file":"pkg/a.go","source_location":"L8"},
              {"id":"wrong","label":".Encode()",
               "source_file":"pkg/b.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"wrong","relation":"calls",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:exact_occurrence_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_multiple_exact_occurrences_reject_absent_same_line_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"run","kind":"function",
               "source_file":"src/main.rs","source_location":"L8","language":"rust"},
              {"id":"first","label":"First::new","kind":"method",
               "source_file":"src/first.rs","source_location":"L2","language":"rust"},
              {"id":"second","label":"Second::new","kind":"method",
               "source_file":"src/second.rs","source_location":"L2","language":"rust"},
              {"id":"wrong","label":".new()","kind":"method",
               "source_file":"src/wrong.rs","source_location":"L2","language":"rust"}
            ],"links":[
              {"source":"owner","target":"first","relation":"calls",
               "source_file":"src/main.rs","source_location":"L10"},
              {"source":"owner","target":"second","relation":"calls",
               "source_file":"src/main.rs","source_location":"L10"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"run()",
               "source_file":"src/main.rs","source_location":"L8"},
              {"id":"wrong","label":".new()",
               "source_file":"src/wrong.rs","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"wrong","relation":"calls",
               "source_file":"src/main.rs","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:exact_occurrence_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_occurrence_resolves_an_ambiguous_sourceless_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"MarshalJSON","kind":"method",
               "source_file":"pkg/a.go","source_location":"L8","language":"go"},
              {"id":"correct","label":".Encode()","kind":"method",
               "source_file":"pkg/a.go","source_location":"L2","language":"go"},
              {"id":"other","label":".Encode()","kind":"method",
               "source_file":"pkg/b.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"owner","target":"correct","relation":"calls",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"MarshalJSON()",
               "source_file":"pkg/a.go","source_location":"L8"},
              {"id":"generated_encode","label":".Encode()"}
            ],"links":[
              {"source":"owner","target":"generated_encode","relation":"calls",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 0)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:exact_occurrence_target_conflict",
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
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:qualified_external_binding",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_extends_occurrence_grounds_a_sourceless_placeholder(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"child","label":"Child","kind":"class",
               "source_file":"pkg/models.py","source_location":"L10","language":"python"},
              {"id":"base","label":"Storage","kind":"class",
               "source_file":"pkg/storage.py","source_location":"L2","language":"python"}
            ],"links":[
              {"source":"child","target":"base","relation":"extends",
               "source_file":"pkg/models.py","source_location":"L12"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"child","label":"Child",
               "source_file":"pkg/models.py","source_location":"L10"},
              {"id":"storage","label":"Storage"}
            ],"links":[
              {"source":"child","target":"storage","relation":"inherits",
               "source_file":"pkg/models.py","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:precise_inheritance_occurrence",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_inheritance_occurrence_rejects_a_wrong_anchored_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"child","label":"Child","kind":"class",
               "source_file":"pkg/models.py","source_location":"L10","language":"python"},
              {"id":"base","label":"Base","kind":"class",
               "source_file":"pkg/base.py","source_location":"L2","language":"python"},
              {"id":"wrong","label":"Wrong","kind":"class",
               "source_file":"pkg/wrong.py","source_location":"L2","language":"python"}
            ],"links":[
              {"source":"child","target":"base","relation":"extends",
               "source_file":"pkg/models.py","source_location":"L11"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"child","label":"Child",
               "source_file":"pkg/models.py","source_location":"L10"},
              {"id":"base","label":"Base",
               "source_file":"pkg/base.py","source_location":"L2"},
              {"id":"wrong","label":"Wrong",
               "source_file":"pkg/wrong.py","source_location":"L2"}
            ],"links":[
              {"source":"child","target":"wrong","relation":"inherits",
               "source_file":"pkg/models.py","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:exact_inheritance_target_conflict",
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

    def test_precise_function_import_owner_dominates_a_file_owner(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"app.py","kind":"file",
               "source_file":"app.py","source_location":"L1"},
              {"id":"function","label":"run","kind":"function",
               "source_file":"app.py","source_location":"L10"},
              {"id":"symbol","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"function","target":"symbol","relation":"imports",
               "source_file":"app.py","source_location":"L11"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"file","label":"app.py",
               "source_file":"app.py","source_location":"L1"},
              {"id":"function","label":"run()",
               "source_file":"app.py","source_location":"L10"},
              {"id":"symbol","label":"Widget",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"file","target":"symbol","relation":"imports",
               "source_file":"app.py","source_location":"L11"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:precise_occurrence_owner",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_symbol_reexport_dominates_a_package_import(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"__init__.py","kind":"file",
               "source_file":"pkg/__init__.py","source_location":"L1"},
              {"id":"symbol","label":"Widget","kind":"class",
               "source_file":"pkg/widget.py","source_location":"L2"}
            ],"links":[
              {"source":"file","target":"symbol","relation":"exports",
               "source_file":"pkg/__init__.py","source_location":"L1"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"file","label":"__init__.py",
               "source_file":"pkg/__init__.py","source_location":"L1"},
              {"id":"symbol","label":"Widget",
               "source_file":"pkg/widget.py","source_location":"L2"}
            ],"links":[
              {"source":"file","target":"symbol","relation":"imports",
               "source_file":"pkg/__init__.py","source_location":"L1"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:symbol_reexport",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_reexport_occurrence_rejects_a_wrong_local_import_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"__init__.py","kind":"file",
               "source_file":"pkg/__init__.py","source_location":"L1"},
              {"id":"external","label":"os","kind":"import",
               "qualified_name":"os"},
              {"id":"wrong","label":"os.py","kind":"file",
               "source_file":"pkg/os.py","source_location":"L1"}
            ],"links":[
              {"source":"file","target":"external","relation":"exports",
               "source_file":"pkg/__init__.py","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"file","label":"__init__.py",
               "source_file":"pkg/__init__.py","source_location":"L1"},
              {"id":"wrong","label":"os.py",
               "source_file":"pkg/os.py","source_location":"L1"}
            ],"links":[
              {"source":"file","target":"wrong","relation":"imports",
               "source_file":"pkg/__init__.py","source_location":"L2"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:reexport_target_conflict",
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
