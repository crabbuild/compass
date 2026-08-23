#!/usr/bin/env python3
"""Pinned, read-only React/frontend production qualification runner.

This runner is deliberately evidence-heavy. It validates immutable external
checkouts, projects only reviewed source globs, invokes one exact release
binary, and scores the result against an independent TypeScript compiler
oracle. Every command, digest, worker mode, resource observation, and
interruption result is retained outside the source trees.
"""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import selectors
import shutil
import signal
import subprocess
import sys
import time
import tomllib
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
MOUNT = Path("/Volumes/Workspace/Github").resolve()
SOURCE_SUFFIXES = {".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"}
CONFIG_NAMES = {
    "package.json", "tsconfig.json", "jsconfig.json",
    "vite.config.ts", "vite.config.js", "vite.config.mjs", "vite.config.cjs",
    "next.config.ts", "next.config.js", "next.config.mjs", "next.config.cjs",
    "react-router.config.ts", "react-router.config.js",
}
ALLOWED_COMMANDS = {"git", "node", "python3", "compass"}
MAX_PROJECT_FILES = 100_000
MAX_PROJECT_BYTES = 2 * 1024 * 1024 * 1024
MAX_PROJECT_FILE_BYTES = 8 * 1024 * 1024
COMMAND_TIMEOUT_SECONDS = 30 * 60
MAX_COMMAND_OUTPUT_BYTES = 8 * 1024 * 1024
MAX_JSON_BYTES = 512 * 1024 * 1024
PERFORMANCE_BASELINE_SCHEMA = "compass.react-frontend-performance-baseline/1"


