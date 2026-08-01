from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from benchmarks.performance.compass.adapters import GraphifyAdapter, ToolAdapter
from benchmarks.performance.compass.model import QueryOracle, RepositorySpec, ToolRevision
from benchmarks.performance.compass.workloads import (
    graph_neutral_mutation,
    run_build_matrix,
    run_compassql_matrix,
    run_query_matrix,
    select_mutation_file,
    validate_query_output,
)
from benchmarks.performance.compass.workspace import QualificationWorkspace


FIXTURE = Path(__file__).parent / "fixtures" / "compass_graph.json"
FAKE_TOOL = Path(__file__).parent / "helpers" / "fake_tool.py"


class FakeAdapter(ToolAdapter):
    def build_command(self, checkout: Path, output: Path, *, force: bool = False):
        return (
            sys.executable,
            str(FAKE_TOOL),
            "build",
            "--output",
            str(output),
            "--graph",
            str(FIXTURE),
        )

    def query_command(self, graph: Path, question: str):
        return (sys.executable, str(FAKE_TOOL), "query", "--text", "URLResolver safe result")

    def compassql_command(self, graph: Path, query: str):
        return (
            sys.executable,
            "-c",
            'print(\'{"columns":["id"],"rows":[{"id":"fixture:function"}]}\')',
        )

    def graph_path(self, output: Path) -> Path:
        return output / "graph.json"


class CheckoutWritingAdapter(FakeAdapter):
    def build_command(self, checkout: Path, output: Path, *, force: bool = False):
        script = (
            "from pathlib import Path; import shutil; "
            f"checkout = Path({str(checkout)!r}); output = Path({str(output)!r}); "
            f"fixture = Path({str(FIXTURE)!r}); "
            "output.mkdir(parents=True, exist_ok=True); "
            "shutil.copyfile(fixture, output / 'graph.json'); "
            "cache = checkout / 'graphify-out' / 'cache'; "
            "cache.mkdir(parents=True, exist_ok=True); "
            "(cache / 'stat-index.json').write_text('{}')"
        )
        return (sys.executable, "-c", script)

    def cleanup_checkout(self, checkout: Path) -> None:
        GraphifyAdapter.cleanup_checkout(self, checkout)


def git(cwd: Path, *arguments: str) -> str:
    return subprocess.check_output(["git", *arguments], cwd=cwd, text=True).strip()


