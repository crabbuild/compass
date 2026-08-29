"""Correctness-gated build and query workload execution."""

from __future__ import annotations

import ast
from contextlib import contextmanager
from dataclasses import replace
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
from typing import Iterator

from .adapters import ToolAdapter
from .correctness import index_graph
from .model import (
    CorrectnessResult,
    QueryNodeOracle,
    QueryOracle,
    RepositorySpec,
    Sample,
    WorkloadResult,
)
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

_DISCOVERY_WORK_LIMITS = {
    "candidateProbes": 291,
    "candidateNodes": 12_801,
    "candidatesAdmitted": 256,
    "visitedNodes": 500,
    "expandedRelationships": 10_000,
    "returnedNodes": 500,
    "returnedEdges": 1_000,
}
_MAX_QUERY_OUTPUT_BYTES = 20 * 1024 * 1024
_GRAPHIFY_NODE = re.compile(
    r"^NODE (?P<label>.*?) \[src=(?P<source>.*?) loc=L(?P<line>[0-9]+)(?: |\])"
)
_GRAPHIFY_START = re.compile(r"\| Start: (?P<starts>\[.*\])(?: \| |$)")


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


def _read_query_output(path: Path) -> str:
    if path.stat().st_size > _MAX_QUERY_OUTPUT_BYTES:
        raise RuntimeError("query output exceeded the 20 MiB harness bound")
    return path.read_text(encoding="utf-8", errors="replace")


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
    legacy_digests = {sample.correctness_digest for sample in samples}
    legacy_reference = bool(samples) and len(legacy_digests) == 1 and "" not in legacy_digests and all(
        sample.evidence.get("legacy_semantic_digest") is True for sample in samples
    )
    if correctness.passed or legacy_reference:
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


