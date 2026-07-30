"""Atomic qualification reports and honest per-workload gates."""

from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile
from typing import Sequence

from .model import GateIssue, GateReport, QualificationRun, WorkloadResult, to_json_value

_PRIMARY_BUILDS = {"cold", "warm", "incremental"}


def _result_key(result: WorkloadResult) -> tuple[str, str]:
    return result.repository, result.workload


def _is_comparable(workload: str) -> bool:
    return workload in _PRIMARY_BUILDS or workload.startswith("query-")


def compare_tools(results: Sequence[WorkloadResult]) -> GateReport:
    """Require an independent 5x median and matched-memory pass for every row."""
    by_tool = {
        (result.tool, result.repository, result.workload): result for result in results
    }
    rows = sorted(
        {
            (result.repository, result.workload)
            for result in results
            if _is_comparable(result.workload)
        }
    )
    issues: list[GateIssue] = []
    ratios: dict[str, float] = {}
    for repository, workload in rows:
        compass = by_tool.get(("compass", repository, workload))
        graphify = by_tool.get(("graphify", repository, workload))
        if compass is None or graphify is None:
            issues.append(
                GateIssue(
                    "missing-comparison",
                    repository,
                    workload,
                    "both Compass and Graphify results are required",
                )
            )
            continue
        if (
            not compass.correctness.passed
            or not graphify.correctness.passed
            or compass.aggregate is None
            or graphify.aggregate is None
        ):
            issues.append(
                GateIssue(
                    "ineligible-comparison",
                    repository,
                    workload,
                    "correct aggregate results are required for both tools",
                )
            )
            continue
        if compass.aggregate.p50_seconds <= 0:
            issues.append(
                GateIssue(
                    "invalid-median",
                    repository,
                    workload,
                    "Compass median must be positive",
                )
            )
            continue
        ratio = graphify.aggregate.p50_seconds / compass.aggregate.p50_seconds
        ratios[f"{repository}/{workload}"] = ratio
        if ratio < 5.0:
            issues.append(
                GateIssue(
                    "speedup-below-5x",
                    repository,
                    workload,
                    f"median speedup is {ratio:.3f}x; required >= 5.000x",
                )
            )
        if (
            workload in _PRIMARY_BUILDS
            and compass.aggregate.peak_rss_kib > graphify.aggregate.peak_rss_kib
        ):
            issues.append(
                GateIssue(
                    "memory-above-graphify",
                    repository,
                    workload,
                    "Compass peak RSS exceeds Graphify "
                    f"({compass.aggregate.peak_rss_kib} > "
                    f"{graphify.aggregate.peak_rss_kib} KiB)",
                )
            )
    if not rows:
        issues.append(
            GateIssue(
                "missing-comparisons",
                "*",
                "*",
                "no comparable build or natural-language query results were recorded",
            )
        )
    return GateReport(not issues, tuple(issues), ratios)


def _compatibility_issues(
    run: QualificationRun, baseline: QualificationRun
) -> list[GateIssue]:
    issues: list[GateIssue] = []
    if run.schema != baseline.schema:
        issues.append(GateIssue("schema-mismatch", "*", "*", "run schemas differ"))
    if run.suite_digest != baseline.suite_digest:
        issues.append(
            GateIssue("suite-mismatch", "*", "*", "query/workload manifests differ")
        )
    if run.environment != baseline.environment:
        issues.append(
            GateIssue("environment-mismatch", "*", "*", "runner environments differ")
        )
    current_corpora = {
        identity.name: (identity.url, identity.commit, identity.tree)
        for identity in run.corpora
    }
    baseline_corpora = {
        identity.name: (identity.url, identity.commit, identity.tree)
        for identity in baseline.corpora
    }
    if current_corpora != baseline_corpora:
        issues.append(
            GateIssue("corpus-mismatch", "*", "*", "corpus revisions differ")
        )
    current_compass = next((tool for tool in run.tools if tool.name == "compass"), None)
    baseline_compass = next(
        (tool for tool in baseline.tools if tool.name == "compass"), None
    )
    if current_compass is None or baseline_compass is None:
        issues.append(
            GateIssue("tool-missing", "*", "*", "both runs require a Compass revision")
        )
    elif current_compass.metadata.get("profile", "release") != baseline_compass.metadata.get(
        "profile", "release"
    ):
        issues.append(
            GateIssue("profile-mismatch", "*", "*", "Compass build profiles differ")
        )
    return issues


def compare_baseline(run: QualificationRun, baseline: QualificationRun) -> GateReport:
    """Apply same-runner Compass p50, p95, and RSS regression gates."""
    issues = _compatibility_issues(run, baseline)
    if issues:
        return GateReport(False, tuple(issues))
    current = {
        _result_key(result): result for result in run.results if result.tool == "compass"
    }
    previous = {
        _result_key(result): result
        for result in baseline.results
        if result.tool == "compass"
    }
    for key in sorted(previous):
        repository, workload = key
        candidate = current.get(key)
        reference = previous[key]
        if candidate is None:
            issues.append(
                GateIssue(
                    "missing-baseline-row",
                    repository,
                    workload,
                    "current run omitted a baseline workload",
                )
            )
            continue
        if (
            not candidate.correctness.passed
            or candidate.aggregate is None
            or not reference.correctness.passed
            or reference.aggregate is None
        ):
            issues.append(
                GateIssue(
                    "ineligible-regression-row",
                    repository,
                    workload,
                    "correct aggregates are required for regression comparison",
                )
            )
            continue
        for metric, current_value, baseline_value in (
            (
                "p50",
                candidate.aggregate.p50_seconds,
                reference.aggregate.p50_seconds,
            ),
            (
                "p95",
                candidate.aggregate.p95_seconds,
                reference.aggregate.p95_seconds,
            ),
            (
                "peak-rss",
                float(candidate.aggregate.peak_rss_kib),
                float(reference.aggregate.peak_rss_kib),
            ),
        ):
            if current_value > baseline_value * 1.10:
                issues.append(
                    GateIssue(
                        f"{metric}-regression",
                        repository,
                        workload,
                        f"{metric} regressed more than 10% "
                        f"({current_value:g} vs {baseline_value:g})",
                    )
                )
    return GateReport(not issues, tuple(issues))