class QualificationError(RuntimeError):
    pass


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def digest_projection(root: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        digest.update(path.relative_to(root).as_posix().encode())
        digest.update(b"\0")
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                digest.update(chunk)
    return digest.hexdigest()


def wilson_lower(successes: int, trials: int, z: float = 1.959963984540054) -> float:
    if trials <= 0:
        return 0.0
    proportion = successes / trials
    denominator = 1.0 + z * z / trials
    centre = proportion + z * z / (2.0 * trials)
    spread = z * math.sqrt((proportion * (1.0 - proportion) + z * z / (4.0 * trials)) / trials)
    return max(0.0, (centre - spread) / denominator)


def fail(message: str) -> None:
    raise QualificationError(message)


def command_name(command: list[str]) -> str:
    return Path(command[0]).name


def checked_command(command: list[str]) -> None:
    if not command or command_name(command) not in ALLOWED_COMMANDS:
        fail(f"qualification attempted a non-allow-listed process: {command!r}")


def qualification_env() -> dict[str, str]:
    return {
        **os.environ,
        "CI": "1", "NO_COLOR": "1", "TSLP_OFFLINE": "1",
        "CARGO_NET_OFFLINE": "true", "COMPASS_OFFLINE": "1",
    }


def _bounded_process(
    command: list[str],
    *,
    cwd: Path,
    interrupt_after: float | None = None,
) -> tuple[subprocess.CompletedProcess[str], bool, int, str]:
    """Run one allow-listed process with bounded pipes and a hard deadline.

    ``communicate()`` is deliberately not used here: it buffers all child
    output before a caller can enforce the limit.  A selector drains both
    streams while the child runs and kills it as soon as the aggregate cap is
    crossed.  The optional interrupt timer is used by the cancellation gate.
    """
    process = subprocess.Popen(
        command,
        cwd=cwd,
        env=qualification_env(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=False,
    )
    assert process.stdout is not None and process.stderr is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ, "stdout")
    selector.register(process.stderr, selectors.EVENT_READ, "stderr")
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    started = time.monotonic()
    signal_sent = False
    child_reaped = False
    child_usage = None

    def reap(*, nonblocking: bool) -> None:
        nonlocal child_reaped, child_usage
        if child_reaped:
            return
        if hasattr(os, "wait4"):
            flags = os.WNOHANG if nonblocking else 0
            try:
                waited_pid, status, usage = os.wait4(process.pid, flags)
            except ChildProcessError:
                waited_pid = process.pid
                status = 0
                usage = None
            if waited_pid == 0:
                return
            process.returncode = os.waitstatus_to_exitcode(status)
            child_usage = usage
            child_reaped = True
            return
        if nonblocking:
            if process.poll() is None:
                return
        else:
            process.wait()
        child_reaped = True

    def close_streams() -> None:
        for stream in (process.stdout, process.stderr):
            try:
                stream.close()
            except OSError:
                pass
        selector.close()

    try:
        while selector.get_map() or not child_reaped:
            now = time.monotonic()
            reap(nonblocking=True)
            if interrupt_after is not None and not signal_sent and now - started >= interrupt_after:
                if not child_reaped:
                    process.send_signal(signal.SIGINT)
                    signal_sent = True
            if now - started >= COMMAND_TIMEOUT_SECONDS:
                if not child_reaped:
                    process.kill()
                    reap(nonblocking=False)
                close_streams()
                fail(f"qualification command exceeded the {COMMAND_TIMEOUT_SECONDS}s timeout: {command!r}")
            events = selector.select(timeout=min(0.1, COMMAND_TIMEOUT_SECONDS - (now - started)))
            for key, _ in events:
                chunk = key.fileobj.read(64 * 1024)
                if not chunk:
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                    continue
                bucket = buffers[key.data]
                bucket.extend(chunk)
                if sum(len(item) for item in buffers.values()) > MAX_COMMAND_OUTPUT_BYTES:
                    if not child_reaped:
                        process.kill()
                        reap(nonblocking=False)
                    close_streams()
                    fail(
                        f"qualification command exceeded the {MAX_COMMAND_OUTPUT_BYTES}-byte output limit: {command!r}"
                    )
        reap(nonblocking=False)
    finally:
        close_streams()
    rss_bytes = 0
    rss_measurement = "unavailable"
    if child_usage is not None:
        rss_bytes = int(child_usage.ru_maxrss)
        if platform.system() != "Darwin":
            rss_bytes *= 1024
        rss_measurement = "wait4-child-ru_maxrss"
    return (
        subprocess.CompletedProcess(
            command,
            process.returncode,
            bytes(buffers["stdout"]).decode("utf-8", errors="replace"),
            bytes(buffers["stderr"]).decode("utf-8", errors="replace"),
        ),
        signal_sent,
        rss_bytes,
        rss_measurement,
    )


def run_checked(command: list[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    checked_command(command)
    completed, _, _, _ = _bounded_process(command, cwd=cwd)
    return completed


def bounded_read(path: Path, *, limit: int = MAX_JSON_BYTES) -> bytes:
    try:
        if path.stat().st_size > limit:
            fail(f"qualification input exceeds the {limit}-byte limit: {path}")
        content = bytearray()
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                content.extend(chunk)
                if len(content) > limit:
                    fail(f"qualification input exceeds the {limit}-byte limit: {path}")
        return bytes(content)
    except OSError as error:
        fail(f"cannot read bounded qualification input {path}: {error}")
        raise AssertionError("unreachable")


def normalized_remote_url(value: str) -> str:
    value = value.strip()
    if value.startswith("git@") and ":" in value:
        host, path = value[4:].split(":", 1)
        value = f"https://{host}/{path}"
    elif value.startswith("ssh://git@"):
        value = "https://" + value.removeprefix("ssh://git@")
    value = value.rstrip("/")
    if value.endswith(".git"):
        value = value[:-4]
    return value.lower()


def load_manifest(path: Path) -> dict[str, Any]:
    try:
        document = tomllib.loads(bounded_read(path, limit=8 * 1024 * 1024).decode("utf-8"))
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        fail(f"cannot read frontend qualification manifest: {error}")
    if document.get("schema") != "compass.frontend-qualification/2":
        fail("frontend qualification manifest schema must be compass.frontend-qualification/2")
    if document.get("readOnly") is not True or document.get("mode") != "pinned":
        fail("frontend qualification manifest must declare readOnly = true and mode = pinned")
    if document.get("checkoutRoot") != str(MOUNT):
        fail(f"frontend qualification checkoutRoot must be {MOUNT}")
    if document.get("oracleProvider") != "typescript_compiler_api_5_9_3_frontend_projection":
        fail("frontend qualification manifest has an unexpected oracle provider")
    if document.get("oracleToolchain") != "node-24;typescript-5.9.3":
        fail("frontend qualification manifest has an unexpected oracle toolchain")
    repositories = document.get("repository")
    if not isinstance(repositories, list) or not repositories:
        fail("frontend qualification manifest must contain repositories")
    ids: set[str] = set()
    for repository in repositories:
        if not isinstance(repository, dict):
            fail("repository entry must be an object")
        required = {"id", "family", "framework", "url", "commit", "license", "licenseFile", "sourceRoot", "sourceGlobs"}
        missing = required - repository.keys()
        if missing:
            fail(f"repository is missing {sorted(missing)}")
        identifier = repository["id"]
        if not isinstance(identifier, str) or not identifier or identifier in ids:
            fail(f"repository ID is empty or duplicated: {identifier!r}")
        ids.add(identifier)
        if not isinstance(repository.get("family"), str) or not repository["family"]:
            fail(f"repository {identifier} has no family")
        commit = repository["commit"]
        if not isinstance(commit, str) or len(commit) != 40 or any(char not in "0123456789abcdef" for char in commit.lower()):
            fail(f"repository {identifier} must use a full commit SHA")
        globs = repository["sourceGlobs"]
        if not isinstance(globs, list) or not globs or any(not isinstance(item, str) or not item for item in globs):
            fail(f"repository {identifier} has invalid sourceGlobs")
        excludes = repository.get("excludeGlobs", [])
        if not isinstance(excludes, list) or any(not isinstance(item, str) or not item for item in excludes):
            fail(f"repository {identifier} has invalid excludeGlobs")
        capabilities = repository.get("capabilities", [])
        if not isinstance(capabilities, list) or any(not isinstance(item, str) or not item for item in capabilities):
            fail(f"repository {identifier} has invalid capabilities")
        if not isinstance(repository.get("oracleDiagnosticBudget"), int) or repository["oracleDiagnosticBudget"] < 0:
            fail(f"repository {identifier} must declare a non-negative oracleDiagnosticBudget")
        if repository.get("stable") is not True:
            fail(f"repository {identifier} must explicitly declare stable = true")
    stable_count = sum(1 for repository in repositories if repository.get("stable") is True)
    budget = document.get("qualityBudget", {})
    if budget.get("stableFamilyCount") != stable_count:
        fail("qualityBudget.stableFamilyCount does not match stable repositories")
    if budget.get("minimumAccepted") != max(2000, 400 * stable_count):
        fail("qualityBudget.minimumAccepted does not equal max(2000, 400 * stableFamilyCount)")
    for key in (
        "minimumPrecision",
        "minimumRecall",
        "minimumWilsonLower95",
        "minimumCapabilityPrecision",
        "minimumCapabilityRecall",
        "maxColdRegression",
        "maxWarmRegression",
        "maxPeakRssRegression",
    ):
        if not isinstance(budget.get(key), (int, float)) or budget[key] <= 0:
            fail(f"qualityBudget.{key} must be positive")
    for key in ("minimumPrecision", "minimumRecall", "minimumWilsonLower95", "minimumCapabilityPrecision", "minimumCapabilityRecall"):
        if budget[key] > 1:
            fail(f"qualityBudget.{key} must not exceed 1")
    return document


def checkout_for(repository: dict[str, Any]) -> Path:
    raw = repository.get("checkout")
    if not isinstance(raw, str) or not raw:
        url = str(repository["url"]).rstrip("/").removesuffix(".git")
        parts = url.split("/")
        if len(parts) < 2:
            fail(f"cannot infer checkout for {repository['id']}")
        raw = str(MOUNT / parts[-2] / parts[-1])
    checkout = Path(raw).expanduser().resolve()
    try:
        checkout.relative_to(MOUNT)
    except ValueError as error:
        raise QualificationError(f"{repository['id']} checkout escapes {MOUNT}") from error
    return checkout


def git_status(checkout: Path) -> str:
    status = run_checked(["git", "-C", str(checkout), "status", "--porcelain=v1", "--untracked-files=all"], cwd=ROOT)
    if status.returncode != 0:
        fail(f"git status failed for {checkout}: {status.stderr.strip()}")
    return status.stdout


def verify_checkout(repository: dict[str, Any]) -> Path:
    checkout = checkout_for(repository)
    if not checkout.is_dir():
        fail(f"missing pinned checkout for {repository['id']}: {checkout}")
    revision = run_checked(["git", "-C", str(checkout), "rev-parse", "HEAD"], cwd=ROOT)
    if revision.returncode != 0 or revision.stdout.strip() != repository["commit"]:
        fail(f"{repository['id']} is not at pinned commit {repository['commit']}")
    remote = run_checked(["git", "-C", str(checkout), "remote", "get-url", "origin"], cwd=ROOT)
    if remote.returncode != 0 or normalized_remote_url(remote.stdout) != normalized_remote_url(str(repository["url"])):
        fail(
            f"{repository['id']} origin URL does not match its manifest: "
            f"{remote.stdout.strip()!r} != {repository['url']!r}"
        )
    if git_status(checkout):
        fail(f"{repository['id']} checkout is not clean")
    license_path = checkout / repository["licenseFile"]
    if not license_path.is_file() or license_path.is_symlink():
        fail(f"{repository['id']} license file is unavailable: {license_path}")
    if repository.get("licenseSha256") and digest_file(license_path) != repository["licenseSha256"]:
        fail(f"{repository['id']} license checksum drifted")
    return checkout


def matches(relative: str, pattern: str) -> bool:
    """Match path segments with ``**`` meaning zero or more segments."""
    path_parts = [part for part in relative.replace("\\", "/").split("/") if part]
    pattern_parts = [part for part in pattern.replace("\\", "/").split("/") if part]

    def visit(path_index: int, pattern_index: int) -> bool:
        if pattern_index == len(pattern_parts):
            return path_index == len(path_parts)
        part = pattern_parts[pattern_index]
        if part == "**":
            return visit(path_index, pattern_index + 1) or (path_index < len(path_parts) and visit(path_index + 1, pattern_index))
        return path_index < len(path_parts) and fnmatch.fnmatchcase(path_parts[path_index], part) and visit(path_index + 1, pattern_index + 1)

    return visit(0, 0)


def project_repository(repository: dict[str, Any], checkout: Path, destination: Path) -> tuple[int, int, str]:
    source_root = (checkout / repository["sourceRoot"]).resolve()
    try:
        source_root.relative_to(checkout)
    except ValueError as error:
        raise QualificationError(f"{repository['id']} sourceRoot escapes checkout") from error
    if not source_root.is_dir():
        fail(f"{repository['id']} sourceRoot is not a directory: {source_root}")
    selected: set[Path] = set()
    selected_bytes = 0
    for path in sorted(source_root.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        relative = path.relative_to(checkout).as_posix()
        source_relative = path.relative_to(source_root).as_posix()
        if any(part in {".git", "node_modules", "dist", "build", "coverage", ".next", ".turbo"} for part in Path(relative).parts):
            continue
        if not any(matches(relative, pattern) or matches(source_relative, pattern) for pattern in repository["sourceGlobs"]):
            continue
        if any(matches(relative, pattern) or matches(source_relative, pattern) for pattern in repository.get("excludeGlobs", [])):
            continue
        if path.suffix.lower() not in SOURCE_SUFFIXES and path.name not in CONFIG_NAMES:
            continue
        if path.stat().st_size > MAX_PROJECT_FILE_BYTES:
            fail(f"{repository['id']} contains a file larger than the {MAX_PROJECT_FILE_BYTES}-byte limit: {path}")
        selected.add(path)
        selected_bytes += path.stat().st_size
        if len(selected) > MAX_PROJECT_FILES or selected_bytes > MAX_PROJECT_BYTES:
            fail(f"{repository['id']} projection exceeds bounded file/byte limits")
    if not selected:
        fail(f"{repository['id']} source globs selected no files")
    for selected_path in list(selected):
        parent = selected_path.parent
        while True:
            for marker in CONFIG_NAMES:
                candidate = parent / marker
                if candidate.is_file() and not candidate.is_symlink():
                    selected.add(candidate)
            if parent == checkout:
                break
            parent = parent.parent
    if len(selected) > MAX_PROJECT_FILES:
        fail(f"{repository['id']} projection exceeds the bounded file limit after config closure")
    oversized = [
        path for path in selected
        if path.stat().st_size > MAX_PROJECT_FILE_BYTES
    ]
    if oversized:
        fail(
            f"{repository['id']} config closure contains a file larger than the "
            f"{MAX_PROJECT_FILE_BYTES}-byte limit: {oversized[0]}"
        )
    selected_bytes = sum(path.stat().st_size for path in selected)
    if selected_bytes > MAX_PROJECT_BYTES:
        fail(f"{repository['id']} projection exceeds the bounded byte limit after config closure")
    for source in sorted(selected):
        target = destination / source.relative_to(checkout)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
    if any(path.is_symlink() for path in destination.rglob("*")):
        fail(f"{repository['id']} projection contains an unsafe symlink")
    return len(selected), sum(path.suffix.lower() in SOURCE_SUFFIXES for path in selected), digest_projection(destination)


def active_graph(output: Path) -> Path:
    pointer = output / "compass-out" / "current-snapshot"
    if not pointer.is_file():
        fail(f"Compass did not publish an active snapshot: {pointer}")
    snapshot = bounded_read(pointer, limit=1024 * 1024).decode("utf-8").strip()
    if not snapshot.startswith("snapshot-") or "/" in snapshot or "\\" in snapshot:
        fail(f"invalid active snapshot pointer: {snapshot!r}")
    graph = output / "compass-out" / "snapshots" / snapshot / "graph.json"
    if not graph.is_file() or (graph.parent / "build-incomplete").exists():
        fail(f"Compass published an incomplete graph: {graph}")
    return graph


def check_graph_completeness(graph: Path, repository_id: str) -> None:
    try:
        document = json.loads(bounded_read(graph).decode("utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"{repository_id} published graph is not valid JSON: {error}")
    metadata = document.get("graph")
    if not isinstance(metadata, dict):
        fail(f"{repository_id} graph omitted graph metadata")
    diagnostics = metadata.get("diagnostics", [])
    if not isinstance(diagnostics, list):
        fail(f"{repository_id} graph diagnostics are not an array")
    errors = [item for item in diagnostics if isinstance(item, dict) and item.get("severity") == "error"]
    omissions = [item for item in diagnostics if isinstance(item, dict) and item.get("code") in {"publication_omission_summary", "publication_omitted_node", "publication_omitted_edge"}]
    if errors or omissions:
        sample = (errors or omissions)[:3]
        fail(f"{repository_id} positive corpus published a partial/invalid graph: {sample}")


def compass_command(binary: Path, source: Path, output: Path, *, force: bool = False, workers: int | None = None) -> list[str]:
    command = [str(binary), "update", str(source), "--out", str(output), "--no-cluster", "--no-viz", "--no-gitignore", "--inference-level", "max"]
    if force:
        command.append("--force")
    if workers is not None:
        command.extend(["--max-workers", str(workers)])
    return command


def run_compass(binary: Path, source: Path, output: Path, *, label: str, force: bool = False, workers: int | None = None) -> dict[str, Any]:
    command = compass_command(binary, source, output, force=force, workers=workers)
    output.mkdir(parents=True, exist_ok=True)
    log_path = output.parent / f"{output.name}-{label}.log"
    started = time.perf_counter()
    checked_command(command)
    completed, _, peak_rss_bytes, rss_measurement = _bounded_process(command, cwd=ROOT)
    log_path.write_text(
        completed.stdout + ("\n[stderr]\n" + completed.stderr if completed.stderr else ""),
        encoding="utf-8",
    )
    elapsed = time.perf_counter() - started
    if completed.returncode != 0:
        tail = (completed.stdout + "\n" + completed.stderr)[-4000:]
        fail(f"Compass failed for {source} ({label}): {tail.strip()}")
    graph = active_graph(output)
    return {"label": label, "graph": graph, "graphSha256": digest_file(graph), "seconds": elapsed, "peakRssBytes": peak_rss_bytes, "rssMeasurement": rss_measurement, "command": command, "returncode": completed.returncode, "log": str(log_path)}


def run_interrupted(binary: Path, source: Path, output: Path, *, expected_digest: str) -> dict[str, Any]:
    command = compass_command(binary, source, output, force=True, workers=1)
    output.mkdir(parents=True, exist_ok=True)
    log_path = output.parent / "interruption.log"
    started = time.perf_counter()
    checked_command(command)
    completed, signal_sent, peak_rss_bytes, rss_measurement = _bounded_process(command, cwd=ROOT, interrupt_after=0.5)
    log_path.write_text(
        completed.stdout + ("\n[stderr]\n" + completed.stderr if completed.stderr else ""),
        encoding="utf-8",
    )
    returncode = completed.returncode
    pointer = output / "compass-out" / "current-snapshot"
    pointer_exists = pointer.is_file()
    partial_artifact = False
    published_digest = None
    if pointer_exists:
        try:
            published_digest = digest_file(active_graph(output))
            partial_artifact = published_digest != expected_digest
        except QualificationError:
            partial_artifact = True
    return {"command": command, "signal": "SIGINT", "signalSent": signal_sent, "returncode": returncode, "seconds": time.perf_counter() - started, "peakRssBytes": peak_rss_bytes, "rssMeasurement": rss_measurement, "pointerExists": pointer_exists, "publishedDigest": published_digest, "partialArtifact": partial_artifact, "cancellationObserved": bool(signal_sent and returncode != 0), "log": str(log_path)}


def run_oracle(source: Path, framework: str, destination: Path) -> Path:
    command = ["node", str(ROOT / "scripts/react_frontend_source_oracle.mjs"), "--root", str(source), "--framework", framework, "--output", str(destination)]
    completed = run_checked(command, cwd=ROOT)
    if completed.returncode != 0:
        fail(f"frontend source oracle failed: {completed.stderr.strip() or completed.stdout.strip()}")
    return destination


def load_oracle(path: Path, repository_id: str) -> dict[str, Any]:
    try:
        document = json.loads(bounded_read(path).decode("utf-8"))
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        fail(f"{repository_id} source oracle is not valid bounded JSON: {error}")
    if not isinstance(document, dict) or not isinstance(document.get("sourceOracle"), dict):
        fail(f"{repository_id} source oracle omitted sourceOracle metadata")
    source_oracle = document["sourceOracle"]
    diagnostics = source_oracle.get("diagnostics", [])
    if not isinstance(diagnostics, list) or source_oracle.get("diagnosticsTruncated") is True:
        fail(f"{repository_id} source oracle diagnostics are missing or truncated")
    count = source_oracle.get("diagnosticCount")
    if not isinstance(count, int) or count != len(diagnostics):
        fail(f"{repository_id} source oracle diagnostic count is inconsistent")
    return document


def score(graph: Path, oracle: Path, result: Path) -> dict[str, Any]:
    command = ["python3", str(ROOT / "scripts/qualify_react_frontend_graph.py"), "--graph", str(graph), "--source-oracle", str(oracle), "--score-only", "--min-precision", "0", "--min-recall", "0", "--result", str(result)]
    completed = run_checked(command, cwd=ROOT)
    if completed.returncode != 0:
        fail(f"frontend scorecard failed: {completed.stderr.strip() or completed.stdout.strip()}")
    try:
        return json.loads(completed.stdout.splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as error:
        fail(f"frontend scorecard emitted invalid JSON: {error}")


def copy_projection(source: Path, destination: Path) -> None:
    if destination.exists():
        fail(f"qualification destination already exists: {destination}")
    shutil.copytree(source, destination, symlinks=False)


def first_source_file(root: Path) -> Path | None:
    return next((path for path in sorted(root.rglob("*")) if path.is_file() and not path.is_symlink() and path.suffix.lower() in SOURCE_SUFFIXES), None)


def capability_report(repository: dict[str, Any], scorecard: dict[str, Any]) -> dict[str, Any]:
    advertised = repository.get("capabilities", [])
    observed = scorecard.get("capabilities", {})
    if not isinstance(observed, dict):
        fail(f"{repository['id']} scorecard omitted capability metrics")
    report: dict[str, Any] = {}
    for capability in advertised:
        metric = observed.get(capability)
        if not isinstance(metric, dict) or metric.get("expected", 0) <= 0:
            fail(f"{repository['id']} advertised capability has no independent oracle records: {capability}")
        report[capability] = metric
    return report


def performance_comparison(result: dict[str, Any], baseline_path: Path | None) -> dict[str, Any]:
    if baseline_path is None:
        return {"status": "candidate-not-compared", "comparisons": []}
    if not baseline_path.is_file():
        fail(f"performance baseline is unavailable: {baseline_path}")
    try:
        baseline = json.loads(bounded_read(baseline_path).decode("utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read performance baseline: {error}")
    if baseline.get("schema") != PERFORMANCE_BASELINE_SCHEMA:
        fail(f"performance baseline schema must be {PERFORMANCE_BASELINE_SCHEMA}")
    if baseline.get("manifestSha256") != result["manifestSha256"]:
        fail("performance baseline was recorded against a different corpus manifest")
    if not isinstance(baseline.get("repositories"), list):
        fail("performance baseline must contain repository observations")
    baseline_by_id = {item.get("id"): item for item in baseline.get("repositories", []) if isinstance(item, dict)}
    comparisons: list[dict[str, Any]] = []
    budget = result["qualityBudget"]
    for report in result["repositories"]:
        prior = baseline_by_id.get(report["id"])
        if not prior:
            fail(f"performance baseline has no repository {report['id']}")
        prior_rows = {row.get("label"): row for row in prior.get("performance", {}).get("rows", [])}
        for row in report.get("performance", {}).get("rows", []):
            old = prior_rows.get(row.get("label"))
            if not old:
                fail(f"performance baseline has no {report['id']}:{row.get('label')} row")
            if row.get("seconds", 0) <= 0 or row.get("peakRssBytes", 0) <= 0:
                fail(f"current performance row is missing positive measurements: {report['id']}:{row.get('label')}")
            if old.get("seconds", 0) <= 0 or old.get("peakRssBytes", 0) <= 0:
                fail(f"performance baseline row is missing positive measurements: {report['id']}:{row.get('label')}")
            seconds_ratio = row["seconds"] / old["seconds"]
            rss_ratio = row["peakRssBytes"] / old["peakRssBytes"]
            comparisons.append({"id": report["id"], "label": row["label"], "secondsRatio": seconds_ratio, "rssRatio": rss_ratio, "secondsPass": seconds_ratio <= (budget["maxColdRegression"] if row["label"] == "cold" else budget["maxWarmRegression"]), "rssPass": rss_ratio <= budget["maxPeakRssRegression"]})
    return {"status": "compared", "baseline": str(baseline_path), "baselineSha256": digest_file(baseline_path), "comparisons": comparisons}


def run(args: argparse.Namespace) -> dict[str, Any]:
    manifest_path = args.manifest.resolve()
    manifest = load_manifest(manifest_path)
    binary = args.compass.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        fail(f"production Compass binary is not executable: {binary}")
    if binary.name != "compass" or binary.parent.name != "release":
        fail("pinned qualification requires an exact release-mode compass binary")
    binary_digest = digest_file(binary)
    revision_result = run_checked(["git", "-C", str(ROOT), "rev-parse", "HEAD"], cwd=ROOT)
    if revision_result.returncode != 0:
        fail(f"cannot identify Compass revision: {revision_result.stderr.strip()}")
    compass_revision = revision_result.stdout.strip()
    manifest_digest = digest_file(manifest_path)
    artifact_root = Path(args.artifact_root or "/Volumes/Workspace/crabbuild-target/compass-021-react-frontend/qualification/react-frontend").resolve()
    artifact_root.mkdir(parents=True, exist_ok=True)
    run_base = artifact_root / f"run-{manifest_digest[:12]}-{binary_digest[:12]}"
    run_root = run_base
    suffix = 1
    while run_root.exists():
        run_root = artifact_root / f"{run_base.name}-{suffix}"
        suffix += 1
    run_root.mkdir(parents=True)
    max_workers = max(2, min(os.cpu_count() or 2, 8))
    reports: list[dict[str, Any]] = []
    interruption_target: tuple[int, Path, Path, str] | None = None
    policy = {"network": "disabled-by-contract;no-network-command-allow-listed", "allowedProcesses": sorted(ALLOWED_COMMANDS), "projectCodeExecuted": False, "projectCheckoutWritable": False}
    for repository in manifest["repository"]:
        checkout = verify_checkout(repository)
        status_before = git_status(checkout)
        projection = run_root / "projections" / repository["id"]
        files, source_files, projection_digest = project_repository(repository, checkout, projection)
        oracle_path = run_root / "oracles" / f"{repository['id']}.json"
        oracle_path.parent.mkdir(parents=True, exist_ok=True)
        run_oracle(projection, repository["framework"], oracle_path)
        oracle_document = load_oracle(oracle_path, repository["id"])
        oracle_metadata = oracle_document["sourceOracle"]
        if oracle_metadata["diagnosticCount"] > repository["oracleDiagnosticBudget"]:
            fail(
                f"{repository['id']} source-oracle diagnostics exceed the reviewed budget: "
                f"{oracle_metadata['diagnosticCount']} > {repository['oracleDiagnosticBudget']}"
            )
        output_root = run_root / "outputs" / repository["id"]
        cold = run_compass(binary, projection, output_root / "cold", label="cold", workers=1)
        check_graph_completeness(cold["graph"], repository["id"])
        warm = run_compass(binary, projection, output_root / "cold", label="warm")
        semantic_projection = run_root / "performance" / repository["id"] / "semantic-projection"
        copy_projection(projection, semantic_projection)
        semantic_file = first_source_file(semantic_projection)
        if semantic_file is None:
            fail(f"{repository['id']} has no source file for semantic-edit qualification")
        original = bounded_read(semantic_file, limit=MAX_PROJECT_FILE_BYTES)
        semantic_file.write_bytes(original + b"\n// Compass qualification semantic edit\n")
        semantic = run_compass(binary, semantic_projection, output_root / "semantic", label="semantic-edit", force=True, workers=1)
        semantic_file.write_bytes(original)
        restored = run_compass(binary, semantic_projection, output_root / "restore", label="restore", force=True, workers=1)
        if restored["graphSha256"] != cold["graphSha256"]:
            fail(f"{repository['id']} restore graph differs from cold graph")
        config_projection = run_root / "performance" / repository["id"] / "config-projection"
        copy_projection(projection, config_projection)
        config_file = next((path for path in sorted(config_projection.rglob("package.json")) if path.is_file()), None) or next((path for path in sorted(config_projection.rglob("tsconfig.json")) if path.is_file()), None)
        if config_file is None:
            fail(f"{repository['id']} has no package/config marker for manifest-edit qualification")
        config_file.write_bytes(bounded_read(config_file, limit=MAX_PROJECT_FILE_BYTES) + b"\n")
        manifest_edit = run_compass(binary, config_projection, output_root / "manifest-edit", label="manifest-edit", force=True, workers=1)
        alternate_projection = run_root / "performance" / repository["id"] / "alternate-projection"
        copy_projection(projection, alternate_projection)
        alternate = run_compass(binary, alternate_projection, output_root / "alternate", label="alternate-checkout", force=True, workers=max_workers)
        if alternate["graphSha256"] != cold["graphSha256"]:
            fail(f"{repository['id']} alternate-worker graph differs from cold graph")
        forced = run_compass(binary, projection, output_root / "forced", label="forced-one-worker", force=True, workers=1)
        if forced["graphSha256"] != cold["graphSha256"]:
            fail(f"{repository['id']} forced one-worker graph differs from cold graph")
        score_path = run_root / "scorecards" / f"{repository['id']}.json"
        score_path.parent.mkdir(parents=True, exist_ok=True)
        score_result = score(forced["graph"], oracle_path, score_path)
        scorecard = score_result.get("scorecard", {})
        report = {
            "id": repository["id"], "family": repository["family"], "framework": repository["framework"], "commit": repository["commit"], "license": repository["license"], "checkout": str(checkout),
            "projectionFiles": files, "sourceFiles": source_files, "projectionSha256": projection_digest, "oracleSha256": digest_file(oracle_path), "graphSha256": cold["graphSha256"],
            "oracleDiagnostics": oracle_metadata["diagnostics"], "oracleDiagnosticCount": oracle_metadata["diagnosticCount"], "oracleDiagnosticBudget": repository["oracleDiagnosticBudget"],
            "workerDeterminism": {"oneWorker": cold["graphSha256"], "defaultWorker": warm["graphSha256"], "forcedOneWorker": forced["graphSha256"], "maximumWorker": alternate["graphSha256"], "byteIdentical": len({cold["graphSha256"], warm["graphSha256"], forced["graphSha256"], alternate["graphSha256"]}) == 1, "maximumWorkers": max_workers},
            "performance": {"rows": [{key: observation[key] for key in ("label", "seconds", "peakRssBytes", "rssMeasurement", "graphSha256", "command")} for observation in (cold, warm, semantic, manifest_edit, restored, alternate)], "semanticEditRestored": restored["graphSha256"] == cold["graphSha256"]},
            "scorecard": scorecard, "capabilities": capability_report(repository, scorecard), "oracleRecords": scorecard.get("aggregate", {}).get("oracleRecords", 0), "oracleCapabilities": scorecard.get("aggregate", {}).get("oracleCapabilities", {}),
            "artifacts": {"projection": str(projection), "oracle": str(oracle_path), "coldGraph": str(cold["graph"]), "forcedGraph": str(forced["graph"]), "scorecard": str(score_path)},
        }
        if interruption_target is None or source_files > interruption_target[0]:
            interruption_target = (source_files, projection, output_root / "interrupted", cold["graphSha256"])
        reports.append(report)
        if git_status(checkout) != status_before:
            fail(f"{repository['id']} source checkout changed during qualification")
    if interruption_target is None:
        fail("no corpus was available for interruption qualification")
    _, interruption_projection, interruption_output, interruption_digest = interruption_target
    interruption = run_interrupted(binary, interruption_projection, interruption_output, expected_digest=interruption_digest)
    resumed = run_compass(binary, interruption_projection, interruption_output, label="interruption-resume", force=True, workers=1)
    interruption["resumeGraphSha256"] = resumed["graphSha256"]
    interruption["resumeMatchesUncanceled"] = resumed["graphSha256"] == interruption_digest
    if interruption["partialArtifact"] or not interruption["resumeMatchesUncanceled"]:
        fail(f"interruption qualification published a partial or divergent artifact: {interruption}")
    oracle_records = sum(report["oracleRecords"] for report in reports)
    aggregate_expected = sum(report["scorecard"].get("aggregate", {}).get("expected", 0) for report in reports)
    aggregate_matched = sum(report["scorecard"].get("aggregate", {}).get("matched", 0) for report in reports)
    aggregate_candidates = sum(report["scorecard"].get("aggregate", {}).get("candidates", 0) for report in reports)
    result: dict[str, Any] = {
        "schema": "compass.react-frontend-pinned-qualification-result/2", "manifest": str(manifest_path), "manifestSha256": manifest_digest, "artifactRoot": str(run_root),
        "compass": {"revision": compass_revision, "binary": str(binary), "binarySha256": binary_digest, "profile": "release", "features": "workspace-default"},
        "oracle": {"provider": manifest["oracleProvider"], "toolchain": manifest["oracleToolchain"], "execution": "source-only;no-project-code"}, "qualityBudget": manifest["qualityBudget"], "repositories": reports,
        "aggregate": {"expected": aggregate_expected, "oracleRecords": oracle_records, "matched": aggregate_matched, "candidates": aggregate_candidates, "precision": aggregate_matched / aggregate_candidates if aggregate_candidates else 0.0, "recall": aggregate_matched / aggregate_expected if aggregate_expected else 0.0, "wilsonLower95": wilson_lower(aggregate_matched, aggregate_candidates), "zeroFabricatedTargets": all(report["scorecard"].get("aggregate", {}).get("zeroFabricatedTargets") is True for report in reports), "oracleDiagnosticCount": sum(report["oracleDiagnosticCount"] for report in reports), "corpora": len(reports)},
        "interruption": interruption, "networkProcessPolicy": policy, "readOnly": True, "sourceTreeUnchanged": True, "machine": {"platform": platform.platform(), "python": platform.python_version(), "cpuCount": os.cpu_count()},
    }
    result["performanceComparison"] = performance_comparison(result, args.baseline.resolve() if args.baseline else None)
    result_path = Path(args.result or artifact_root / "react-frontend-pinned-result.json").resolve()
    result_path.parent.mkdir(parents=True, exist_ok=True)
    result["result"] = {"path": str(result_path), "sha256": hashlib.sha256(canonical(result)).hexdigest()}
    result_path.write_bytes(canonical(result))
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=ROOT / "tests/qualification/react-frontend-repositories.toml")
    parser.add_argument("--compass", type=Path, required=True)
    parser.add_argument("--artifact-root", type=Path)
    parser.add_argument("--result", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--audit-only", action="store_true", help="emit evidence without applying quality/performance thresholds")
    args = parser.parse_args()
    try:
        result = run(args)
        budget = result["qualityBudget"]
        aggregate = result["aggregate"]
        if not args.audit_only:
            if aggregate["oracleRecords"] < budget["minimumAccepted"]:
                fail(f"independent sample floor not met: {aggregate['oracleRecords']} < {budget['minimumAccepted']}")
            if aggregate["precision"] < budget["minimumPrecision"] or aggregate["recall"] < budget["minimumRecall"]:
                fail(f"frontend quality thresholds failed: {aggregate}")
            capability_failures = [
                {
                    "id": report["id"],
                    "capability": capability,
                    "metric": metric,
                }
                for report in result["repositories"]
                for capability, metric in report["capabilities"].items()
                if metric["precision"] < budget["minimumCapabilityPrecision"]
                or metric["recall"] < budget["minimumCapabilityRecall"]
            ]
            if capability_failures:
                fail(f"frontend capability thresholds failed: {capability_failures}")
            if aggregate["wilsonLower95"] < budget["minimumWilsonLower95"]:
                fail(f"frontend Wilson lower bound failed: {aggregate}")
            if not aggregate["zeroFabricatedTargets"]:
                fail("frontend scorecard contains fabricated targets")
            if not result["interruption"].get("cancellationObserved"):
                fail("interruption run completed before observing cancellation")
            if result["performanceComparison"].get("status") != "compared":
                fail("pinned qualification requires an approved --baseline for performance comparison")
            failed = [item for item in result["performanceComparison"]["comparisons"] if not item["secondsPass"] or not item["rssPass"]]
            if failed:
                fail(f"frontend performance regression exceeds budget: {failed}")
        print(json.dumps(result, sort_keys=True, separators=(",", ":")))
        return 0
    except (OSError, subprocess.SubprocessError, QualificationError) as error:
        print(f"react frontend pinned qualification failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
