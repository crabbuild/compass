"""Correctness-gated build and query workload execution."""

from __future__ import annotations

from contextlib import contextmanager
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
from typing import Iterator

from .adapters import ToolAdapter
from .correctness import compare_graphs, index_graph
from .model import CorrectnessResult, QueryOracle, RepositorySpec, Sample, WorkloadResult
from .process import ProcessSpec, run_measured
from .stats import summarize
from .workspace import guarded_remove

_EXCLUDED_PARTS = {
    "vendor",
    "third_party",
    "node_modules",
    "generated",
    "fixtures",
    "fixture",
    "tests",
    "test",
}

_COMPASSQL_QUERIES = (
    ("scan", "MATCH (n) RETURN n.id AS id ORDER BY id LIMIT 100"),
    ("anchored", "MATCH (n:Function) RETURN n.id AS id ORDER BY id LIMIT 100"),
    (
        "one-hop",
        "MATCH (a)-[r]->(b) RETURN a.id AS source, b.id AS target "
        "ORDER BY source, target LIMIT 100",
    ),
    (
        "bounded-path",
        "MATCH p=(a)-[:CALLS|IMPORTS_FROM*1..2]->(b) "
        "RETURN a.id AS source, b.id AS target ORDER BY source, target LIMIT 100",
    ),
    ("aggregate", "MATCH (n) RETURN count(n) AS nodes"),
    (
        "optional",
        "MATCH (n) OPTIONAL MATCH (n)-[:CALLS]->(target) "
        "RETURN n.id AS source, target.id AS target ORDER BY source, target LIMIT 100",
    ),
    (
        "policy-shaped",
        "MATCH (n) WHERE EXISTS { MATCH (n)-[:CALLS]->(target) } "
        "RETURN n.id AS id ORDER BY id LIMIT 100",
    ),
)


def _git(checkout: Path, *arguments: str) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=checkout,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        encoding="utf-8",
    )
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.strip())
    return completed.stdout


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def select_mutation_file(checkout: Path, suffix: str) -> Path:
    raw = _git(checkout, "ls-files", "-z")
    candidates: list[Path] = []
    for relative_text in raw.split("\0"):
        if not relative_text:
            continue
        relative = Path(relative_text)
        if relative.suffix.lower() != suffix.lower():
            continue
        if any(part.lower() in _EXCLUDED_PARTS for part in relative.parts):
            continue
        path = checkout / relative
        if not path.is_file():
            continue
        size = path.stat().st_size
        if 1024 <= size <= 256 * 1024:
            candidates.append(relative)
    if not candidates:
        raise RuntimeError(f"no safe {suffix} mutation candidate in {checkout}")
    return checkout / sorted(candidates, key=lambda item: item.as_posix())[0]


@contextmanager
def graph_neutral_mutation(checkout: Path, path: Path) -> Iterator[None]:
    original = path.read_bytes()
    original_digest = hashlib.sha256(original).hexdigest()
    path.write_bytes(original + b"\n")
    if _file_sha256(path) == original_digest:
        raise RuntimeError(f"mutation did not change {path}")
    status = [
        line
        for line in _git(checkout, "status", "--porcelain=v1", "--untracked-files=all").splitlines()
        if line
    ]
    relative = path.relative_to(checkout).as_posix()
    if len(status) != 1 or status[0][3:] != relative:
        path.write_bytes(original)
        raise RuntimeError(f"mutation changed unexpected files: {status}")
    try:
        yield
    finally:
        path.write_bytes(original)
        if _file_sha256(path) != original_digest:
            raise RuntimeError(f"failed to restore {path}")
        restored = _git(checkout, "status", "--porcelain=v1", "--untracked-files=all").strip()
        if restored:
            raise RuntimeError(f"checkout remained dirty after restoration: {restored}")


