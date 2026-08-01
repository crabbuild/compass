from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from benchmarks.performance.tools.compare_language_fixtures import (
    FIXTURE_SCHEMA,
    build_report,
    render_markdown,
    write_report,
)


class LanguageFixtureCompareTests(unittest.TestCase):
    def fixture(
        self,
        root: Path,
        name: str,
        compass: dict[str, object],
        graphify: dict[str, object],
        *,
        language: str = "rust",
    ) -> Path:
        directory = root / name
        directory.mkdir()
        (directory / "compass.json").write_text(json.dumps(compass), encoding="utf-8")
        (directory / "graphify.json").write_text(json.dumps(graphify), encoding="utf-8")
        manifest = directory / "fixture.json"
        manifest.write_text(
            json.dumps(
                {
                    "schema": FIXTURE_SCHEMA,
                    "language": language,
                    "fixture": name,
                    "compass_graph": "compass.json",
                    "graphify_graph": "graphify.json",
                }
            ),
            encoding="utf-8",
        )
        return manifest

    def test_normalizes_relations_and_preserves_exact_occurrences(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            nodes = [
                {
                    "id": "owner",
                    "label": "run",
                    "kind": "function",
                    "source_file": "src/lib.rs",
                    "source_location": "L1",
                },
                {
                    "id": "target",
                    "label": "Target::new",
                    "kind": "method",
                    "source_file": "src/lib.rs",
                    "source_location": "L2",
                },
            ]
            manifest = self.fixture(
                root,
                "exact-call",
                {
                    "graph": {"diagnostics": []},
                    "nodes": nodes,
                    "edges": [
                        {
                            "source": "owner",
                            "target": "target",
                            "kind": "instantiates",
                            "source_file": "src/lib.rs",
                            "source_location": "L4",
                        }
                    ],
                },
                {
                    "nodes": nodes,
                    "links": [
                        {
                            "source": "owner",
                            "target": "target",
                            "relation": "calls",
                            "source_file": "src/lib.rs",
                            "source_location": "L4",
                        }
                    ],
                },
            )
            report = build_report([manifest])
        self.assertEqual(
            report["coverage"],
            [
                {
                    "language": "rust",
                    "fixture": "exact-call",
                    "relation": "calls",
                    "graphify_total": 1,
                    "exact": 1,
                    "dominated": 0,
                    "rejected": 0,
                    "ambiguous": 0,
                    "missing": 0,
                    "handled": 1,
                }
            ],
        )

    def test_typed_relations_dominate_only_same_site_generic_references(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            nodes = [
                {
                    "id": "owner",
                    "label": "build",
                    "kind": "function",
                    "source_file": "src/lib.rs",
                    "source_location": "L1",
                },
                {
                    "id": "target",
                    "label": "Result",
                    "kind": "struct",
                    "source_file": "src/result.rs",
                    "source_location": "L2",
                },
            ]
            manifest = self.fixture(
                root,
                "typed-reference",
                {
                    "graph": {"diagnostics": []},
                    "nodes": nodes,
                    "edges": [
                        {
                            "source": "owner",
                            "target": "target",
                            "kind": "returns",
                            "source_file": "src/lib.rs",
                            "source_location": "L1",
                        }
                    ],
                },
                {
                    "nodes": nodes,
                    "links": [
                        {
                            "source": "owner",
                            "target": "target",
                            "relation": "uses",
                            "source_file": "src/lib.rs",
                            "source_location": "L1",
                        },
                        {
                            "source": "owner",
                            "target": "target",
                            "relation": "uses",
                            "source_file": "src/lib.rs",
                            "source_location": "L8",
                        },
                    ],
                },
            )
            report = build_report([manifest])
        self.assertEqual(report["coverage"][0]["dominated"], 1)
        self.assertEqual(report["coverage"][0]["missing"], 1)

    def test_precise_multiline_base_sites_dominate_only_the_bounded_heritage_clause(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            nodes = [
                {
                    "id": "derived",
                    "label": "Derived",
                    "kind": "interface",
                    "source_file": "src/types.ts",
                    "source_location": "L1",
                },
                {
                    "id": "base",
                    "label": "Base",
                    "kind": "interface",
                    "source_file": "src/base.ts",
                    "source_location": "L1",
                },
            ]
            manifest = self.fixture(
                root,
                "multiline-heritage",
                {
                    "graph": {"diagnostics": []},
                    "nodes": nodes,
                    "edges": [
                        {
                            "source": "derived",
                            "target": "base",
                            "kind": "extends",
                            "source_file": "src/types.ts",
                            "source_location": "L3",
                        }
                    ],
                },
                {
                    "nodes": nodes,
                    "links": [
                        {
                            "source": "derived",
                            "target": "base",
                            "relation": "extends",
                            "source_file": "src/types.ts",
                            "source_location": "L2",
                        },
                        {
                            "source": "derived",
                            "target": "base",
                            "relation": "extends",
                            "source_file": "src/types.ts",
                            "source_location": "L20",
                        },
                    ],
                },
                language="typescript",
            )
            row = build_report([manifest])["coverage"][0]
        self.assertEqual(row["dominated"], 1)
        self.assertEqual(row["missing"], 1)

    def test_reports_rejected_qualified_external_rebinding(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.fixture(
                root,
                "external",
                {
                    "graph": {"diagnostics": []},
                    "nodes": [
                        {
                            "id": "owner",
                            "label": "run",
                            "kind": "function",
                            "source_file": "src/lib.rs",
                            "source_location": "L1",
                            "language": "rust",
                        },
                        {
                            "id": "external",
                            "label": "World",
                            "kind": "type_alias",
                            "qualified_name": "bevy::prelude::World",
                            "language": "rust",
                        },
                        {
                            "id": "local",
                            "label": "World",
                            "kind": "struct",
                            "source_file": "src/world.rs",
                            "source_location": "L2",
                            "language": "rust",
                        },
                    ],
                    "edges": [
                        {
                            "source": "owner",
                            "target": "external",
                            "kind": "references",
                            "source_file": "src/lib.rs",
                            "source_location": "L4",
                        }
                    ],
                },
                {
                    "nodes": [
                        {
                            "id": "owner",
                            "label": "run()",
                            "source_file": "src/lib.rs",
                            "source_location": "L1",
                        },
                        {
                            "id": "local",
                            "label": "World",
                            "source_file": "src/world.rs",
                            "source_location": "L2",
                        },
                    ],
                    "links": [
                        {
                            "source": "owner",
                            "target": "local",
                            "relation": "uses",
                            "source_file": "src/lib.rs",
                            "source_location": "L4",
                        }
                    ],
                },
            )
            row = build_report([manifest])["coverage"][0]
        self.assertEqual(row["rejected"], 1)
        self.assertEqual(row["missing"], 0)

    def test_reports_and_excludes_graphify_dangling_edges(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            nodes = [{"id": "owner", "label": "owner", "kind": "function"}]
            manifest = self.fixture(
                root,
                "dangling-graphify",
                {"graph": {"diagnostics": []}, "nodes": nodes, "edges": []},
                {
                    "nodes": nodes,
                    "links": [
                        {
                            "source": "owner",
                            "target": "missing",
                            "relation": "calls",
                        }
                    ],
                },
            )
            report = build_report([manifest])

        self.assertEqual(report["fixtures"][0]["graphify"]["dangling_edges"], 1)
        self.assertEqual(report["fixtures"][0]["graphify"]["edges"], 0)
        self.assertEqual(report["fixtures"][0]["node_coverage"]["exact"], 1)

    def test_reports_missing_endpoints(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.fixture(
                root,
                "missing",
                {
                    "graph": {"diagnostics": []},
                    "nodes": [
                        {
                            "id": "owner",
                            "label": "run",
                            "kind": "function",
                            "source_file": "src/lib.rs",
                            "source_location": "L1",
                        }
                    ],
                    "edges": [],
                },
                {
                    "nodes": [
                        {
                            "id": "owner",
                            "label": "run()",
                            "source_file": "src/lib.rs",
                            "source_location": "L1",
                        },
                        {
                            "id": "target",
                            "label": "missing()",
                            "source_file": "src/lib.rs",
                            "source_location": "L2",
                        },
                    ],
                    "links": [
                        {
                            "source": "owner",
                            "target": "target",
                            "relation": "calls",
                            "source_file": "src/lib.rs",
                            "source_location": "L3",
                        }
                    ],
                },
            )
            row = build_report([manifest])["coverage"][0]
        self.assertEqual(row["missing"], 1)
        self.assertEqual(row["handled"], 0)

    def test_matches_callable_owner_spelling_at_the_exact_source_site(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.fixture(
                root,
                "java-callable-owner",
                {
                    "graph": {"diagnostics": []},
                    "nodes": [
                        {
                            "id": "candidate-owner",
                            "label": "Owner",
                            "kind": "class",
                            "source_file": "src/Owner.java",
                            "source_location": "L1",
                            "language": "java",
                        },
                        {
                            "id": "candidate-method",
                            "label": ".run()",
                            "kind": "method",
                            "source_file": "src/Owner.java",
                            "source_location": "L3",
                            "language": "java",
                        },
                        {
                            "id": "candidate-get-route",
                            "label": "GET /run",
                            "kind": "route",
                            "source_file": "src/Owner.java",
                            "source_location": "L2",
                            "language": "java",
                        },
                        {
                            "id": "candidate-post-route",
                            "label": "POST /run",
                            "kind": "route",
                            "source_file": "src/Owner.java",
                            "source_location": "L2",
                            "language": "java",
                        },
                    ],
                    "edges": [
                        {
                            "source": "candidate-owner",
                            "target": "candidate-method",
                            "kind": "contains",
                            "source_file": "src/Owner.java",
                            "source_location": "L3",
                        }
                    ],
                },
                {
                    "nodes": [
                        {
                            "id": "baseline-owner",
                            "label": "Owner",
                            "kind": "class",
                            "source_file": "src/Owner.java",
                            "source_location": "L1",
                            "language": "java",
                        },
                        {
                            "id": "baseline-method",
                            "label": "Owner::run()",
                            "kind": "method",
                            "source_file": "src/Owner.java",
                            "source_location": "L2",
                            "language": "java",
                        },
                    ],
                    "links": [
                        {
                            "source": "baseline-owner",
                            "target": "baseline-method",
                            "relation": "contains",
                            "source_file": "src/Owner.java",
                            "source_location": "L2",
                        }
                    ],
                },
                language="java",
            )
            row = build_report([manifest])["coverage"][0]
        self.assertEqual(row["handled"], 1)
        self.assertEqual(row["missing"], 0)

    def test_matches_the_unique_nearest_java_overload_declaration(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.fixture(
                root,
                "java-overload-anchor",
                {
                    "graph": {"diagnostics": []},
                    "nodes": [
                        {
                            "id": "candidate-owner",
                            "label": "Owner",
                            "kind": "class",
                            "source_file": "src/Owner.java",
                            "source_location": "L1",
                            "language": "java",
                        },
                        {
                            "id": "candidate-first",
                            "label": ".run()",
                            "kind": "method",
                            "source_file": "src/Owner.java",
                            "source_location": "L4",
                            "language": "java",
                        },
                        {
                            "id": "candidate-second",
                            "label": ".run()",
                            "kind": "method",
                            "source_file": "src/Owner.java",
                            "source_location": "L9",
                            "language": "java",
                        },
                    ],
                    "edges": [
                        {
                            "source": "candidate-owner",
                            "target": "candidate-first",
                            "kind": "contains",
                            "source_file": "src/Owner.java",
                            "source_location": "L4",
                        }
                    ],
                },
                {
                    "nodes": [
                        {
                            "id": "baseline-owner",
                            "label": "Owner",
                            "kind": "class",
                            "source_file": "src/Owner.java",
                            "source_location": "L1",
                            "language": "java",
                        },
                        {
                            "id": "baseline-first",
                            "label": "Owner::run()",
                            "kind": "method",
                            "source_file": "src/Owner.java",
                            "source_location": "L3",
                            "language": "java",
                        },
                    ],
                    "links": [
                        {
                            "source": "baseline-owner",
                            "target": "baseline-first",
                            "relation": "contains",
                            "source_file": "src/Owner.java",
                            "source_location": "L3",
                        }
                    ],
                },
                language="java",
            )
            row = build_report([manifest])["coverage"][0]
        self.assertEqual(row["handled"], 1)
        self.assertEqual(row["ambiguous"], 0)

    def test_matches_java_class_and_constructor_across_graphify_spelling(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.fixture(
                root,
                "java-constructor",
                {
                    "graph": {"diagnostics": []},
                    "nodes": [
                        {
                            "id": "candidate-owner",
                            "label": "Owner",
                            "kind": "class",
                            "source_file": "src/Owner.java",
                            "source_location": "L4",
                            "qualified_name": "example.Owner",
                            "language": "java",
                        },
                        {
                            "id": "candidate-constructor",
                            "label": "<init>",
                            "kind": "constructor",
                            "source_file": "src/Owner.java",
                            "source_location": "L8",
                            "qualified_name": "example.Owner::<init>",
                            "language": "java",
                        },
                    ],
                    "edges": [
                        {
                            "source": "candidate-owner",
                            "target": "candidate-constructor",
                            "kind": "contains",
                            "source_file": "src/Owner.java",
                            "source_location": "L8",
                        }
                    ],
                },
                {
                    "nodes": [
                        {
                            "id": "baseline-owner",
                            "label": "Owner",
                            "source_file": "src/Owner.java",
                            "source_location": "L3",
                        },
                        {
                            "id": "baseline-constructor",
                            "label": ".Owner()",
                            "source_file": "src/Owner.java",
                            "source_location": "L8",
                        },
                    ],
                    "links": [
                        {
                            "source": "baseline-owner",
                            "target": "baseline-constructor",
                            "relation": "contains",
                            "source_file": "src/Owner.java",
                            "source_location": "L8",
                        }
                    ],
                },
                language="java",
            )
            row = build_report([manifest])["coverage"][0]
        self.assertEqual(row["handled"], 1)
        self.assertEqual(row["missing"], 0)

    def test_output_is_deterministic_and_sorted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            graph = {"graph": {"diagnostics": []}, "nodes": [], "edges": []}
            zeta = self.fixture(root, "zeta", graph, graph)
            alpha = self.fixture(root, "alpha", graph, graph)
            first_json = root / "first.json"
            first_markdown = root / "first.md"
            second_json = root / "second.json"
            second_markdown = root / "second.md"
            first = write_report([zeta, alpha], first_json, first_markdown)
            second = write_report([alpha, zeta], second_json, second_markdown)
            self.assertEqual(first, second)
            self.assertEqual(first_json.read_bytes(), second_json.read_bytes())
            self.assertEqual(first_markdown.read_bytes(), second_markdown.read_bytes())
            self.assertEqual(
                [item["fixture"] for item in first["fixtures"]], ["alpha", "zeta"]
            )
            self.assertIn("| Language | Fixture | Relation |", render_markdown(first))

    def test_three_run_manifest_enforces_byte_and_occurrence_determinism(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            graph = {"graph": {"diagnostics": []}, "nodes": [], "edges": []}
            manifest = self.fixture(root, "repeated", graph, graph, language="java")
            fixture_root = manifest.parent
            for name in ("candidate-2.json", "candidate-3.json"):
                (fixture_root / name).write_text(json.dumps(graph), encoding="utf-8")
            document = json.loads(manifest.read_text(encoding="utf-8"))
            document.pop("compass_graph")
            document["compass_graphs"] = [
                "compass.json",
                "candidate-2.json",
                "candidate-3.json",
            ]
            manifest.write_text(json.dumps(document), encoding="utf-8")
            report = build_report([manifest])
            self.assertEqual(report["fixtures"][0]["compass"]["runs"], 3)

            (fixture_root / "candidate-3.json").write_text(
                json.dumps(graph, indent=2), encoding="utf-8"
            )
            with self.assertRaisesRegex(ValueError, "run 3 graph bytes differ"):
                build_report([manifest])

    def test_invalid_manifest_and_graph_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            invalid = root / "invalid.json"
            invalid.write_text("[]", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "must be a JSON object"):
                build_report([invalid])

            dangling = self.fixture(
                root,
                "dangling",
                {
                    "graph": {"diagnostics": []},
                    "nodes": [{"id": "owner"}],
                    "edges": [
                        {"source": "owner", "target": "absent", "kind": "calls"}
                    ],
                },
                {"nodes": [], "links": []},
            )
            with self.assertRaisesRegex(ValueError, "dangling"):
                build_report([dangling])


if __name__ == "__main__":
    unittest.main()
