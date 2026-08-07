from __future__ import annotations

import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from benchmarks.performance.compass import adapters as adapters_module
from benchmarks.performance.compass.adapters import (
    CompassAdapter,
    GraphifyAdapter,
    cargo_target_directory,
)
from benchmarks.performance.compass.model import ToolRevision
from benchmarks.performance.compass.workspace import QualificationWorkspace


def revision(name: str) -> ToolRevision:
    return ToolRevision(name, "https://example.invalid/tool.git", "a" * 40, "b" * 40, False, "c" * 64)


class AdapterTests(unittest.TestCase):
    def test_cargo_target_directory_matches_cargo_environment_rules(self) -> None:
        source = Path("/work/compass")
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(source / "target", cargo_target_directory(source))
        with mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": "shared-target"}, clear=True):
            self.assertEqual(source / "shared-target", cargo_target_directory(source))
        with mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": "/cache/cargo"}, clear=True):
            self.assertEqual(Path("/cache/cargo"), cargo_target_directory(source))

    def test_compass_prepare_selects_binary_from_cargo_target_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            binary = source / "shared-target" / "release" / "compass"
            binary.parent.mkdir(parents=True)
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
            expected_revision = revision("compass")

            with (
                mock.patch.dict(
                    os.environ,
                    {"CARGO_TARGET_DIR": "shared-target"},
                    clear=False,
                ),
                mock.patch.object(adapters_module, "_git_value", return_value=""),
                mock.patch.object(adapters_module, "_run", return_value="tool version"),
                mock.patch.object(
                    adapters_module,
                    "_revision",
                    return_value=expected_revision,
                ),
            ):
                adapter = CompassAdapter.prepare(source)

            self.assertEqual(binary.resolve(), adapter.executable)

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
                "--no-cluster",
                "--no-viz",
                "--store",
                "json",
                "--timing",
                "--out",
                "/output",
            ),
        )
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

    def test_graphify_removes_only_its_checkout_artifacts(self) -> None:
        adapter = GraphifyAdapter(Path("/venv/bin/python"), revision("graphify"))
        with tempfile.TemporaryDirectory() as directory:
            workspace = QualificationWorkspace.create(Path(directory) / "workspace")
            checkout = workspace.root / "corpora" / "fixture"
            generated = checkout / "graphify-out" / "cache"
            generated.mkdir(parents=True)
            (generated / "stat-index.json").write_text("{}")
            source = checkout / "source.py"
            source.write_text("pass\n")

            adapter.cleanup_checkout(checkout)

            self.assertFalse((checkout / "graphify-out").exists())
            self.assertEqual(source.read_text(), "pass\n")

    def test_compass_current_snapshot_is_validated(self) -> None:
        adapter = CompassAdapter(Path("/opt/compass"), revision("compass"))
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            compass_out = output / "compass-out"
            active = compass_out / "snapshots" / "snapshot-123"
            active.mkdir(parents=True)
            (compass_out / "current-snapshot").write_text("snapshot-123\n")
            graph = active / "graph.json"
            graph.write_text("{}")
            self.assertEqual(adapter.graph_path(output), graph)
            (active / "build-incomplete").touch()
            with self.assertRaisesRegex(RuntimeError, "incomplete"):
                adapter.graph_path(output)

    def test_snapshot_pointer_cannot_escape(self) -> None:
        adapter = CompassAdapter(Path("/opt/compass"), revision("compass"))
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            compass_out = output / "compass-out"
            compass_out.mkdir()
            (compass_out / "current-snapshot").write_text("../outside")
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

    def test_compass_prunes_only_incurrent_snapshots(self) -> None:
        adapter = CompassAdapter(Path("/opt/compass"), revision("compass"))
        with tempfile.TemporaryDirectory() as directory:
            workspace = QualificationWorkspace.create(Path(directory) / "workspace")
            output = workspace.root / "artifacts" / "fixture"
            snapshots = output / "compass-out" / "snapshots"
            active = snapshots / "snapshot-active"
            inactive = snapshots / "snapshot-inactive"
            active.mkdir(parents=True)
            inactive.mkdir()
            graph = active / "graph.json"
            graph.write_text("{}")

            adapter.prune_superseded_artifacts(output, graph)

            self.assertTrue(active.is_dir())
            self.assertFalse(inactive.exists())


if __name__ == "__main__":
    unittest.main()