def _validate_graph(tool: str, graph: Path) -> CorrectnessResult:
    database = sqlite3.connect(":memory:")
    try:
        summary = index_graph(tool, graph, database)
        comparison = compare_graphs(database)
        return CorrectnessResult(
            passed=comparison.passed,
            digest=summary.digest,
            failures=comparison.failures,
            warnings=comparison.warnings,
            metrics={**comparison.metrics, "canonical_graph_digest": summary.digest},
        )
    except (OSError, ValueError, sqlite3.Error) as error:
        payload = str(error)
        return CorrectnessResult(
            passed=False,
            digest=hashlib.sha256(payload.encode("utf-8")).hexdigest(),
            failures=(payload,),
        )
    finally:
        database.close()


def _result(
    tool: str,
    repository: str,
    workload: str,
    samples: list[Sample],
    failures: list[str],
) -> WorkloadResult:
    correctness_payload = json.dumps(failures, sort_keys=True, separators=(",", ":"))
    correctness = CorrectnessResult(
        passed=not failures,
        digest=hashlib.sha256(correctness_payload.encode("utf-8")).hexdigest(),
        failures=tuple(failures),
    )
    aggregate = None
    if correctness.passed:
        try:
            aggregate = summarize(samples)
        except ValueError as error:
            correctness = CorrectnessResult(
                passed=False,
                digest=correctness.digest,
                failures=(str(error),),
            )
    return WorkloadResult(tool, repository, workload, tuple(samples), aggregate, correctness)


def _run_build(
    adapter: ToolAdapter,
    checkout: Path,
    output: Path,
    logs: Path,
    repository: str,
    workload: str,
    iteration: int,
    timeout_seconds: float,
) -> tuple[Sample, str | None]:
    logs.mkdir(parents=True, exist_ok=True)
    metrics = run_measured(
        ProcessSpec(
            command=adapter.build_command(checkout, output),
            cwd=checkout,
            stdout_path=logs / f"{workload}-{iteration}.out",
            stderr_path=logs / f"{workload}-{iteration}.err",
            timeout_seconds=timeout_seconds,
        )
    )
    error: str | None = None
    correctness_digest = ""
    evidence: dict[str, float | int | str | bool] = {}
    if metrics.return_code != 0 or metrics.timed_out:
        error = f"command failed with return code {metrics.return_code}"
    else:
        try:
            graph = adapter.graph_path(output)
            correctness = _validate_graph(adapter.name, graph)
            correctness_digest = correctness.digest
            if not correctness.passed:
                error = "; ".join(correctness.failures)
            evidence["graph_sha256"] = _file_sha256(graph)
            evidence.update(
                adapter.parse_build_evidence(
                    Path(metrics.stderr_path).read_text(encoding="utf-8", errors="replace")
                )
            )
            adapter.prune_superseded_artifacts(output, graph)
        except (OSError, RuntimeError) as exception:
            error = str(exception)
    sample = Sample(
        sample_id=f"{adapter.name}:{repository}:{workload}:{iteration}",
        tool=adapter.name,
        repository=repository,
        workload=workload,
        iteration=iteration,
        eligible=error is None,
        metrics=metrics,
        correctness_digest=correctness_digest,
        error=error,
        evidence=evidence,
    )
    return sample, error