class WorkloadTests(unittest.TestCase):
    def make_checkout(self, root: Path) -> Path:
        checkout = root / "checkout"
        checkout.mkdir()
        git(checkout, "init", "-q")
        git(checkout, "config", "user.name", "Compass")
        git(checkout, "config", "user.email", "compass@example.invalid")
        source = checkout / "src" / "main.py"
        source.parent.mkdir()
        source.write_text("def run():\n    return 1\n" + "# filler\n" * 200, encoding="utf-8")
        git(checkout, "add", "src/main.py")
        git(checkout, "commit", "-q", "-m", "fixture")
        return checkout

    def adapter(self) -> FakeAdapter:
        revision = ToolRevision(
            "compass",
            "https://example.invalid/compass.git",
            "a" * 40,
            "b" * 40,
            False,
            "c" * 64,
        )
        return FakeAdapter(Path(sys.executable), revision)

    def graphify_adapter(self) -> FakeAdapter:
        revision = ToolRevision(
            "graphify",
            "https://example.invalid/graphify.git",
            "d" * 40,
            "e" * 40,
            False,
            "f" * 64,
        )
        return FakeAdapter(Path(sys.executable), revision)

    def test_mutation_selection_and_restoration_are_clean(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            checkout = self.make_checkout(Path(directory))
            source = select_mutation_file(checkout, ".py")
            original = source.read_bytes()
            with graph_neutral_mutation(checkout, source):
                self.assertNotEqual(source.read_bytes(), original)
            self.assertEqual(source.read_bytes(), original)
            self.assertEqual(git(checkout, "status", "--porcelain"), "")

    def test_mutation_allows_graphify_cache_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            checkout = self.make_checkout(Path(directory))
            cache = checkout / "graphify-out" / "cache"
            cache.mkdir(parents=True)
            (cache / "stat-index.json").write_text("{}", encoding="utf-8")
            source = select_mutation_file(checkout, ".py")
            original = source.read_bytes()
            with graph_neutral_mutation(checkout, source):
                self.assertNotEqual(source.read_bytes(), original)
            self.assertEqual(source.read_bytes(), original)
            self.assertEqual(git(checkout, "status", "--porcelain"), "")
            self.assertFalse(cache.exists())

    def test_build_matrix_produces_three_correct_workloads(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = QualificationWorkspace.create(root / "workspace")
            checkout = self.make_checkout(root)
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (QueryOracle("where", ("URLResolver",)), QueryOracle("how", ("safe",))),
            )
            results = run_build_matrix(
                self.adapter(),
                checkout,
                workspace.root / "artifacts",
                spec,
                timeout_seconds=5,
            )
            self.assertEqual([result.workload for result in results], ["cold", "warm", "incremental"])
            self.assertTrue(all(result.correctness.passed for result in results))
            self.assertTrue(all(result.aggregate is not None for result in results))

    def test_graphify_build_matrix_validates_without_a_compass_index(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = QualificationWorkspace.create(root / "workspace")
            checkout = self.make_checkout(root)
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (),
            )
            results = run_build_matrix(
                self.graphify_adapter(),
                checkout,
                workspace.root / "artifacts",
                spec,
                timeout_seconds=5,
            )
            self.assertTrue(all(result.correctness.passed for result in results))
            self.assertTrue(all(result.aggregate is not None for result in results))

    def test_build_matrix_cleans_tool_owned_checkout_artifacts_after_every_run(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = QualificationWorkspace.create(root / "workspace")
            checkout = self.make_checkout(workspace.root)
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (),
            )
            adapter = CheckoutWritingAdapter(Path(sys.executable), self.graphify_adapter().revision)

            results = run_build_matrix(
                adapter,
                checkout,
                workspace.root / "artifacts",
                spec,
                timeout_seconds=5,
            )

            self.assertTrue(all(result.correctness.passed for result in results))
            self.assertFalse((checkout / "graphify-out").exists())
            self.assertEqual(git(checkout, "status", "--porcelain"), "")

    def test_query_matrix_requires_oracle_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            graph = root / "graph.json"
            graph.write_text("{}", encoding="utf-8")
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (
                    QueryOracle("where", ("URLResolver",), ("forbidden",)),
                    QueryOracle("how", ("safe",)),
                ),
            )
            results = run_query_matrix(
                self.adapter(),
                graph,
                root / "artifacts",
                spec,
                batches=10,
                timeout_seconds=5,
            )
            self.assertEqual(len(results), 2)
            self.assertTrue(all(result.correctness.passed for result in results))

    def test_query_validation_rejects_forbidden_evidence(self) -> None:
        result = validate_query_output(
            "URLResolver also emitted WrongRoute",
            QueryOracle("where", ("URLResolver",), ("WrongRoute",)),
        )
        self.assertFalse(result.passed)

    def test_compassql_matrix_canonicalizes_results(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            graph = root / "graph.json"
            graph.write_text("{}", encoding="utf-8")
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (QueryOracle("where", ("URLResolver",)), QueryOracle("how", ("safe",))),
            )
            results = run_compassql_matrix(
                self.adapter(),
                graph,
                root / "artifacts",
                spec,
                batches=10,
                timeout_seconds=5,
            )
            self.assertEqual(7, len(results))
            self.assertTrue(all(result.correctness.passed for result in results))
            self.assertTrue(all(len(result.samples) == 10 for result in results))


if __name__ == "__main__":
    unittest.main()
