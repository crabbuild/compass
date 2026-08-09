#!/usr/bin/env python3
"""Correctness-first Compass performance qualification."""

from __future__ import annotations

import argparse
from dataclasses import replace
from datetime import datetime, timezone
import hashlib
import json
import math
import multiprocessing
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import sqlite3
import subprocess
import sys
from typing import Sequence

if __package__ in {None, ""}:
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from benchmarks.performance.compass import RUN_SCHEMA
from benchmarks.performance.compass.adapters import CompassAdapter, GraphifyAdapter
from benchmarks.performance.compass.audit import (
    audit_result_json_value,
    export_comparison_candidates,
    run_audit,
)
from benchmarks.performance.compass.typescript_scorecard import (
    scorecard_result,
    write_scorecard_result,
)
from benchmarks.performance.compass.config import load_suite
from benchmarks.performance.compass.correctness import compare_graphs, index_graph
from benchmarks.performance.compass.model import (
    EnvironmentIdentity,
    GateIssue,
    GateReport,
    QualificationRun,
    RepositorySpec,
)
from benchmarks.performance.compass.report import (
    compare_baseline,
    compare_tools,
    load_run,
    promote_baseline,
    render_markdown,
    write_run,
)
from benchmarks.performance.compass.workloads import (
    run_build_matrix,
    run_compassql_matrix,
    run_query_matrix,
)
from benchmarks.performance.compass.workspace import (
    QualificationWorkspace,
    prepare_checkout,
    resolve_remote_head,
)

SOURCE_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SUITE = SOURCE_ROOT / "benchmarks" / "performance" / "repositories.toml"
DEFAULT_WORKSPACE = SOURCE_ROOT / "target" / "performance" / "workspace"
DEFAULT_RUNS = SOURCE_ROOT / "target" / "performance" / "runs"
FULL_SUITE_DISK_BYTES = 100 * 1024**3
SELECTED_REPOSITORY_DISK_BYTES = 5 * 1024**3
_OBJECT_ID = re.compile(r"^[0-9a-fA-F]{40}$")


def _command(arguments: Sequence[str], cwd: Path = SOURCE_ROOT) -> str:
    completed = subprocess.run(
        arguments,
        cwd=cwd,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"{' '.join(arguments)} failed: {detail}")
    return completed.stdout.strip()


def _sysctl(name: str) -> str:
    return _command(("sysctl", "-n", name))


def _cpu_model() -> str:
    if platform.system() == "Darwin":
        return _sysctl("machdep.cpu.brand_string")
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.is_file():
        for line in cpuinfo.read_text(encoding="utf-8", errors="replace").splitlines():
            if line.lower().startswith("model name"):
                return line.split(":", 1)[1].strip()
    return platform.processor() or "unknown"


def _physical_cores() -> int:
    if platform.system() == "Darwin":
        return int(_sysctl("hw.physicalcpu"))
    return os.cpu_count() or 1


def _total_memory() -> int:
    if platform.system() == "Darwin":
        return int(_sysctl("hw.memsize"))
    meminfo = Path("/proc/meminfo")
    if meminfo.is_file():
        for line in meminfo.read_text(encoding="utf-8").splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) * 1024
    return 0


def environment_identity() -> EnvironmentIdentity:
    host = socket.gethostname()
    cpu = _cpu_model()
    memory = _total_memory()
    runner_payload = "\0".join(
        (platform.system(), platform.release(), platform.machine(), cpu, str(memory), host)
    )
    runner_id = hashlib.sha256(runner_payload.encode("utf-8")).hexdigest()[:16]
    return EnvironmentIdentity(
        system=platform.system(),
        release=platform.release(),
        architecture=platform.machine(),
        cpu_model=cpu,
        physical_cores=_physical_cores(),
        logical_cores=os.cpu_count() or 1,
        total_memory_bytes=memory,
        python_version=platform.python_version(),
        rust_version=_command(("rustc", "--version")),
        cargo_version=_command(("cargo", "--version")),
        hostname=host,
        runner_id=runner_id,
    )


