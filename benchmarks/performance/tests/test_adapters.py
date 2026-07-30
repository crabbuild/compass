from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from benchmarks.performance.compass_perf.adapters import CompassAdapter, GraphifyAdapter
from benchmarks.performance.compass_perf.model import ToolRevision
from benchmarks.performance.compass_perf.workspace import QualificationWorkspace


def revision(name: str) -> ToolRevision:
    return ToolRevision(name, "https://example.invalid/tool.git", "a" * 40, "b" * 40, False, "c" * 64)


class AdapterTests(unittest.TestCase):
    def test_compass_build_and_query_contracts(self) -> None:
        adapter = CompassAdapter(Path("/opt/compass"), revision("compass"))
        build = adapter.build_command(Path("/repo"), Path("/output"))
        self.assertEqual(
            build,
            (
                "/opt/compass",
                "extract",
                "/repo",
                "--code-only",
                "--timing",
                "--out",
                "/output",
            ),
        )
        self.assertNotIn("--no-cluster", build)
        self.assertEqual(
            adapter.query_command(Path("/graph.json"), "authentication"),
            ("/opt/compass", "query", "authentication", "--graph", "/graph.json"),
        )

    def test_graphify_is_explicit_and_isolated(self) -> None:
        adapter = GraphifyAdapter(Path("/venv/bin/python"), revision("graphify"))
        build = adapter.build_command(Path("/repo"), Path("/output"), force=True)
        self.assertEqual(build[:4], ("/venv/bin/python", "-m", "graphify", "extract"))
        self.assertIn("--code-only", build)
        self.assertEqual(build[-1], "--force")

    def test_compass_active_generation_is_validated(self) -> None:
        adapter = CompassAdapter(Path("/opt/compass"), revision("compass"))
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            compass_out = output / "compass-out"
            active = compass_out / ".compass-generations" / "generation-123"
            active.mkdir(parents=True)
            (compass_out / ".compass-active-generation").write_text("generation-123\n")
            graph = active / "graph.json"
            graph.write_text("{}")
            self.assertEqual(adapter.graph_path(output), graph)
            (active / ".compass-build-incomplete").touch()
            with self.assertRaisesRegex(RuntimeError, "incomplete"):
                adapter.graph_path(output)

    def test_generation_pointer_cannot_escape(self) -> None:
        adapter = CompassAdapter(Path("/opt/compass"), revision("compass"))
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            compass_out = output / "compass-out"
            compass_out.mkdir()
            (compass_out / ".compass-active-generation").write_text("../outside")
            with self.assertRaisesRegex(RuntimeError, "invalid"):
                adapter.graph_path(output)

    def test_timing_evidence_is_parsed(self) -> None:
        adapter = CompassAdapter(Path("/opt/compass"), revision("compass"))
        evidence = adapter.parse_build_evidence(
            "[compass timing] detect: 1.2s\n"
            "[compass timing] deterministic extract: 2.5s\n"
            "[compass timing] total: 4.0s\n"
        )
        self.assertEqual(
            evidence,
            {"detect": 1.2, "deterministic_extract": 2.5, "total": 4.0},
        )

    def test_compass_prunes_only_inactive_generations(self) -> None:
        adapter = CompassAdapter(Path("/opt/compass"), revision("compass"))
        with tempfile.TemporaryDirectory() as directory:
            workspace = QualificationWorkspace.create(Path(directory) / "workspace")
            output = workspace.root / "artifacts" / "fixture"
            generations = output / "compass-out" / ".compass-generations"
            active = generations / "generation-active"
            inactive = generations / "generation-inactive"
            active.mkdir(parents=True)
            inactive.mkdir()
            graph = active / "graph.json"
            graph.write_text("{}")

            adapter.prune_superseded_artifacts(output, graph)

            self.assertTrue(active.is_dir())
            self.assertFalse(inactive.exists())


if __name__ == "__main__":
    unittest.main()
