#!/usr/bin/env python3
"""Measure the current Compass CLI against a materialized qualification graph."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import zlib

if __package__ in {None, ""}:
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from benchmarks.performance.compass.process import ProcessSpec, run_measured
from benchmarks.qualification.io import atomic_write_text

SCHEMA = "compass.qualification-current-engine-baseline/1"
RAW_TRAVERSAL_SCHEMA = "compass.qualification-raw-traversal/1"
MAX_BINARY_BYTES = 512 * 1024 * 1024
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
GIT_OID_PATTERN = re.compile(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$")
MAX_PATCH_BYTES = 64 * 1024 * 1024
PATCH_TIMEOUT_SECONDS = 30.0
MAX_METADATA_OUTPUT_BYTES = 16 * 1024
MAX_WORKLOAD_OUTPUT_BYTES = 64 * 1024 * 1024
MAX_SOURCE_FILES = 100_000
MAX_SOURCE_BYTES = 512 * 1024 * 1024
MAX_SOURCE_FILE_BYTES = 64 * 1024 * 1024
DEFAULT_PATCH_PATHS = (
    ".cargo/config.toml",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "crates",
    "vendor/compass-tree-sitter-language-pack",
    "vendor/gemm-common",
)


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def compressed_size(path: Path) -> int:
    if path.stat().st_size > MAX_BINARY_BYTES:
        raise ValueError(f"binary exceeds compression limit {MAX_BINARY_BYTES}")
    sink = _CountingSink()
    with path.open("rb") as source, gzip.GzipFile(
        filename="", mode="wb", compresslevel=9, fileobj=sink, mtime=0
    ) as compressor:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            compressor.write(chunk)
    return sink.bytes_written


class _CountingSink:
    def __init__(self) -> None:
        self.bytes_written = 0

    def write(self, value: bytes) -> int:
        self.bytes_written += len(value)
        return len(value)

    def flush(self) -> None:
        return None


def percentile95(values: list[int]) -> int:
    if len(values) < 5:
        raise ValueError("p95 requires at least five samples")
    ordered = sorted(values)
    return ordered[max(0, math.ceil(0.95 * len(ordered)) - 1)]


def repository_relative(path: Path, repository_root: Path, *, label: str) -> Path:
    try:
        return path.relative_to(repository_root)
    except ValueError as error:
        raise ValueError(f"{label} must be inside repository root {repository_root}") from error


def validated_sha256(value: str, *, label: str) -> str:
    if not SHA256_PATTERN.fullmatch(value):
        raise ValueError(f"{label} must be a lowercase SHA-256 digest")
    return value


def validated_git_oid(value: str) -> str:
    if not GIT_OID_PATTERN.fullmatch(value):
        raise ValueError("git HEAD must be a lowercase 40- or 64-hex object ID")
    return value


def bounded_capture(
    command: tuple[str, ...],
    *,
    repository_root: Path,
    work_dir: Path,
    name: str,
    timeout_seconds: float,
    max_output_bytes: int,
) -> Path:
    stdout_path = work_dir / "metadata" / f"{name}.out"
    stderr_path = work_dir / "metadata" / f"{name}.err"
    stdout_path.parent.mkdir(parents=True, exist_ok=True)
    metrics = run_measured(
        ProcessSpec(
            command=command,
            cwd=repository_root,
            stdout_path=stdout_path,
            stderr_path=stderr_path,
            timeout_seconds=timeout_seconds,
        )
    )
    if metrics.return_code != 0 or metrics.timed_out or metrics.signal is not None:
        raise RuntimeError(
            f"{name} failed: return={metrics.return_code} "
            f"signal={metrics.signal} timed_out={metrics.timed_out}"
        )
    for label, path in (("stdout", stdout_path), ("stderr", stderr_path)):
        size = path.stat().st_size
        if size > max_output_bytes:
            raise ValueError(
                f"{name} {label} is {size} bytes; maximum is {max_output_bytes}"
            )
    return stdout_path


def bounded_text_command(
    command: tuple[str, ...],
    *,
    repository_root: Path,
    work_dir: Path,
    name: str,
    timeout_seconds: float,
) -> str:
    stdout_path = bounded_capture(
        command,
        repository_root=repository_root,
        work_dir=work_dir,
        name=name,
        timeout_seconds=timeout_seconds,
        max_output_bytes=MAX_METADATA_OUTPUT_BYTES,
    )
    return stdout_path.read_text(encoding="utf-8").strip()


def workspace_patch_metadata(
    repository_root: Path,
    paths: tuple[str, ...],
    *,
    work_dir: Path,
) -> dict[str, str]:
    if not paths:
        raise ValueError("workspace patch paths cannot be empty")
    for value in paths:
        path = Path(value)
        if path.is_absolute() or ".." in path.parts:
            raise ValueError(f"workspace patch path must be repository-relative: {value}")
    patch_path = bounded_capture(
        ("git", "diff", "--binary", "HEAD", "--", *paths),
        repository_root=repository_root,
        work_dir=work_dir,
        name="workspace-patch",
        timeout_seconds=PATCH_TIMEOUT_SECONDS,
        max_output_bytes=MAX_PATCH_BYTES,
    )
    return {
        "workspacePatchScope": " ".join(paths)
        + "; SHA-256 of git diff --binary HEAD -- listed paths",
        "workspacePatchSha256": file_sha256(patch_path),
    }


def _digest_source_files(
    repository_root: Path,
    paths: list[Path],
    *,
    domain: bytes,
) -> dict[str, object]:
    files: dict[bytes, Path] = {}
    for path in paths:
        relative = path.relative_to(repository_root)
        encoded = os.fsencode(relative.as_posix())
        files[encoded] = path
    if len(files) > MAX_SOURCE_FILES:
        raise ValueError(f"source file count exceeds {MAX_SOURCE_FILES}")
    digest = hashlib.sha256(domain)
    total_bytes = 0
    for encoded_path in sorted(files):
        path = files[encoded_path]
        digest.update(len(encoded_path).to_bytes(8, "big"))
        digest.update(encoded_path)
        if path.is_symlink():
            target = os.fsencode(os.readlink(path))
            size = len(target)
            digest.update(b"L")
            digest.update(size.to_bytes(8, "big"))
            digest.update(target)
        else:
            size = path.stat().st_size
            if size > MAX_SOURCE_FILE_BYTES:
                raise ValueError(
                    f"source file {path} is {size} bytes; maximum is "
                    f"{MAX_SOURCE_FILE_BYTES}"
                )
            digest.update(b"F")
            digest.update(size.to_bytes(8, "big"))
            with path.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
        total_bytes += size
        if total_bytes > MAX_SOURCE_BYTES:
            raise ValueError(f"source bytes exceed {MAX_SOURCE_BYTES}")
    return {
        "files": len(files),
        "bytes": total_bytes,
        "sha256": digest.hexdigest(),
    }


def _git_file_inventory(
    repository_root: Path,
    paths: tuple[str, ...],
    *,
    work_dir: Path,
    name: str,
    arguments: tuple[str, ...],
) -> list[Path]:
    inventory_path = bounded_capture(
        ("git", "ls-files", *arguments, "-z", "--", *paths),
        repository_root=repository_root,
        work_dir=work_dir,
        name=name,
        timeout_seconds=PATCH_TIMEOUT_SECONDS,
        max_output_bytes=MAX_PATCH_BYTES,
    )
    raw_inventory = inventory_path.read_bytes()
    if raw_inventory and not raw_inventory.endswith(b"\0"):
        raise ValueError(f"git {name} inventory is not NUL-terminated")
    files = [
        repository_root / Path(os.fsdecode(value))
        for value in raw_inventory.split(b"\0")
        if value
    ]
    return [path for path in files if path.exists() or path.is_symlink()]


def workspace_source_metadata(
    repository_root: Path,
    paths: tuple[str, ...],
    *,
    work_dir: Path,
) -> dict[str, object]:
    patch = workspace_patch_metadata(repository_root, paths, work_dir=work_dir)
    for value in paths:
        candidate = repository_root / value
        if not candidate.exists() and not candidate.is_symlink():
            raise ValueError(f"workspace source path does not exist: {value}")
    tracked_files = _git_file_inventory(
        repository_root,
        paths,
        work_dir=work_dir,
        name="workspace-tracked",
        arguments=("--cached",),
    )
    untracked_files = _git_file_inventory(
        repository_root,
        paths,
        work_dir=work_dir,
        name="workspace-untracked",
        arguments=("--others", "--exclude-standard"),
    )
    tracked = _digest_source_files(
        repository_root,
        tracked_files,
        domain=b"compass.qualification-workspace-tracked/1",
    )
    untracked = _digest_source_files(
        repository_root,
        untracked_files,
        domain=b"compass.qualification-workspace-untracked/1",
    )
    tree = _digest_source_files(
        repository_root,
        [*tracked_files, *untracked_files],
        domain=b"compass.qualification-workspace-tree/1",
    )
    return {
        **patch,
        "workspaceTreePolicy": "existing tracked plus non-ignored untracked files in scope",
        "workspaceTreeFiles": tree["files"],
        "workspaceTreeBytes": tree["bytes"],
        "workspaceTreeSha256": tree["sha256"],
        "workspaceTrackedFiles": tracked["files"],
        "workspaceTrackedBytes": tracked["bytes"],
        "workspaceTrackedSha256": tracked["sha256"],
        "workspaceUntrackedFiles": untracked["files"],
        "workspaceUntrackedBytes": untracked["bytes"],
        "workspaceUntrackedSha256": untracked["sha256"],
    }


def raw_traversal_summary(
    path: Path,
    *,
    measurement: dict[str, object],
    command: tuple[str, ...],
) -> dict[str, object]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != RAW_TRAVERSAL_SCHEMA:
        raise ValueError(f"unsupported raw traversal schema in {path}")
    results = document.get("results")
    limits = document.get("limits")
    if not isinstance(results, list) or len(results) != 30 or not isinstance(limits, dict):
        raise ValueError("raw traversal evidence must contain 30 results and a limits object")
    if document.get("oracleStatus") != "PASS":
        raise ValueError("raw traversal evidence must have oracleStatus PASS")
    total_elapsed = document.get("elapsedMicroseconds")
    if isinstance(total_elapsed, bool) or not isinstance(total_elapsed, int):
        raise ValueError("raw traversal evidence requires integer elapsedMicroseconds")
    elapsed = []
    for index, item in enumerate(results):
        if not isinstance(item, dict):
            raise ValueError(f"raw traversal result {index} must be an object")
        value = item.get("elapsedMicroseconds")
        if isinstance(value, bool) or not isinstance(value, int) or value < 0:
            raise ValueError(
                f"raw traversal result {index} requires non-negative integer "
                "elapsedMicroseconds"
            )
        elapsed.append(value)
    return {
        "schema": RAW_TRAVERSAL_SCHEMA,
        "taskSuite": "agent-tasks-v1.json",
        "taskCount": len(results),
        "oracleStatus": "PASS",
        "command": ["python3", *command[1:]],
        "commandInterpreter": {
            "recordedLabel": "python3",
            "measuredExecutableName": Path(command[0]).name,
            "versionField": "host.python",
        },
        "wallMicroseconds": int(measurement["wallMicroseconds"]),
        "reportedElapsedMicroseconds": total_elapsed,
        "peakRssBytes": int(measurement["peakRssBytes"]),
        "taskP95Microseconds": percentile95(elapsed),
        "taskMaximumMicroseconds": max(elapsed),
        "limits": limits,
    }


def validated_graph_contract(
    graph: Path,
    *,
    metadata_path: Path,
    scale_digests_path: Path,
) -> dict[str, object]:
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    if metadata.get("schema") != "compass.qualification-graph-generator/1":
        raise ValueError(f"unsupported graph metadata schema in {metadata_path}")
    digests = json.loads(scale_digests_path.read_text(encoding="utf-8"))
    if digests.get("schema") != "compass.qualification-profile-digests/1":
        raise ValueError(f"unsupported scale digest schema in {scale_digests_path}")
    profile = metadata.get("profile")
    matches = [item for item in digests.get("profiles", []) if item.get("name") == profile]
    if len(matches) != 1:
        raise ValueError(f"graph profile {profile!r} must resolve exactly once")
    expected = matches[0]
    actual_bytes = graph.stat().st_size
    actual_sha256 = file_sha256(graph)
    checks = {
        "nodes": (metadata.get("nodes"), expected.get("nodes")),
        "edges": (metadata.get("edges"), expected.get("edges")),
        "nodeRecordsSha256": (
            metadata.get("nodeRecordsSha256"),
            expected.get("nodeRecordsSha256"),
        ),
        "edgeRecordsSha256": (
            metadata.get("edgeRecordsSha256"),
            expected.get("edgeRecordsSha256"),
        ),
        "graphBytes": (metadata.get("graphBytes"), actual_bytes),
        "graphSha256": (metadata.get("graphSha256"), actual_sha256),
    }
    mismatches = [name for name, (actual, wanted) in checks.items() if actual != wanted]
    if mismatches:
        raise ValueError(
            "graph metadata does not match the pinned profile/input: "
            + ", ".join(mismatches)
        )
    return {
        "profile": profile,
        "bytes": actual_bytes,
        "sha256": actual_sha256,
        "nodes": int(expected["nodes"]),
        "edges": int(expected["edges"]),
    }


def require_medium_baseline_profile(graph_contract: dict[str, object]) -> None:
    if graph_contract.get("profile") != "qualification-medium":
        raise ValueError(
            "current-engine baseline requires the qualification-medium profile"
        )


def query_commands(binary: Path, graph: Path, cache: Path) -> dict[str, tuple[str, ...]]:
    common = ("--graph", str(graph), "--cache", str(cache), "--engine", "json", "--format", "json")
    return {
        "search": (
            str(binary),
            "search",
            "qualification::Node0099999",
            *common,
            "--max-candidates",
            "256",
        ),
        "callers": (str(binary), "callers", "qualification::Node0099999", *common, "--max-nodes", "512", "--max-edges", "1024"),
        "callees": (str(binary), "callees", "qualification::Node0000000", *common, "--max-nodes", "512", "--max-edges", "1024"),
        "impact-depth-3": (str(binary), "impact", "qualification::Node0099999", *common, "--max-depth", "3", "--max-nodes", "512", "--max-edges", "1024"),
        # The legacy `path` command exposes no traversal-limit flags. Node 9 has
        # an exact three-hop shortest path in this finite, versioned fixture;
        # ProcessSpec supplies the independent wall-time/output bounds.
        "path-depth-3": (
            str(binary),
            "path",
            "qualification::Node0000000",
            "qualification::Node0000009",
            "--graph",
            str(graph),
        ),
    }


def _measure(
    command: tuple[str, ...],
    *,
    name: str,
    iteration: int,
    work_dir: Path,
    repository_root: Path,
    timeout_seconds: float,
    max_output_bytes: int = MAX_WORKLOAD_OUTPUT_BYTES,
) -> dict[str, object]:
    stdout_path = work_dir / "logs" / f"{name}-{iteration}.out"
    stderr_path = work_dir / "logs" / f"{name}-{iteration}.err"
    metrics = run_measured(
        ProcessSpec(
            command=command,
            cwd=repository_root,
            stdout_path=stdout_path,
            stderr_path=stderr_path,
            timeout_seconds=timeout_seconds,
        )
    )
    if metrics.return_code != 0 or metrics.timed_out or metrics.signal is not None:
        raise RuntimeError(
            f"{name}[{iteration}] failed: return={metrics.return_code} "
            f"signal={metrics.signal} timed_out={metrics.timed_out}"
        )
    for label, path in (("stdout", stdout_path), ("stderr", stderr_path)):
        size = path.stat().st_size
        if size > max_output_bytes:
            raise ValueError(
                f"{name}[{iteration}] {label} is {size} bytes; "
                f"maximum is {max_output_bytes}"
            )
    return {
        "iteration": iteration,
        "wallMicroseconds": round(metrics.wall_seconds * 1_000_000),
        "userMicroseconds": round(metrics.user_seconds * 1_000_000),
        "systemMicroseconds": round(metrics.system_seconds * 1_000_000),
        "peakRssBytes": metrics.peak_rss_kib * 1024,
        "stdoutSha256": metrics.stdout_sha256,
        "stderrSha256": metrics.stderr_sha256,
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument(
        "--graph-metadata",
        type=Path,
        help="Generator metadata; defaults to generation.json beside --graph",
    )
    parser.add_argument(
        "--scale-digests",
        type=Path,
        default=Path(__file__).with_name("scale-profile-digests-v1.json"),
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument(
        "--raw-traversal-script",
        type=Path,
        default=Path(__file__).with_name("raw_traversal.py"),
    )
    parser.add_argument(
        "--task-suite",
        type=Path,
        default=Path(__file__).with_name("agent-tasks-v1.json"),
    )
    parser.add_argument("--repository-root", type=Path, default=Path.cwd())
    parser.add_argument(
        "--workspace-patch-path",
        action="append",
        help="Repository-relative path included in source-state hashing; repeatable",
    )
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--timeout-seconds", type=float, default=120.0)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.samples < 5:
        raise ValueError("at least five samples are required")
    repository_root = args.repository_root.resolve(strict=True)
    binary = args.binary.resolve(strict=True)
    graph = args.graph.resolve(strict=True)
    graph_metadata_path = (
        args.graph_metadata.resolve(strict=True)
        if args.graph_metadata is not None
        else graph.with_name("generation.json").resolve(strict=True)
    )
    scale_digests_path = args.scale_digests.resolve(strict=True)
    work_dir = args.work_dir.resolve(strict=False)
    work_dir.mkdir(parents=True, exist_ok=True)
    (work_dir / "logs").mkdir(parents=True, exist_ok=True)
    binary_relative = repository_relative(binary, repository_root, label="binary")
    graph_relative = repository_relative(graph, repository_root, label="graph")
    graph_contract = validated_graph_contract(
        graph,
        metadata_path=graph_metadata_path,
        scale_digests_path=scale_digests_path,
    )
    require_medium_baseline_profile(graph_contract)
    raw_script = args.raw_traversal_script.resolve(strict=True)
    task_suite = args.task_suite.resolve(strict=True)
    raw_script_relative = repository_relative(
        raw_script, repository_root, label="raw traversal script"
    )
    task_suite_relative = repository_relative(task_suite, repository_root, label="task suite")
    patch_paths = tuple(args.workspace_patch_path or DEFAULT_PATCH_PATHS)
    patch_metadata = workspace_source_metadata(
        repository_root, patch_paths, work_dir=work_dir
    )
    git_head = bounded_text_command(
        ("git", "rev-parse", "HEAD"),
        repository_root=repository_root,
        work_dir=work_dir,
        name="git-head",
        timeout_seconds=PATCH_TIMEOUT_SECONDS,
    )
    validated_git_oid(git_head)
    cache = work_dir / "query-cache"
    version = bounded_text_command(
        (str(binary_relative), "--version"),
        repository_root=repository_root,
        work_dir=work_dir,
        name="binary-version",
        timeout_seconds=args.timeout_seconds,
    )
    rustc_version = bounded_text_command(
        ("rustc", "--version"),
        repository_root=repository_root,
        work_dir=work_dir,
        name="rustc-version",
        timeout_seconds=args.timeout_seconds,
    )
    cargo_version = bounded_text_command(
        ("cargo", "--version"),
        repository_root=repository_root,
        work_dir=work_dir,
        name="cargo-version",
        timeout_seconds=args.timeout_seconds,
    )
    cache_relative = repository_relative(cache, repository_root, label="cache")
    commands = query_commands(binary_relative, graph_relative, cache_relative)
    for name, command in commands.items():
        _measure(
            command,
            name=f"warmup-{name}",
            iteration=0,
            work_dir=work_dir,
            repository_root=repository_root,
            timeout_seconds=args.timeout_seconds,
        )
    workloads: dict[str, object] = {}
    cold_command = (str(binary_relative), "--version")
    all_commands = {"cold-start": cold_command, **commands}
    for name, command in all_commands.items():
        samples = [
            _measure(
                command,
                name=name,
                iteration=iteration,
                work_dir=work_dir,
                repository_root=repository_root,
                timeout_seconds=args.timeout_seconds,
            )
            for iteration in range(1, args.samples + 1)
        ]
        wall = [int(item["wallMicroseconds"]) for item in samples]
        rss = [int(item["peakRssBytes"]) for item in samples]
        workloads[name] = {
            "command": list(command),
            "samples": samples,
            "p95WallMicroseconds": percentile95(wall),
            "maximumPeakRssBytes": max(rss),
        }
    raw_command = (
        sys.executable,
        str(raw_script_relative),
        "--graph",
        str(graph_relative),
        "--tasks",
        str(task_suite_relative),
        "--max-graph-bytes",
        "268435456",
        "--max-nodes",
        "100000",
        "--max-edges",
        "250000",
        "--max-depth",
        "32",
        "--max-results",
        "10000",
        "--timeout-seconds",
        str(args.timeout_seconds),
    )
    raw_measurement = _measure(
        raw_command,
        name="raw-traversal",
        iteration=1,
        work_dir=work_dir,
        repository_root=repository_root,
        timeout_seconds=args.timeout_seconds,
    )
    raw_traversal = raw_traversal_summary(
        work_dir / "logs" / "raw-traversal-1.out",
        measurement=raw_measurement,
        command=raw_command,
    )
    payload = {
        "schema": SCHEMA,
        "recordedAt": datetime.now(timezone.utc).isoformat(),
        "host": {
            "architecture": platform.machine(),
            "operatingSystem": platform.platform(),
            "python": platform.python_version(),
            "zlib": zlib.ZLIB_RUNTIME_VERSION,
            "rustc": rustc_version,
            "cargo": cargo_version,
        },
        "binary": {
            "path": binary_relative.as_posix(),
            "version": version,
            "bytes": binary.stat().st_size,
            "gzip9Bytes": compressed_size(binary),
            "sha256": file_sha256(binary),
            "buildCommand": "cargo build -p compass-cli --release --locked",
        },
        "graph": {
            "path": graph_relative.as_posix(),
            **graph_contract,
        },
        "measurement": {
            "samplesPerWorkload": args.samples,
            "p95Method": "nearest-rank",
            "condition": "separate CLI processes after one unmeasured cache warmup per query; cold-start is --version",
            "pathBound": "the legacy path command has no depth flag; the selected deterministic endpoints have an exact three-hop shortest path, the graph is capped at 100000 nodes/250000 edges/268435456 bytes, and the subprocess has an explicit timeout",
            "workloads": workloads,
        },
        "rawTraversal": raw_traversal,
        "source": {
            "gitHead": git_head,
            **patch_metadata,
        },
    }
    atomic_write_text(args.output, json.dumps(payload, indent=2, sort_keys=True) + "\n")
    print(args.output)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (KeyError, OSError, RuntimeError, TypeError, ValueError, subprocess.SubprocessError) as error:
        print(f"baseline failed: {error}", file=sys.stderr)
        raise SystemExit(2) from error