def _selected(suite_path: Path, names: Sequence[str]):
    suite = load_suite(suite_path)
    if not names:
        return suite, suite.repositories
    requested = set(names)
    selected = tuple(item for item in suite.repositories if item.name in requested)
    missing = requested - {item.name for item in selected}
    if missing:
        raise ValueError(f"unknown repositories: {', '.join(sorted(missing))}")
    return suite, selected


def requested_repository_commits(
    values: Sequence[str],
    selected: Sequence[RepositorySpec],
) -> dict[str, str]:
    selected_names = {repository.name for repository in selected}
    commits: dict[str, str] = {}
    for value in values:
        name, separator, commit = value.partition("=")
        if not separator or not name or _OBJECT_ID.fullmatch(commit) is None:
            raise ValueError(
                f"invalid repository commit override {value!r}; expected NAME=40_HEX_SHA"
            )
        if name not in selected_names:
            raise ValueError(f"repository commit override is not selected: {name}")
        if name in commits:
            raise ValueError(f"duplicate repository commit override: {name}")
        commits[name] = commit.lower()
    return commits


def _existing_ancestor(path: Path) -> Path:
    candidate = path.resolve(strict=False)
    while not candidate.exists():
        if candidate.parent == candidate:
            raise ValueError(f"no existing ancestor for {path}")
        candidate = candidate.parent
    return candidate


def doctor(args: argparse.Namespace) -> int:
    checks: list[dict[str, object]] = []

    def check(name: str, action) -> None:
        try:
            detail = action()
            checks.append({"name": name, "passed": True, "detail": str(detail)})
        except Exception as error:
            checks.append({"name": name, "passed": False, "detail": str(error)})

    check(
        "python",
        lambda: platform.python_version()
        if sys.version_info >= (3, 11)
        else (_ for _ in ()).throw(RuntimeError("Python 3.11 or newer is required")),
    )
    check("git", lambda: _command(("git", "--version")))
    check("rustc", lambda: _command(("rustc", "--version")))
    check("cargo", lambda: _command(("cargo", "--version")))
    check(
        "platform",
        lambda: platform.system()
        if platform.system() in {"Darwin", "Linux"}
        else (_ for _ in ()).throw(RuntimeError("peak RSS requires macOS or Linux")),
    )
    required_disk = (
        SELECTED_REPOSITORY_DISK_BYTES if args.repository else FULL_SUITE_DISK_BYTES
    )
    disk_anchor = _existing_ancestor(args.workspace)
    check(
        "disk",
        lambda: f"{shutil.disk_usage(disk_anchor).free} bytes free"
        if shutil.disk_usage(disk_anchor).free >= required_disk
        else (_ for _ in ()).throw(
            RuntimeError(
                f"{required_disk} bytes required; "
                f"{shutil.disk_usage(disk_anchor).free} available"
            )
        ),
    )
    check(
        "compass-worktree",
        lambda: "clean"
        if not _command(
            ("git", "status", "--porcelain=v1", "--untracked-files=all"), args.source_root
        )
        else (_ for _ in ()).throw(RuntimeError("Compass source checkout is dirty")),
    )
    workspace = QualificationWorkspace.create(args.workspace)
    check(
        "workspace-lock",
        lambda: "available"
        if not workspace.lock.exists()
        else (_ for _ in ()).throw(RuntimeError(f"lock exists: {workspace.lock}")),
    )
    try:
        _, repositories = _selected(args.suite, args.repository)
    except Exception as error:
        checks.append({"name": "suite", "passed": False, "detail": str(error)})
        repositories = ()
    else:
        checks.append(
            {"name": "suite", "passed": True, "detail": f"{len(repositories)} repositories"}
        )
    if not args.skip_network:
        for repository in repositories:
            check(
                f"remote:{repository.name}",
                lambda repository=repository: resolve_remote_head(repository.url)[1],
            )
    payload = {"passed": all(item["passed"] for item in checks), "checks": checks}
    print(json.dumps(payload, sort_keys=True, indent=2))
    return 0 if payload["passed"] else 1