def prepare_query_artifact(
    adapter: ToolAdapter,
    checkout: Path,
    artifact_root: Path,
    spec: RepositorySpec,
    *,
    reuse_root: Path | None = None,
    timeout_seconds: float = 1800,
) -> Path:
    """Materialize or validate one persistent query backend for a repository."""
    root = reuse_root if reuse_root is not None else artifact_root / "query-artifacts"
    output = root / adapter.name / spec.name
    if reuse_root is None:
        if output.exists():
            guarded_remove(output)
        logs = artifact_root / "logs" / adapter.name / spec.name / "query-materialization"
        logs.mkdir(parents=True, exist_ok=True)
        metrics = run_measured(
            ProcessSpec(
                command=adapter.query_artifact_command(checkout, output),
                cwd=checkout,
                stdout_path=logs / "1.out",
                stderr_path=logs / "1.err",
                timeout_seconds=timeout_seconds,
            )
        )
        if metrics.return_code != 0 or metrics.timed_out:
            detail = _read_query_output(Path(metrics.stderr_path)).strip()
            raise RuntimeError(
                f"query artifact materialization failed for {spec.name}: "
                f"{detail or f'return code {metrics.return_code}'}"
            )
    graph = adapter.graph_path(output)
    evidence = adapter.validate_query_artifact(graph, timeout_seconds=timeout_seconds)
    correctness = _validate_graph(adapter.name, graph)
    if not correctness.passed:
        raise RuntimeError("; ".join(correctness.failures))
    evidence.update(
        {
            "canonical_graph_digest": correctness.digest,
            **correctness.metrics,
            "graph_path": str(graph.resolve()),
            "read_only_reuse": str(reuse_root is not None).lower(),
        }
    )
    evidence_path = (
        artifact_root
        / "logs"
        / adapter.name
        / spec.name
        / "query-artifact-evidence.json"
    )
    evidence_path.parent.mkdir(parents=True, exist_ok=True)
    evidence_path.write_text(
        json.dumps(evidence, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    if reuse_root is None:
        adapter.prune_superseded_artifacts(output, graph)
    return graph


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


def _validate_compass_discovery(
    text: str, oracle: QueryOracle, *, allow_legacy_digest: bool = False
) -> CorrectnessResult:
    try:
        payload = json.loads(text)
    except json.JSONDecodeError as error:
        message = f"invalid Compass discovery JSON: {error}"
        return CorrectnessResult(False, hashlib.sha256(message.encode()).hexdigest(), (message,))
    if isinstance(payload, dict) and payload.get("schema") == "compass.query.discovery-result/1":
        envelope_digest = payload.get("semanticResultDigest")
        result = payload.get("result")
        if not isinstance(envelope_digest, str) or not isinstance(result, dict):
            message = "Compass discovery result envelope is malformed"
            return CorrectnessResult(
                False, hashlib.sha256(message.encode()).hexdigest(), (message,)
            )
        payload = dict(result)
        payload["__semanticResultDigest"] = envelope_digest
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
    ambiguous = bool(seed_records[0].get("ambiguous")) if seed_records else False
    if ambiguous != oracle.expected_ambiguous:
        failures.append(
            f"ambiguity mismatch: expected {oracle.expected_ambiguous}, got {ambiguous}"
        )
    no_match = any(
        isinstance(item, dict) and item.get("code") == "no_match" for item in diagnostics
    )
    bounded_truncation = any(
        isinstance(item, dict) and item.get("code") == "bounded_truncation"
        for item in diagnostics
    )
    truncated = payload.get("truncated") is True
    no_match_false_positive = no_match and not oracle.allow_no_match
    if no_match_false_positive:
        failures.append("unexpected no_match diagnostic")
    if oracle.allow_no_match and not no_match:
        failures.append("expected no_match diagnostic")
    if oracle.allow_no_match and seed_records:
        failures.append("expected no_match response returned seeds")
    elif no_match and seed_records:
        failures.append("no_match response returned seeds")
    if not no_match and not seed_records and not truncated:
        failures.append("empty result omitted the no_match diagnostic")
    if not seed_records and truncated and not bounded_truncation:
        failures.append("truncated empty result omitted the bounded_truncation diagnostic")
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
    work: dict[str, int] = {}
    for field, ceiling in _DISCOVERY_WORK_LIMITS.items():
        value = stats.get(field)
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            failures.append(f"discovery stats field {field} must be a nonnegative integer")
            continue
        work[field] = value
        if value > ceiling:
            failures.append(f"discovery stats field {field} exceeds {ceiling}: {value}")
    for field, observed in (("returnedNodes", len(node_records)), ("returnedEdges", len(edge_records))):
        if field in work and work[field] != observed:
            failures.append(
                f"discovery stats field {field} is {work[field]}, expected {observed}"
            )
    if "candidatesAdmitted" in work and work["candidatesAdmitted"] < len(seed_records):
        failures.append("candidatesAdmitted is smaller than the returned seed count")
    transport_digest = payload.pop("__semanticResultDigest", None)
    semantic_digest = transport_digest
    if semantic_digest is None and allow_legacy_digest:
        canonical = json.dumps(
            payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False
        ).encode("utf-8")
        semantic_digest = (
            "legacy-python-full-payload:sha256:" + hashlib.sha256(canonical).hexdigest()
        )
    legacy_digest = isinstance(semantic_digest, str) and semantic_digest.startswith(
        "legacy-python-full-payload:sha256:"
    )
    if legacy_digest:
        digest = semantic_digest.removeprefix("legacy-python-full-payload:sha256:")
    elif not isinstance(semantic_digest, str) or not semantic_digest.startswith("sha256:"):
        failures.append("query transport omitted the Rust-owned semantic result digest")
        digest = hashlib.sha256(b"missing semantic result digest").hexdigest()
    else:
        digest = semantic_digest.removeprefix("sha256:")
    if len(digest) != 64 or any(character not in "0123456789abcdef" for character in digest):
        failures.append("query transport emitted an invalid semantic result digest")
    return CorrectnessResult(
        passed=not failures,
        digest=digest,
        failures=tuple(failures),
        metrics={
            "top1": top_one,
            "mrr_at_10": reciprocal_rank_at_ten,
            "recall_at_10": recall_at_ten,
            "direction_correct": selected_direction == oracle.expected_direction,
            "ambiguity_correct": ambiguous == oracle.expected_ambiguous,
            "source_anchor_count": source_anchor_count,
            "no_match_false_positive": no_match_false_positive,
            "no_match_correct": oracle.allow_no_match and no_match and not seed_records,
            "independently_labeled": oracle.judgment_source is not None,
            "oracle_judgment_source": oracle.judgment_source or "unlabeled",
            **{_camel_to_snake(field): value for field, value in work.items()},
            "complete": not bool(payload.get("truncated")),
            "legacy_semantic_digest": legacy_digest,
        },
    )


def _camel_to_snake(value: str) -> str:
    return "".join(f"_{character.lower()}" if character.isupper() else character for character in value)


def validate_query_output(
    text: str,
    oracle: QueryOracle,
    *,
    tool: str = "cross-tool",
    allow_legacy_digest: bool = False,
) -> CorrectnessResult:
    if tool == "compass":
        return _validate_compass_discovery(
            text, oracle, allow_legacy_digest=allow_legacy_digest
        )
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
    graphify_nodes = sum(
        line.lstrip().startswith("NODE ") for line in text.splitlines()
    )
    observed_graphify_nodes = _graphify_nodes(text)
    observed_graphify_seeds = _graphify_seed_nodes(text, observed_graphify_nodes)
    expected_or_acceptable = oracle.expected_seeds + oracle.acceptable_seeds
    missing_graphify_seeds = [
        node
        for node in oracle.expected_seeds
        if not _graphify_node_matches(observed_graphify_seeds, node)
    ]
    if missing_graphify_seeds:
        failures.append(
            "missing independently labeled Graphify seeds: "
            f"{[node.qualified_name for node in missing_graphify_seeds]!r}"
        )
    if not oracle.expected_seeds and oracle.acceptable_seeds and not any(
        _graphify_node_matches(observed_graphify_seeds, node)
        for node in oracle.acceptable_seeds
    ):
        failures.append("no independently labeled acceptable Graphify seed was returned")
    returned_graphify_forbidden = [
        node
        for node in oracle.forbidden_seeds
        if _graphify_node_matches(observed_graphify_seeds, node)
    ]
    if returned_graphify_forbidden:
        failures.append(
            "independently labeled forbidden Graphify seed returned: "
            f"{[node.qualified_name for node in returned_graphify_forbidden]!r}"
        )
    top_one = bool(observed_graphify_seeds) and any(
        _graphify_node_matches(observed_graphify_seeds[:1], node)
        for node in expected_or_acceptable
    )
    if observed_graphify_seeds and expected_or_acceptable and not top_one:
        failures.append("top-ranked Graphify seed is neither expected nor acceptable")
    relevant_hits_at_ten = sum(
        _graphify_node_matches(observed_graphify_seeds[:10], node)
        for node in oracle.relevant_nodes
    )
    recall_at_ten = (
        relevant_hits_at_ten / len(oracle.relevant_nodes)
        if oracle.relevant_nodes
        else 0.0
    )
    reciprocal_rank_at_ten = 0.0
    for rank, seed in enumerate(observed_graphify_seeds[:10], 1):
        if any(_graphify_node_matches([seed], node) for node in oracle.relevant_nodes):
            reciprocal_rank_at_ten = 1.0 / rank
            break
    if oracle.allow_no_match and graphify_nodes:
        failures.append(
            f"independent no-match oracle returned {graphify_nodes} graph node(s)"
        )
    canonical = "\n".join(sorted(line.strip() for line in text.splitlines() if line.strip()))
    return CorrectnessResult(
        passed=not failures,
        digest=hashlib.sha256(canonical.encode("utf-8")).hexdigest(),
        failures=tuple(failures),
        metrics={
            "top1": top_one,
            "mrr_at_10": reciprocal_rank_at_ten,
            "recall_at_10": recall_at_ten,
            "no_match_correct": oracle.allow_no_match and graphify_nodes == 0,
            "independently_labeled": oracle.judgment_source is not None,
            "oracle_judgment_source": oracle.judgment_source or "unlabeled",
            "source_anchor_count": sum(bool(source) for _, source, _ in observed_graphify_nodes),
            "complete": "[!] TRUNCATED" not in text,
        },
    )


def _graphify_nodes(text: str) -> list[tuple[str, str, int]]:
    nodes = []
    for line in text.splitlines():
        matched = _GRAPHIFY_NODE.match(line.strip())
        if matched is not None:
            nodes.append(
                (
                    matched.group("label").removeprefix(".").removesuffix("()").casefold(),
                    matched.group("source").replace("\\", "/"),
                    int(matched.group("line")),
                )
            )
    return nodes


def _graphify_seed_nodes(
    text: str, observed: list[tuple[str, str, int]]
) -> list[tuple[str, str, int]]:
    for line in text.splitlines():
        matched = _GRAPHIFY_START.search(line.strip())
        if matched is None:
            continue
        try:
            starts = ast.literal_eval(matched.group("starts"))
        except (SyntaxError, ValueError):
            return []
        if not isinstance(starts, list) or not all(
            isinstance(start, str) for start in starts
        ):
            return []
        return observed[: len(starts)]
    return []


def _graphify_node_matches(
    observed: list[tuple[str, str, int]], oracle: QueryNodeOracle
) -> bool:
    terminal = re.split(r"::|[./]", oracle.qualified_name)[-1]
    expected_label = terminal.removeprefix(".").removesuffix("()").casefold()
    if oracle.source is None:
        return any(label == expected_label for label, _, _ in observed)
    expected_source = oracle.source.file.replace("\\", "/")
    return any(
        label == expected_label
        and source == expected_source
        and (oracle.source.start_line is None or line == oracle.source.start_line)
        for label, source, line in observed
    )


def _mcp_worker_command(
    adapter: ToolAdapter,
    graph: Path,
    questions: Path,
    output: Path,
    server_stderr: Path,
    batches: int,
    timeout_seconds: float,
    allow_legacy_digest: bool = False,
) -> tuple[str, ...]:
    command = (
        sys.executable,
        str(Path(__file__).with_name("mcp_query_session.py")),
        "--binary",
        str(adapter.executable),
        "--graph",
        str(graph),
        "--questions",
        str(questions),
        "--output",
        str(output),
        "--server-stderr",
        str(server_stderr),
        "--batches",
        str(batches),
        "--timeout-seconds",
        str(timeout_seconds),
    )
    return (*command, "--allow-legacy-digest") if allow_legacy_digest else command


def _mcp_records(
    metrics,
    output_root: Path,
    *,
    query_count: int,
    batches: int,
) -> list[dict[str, object]]:
    if metrics.return_code != 0 or metrics.timed_out:
        detail_path = Path(metrics.stderr_path)
        detail = (
            _read_query_output(detail_path)
            if detail_path.is_file()
            else "worker stderr unavailable"
        )
        raise RuntimeError(f"MCP query worker failed: {detail.strip()}")
    value = json.loads(_read_query_output(Path(metrics.stdout_path)))
    if not isinstance(value, dict) or value.get("schema") != "compass.performance.mcp-query-session/1":
        raise RuntimeError("MCP query worker returned an unsupported session schema")
    records = value.get("records")
    if not isinstance(records, list) or not all(isinstance(item, dict) for item in records):
        raise RuntimeError("MCP query worker returned malformed records")
    expected_pairs = {
        (query_index, iteration)
        for query_index in range(1, query_count + 1)
        for iteration in range(batches + 1)
    }
    observed_pairs: set[tuple[int, int]] = set()
    resolved_root = output_root.resolve()
    for record in records:
        if record.get("schema") != "compass.performance.mcp-query-session-record/1":
            raise RuntimeError("MCP query worker returned an unsupported record schema")
        query_index = record.get("query_index")
        iteration = record.get("iteration")
        wall_seconds = record.get("wall_seconds")
        peak_rss_kib = record.get("peak_rss_kib")
        if (
            not isinstance(query_index, int)
            or isinstance(query_index, bool)
            or not isinstance(iteration, int)
            or isinstance(iteration, bool)
        ):
            raise RuntimeError("MCP query worker returned invalid record coordinates")
        pair = (query_index, iteration)
        if pair in observed_pairs:
            raise RuntimeError("MCP query worker returned duplicate record coordinates")
        observed_pairs.add(pair)
        if (
            not isinstance(wall_seconds, (int, float))
            or isinstance(wall_seconds, bool)
            or not math.isfinite(float(wall_seconds))
            or float(wall_seconds) < 0
        ):
            raise RuntimeError("MCP query worker returned invalid wall time")
        if (
            not isinstance(peak_rss_kib, int)
            or isinstance(peak_rss_kib, bool)
            or peak_rss_kib < 0
        ):
            raise RuntimeError("MCP query worker returned invalid peak RSS")
        output = Path(str(record.get("output", ""))).resolve()
        if not output.is_relative_to(resolved_root) or not output.is_file():
            raise RuntimeError("MCP query worker record escaped its output directory")
        if output.stat().st_size > 20 * 1024 * 1024:
            raise RuntimeError("MCP query worker response exceeded the 20 MiB bound")
    if observed_pairs != expected_pairs:
        raise RuntimeError("MCP query worker returned incomplete record coordinates")
    return records


def _record_metrics(session_metrics, record: dict[str, object], *, fresh: bool):
    output = Path(str(record["output"]))
    wall = session_metrics.wall_seconds if fresh else float(record["wall_seconds"])
    return replace(
        session_metrics,
        wall_seconds=wall,
        user_seconds=session_metrics.user_seconds if fresh else 0.0,
        system_seconds=session_metrics.system_seconds if fresh else 0.0,
        peak_rss_kib=(
            session_metrics.peak_rss_kib
            if fresh
            else int(record["peak_rss_kib"])
        ),
        stdout_path=str(output),
        stdout_sha256=_file_sha256(output),
    )


def _append_query_sample(
    samples: list[Sample],
    failures: list[str],
    adapter: ToolAdapter,
    spec: RepositorySpec,
    workload: str,
    iteration: int,
    metrics,
    oracle: QueryOracle,
    artifact_evidence: dict[str, str],
    *,
    allow_legacy_digest: bool = False,
) -> str:
    output = _read_query_output(Path(metrics.stdout_path))
    correctness = validate_query_output(
        output,
        oracle,
        tool=adapter.name,
        allow_legacy_digest=allow_legacy_digest,
    )
    errors: list[str] = []
    if metrics.timed_out:
        errors.append("query timed out")
    elif metrics.return_code != 0:
        errors.append(f"query failed with return code {metrics.return_code}")
    if not correctness.passed and not allow_legacy_digest:
        errors.extend(correctness.failures)
    error = "; ".join(errors) if errors else None
    if error:
        failures.append(f"{workload}[{iteration}]: {error}")
    elif not correctness.passed:
        failures.append(
            f"{workload}[{iteration}]: {'; '.join(correctness.failures)}"
        )
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
            evidence={**correctness.metrics, **artifact_evidence},
        )
    )
    return correctness.digest


