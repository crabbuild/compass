from __future__ import annotations

from dataclasses import replace
from pathlib import Path
import tempfile
import unittest

from benchmarks.performance.compass.model import (
    Aggregate,
    CheckoutIdentity,
    CorrectnessResult,
    EnvironmentIdentity,
    GateReport,
    ProcessMetrics,
    QualificationRun,
    Sample,
    ToolRevision,
    WorkloadResult,
)
from benchmarks.performance.compass.report import (
    compare_baseline,
    compare_tools,
    load_run,
    promote_baseline,
    write_run,
)


def aggregate(seconds: float, rss: int = 100) -> Aggregate:
    return Aggregate(3, seconds, seconds, seconds, seconds, 0.0, rss)


def result(
    tool: str,
    repository: str,
    workload: str,
    seconds: float,
    rss: int = 100,
    *,
    eligible: bool = True,
) -> WorkloadResult:
    metrics = ProcessMetrics(
        seconds,
        0.0,
        0.0,
        rss,
        0,
        None,
        False,
        ("tool",),
        "/tmp",
        "/tmp/out",
        "/tmp/err",
        "a",
        "b",
    )
    sample = Sample(
        f"{tool}:{repository}:{workload}:1",
        tool,
        repository,
        workload,
        1,
        eligible,
        metrics,
        error=None if eligible else "bad output",
    )
    correctness = CorrectnessResult(eligible, "digest", () if eligible else ("bad",))
    return WorkloadResult(
        tool,
        repository,
        workload,
        (sample,),
        aggregate(seconds, rss) if eligible else None,
        correctness,
    )


def environment() -> EnvironmentIdentity:
    return EnvironmentIdentity(
        "Darwin", "1", "arm64", "cpu", 8, 8, 1024, "3.11", "rust", "cargo", "host", "runner"
    )


def run(results: tuple[WorkloadResult, ...]) -> QualificationRun:
    corpora = tuple(
        CheckoutIdentity(f"repo-{index}", "https://example.com/repo", "main", "c", "t", "/tmp")
        for index in range(8)
    )
    tool = ToolRevision(
        "compass", "https://example.com/compass", "c", "t", False, "sha", {"profile": "release"}
    )
    return QualificationRun(
        "compass.performance-run/1",
        "test-run",
        "2026-01-01T00:00:00Z",
        "2026-01-01T00:01:00Z",
        True,
        "suite",
        environment(),
        (tool,),
        corpora,
        results,
        GateReport(True, ()),
    )


class ReportTests(unittest.TestCase):
    def test_exact_five_times_passes(self) -> None:
        report = compare_tools(
            (
                result("compass", "repo", "cold", 1.0, 100),
                result("graphify", "repo", "cold", 5.0, 100),
            )
        )
        self.assertTrue(report.passed)
        self.assertEqual(5.0, report.ratios["repo/cold"])

    def test_four_point_nine_nine_fails(self) -> None:
        report = compare_tools(
            (
                result("compass", "repo", "cold", 1.0),
                result("graphify", "repo", "cold", 4.99),
            )
        )
        self.assertFalse(report.passed)
        self.assertEqual("speedup-below-5x", report.issues[0].code)

    def test_one_failed_row_is_not_hidden_by_a_fast_row(self) -> None:
        report = compare_tools(
            (
                result("compass", "a", "cold", 1.0),
                result("graphify", "a", "cold", 100.0),
                result("compass", "b", "cold", 1.0),
                result("graphify", "b", "cold", 4.0),
            )
        )
        self.assertFalse(report.passed)
        self.assertTrue(any(issue.repository == "b" for issue in report.issues))

    def test_build_memory_must_not_exceed_graphify(self) -> None:
        report = compare_tools(
            (
                result("compass", "repo", "warm", 1.0, 101),
                result("graphify", "repo", "warm", 5.0, 100),
            )
        )
        self.assertFalse(report.passed)
        self.assertTrue(any(issue.code == "memory-above-graphify" for issue in report.issues))

    def test_baseline_allows_exactly_ten_percent(self) -> None:
        baseline = run((result("compass", "repo", "cold", 10.0, 100),))
        candidate = run((result("compass", "repo", "cold", 11.0, 110),))
        self.assertTrue(compare_baseline(candidate, baseline).passed)

    def test_baseline_rejects_regression_and_incompatibility(self) -> None:
        baseline = run((result("compass", "repo", "cold", 10.0),))
        slower = run((result("compass", "repo", "cold", 11.01),))
        self.assertFalse(compare_baseline(slower, baseline).passed)
        incompatible = replace(slower, suite_digest="other")
        report = compare_baseline(incompatible, baseline)
        self.assertEqual(("suite-mismatch",), tuple(issue.code for issue in report.issues))

    def test_atomic_report_discloses_exclusions_and_replaces(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            first = run((result("compass", "repo", "cold", 1.0, eligible=False),))
            run_path, summary_path = write_run(first, output)
            self.assertIn("Excluded samples", summary_path.read_text(encoding="utf-8"))
            second = run((result("compass", "repo", "cold", 1.0),))
            write_run(second, output)
            self.assertNotIn("Excluded samples", summary_path.read_text(encoding="utf-8"))
            self.assertTrue(run_path.is_file())
            self.assertEqual(second, load_run(run_path))
            self.assertEqual([], list(output.glob("*.tmp")))

    def test_promotion_rejects_interrupted_and_promotes_complete(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            complete = run((result("compass", "repo", "cold", 1.0),))
            run_path, _ = write_run(complete, root / "run")
            destination = root / "baseline" / "baseline.json"
            self.assertEqual(destination, promote_baseline(run_path, destination))
            interrupted = replace(complete, complete=False, completed_at=None)
            interrupted_path, _ = write_run(interrupted, root / "interrupted")
            with self.assertRaisesRegex(ValueError, "interrupted"):
                promote_baseline(interrupted_path, destination)


if __name__ == "__main__":
    unittest.main()