def _atomic_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory_descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
    finally:
        if temporary.exists():
            temporary.unlink()


def render_markdown(run: QualificationRun) -> str:
    lines = [
        "# Compass performance qualification",
        "",
        f"- Run: `{run.run_id}`",
        f"- Complete: `{str(run.complete).lower()}`",
        f"- Runner: `{run.environment.runner_id}`",
        f"- Suite: `{run.suite_digest}`",
        "",
        "| Tool | Repository | Workload | Eligible | p50 (s) | p95 (s) | Peak RSS (MiB) |",
        "| --- | --- | --- | ---: | ---: | ---: | ---: |",
    ]
    for result in sorted(
        run.results, key=lambda item: (item.tool, item.repository, item.workload)
    ):
        aggregate = result.aggregate
        eligible = sum(sample.eligible for sample in result.samples)
        total = len(result.samples)
        lines.append(
            "| "
            + " | ".join(
                (
                    result.tool,
                    result.repository,
                    result.workload,
                    f"{eligible}/{total}",
                    f"{aggregate.p50_seconds:.6f}" if aggregate else "—",
                    f"{aggregate.p95_seconds:.6f}" if aggregate else "—",
                    f"{aggregate.peak_rss_kib / 1024:.2f}" if aggregate else "—",
                )
            )
            + " |"
        )
    excluded = [
        sample
        for result in run.results
        for sample in result.samples
        if not sample.eligible
    ]
    if excluded:
        lines.extend(("", "## Excluded samples", ""))
        for sample in excluded:
            lines.append(f"- `{sample.sample_id}`: {sample.error or 'ineligible'}")
    failures = [
        (result, failure)
        for result in run.results
        for failure in result.correctness.failures
    ]
    if failures:
        lines.extend(("", "## Correctness failures", ""))
        for result, failure in failures:
            lines.append(
                f"- `{result.tool}/{result.repository}/{result.workload}`: {failure}"
            )
    if run.gates is not None:
        lines.extend(
            (
                "",
                "## Gates",
                "",
                f"Overall: **{'PASS' if run.gates.passed else 'FAIL'}**",
            )
        )
        for issue in run.gates.issues:
            lines.append(
                f"- `{issue.code}` `{issue.repository}/{issue.workload}`: {issue.message}"
            )
        if run.gates.ratios:
            lines.extend(("", "### Speed ratios", ""))
            for key, ratio in sorted(run.gates.ratios.items()):
                lines.append(f"- `{key}`: {ratio:.3f}x")
    lines.append("")
    return "\n".join(lines)


def write_run(run: QualificationRun, output: Path) -> tuple[Path, Path]:
    """Write run.json and summary.md atomically from the same immutable value."""
    run_path = output / "run.json"
    summary_path = output / "summary.md"
    payload = json.dumps(
        to_json_value(run), sort_keys=True, separators=(",", ":"), ensure_ascii=False
    )
    _atomic_text(run_path, payload + "\n")
    _atomic_text(summary_path, render_markdown(run))
    return run_path, summary_path


def promote_baseline(run_path: Path, destination: Path) -> Path:
    """Atomically promote only a complete, clean, passing eight-corpus run."""
    if run_path.is_symlink() or not run_path.is_file():
        raise ValueError(f"baseline source must be a regular file: {run_path}")
    payload = json.loads(run_path.read_text(encoding="utf-8"))
    if payload.get("schema") != "compass.performance-run/1":
        raise ValueError("unsupported qualification run schema")
    if not payload.get("complete") or not payload.get("completed_at"):
        raise ValueError("interrupted qualification runs cannot be promoted")
    corpora = payload.get("corpora", [])
    if len(corpora) != 8 or len({item.get("name") for item in corpora}) != 8:
        raise ValueError("a promoted baseline must contain all eight repositories")
    tools = payload.get("tools", [])
    if not tools or any(tool.get("dirty", True) for tool in tools):
        raise ValueError("dirty or missing tool revisions cannot be promoted")
    results = payload.get("results", [])
    if not results or any(
        not result.get("correctness", {}).get("passed") or result.get("aggregate") is None
        for result in results
    ):
        raise ValueError("all promoted workloads must be correct and aggregated")
    gates = payload.get("gates")
    if gates is not None and not gates.get("passed"):
        raise ValueError("a failing qualification run cannot be promoted")
    compact = dict(payload)
    for result in compact["results"]:
        for sample in result.get("samples", []):
            sample.pop("evidence", None)
            metrics = sample.get("metrics", {})
            metrics.pop("stdout_path", None)
            metrics.pop("stderr_path", None)
    encoded = json.dumps(
        compact, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    )
    _atomic_text(destination, encoded + "\n")
    return destination
