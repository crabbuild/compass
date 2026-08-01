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