def run_build_matrix(
    adapter: ToolAdapter,
    checkout: Path,
    artifact_root: Path,
    spec: RepositorySpec,
    *,
    repeats: int = 3,
    timeout_seconds: float = 1800,
) -> tuple[WorkloadResult, ...]:
    if repeats < 3:
        raise ValueError("build qualification requires at least three repeats")
    output = artifact_root / adapter.name / spec.name
    logs = artifact_root / "logs" / adapter.name / spec.name
    cold_samples: list[Sample] = []
    cold_failures: list[str] = []
    baseline_digest: str | None = None
    for iteration in range(1, repeats + 1):
        if output.exists():
            guarded_remove(output)
        sample, error = _run_build(
            adapter,
            checkout,
            output,
            logs,
            spec.name,
            "cold",
            iteration,
            timeout_seconds,
        )
        cold_samples.append(sample)
        if error:
            cold_failures.append(f"cold[{iteration}]: {error}")
        elif baseline_digest is None:
            baseline_digest = sample.correctness_digest
        elif sample.correctness_digest != baseline_digest:
            cold_failures.append(f"cold[{iteration}] graph digest changed")

    if baseline_digest is None:
        return (_result(adapter.name, spec.name, "cold", cold_samples, cold_failures),)

    _run_build(
        adapter,
        checkout,
        output,
        logs,
        spec.name,
        "warmup",
        0,
        timeout_seconds,
    )
    warm_samples: list[Sample] = []
    warm_failures: list[str] = []
    for iteration in range(1, repeats + 1):
        sample, error = _run_build(
            adapter,
            checkout,
            output,
            logs,
            spec.name,
            "warm",
            iteration,
            timeout_seconds,
        )
        warm_samples.append(sample)
        if error:
            warm_failures.append(f"warm[{iteration}]: {error}")
        elif sample.correctness_digest != baseline_digest:
            warm_failures.append(f"warm[{iteration}] graph digest changed")

    mutation = select_mutation_file(checkout, spec.mutation_suffix)
    incremental_samples: list[Sample] = []
    incremental_failures: list[str] = []
    for iteration in range(1, repeats + 1):
        with graph_neutral_mutation(checkout, mutation):
            sample, error = _run_build(
                adapter,
                checkout,
                output,
                logs,
                spec.name,
                "incremental",
                iteration,
                timeout_seconds,
            )
        incremental_samples.append(sample)
        if error:
            incremental_failures.append(f"incremental[{iteration}]: {error}")
        elif sample.correctness_digest != baseline_digest:
            incremental_failures.append(f"incremental[{iteration}] graph digest changed")
        restore, restore_error = _run_build(
            adapter,
            checkout,
            output,
            logs,
            spec.name,
            "restore",
            iteration,
            timeout_seconds,
        )
        if restore_error or restore.correctness_digest != baseline_digest:
            incremental_failures.append(
                f"restore[{iteration}]: {restore_error or 'graph digest changed'}"
            )

    return (
        _result(adapter.name, spec.name, "cold", cold_samples, cold_failures),
        _result(adapter.name, spec.name, "warm", warm_samples, warm_failures),
        _result(
            adapter.name,
            spec.name,
            "incremental",
            incremental_samples,
            incremental_failures,
        ),
    )


def validate_query_output(text: str, oracle: QueryOracle) -> CorrectnessResult:
    folded = text.casefold()
    failures = [
        f"missing required query evidence: {required}"
        for required in oracle.required
        if required.casefold() not in folded
    ]
    failures.extend(
        f"forbidden query evidence present: {forbidden}"
        for forbidden in oracle.forbidden
        if forbidden.casefold() in folded
    )
    canonical = "\n".join(sorted(line.strip() for line in text.splitlines() if line.strip()))
    return CorrectnessResult(
        passed=not failures,
        digest=hashlib.sha256(canonical.encode("utf-8")).hexdigest(),
        failures=tuple(failures),
    )