def prepare(args: argparse.Namespace) -> int:
    suite, repositories = _selected(args.suite, args.repository)
    workspace = QualificationWorkspace.create(args.workspace)
    identities = []
    with workspace.acquire():
        for repository in repositories:
            commit = repository.commit
            identities.append(
                prepare_checkout(
                    repository,
                    commit,
                    workspace.root / "corpora" / repository.name,
                    pinned=True,
                )
            )
    payload = {
        "schema": "compass.performance-preparation/1",
        "suite_digest": suite.digest,
        "corpora": [identity.__dict__ for identity in identities],
    }
    destination = workspace.root / "preparation.json"
    destination.write_text(
        json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    print(destination)
    return 0


def _correctness_gate(results) -> GateReport:
    issues = [
        GateIssue(
            "correctness-failure",
            result.repository,
            result.workload,
            "; ".join(result.correctness.failures) or "workload was ineligible",
        )
        for result in results
        if not result.correctness.passed or result.aggregate is None
    ]
    return GateReport(not issues, tuple(issues))


def _merge_gates(*reports: GateReport) -> GateReport:
    issues = tuple(issue for report in reports for issue in report.issues)
    ratios = {
        key: value for report in reports for key, value in report.ratios.items()
    }
    return GateReport(not issues, issues, ratios)


def _shared_graph_gate_child(
    connection,
    compass_graph: Path,
    graphify_graph: Path,
    source_root: Path,
) -> None:
    try:
        database = sqlite3.connect(":memory:")
        try:
            index_graph("compass", compass_graph, database)
            index_graph("graphify", graphify_graph, database)
            comparison = compare_graphs(database, source_root)
        finally:
            database.close()
        connection.send({"passed": comparison.passed, "failures": comparison.failures})
    except (OSError, RuntimeError, ValueError, sqlite3.Error) as error:
        connection.send({"error": str(error)})
    finally:
        connection.close()


def _shared_graph_gate(
    compass_graph: Path,
    graphify_graph: Path,
    repository: str,
    source_root: Path,
    timeout_seconds: float,
) -> GateReport:
    if not math.isfinite(timeout_seconds) or timeout_seconds <= 0:
        raise ValueError("graph comparison timeout must be finite and positive")
    context = multiprocessing.get_context("spawn")
    parent, child = context.Pipe(duplex=False)
    process = context.Process(
        target=_shared_graph_gate_child,
        args=(child, compass_graph, graphify_graph, source_root),
    )
    process.start()
    child.close()
    try:
        if not parent.poll(timeout_seconds):
            process.terminate()
            process.join()
            issues = (
                GateIssue(
                    "graph-comparison-timeout",
                    repository,
                    "cold",
                    f"graph comparator exceeded {timeout_seconds:g}s",
                ),
            )
            return GateReport(False, issues)
        payload = parent.recv()
    finally:
        parent.close()
        if process.is_alive():
            process.terminate()
        process.join()
    if "error" in payload:
        issues = (
            GateIssue(
                "graph-comparison-failure",
                repository,
                "cold",
                str(payload["error"]),
            ),
        )
        return GateReport(False, issues)
    issues = tuple(
        GateIssue("graph-quality", repository, "cold", failure)
        for failure in payload.get("failures", ())
    )
    return GateReport(not issues, issues)


def qualify(args: argparse.Namespace, *, comparison: bool) -> int:
    suite, repositories = _selected(args.suite, args.repository)
    repository_commits = requested_repository_commits(
        args.repository_commit,
        repositories,
    )
    graphify_commit = getattr(args, "graphify_commit", None)
    if graphify_commit is not None and _OBJECT_ID.fullmatch(graphify_commit) is None:
        raise ValueError("--graphify-commit must be a 40-character hexadecimal SHA")
    workspace = QualificationWorkspace.create(args.workspace)
    started = datetime.now(timezone.utc)
    run_id = args.run_id or started.strftime("%Y%m%dT%H%M%SZ")
    output = args.output or DEFAULT_RUNS / run_id
    artifact_root = workspace.root / "artifacts" / run_id
    results = []
    corpora = []
    shared_gates: list[GateReport] = []
    with workspace.acquire():
        compass = CompassAdapter.prepare(args.source_root)
        graphify = (
            GraphifyAdapter.prepare(workspace, commit=graphify_commit)
            if comparison
            else None
        )
        for repository in repositories:
            pinned_commit = repository_commits.get(repository.name, repository.commit)
            commit = pinned_commit
            identity = prepare_checkout(
                repository,
                commit,
                workspace.root / "corpora" / repository.name,
                pinned=True,
            )
            corpora.append(identity)
            checkout = Path(identity.path)
            compass_builds = run_build_matrix(
                compass,
                checkout,
                artifact_root,
                repository,
                repeats=args.build_repeats,
                timeout_seconds=args.build_timeout,
            )
            results.extend(compass_builds)
            compass_graph = compass.graph_path(
                artifact_root / "compass" / repository.name
            )
            if args.workload in {"all", "query"}:
                results.extend(
                    run_query_matrix(
                        compass,
                        compass_graph,
                        artifact_root,
                        repository,
                        batches=args.query_batches,
                        timeout_seconds=args.query_timeout,
                    )
                )
            if args.workload in {"all", "compassql"}:
                results.extend(
                    run_compassql_matrix(
                        compass,
                        compass_graph,
                        artifact_root,
                        repository,
                        batches=args.query_batches,
                        timeout_seconds=args.query_timeout,
                    )
                )
            if graphify is not None:
                results.extend(
                    run_build_matrix(
                        graphify,
                        checkout,
                        artifact_root,
                        repository,
                        repeats=args.build_repeats,
                        timeout_seconds=args.build_timeout,
                    )
                )
                graphify_graph = graphify.graph_path(
                    artifact_root / "graphify" / repository.name
                )
                if args.workload in {"all", "query"}:
                    results.extend(
                        run_query_matrix(
                            graphify,
                            graphify_graph,
                            artifact_root,
                            repository,
                            batches=args.query_batches,
                            timeout_seconds=args.query_timeout,
                        )
                    )
                shared_gates.append(
                    _shared_graph_gate(
                        compass_graph,
                        graphify_graph,
                        repository.name,
                        checkout,
                        args.graph_comparison_timeout,
                    )
                )
    tools = (compass.revision,) if graphify is None else (compass.revision, graphify.revision)
    completed = datetime.now(timezone.utc)
    provisional = QualificationRun(
        schema=RUN_SCHEMA,
        run_id=run_id,
        started_at=started.isoformat(),
        completed_at=completed.isoformat(),
        complete=True,
        suite_digest=suite.digest,
        environment=environment_identity(),
        tools=tools,
        corpora=tuple(corpora),
        results=tuple(results),
    )
    gates = [_correctness_gate(results), *shared_gates]
    if comparison:
        gates.append(compare_tools(results))
    baseline_path = args.baseline
    if baseline_path is None:
        candidate = (
            SOURCE_ROOT
            / "benchmarks"
            / "performance"
            / "baselines"
            / provisional.environment.runner_id
            / "baseline.json"
        )
        baseline_path = candidate if candidate.is_file() else None
    if baseline_path is not None:
        gates.append(compare_baseline(provisional, load_run(baseline_path)))
    finished = replace(provisional, gates=_merge_gates(*gates))
    run_path, summary_path = write_run(finished, output)
    print(run_path)
    print(summary_path)
    return 0 if finished.gates and finished.gates.passed else 1


def regenerate_report(args: argparse.Namespace) -> int:
    run = load_run(args.run)
    destination = args.output or args.run.with_name("summary.md")
    destination.write_text(render_markdown(run), encoding="utf-8")
    print(destination)
    return 0


def promote(args: argparse.Namespace) -> int:
    run = load_run(args.run)
    destination = args.destination
    if destination is None:
        destination = (
            SOURCE_ROOT
            / "benchmarks"
            / "performance"
            / "baselines"
            / run.environment.runner_id
            / "baseline.json"
        )
    print(promote_baseline(args.run, destination))
    return 0


def audit(args: argparse.Namespace) -> int:
    result = run_audit(args.manifest, args.graph, args.corpus)
    print(
        json.dumps(
            audit_result_json_value(result),
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
        )
    )
    return 0 if result.passed else 1


def audit_candidates(args: argparse.Namespace) -> int:
    destination = export_comparison_candidates(
        args.database,
        args.graph,
        args.corpus,
        args.name,
        args.adapter,
        args.output,
    )
    print(destination)
    return 0


def typescript_scorecard(args: argparse.Namespace) -> int:
    if args.output is None:
        result = scorecard_result(args.scorecard)
        print(
            json.dumps(
                result,
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=False,
            )
        )
    else:
        result = write_scorecard_result(args.scorecard, args.output)
        print(json.dumps(result, sort_keys=True, separators=(",", ":")))
        print(args.output)
    return 0 if result["passed"] else 1


def _common(parser: argparse.ArgumentParser, *, execution: bool = False) -> None:
    parser.add_argument("--suite", type=Path, default=DEFAULT_SUITE)
    parser.add_argument("--workspace", type=Path, default=DEFAULT_WORKSPACE)
    parser.add_argument("--source-root", type=Path, default=SOURCE_ROOT)
    parser.add_argument("--repository", action="append", default=[])
    if execution:
        parser.add_argument("--output", type=Path)
        parser.add_argument("--run-id")
        parser.add_argument(
            "--workload", choices=("all", "build", "query", "compassql"), default="all"
        )
        parser.add_argument("--build-repeats", type=int, default=3)
        parser.add_argument("--query-batches", type=int, default=10)
        parser.add_argument("--build-timeout", type=float, default=1800)
        parser.add_argument("--graph-comparison-timeout", type=float, default=600)
        parser.add_argument("--query-timeout", type=float, default=120)
        parser.add_argument("--baseline", type=Path)
        parser.add_argument(
            "--repository-commit",
            action="append",
            default=[],
            metavar="NAME=SHA",
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    doctor_parser = subparsers.add_parser("doctor")
    _common(doctor_parser)
    doctor_parser.add_argument("--skip-network", action="store_true")
    prepare_parser = subparsers.add_parser("prepare")
    _common(prepare_parser)
    for name in ("run", "compare"):
        execution_parser = subparsers.add_parser(name)
        _common(execution_parser, execution=True)
        if name == "compare":
            execution_parser.add_argument("--graphify-commit")
    report_parser = subparsers.add_parser("report")
    report_parser.add_argument("run", type=Path)
    report_parser.add_argument("--output", type=Path)
    promote_parser = subparsers.add_parser("promote")
    promote_parser.add_argument("run", type=Path)
    promote_parser.add_argument("--destination", type=Path)
    audit_parser = subparsers.add_parser("audit")
    audit_parser.add_argument("--manifest", type=Path, required=True)
    audit_parser.add_argument("--graph", type=Path, required=True)
    audit_parser.add_argument("--corpus", type=Path, required=True)
    candidate_parser = subparsers.add_parser("audit-candidates")
    candidate_parser.add_argument("--database", type=Path, required=True)
    candidate_parser.add_argument("--graph", type=Path, required=True)
    candidate_parser.add_argument("--corpus", type=Path, required=True)
    candidate_parser.add_argument("--name", required=True)
    candidate_parser.add_argument("--adapter", required=True)
    candidate_parser.add_argument("--output", type=Path, required=True)
    scorecard_parser = subparsers.add_parser(
        "typescript-scorecard",
        help="evaluate an explicitly adjudicated TypeScript/JavaScript scorecard",
    )
    scorecard_parser.add_argument("--scorecard", type=Path, required=True)
    scorecard_parser.add_argument("--output", type=Path)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if hasattr(args, "build_repeats") and args.build_repeats < 3:
        parser.error("--build-repeats must be at least 3")
    if hasattr(args, "query_batches") and args.query_batches < 10:
        parser.error("--query-batches must be at least 10")
    try:
        if args.command == "doctor":
            return doctor(args)
        if args.command == "prepare":
            return prepare(args)
        if args.command == "run":
            return qualify(args, comparison=False)
        if args.command == "compare":
            return qualify(args, comparison=True)
        if args.command == "report":
            return regenerate_report(args)
        if args.command == "promote":
            return promote(args)
        if args.command == "audit":
            return audit(args)
        if args.command == "audit-candidates":
            return audit_candidates(args)
        if args.command == "typescript-scorecard":
            return typescript_scorecard(args)
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    parser.error(f"unknown command: {args.command}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