def run_compass_mcp_query_matrix(
    adapter: ToolAdapter,
    graph: Path,
    artifact_root: Path,
    spec: RepositorySpec,
    *,
    batches: int,
    timeout_seconds: float,
    allow_legacy_digest: bool = False,
) -> tuple[WorkloadResult, ...]:
    root = artifact_root / "logs" / adapter.name / spec.name / "mcp"
    root.mkdir(parents=True, exist_ok=True)
    results: list[WorkloadResult] = []
    expected_digests: dict[int, str] = {}
    artifact_evidence_value = json.loads(
        (root.parent / "query-artifact-evidence.json").read_text(encoding="utf-8")
    )
    if not isinstance(artifact_evidence_value, dict):
        raise RuntimeError("query artifact evidence must be an object")
    artifact_evidence = {
        str(key): str(value) for key, value in artifact_evidence_value.items()
    }
    for query_index, oracle in enumerate(spec.queries, 1):
        workload = f"query-{query_index}-fresh"
        samples: list[Sample] = []
        failures: list[str] = []
        for iteration in range(batches + 1):
            run_root = root / workload / str(iteration)
            metrics = run_measured(
                ProcessSpec(
                    command=(
                        adapter.query_command(graph, oracle.question)
                        if allow_legacy_digest
                        else (*adapter.query_command(graph, oracle.question), "--result-envelope")
                    ),
                    cwd=graph.parent,
                    stdout_path=run_root / "query.out",
                    stderr_path=run_root / "query.err",
                    timeout_seconds=timeout_seconds,
                )
            )
            if iteration == 0:
                warmup = validate_query_output(
                    _read_query_output(Path(metrics.stdout_path)),
                    oracle,
                    tool=adapter.name,
                    allow_legacy_digest=allow_legacy_digest,
                )
                warmup_failures = list(warmup.failures)
                if metrics.timed_out:
                    warmup_failures.insert(0, "query timed out")
                elif metrics.return_code != 0:
                    warmup_failures.insert(
                        0, f"query failed with return code {metrics.return_code}"
                    )
                if warmup_failures:
                    failures.append(f"{workload}[warmup]: {'; '.join(warmup_failures)}")
                expected_digests[query_index] = warmup.digest
                continue
            digest = _append_query_sample(
                samples,
                failures,
                adapter,
                spec,
                workload,
                iteration,
                metrics,
                oracle,
                artifact_evidence,
                allow_legacy_digest=allow_legacy_digest,
            )
            if digest != expected_digests[query_index]:
                failures.append(f"{workload}[{iteration}]: semantic digest changed")
                samples[-1] = replace(
                    samples[-1], eligible=False, error="semantic digest changed"
                )
        results.append(_result(adapter.name, spec.name, workload, samples, failures))

    questions = root / "warm-questions.json"
    questions.write_text(
        json.dumps([oracle.question for oracle in spec.queries]), encoding="utf-8"
    )
    session_metrics = run_measured(
        ProcessSpec(
            command=_mcp_worker_command(
                adapter,
                graph,
                questions,
                root / "warm-responses",
                root / "warm-server.err",
                batches,
                timeout_seconds,
                allow_legacy_digest,
            ),
            cwd=graph.parent,
            stdout_path=root / "warm-worker.out",
            stderr_path=root / "warm-worker.err",
            timeout_seconds=(batches + 1) * len(spec.queries) * timeout_seconds + 30,
        )
    )
    try:
        records = _mcp_records(
            session_metrics,
            root / "warm-responses",
            query_count=len(spec.queries),
            batches=batches,
        )
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as error:
        if not allow_legacy_digest:
            raise RuntimeError("current persistent MCP warm session failed") from error
        limitation = f"persistent MCP warm session unavailable: {error}"
        for query_index, _oracle in enumerate(spec.queries, 1):
            workload = f"query-{query_index}-warm"
            results.append(_result(adapter.name, spec.name, workload, [], [limitation]))
        return tuple(results)
    for query_index, oracle in enumerate(spec.queries, 1):
        workload = f"query-{query_index}-warm"
        samples = []
        failures = []
        selected = [
            record for record in records if int(record.get("query_index", 0)) == query_index
        ]
        for record in selected:
            iteration = int(record["iteration"])
            metrics = _record_metrics(session_metrics, record, fresh=False)
            correctness = validate_query_output(
                _read_query_output(Path(metrics.stdout_path)),
                oracle,
                tool=adapter.name,
                allow_legacy_digest=allow_legacy_digest,
            )
            if correctness.digest != expected_digests[query_index]:
                failures.append(f"{workload}[{iteration}]: fresh/warm semantic digest mismatch")
            if iteration == 0:
                if not correctness.passed:
                    failures.append(
                        f"{workload}[warmup]: {'; '.join(correctness.failures)}"
                    )
                continue
            _append_query_sample(
                samples,
                failures,
                adapter,
                spec,
                workload,
                iteration,
                metrics,
                oracle,
                artifact_evidence,
                allow_legacy_digest=allow_legacy_digest,
            )
            if correctness.digest != expected_digests[query_index]:
                samples[-1] = replace(
                    samples[-1],
                    eligible=False,
                    error="fresh/warm semantic digest mismatch",
                )
        results.append(_result(adapter.name, spec.name, workload, samples, failures))
    return tuple(results)


def run_query_matrix(
    adapter: ToolAdapter,
    graph: Path,
    artifact_root: Path,
    spec: RepositorySpec,
    *,
    batches: int = 10,
    timeout_seconds: float = 120,
    allow_legacy_digest: bool = False,
) -> tuple[WorkloadResult, ...]:
    if batches < 10:
        raise ValueError("query qualification requires at least ten batches")
    if adapter.supports_persistent_queries:
        return run_compass_mcp_query_matrix(
            adapter,
            graph,
            artifact_root,
            spec,
            batches=batches,
            timeout_seconds=timeout_seconds,
            allow_legacy_digest=allow_legacy_digest,
        )
    results: list[WorkloadResult] = []
    for query_index, oracle in enumerate(spec.queries):
        workload = f"query-{query_index + 1}-fresh"
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
            output = _read_query_output(Path(metrics.stdout_path))
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
            output = _read_query_output(Path(metrics.stdout_path))
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
