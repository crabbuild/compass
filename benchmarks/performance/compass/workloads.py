"""Correctness-gated build and query workload execution."""

from __future__ import annotations

from contextlib import contextmanager
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import shutil
from typing import Iterator

from .adapters import ToolAdapter
from .correctness import index_graph
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


def _prune_graphify_mutation_artifacts(checkout: Path) -> None:
    graphify_cache = checkout / "graphify-out" / "cache"
    if graphify_cache.exists():
        shutil.rmtree(graphify_cache, ignore_errors=True)
        graphify_parent = graphify_cache.parent
        if graphify_parent.is_dir() and not any(graphify_parent.iterdir()):
            graphify_parent.rmdir()


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
    _prune_graphify_mutation_artifacts(checkout)
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
    status = [line for line in status if "graphify-out/cache/" not in line[3:]]
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
        _prune_graphify_mutation_artifacts(checkout)
        restored = _git(checkout, "status", "--porcelain=v1", "--untracked-files=all").strip()
        restored = "\n".join(
            line for line in restored.splitlines() if "graphify-out/cache/" not in line[3:]
        )
        if restored:
            raise RuntimeError(f"checkout remained dirty after restoration: {restored}")


def _validate_graph(tool: str, graph: Path) -> CorrectnessResult:
    database = sqlite3.connect(":memory:")
    try:
        summary = index_graph(tool, graph, database)
        failures = (
            (f"{tool.title()} graph reports {summary.validation_errors} validation errors",)
            if summary.validation_errors
            else ()
        )
        return CorrectnessResult(
            passed=not failures,
            digest=summary.digest,
            failures=failures,
            metrics={
                f"{tool}_nodes": summary.nodes,
                f"{tool}_edges": summary.edges,
                f"{tool}_validation_errors": summary.validation_errors,
                "canonical_graph_digest": summary.digest,
            },
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
    try:
        if metrics.return_code != 0 or metrics.timed_out:
            error = f"command failed with return code {metrics.return_code}"
        else:
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
    finally:
        try:
            adapter.cleanup_checkout(checkout)
        except (OSError, RuntimeError, ValueError) as exception:
            if error is None:
                error = f"checkout cleanup failed: {exception}"
            else:
                error = f"{error}; checkout cleanup failed: {exception}"
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


def _source_file(record: dict[str, object]) -> str:
    source = record.get("source")
    return str(source.get("file", "")) if isinstance(source, dict) else ""


def _source_line(record: dict[str, object]) -> int | None:
    source = record.get("source")
    value = source.get("startLine") if isinstance(source, dict) else None
    return value if isinstance(value, int) else None


def _oracle_pair(oracle) -> tuple[str, str, int | None]:
    if oracle.source is None:
        return (oracle.qualified_name, "", None)
    return (oracle.qualified_name, oracle.source.file, oracle.source.start_line)


def _node_pair(node: dict[str, object]) -> tuple[str, str, int | None]:
    return (str(node.get("qualifiedName", "")), _source_file(node), _source_line(node))


def _pair_matches(
    observed: tuple[str, str, int | None], expected: tuple[str, str, int | None]
) -> bool:
    return (
        observed[0] == expected[0]
        and (not expected[1] or observed[1] == expected[1])
        and (expected[2] is None or observed[2] == expected[2])
    )


def _validate_compass_discovery(text: str, oracle: QueryOracle) -> CorrectnessResult:
    try:
        payload = json.loads(text)
    except json.JSONDecodeError as error:
        message = f"invalid Compass discovery JSON: {error}"
        return CorrectnessResult(False, hashlib.sha256(message.encode()).hexdigest(), (message,))
    if not isinstance(payload, dict) or payload.get("schema") != "compass.query.discovery/1":
        message = "Compass query did not emit compass.query.discovery/1"
        return CorrectnessResult(False, hashlib.sha256(message.encode()).hexdigest(), (message,))
    seeds = payload.get("seeds", [])
    nodes = payload.get("nodes", [])
    edges = payload.get("edges", [])
    diagnostics = payload.get("diagnostics", [])
    if not all(isinstance(value, list) for value in (seeds, nodes, edges, diagnostics)):
        message = "Compass discovery arrays are malformed"
        return CorrectnessResult(False, hashlib.sha256(message.encode()).hexdigest(), (message,))
    seed_records = [item for item in seeds if isinstance(item, dict)]
    node_records = [item for item in nodes if isinstance(item, dict)]
    edge_records = [item for item in edges if isinstance(item, dict)]
    nodes_by_id = {str(node.get("id", "")): node for node in node_records}
    seed_pairs = [
        _node_pair(nodes_by_id[str(seed.get("nodeId", ""))])
        for seed in seed_records
        if str(seed.get("nodeId", "")) in nodes_by_id
    ]
    expected = [_oracle_pair(item) for item in oracle.expected_seeds]
    acceptable = [_oracle_pair(item) for item in oracle.acceptable_seeds]
    forbidden = [_oracle_pair(item) for item in oracle.forbidden_seeds]
    failures: list[str] = []
    missing_expected = [
        item for item in expected if not any(_pair_matches(seed, item) for seed in seed_pairs)
    ]
    if missing_expected:
        failures.append(f"missing expected seeds: {missing_expected!r}")
    if not expected and acceptable and not any(
        _pair_matches(seed, item) for seed in seed_pairs for item in acceptable
    ):
        failures.append("no acceptable seed was returned")
    returned_forbidden = [
        item for item in forbidden if any(_pair_matches(seed, item) for seed in seed_pairs)
    ]
    if returned_forbidden:
        failures.append(f"forbidden seed returned: {returned_forbidden!r}")
    node_pairs = [_node_pair(node) for node in node_records]
    relevant = [_oracle_pair(item) for item in oracle.relevant_nodes]
    missing_nodes = [
        item for item in relevant if not any(_pair_matches(node, item) for node in node_pairs)
    ]
    if missing_nodes:
        failures.append(f"missing relevant nodes: {missing_nodes!r}")
    selected_direction = payload.get("selectedDirection")
    if selected_direction != oracle.expected_direction:
        failures.append(
            f"direction mismatch: expected {oracle.expected_direction}, got {selected_direction}"
        )
    ambiguous = any(bool(seed.get("ambiguous")) for seed in seed_records)
    if ambiguous != oracle.expected_ambiguous:
        failures.append(
            f"ambiguity mismatch: expected {oracle.expected_ambiguous}, got {ambiguous}"
        )
    no_match = any(
        isinstance(item, dict) and item.get("code") == "no_match" for item in diagnostics
    )
    no_match_false_positive = no_match and not oracle.allow_no_match
    if no_match_false_positive:
        failures.append("unexpected no_match diagnostic")
    if oracle.allow_no_match and not no_match:
        failures.append("expected no_match diagnostic")
    if oracle.allow_no_match and seed_records:
        failures.append("expected no_match response returned seeds")
    elif no_match and seed_records:
        failures.append("no_match response returned seeds")
    if not no_match and not seed_records:
        failures.append("empty result omitted the no_match diagnostic")
    for expected_edge in oracle.expected_edges:
        matching = [
            edge
            for edge in edge_records
            if nodes_by_id.get(str(edge.get("source")), {}).get("qualifiedName")
            == expected_edge.source
            and nodes_by_id.get(str(edge.get("target")), {}).get("qualifiedName")
            == expected_edge.target
            and edge.get("kind") == expected_edge.relation
        ]
        if expected_edge.site is not None:
            matching = [edge for edge in matching if _source_file({"source": edge.get("relationshipSite")}) == expected_edge.site]
        if not matching:
            failures.append(
                "missing expected edge: "
                f"{expected_edge.source} {expected_edge.relation} {expected_edge.target}"
            )
            continue
        seed_ids = {str(seed.get("nodeId", "")) for seed in seed_records}
        direction_matches = any(
            (expected_edge.direction == "outgoing" and str(edge.get("source")) in seed_ids)
            or (expected_edge.direction == "incoming" and str(edge.get("target")) in seed_ids)
            for edge in matching
        )
        if not direction_matches:
            failures.append(
                "expected edge direction mismatch: "
                f"{expected_edge.source} {expected_edge.relation} {expected_edge.target} "
                f"must be {expected_edge.direction} relative to a seed"
            )
    source_anchor_count = sum(bool(_source_file(node)) for node in node_records)
    top_one = bool(seed_pairs) and any(
        _pair_matches(seed_pairs[0], item) for item in expected + acceptable
    )
    if seed_records and (expected or acceptable) and not top_one:
        failures.append("top-ranked seed is neither expected nor acceptable")
    top_ten = seed_pairs[:10]
    relevant_hits_at_ten = sum(
        any(_pair_matches(seed, item) for seed in top_ten) for item in relevant
    )
    recall_at_ten = relevant_hits_at_ten / len(relevant) if relevant else 0.0
    reciprocal_rank_at_ten = 0.0
    for rank, seed in enumerate(seed_records[:10], 1):
        node = nodes_by_id.get(str(seed.get("nodeId", "")))
        if node is not None and any(_pair_matches(_node_pair(node), item) for item in relevant):
            reciprocal_rank_at_ten = 1.0 / rank
            break
    stats = payload.get("stats") if isinstance(payload.get("stats"), dict) else {}
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return CorrectnessResult(
        passed=not failures,
        digest=hashlib.sha256(canonical.encode("utf-8")).hexdigest(),
        failures=tuple(failures),
        metrics={
            "top1": top_one,
            "mrr_at_10": reciprocal_rank_at_ten,
            "recall_at_10": recall_at_ten,
            "direction_correct": selected_direction == oracle.expected_direction,
            "ambiguity_correct": ambiguous == oracle.expected_ambiguous,
            "source_anchor_count": source_anchor_count,
            "no_match_false_positive": no_match_false_positive,
            "candidate_nodes": int(stats.get("candidateNodes", 0)),
            "expanded_relationships": int(stats.get("expandedRelationships", 0)),
            "complete": not bool(payload.get("truncated")),
        },
    )


def validate_query_output(
    text: str, oracle: QueryOracle, *, tool: str = "cross-tool"
) -> CorrectnessResult:
    if tool == "compass":
        return _validate_compass_discovery(text, oracle)
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
            correctness = validate_query_output(output, oracle, tool=adapter.name)
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
                    evidence=correctness.metrics,
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