def run_query_matrix(
    adapter: ToolAdapter,
    graph: Path,
    artifact_root: Path,
    spec: RepositorySpec,
    *,
    batches: int = 10,
    timeout_seconds: float = 120,
) -> tuple[WorkloadResult, ...]:
    if batches < 10:
        raise ValueError("query qualification requires at least ten batches")
    results: list[WorkloadResult] = []
    for query_index, oracle in enumerate(spec.queries):
        workload = f"query-{query_index + 1}"
        samples: list[Sample] = []
        failures: list[str] = []
        logs = artifact_root / "logs" / adapter.name / spec.name / workload
        logs.mkdir(parents=True, exist_ok=True)
        for iteration in range(0, batches + 1):
            metrics = run_measured(
                ProcessSpec(
                    command=adapter.query_command(graph, oracle.question),
                    cwd=graph.parent,
                    stdout_path=logs / f"{iteration}.out",
                    stderr_path=logs / f"{iteration}.err",
                    timeout_seconds=timeout_seconds,
                )
            )
            output = Path(metrics.stdout_path).read_text(encoding="utf-8", errors="replace")
            correctness = validate_query_output(output, oracle)
            error = None
            if metrics.return_code != 0 or metrics.timed_out:
                error = f"query failed with return code {metrics.return_code}"
            elif not correctness.passed:
                error = "; ".join(correctness.failures)
            if iteration == 0:
                if error:
                    failures.append(f"{workload}[warmup]: {error}")
                continue
            if error:
                failures.append(f"{workload}[{iteration}]: {error}")
            samples.append(
                Sample(
                    sample_id=f"{adapter.name}:{spec.name}:{workload}:{iteration}",
                    tool=adapter.name,
                    repository=spec.name,
                    workload=workload,
                    iteration=iteration,
                    eligible=error is None,
                    metrics=metrics,
                    correctness_digest=correctness.digest,
                    error=error,
                )
            )
        results.append(_result(adapter.name, spec.name, workload, samples, failures))
    return tuple(results)


def _canonical_json_output(text: str) -> CorrectnessResult:
    try:
        value = json.loads(text)
    except json.JSONDecodeError as error:
        message = f"invalid CompassQL JSON: {error}"
        return CorrectnessResult(
            False,
            hashlib.sha256(message.encode("utf-8")).hexdigest(),
            (message,),
        )
    canonical = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return CorrectnessResult(
        True,
        hashlib.sha256(canonical.encode("utf-8")).hexdigest(),
        metrics={"json_bytes": len(canonical.encode("utf-8"))},
    )


def run_compassql_matrix(
    adapter: ToolAdapter,
    graph: Path,
    artifact_root: Path,
    spec: RepositorySpec,
    *,
    batches: int = 10,
    timeout_seconds: float = 120,
) -> tuple[WorkloadResult, ...]:
    """Measure representative CompassQL plans; these have no Graphify ratio."""
    if adapter.name != "compass":
        raise ValueError("CompassQL workloads are Compass-only")
    if batches < 10:
        raise ValueError("CompassQL qualification requires at least ten batches")
    results: list[WorkloadResult] = []
    for name, query in _COMPASSQL_QUERIES:
        workload = f"compassql-{name}"
        logs = artifact_root / "logs" / adapter.name / spec.name / workload
        logs.mkdir(parents=True, exist_ok=True)
        samples: list[Sample] = []
        failures: list[str] = []
        expected_digest: str | None = None
        for iteration in range(0, batches + 1):
            metrics = run_measured(
                ProcessSpec(
                    command=adapter.compassql_command(graph, query),
                    cwd=graph.parent,
                    stdout_path=logs / f"{iteration}.out",
                    stderr_path=logs / f"{iteration}.err",
                    timeout_seconds=timeout_seconds,
                )
            )
            output = Path(metrics.stdout_path).read_text(
                encoding="utf-8", errors="replace"
            )
            correctness = _canonical_json_output(output)
            error = None
            if metrics.return_code != 0 or metrics.timed_out:
                error = f"CompassQL failed with return code {metrics.return_code}"
            elif not correctness.passed:
                error = "; ".join(correctness.failures)
            elif expected_digest is None:
                expected_digest = correctness.digest
            elif correctness.digest != expected_digest:
                error = "CompassQL result digest changed"
            if iteration == 0:
                if error:
                    failures.append(f"{workload}[warmup]: {error}")
                continue
            if error:
                failures.append(f"{workload}[{iteration}]: {error}")
            samples.append(
                Sample(
                    sample_id=f"{adapter.name}:{spec.name}:{workload}:{iteration}",
                    tool=adapter.name,
                    repository=spec.name,
                    workload=workload,
                    iteration=iteration,
                    eligible=error is None,
                    metrics=metrics,
                    correctness_digest=correctness.digest,
                    error=error,
                    evidence=correctness.metrics,
                )
            )
        results.append(_result(adapter.name, spec.name, workload, samples, failures))
    return tuple(results)
