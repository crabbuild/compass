#!/usr/bin/env python3
"""Shared qualification harness for the Swift/Dart/Scala/Groovy wave.

Language entry points provide their manifest, oracle, and source suffixes.  All
modes are fail-closed and consume caller-provided checkouts; the harness never
clones, updates, builds, or executes a qualification repository.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import tomllib
from typing import Any

try:
    import resource
except ImportError:  # pragma: no cover - Windows has no resource module.
    resource = None


ROOT = Path(__file__).resolve().parents[1]
MOUNTED_ROOT = Path("/Volumes/Workspace/Github").resolve()


class QualificationError(RuntimeError):
    """A reproducibility, safety, or contract failure."""


def canonical_bytes(value: Any) -> bytes:
    return (
        json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
        + "\n"
    ).encode("utf-8")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run_oracle(
    oracle: Path,
    root: Path,
    options: tuple[str, ...] = (),
) -> tuple[dict[str, Any], bytes]:
    if not oracle.is_file():
        raise QualificationError(f"source oracle does not exist: {oracle}")
    with tempfile.TemporaryDirectory(prefix="compass-language-oracle-") as directory:
        output = Path(directory) / "oracle.json"
        completed = subprocess.run(
            [
                sys.executable,
                str(oracle),
                "--root",
                str(root),
                "--output",
                str(output),
                *options,
            ],
            cwd=ROOT,
            check=False,
            text=True,
            capture_output=True,
        )
        if completed.returncode:
            raise QualificationError(
                f"source oracle failed for {root}: "
                f"{completed.stderr.strip() or completed.stdout.strip()}"
            )
        raw = output.read_bytes()
    try:
        document = json.loads(raw)
    except json.JSONDecodeError as error:
        raise QualificationError(f"source oracle emitted invalid JSON: {error}") from error
    if not isinstance(document, dict) or not document.get("schema", "").endswith(
        "-source-oracle/1"
    ):
        raise QualificationError(f"unexpected source-oracle schema: {document.get('schema')!r}")
    files = document.get("files")
    if not isinstance(files, list):
        raise QualificationError("source oracle files inventory is not a list")
    paths: list[str] = []
    for index, item in enumerate(files):
        if not isinstance(item, dict):
            raise QualificationError(f"source oracle file {index} is not an object")
        path = item.get("path")
        if (
            not isinstance(path, str)
            or not path
            or Path(path).is_absolute()
            or "\\" in path
            or path == "."
            or path.startswith("../")
            or "/../" in f"/{path}"
        ):
            raise QualificationError(f"source oracle file {index} has an unsafe path")
        if item.get("status") not in {"ok", "partial"}:
            raise QualificationError(f"source oracle file {path!r} has an invalid status")
        paths.append(path)
    if paths != sorted(set(paths)):
        raise QualificationError("source oracle files are not unique and deterministically ordered")
    inventory_digest = document.get("inventorySha256")
    for field in ("language", "provider", "toolchain", "implementation"):
        if not isinstance(document.get(field), str) or not document[field].strip():
            raise QualificationError(f"source oracle {field} identity is missing")
    if not isinstance(document.get("parserAvailable"), bool):
        raise QualificationError("source oracle parserAvailable must be boolean")
    inventory = {
        "language": document.get("language"),
        "provider": document.get("provider"),
        "toolchain": document.get("toolchain"),
        "rootRelativeFiles": paths,
        "files": files,
    }
    expected_digest = sha256(canonical_bytes(inventory).rstrip(b"\n"))
    if inventory_digest != expected_digest:
        raise QualificationError(
            f"source inventory digest mismatch: {inventory_digest!r} != {expected_digest!r}"
        )
    if document.get("parsedFiles", 0) + document.get("partialFiles", 0) != document.get(
        "scannedFiles"
    ):
        raise QualificationError("source oracle coverage counts do not add up")
    return document, raw


def deterministic_oracle(
    oracle: Path,
    root: Path,
    options: tuple[str, ...] = (),
) -> dict[str, Any]:
    first, first_raw = run_oracle(oracle, root, options)
    second, second_raw = run_oracle(oracle, root, options)
    if first_raw != second_raw:
        raise QualificationError(f"source oracle output is not byte deterministic for {root}")
    return {
        "provider": first["provider"],
        "toolchain": first["toolchain"],
        "implementation": first.get("implementation"),
        "parserAvailable": first.get("parserAvailable", False),
        "scannedFiles": first["scannedFiles"],
        "parsedFiles": first["parsedFiles"],
        "partialFiles": first["partialFiles"],
        "inventorySha256": first["inventorySha256"],
        "oracleSha256": sha256(first_raw),
        "deterministic": True,
    }


def inferred_checkout(url: str) -> Path:
    parts = [part for part in url.rstrip("/").split("/") if part]
    if len(parts) < 2:
        raise QualificationError(f"cannot infer mounted checkout from URL {url!r}")
    return MOUNTED_ROOT / parts[-2] / parts[-1].removesuffix(".git")


def verify_clean_pinned_checkout(repository: dict[str, Any], checkout: Path) -> None:
    checkout = checkout.resolve()
    try:
        checkout.relative_to(MOUNTED_ROOT)
    except ValueError as error:
        raise QualificationError(
            f"{repository.get('name')} checkout must live under {MOUNTED_ROOT}"
        ) from error
    if not checkout.is_dir():
        raise QualificationError(
            f"missing checkout for {repository.get('name')}; expected {checkout}"
        )
    revision = subprocess.run(
        ["git", "-C", str(checkout), "rev-parse", "HEAD"],
        check=False,
        text=True,
        capture_output=True,
    )
    if revision.returncode or revision.stdout.strip() != repository.get("commit"):
        raise QualificationError(
            f"{repository.get('name')} is not pinned to {repository.get('commit')}"
        )
    status = subprocess.run(
        ["git", "-C", str(checkout), "status", "--porcelain=v1", "--untracked-files=all"],
        check=False,
        text=True,
        capture_output=True,
    )
    if status.returncode or status.stdout:
        raise QualificationError(f"{repository.get('name')} checkout is not clean")


def parse_overrides(values: list[str]) -> dict[str, Path]:
    overrides: dict[str, Path] = {}
    for value in values:
        name, separator, raw_path = value.partition("=")
        if not separator or not name or not raw_path:
            raise QualificationError(f"--repository must be NAME=PATH, got {value!r}")
        if name in overrides:
            raise QualificationError(f"duplicate repository override: {name}")
        overrides[name] = Path(raw_path).expanduser().resolve()
    return overrides


def load_manifest(path: Path, schema: str) -> dict[str, Any]:
    try:
        manifest = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise QualificationError(f"invalid qualification manifest {path}: {error}") from error
    if manifest.get("schema") != schema:
        raise QualificationError(f"manifest schema must be {schema!r}")
    entries = manifest.get("repository")
    if not isinstance(entries, list) or not entries:
        raise QualificationError("qualification manifest must contain repositories")
    if manifest.get("checkoutRoot") != str(MOUNTED_ROOT):
        raise QualificationError(
            f"qualification manifest checkoutRoot must be {MOUNTED_ROOT}"
        )
    if manifest.get("readOnly") is not True:
        raise QualificationError("qualification manifest must declare readOnly = true")
    for field in ("oracleProvider", "oracleToolchain"):
        if not isinstance(manifest.get(field), str) or not manifest[field].strip():
            raise QualificationError(f"qualification manifest must declare {field}")
    names: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict) or not entry.get("name") or not entry.get("url"):
            raise QualificationError(f"repository entry is missing identity fields: {entry!r}")
        if entry["name"] in names:
            raise QualificationError(f"duplicate repository name: {entry['name']}")
        names.add(entry["name"])
        commit = entry.get("commit", "")
        if not isinstance(commit, str) or len(commit) != 40 or any(
            character not in "0123456789abcdef" for character in commit.casefold()
        ):
            raise QualificationError(f"{entry['name']} must use a full 40-hex commit SHA")
        if not isinstance(entry.get("sourceGlobs"), list) or not entry["sourceGlobs"]:
            raise QualificationError(f"{entry['name']} must declare sourceGlobs")
        if any(not isinstance(pattern, str) or not pattern for pattern in entry["sourceGlobs"]):
            raise QualificationError(f"{entry['name']} sourceGlobs must be non-empty strings")
        if not isinstance(entry.get("excludeGlobs", []), list) or any(
            not isinstance(pattern, str) or not pattern
            for pattern in entry.get("excludeGlobs", [])
        ):
            raise QualificationError(f"{entry['name']} excludeGlobs must be non-empty strings")
    return manifest


def fixture_mode(manifest: dict[str, Any], oracle: Path, root: Path) -> dict[str, Any]:
    if not root.is_dir():
        raise QualificationError(f"fixture root does not exist: {root}")
    result = deterministic_oracle(oracle, root)
    if result["scannedFiles"] == 0:
        raise QualificationError(f"fixture root contains no source files for {manifest['language']}")
    return {"mode": "fixture", "root": str(root), "oracle": result}


def pinned_mode(
    manifest_path: Path,
    manifest: dict[str, Any],
    oracle: Path,
    overrides: dict[str, Path],
) -> dict[str, Any]:
    reports: list[dict[str, Any]] = []
    for repository in manifest["repository"]:
        checkout = overrides.get(repository["name"], inferred_checkout(repository["url"]))
        verify_clean_pinned_checkout(repository, checkout)
        oracle_options = tuple(
            argument
            for pattern in repository.get("sourceGlobs", [])
            for argument in ("--include", pattern)
        ) + tuple(
            argument
            for pattern in repository.get("excludeGlobs", [])
            for argument in ("--exclude", pattern)
        )
        reports.append(
            {
                "name": repository["name"],
                "url": repository["url"],
                "commit": repository["commit"],
                "purpose": repository.get("purpose", ""),
                "sourceGlobs": repository["sourceGlobs"],
                "excludeGlobs": repository.get("excludeGlobs", []),
                "oracle": deterministic_oracle(oracle, checkout, oracle_options),
            }
        )
    return {"mode": "pinned", "manifest": str(manifest_path), "repositories": reports}


def active_graph(output: Path) -> Path:
    pointer = output / "compass-out" / "current-snapshot"
    if not pointer.is_file():
        raise QualificationError(f"Compass did not publish an active snapshot: {pointer}")
    snapshot = pointer.read_text(encoding="utf-8").strip()
    if not snapshot.startswith("snapshot-") or "/" in snapshot or "\\" in snapshot:
        raise QualificationError(f"invalid active snapshot pointer: {snapshot!r}")
    graph = output / "compass-out" / "snapshots" / snapshot / "graph.json"
    if not graph.is_file():
        raise QualificationError(f"active graph is missing: {graph}")
    return graph


def _children_peak_rss_bytes() -> int | None:
    if resource is None:
        return None
    usage = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    if usage <= 0:
        return None
    # macOS reports bytes; Linux and the other Unix implementations report KiB.
    return int(usage if sys.platform == "darwin" else usage * 1024)


def run_compass(
    compass: Path,
    root: Path,
    output: Path,
    *,
    force: bool = False,
) -> tuple[float, str, int | None]:
    command = [
        str(compass),
        "update",
        str(root),
        "--out",
        str(output),
        "--no-cluster",
        "--no-viz",
        "--inference-level",
        "max",
    ]
    if force:
        command.append("--force")
    started = time.perf_counter()
    completed = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        text=True,
        capture_output=True,
    )
    elapsed = time.perf_counter() - started
    if completed.returncode:
        raise QualificationError(
            f"Compass update failed: {completed.stderr.strip() or completed.stdout.strip()}"
        )
    graph = active_graph(output)
    return elapsed, sha256(graph.read_bytes()), _children_peak_rss_bytes()


def performance_mode(root: Path, compass: Path, samples: int) -> dict[str, Any]:
    if samples < 1 or samples > 20:
        raise QualificationError("--samples must be between 1 and 20")
    if not compass.is_file():
        raise QualificationError(f"Compass binary does not exist: {compass}")
    with tempfile.TemporaryDirectory(prefix="compass-language-performance-") as directory:
        workload = Path(directory) / "source"
        output = Path(directory) / "output"
        shutil.copytree(root, workload, symlinks=True)
        cold_time, cold_hash, cold_rss = run_compass(compass, workload, output)
        warm_runs = [run_compass(compass, workload, output) for _ in range(samples)]
        warm_times = [elapsed for elapsed, _, _ in warm_runs]
        if any(graph_hash != cold_hash for _, graph_hash, _ in warm_runs):
            raise QualificationError("warm graph is not byte-identical to the cold graph")
        source_files = sorted(
            path for path in workload.rglob("*") if path.is_file() and path.suffix.casefold() in {".swift", ".dart", ".scala", ".groovy", ".gradle"}
        )
        if not source_files:
            raise QualificationError("performance root contains no supported source")
        edited = source_files[0]
        baseline = edited.read_bytes()
        edited.write_bytes(baseline + b"\n// compass-language-neutral\n")
        neutral_time, neutral_hash, neutral_rss = run_compass(compass, workload, output)
        edited.write_bytes(baseline + b"\n// compass-language-semantic-marker\nclass CompassQualificationMarker {}\n")
        semantic_time, semantic_hash, semantic_rss = run_compass(compass, workload, output)
        if semantic_hash == cold_hash:
            raise QualificationError("semantic edit did not change the published graph")
        edited.write_bytes(baseline)
        restore_time, restore_hash, restore_rss = run_compass(compass, workload, output)
        if restore_hash != cold_hash:
            raise QualificationError("restored graph is not byte-identical to cold graph")

        forced_time, forced_hash, forced_rss = run_compass(
            compass, workload, output, force=True
        )
        if forced_hash != cold_hash:
            raise QualificationError("forced graph is not byte-identical to cold graph")

        alternate_workload = Path(directory) / "alternate-source"
        alternate_output = Path(directory) / "alternate-output"
        shutil.copytree(root, alternate_workload, symlinks=True)
        alternate_time, alternate_hash, alternate_rss = run_compass(
            compass, alternate_workload, alternate_output
        )
        if alternate_hash != cold_hash:
            raise QualificationError(
                "alternate-checkout graph is not byte-identical to cold graph"
            )

        deleted_path = source_files[0]
        deleted_bytes = deleted_path.read_bytes()
        deleted_path.unlink()
        delete_time, delete_hash, delete_rss = run_compass(compass, workload, output)
        deleted_path.write_bytes(deleted_bytes)
        delete_restore_time, delete_restore_hash, delete_restore_rss = run_compass(
            compass, workload, output
        )
        if delete_restore_hash != cold_hash:
            raise QualificationError(
                "delete/restore graph is not byte-identical to cold graph"
            )

        renamed_path = source_files[0]
        renamed_target = renamed_path.with_name(
            f"{renamed_path.stem}.compass-renamed{renamed_path.suffix}"
        )
        renamed_path.rename(renamed_target)
        rename_time, rename_hash, rename_rss = run_compass(compass, workload, output)
        renamed_target.rename(renamed_path)
        rename_restore_time, rename_restore_hash, rename_restore_rss = run_compass(
            compass, workload, output
        )
        if rename_restore_hash != cold_hash:
            raise QualificationError(
                "rename/restore graph is not byte-identical to cold graph"
            )
        report = {
            "mode": "performance",
            "root": str(root),
            "compass": str(compass),
            "cold": {"seconds": cold_time, "graphSha256": cold_hash},
            "warm": {
                "samples": warm_times,
                "medianSeconds": statistics.median(warm_times),
                "graphSha256": cold_hash,
            },
            "factNeutral": {"seconds": neutral_time, "graphSha256": neutral_hash},
            "semanticEdit": {"seconds": semantic_time, "graphSha256": semantic_hash},
            "restore": {"seconds": restore_time, "graphSha256": restore_hash},
            "forced": {"seconds": forced_time, "graphSha256": forced_hash},
            "alternateCheckout": {
                "seconds": alternate_time,
                "graphSha256": alternate_hash,
            },
            "delete": {"seconds": delete_time, "graphSha256": delete_hash},
            "deleteRestore": {
                "seconds": delete_restore_time,
                "graphSha256": delete_restore_hash,
            },
            "rename": {"seconds": rename_time, "graphSha256": rename_hash},
            "renameRestore": {
                "seconds": rename_restore_time,
                "graphSha256": rename_restore_hash,
            },
            "changedFiles": 1,
            "reusedFiles": max(0, len(source_files) - 1),
            "peakRssBytes": max(
                (
                    value
                    for value in (
                        cold_rss,
                        *(rss for _, _, rss in warm_runs),
                        neutral_rss,
                        semantic_rss,
                        restore_rss,
                        forced_rss,
                        alternate_rss,
                        delete_rss,
                        delete_restore_rss,
                        rename_rss,
                        rename_restore_rss,
                    )
                    if value is not None
                ),
                default=None,
            ),
            "rssNote": "Peak child RSS sampled from the qualification process; null only when the platform exposes no resource sampler.",
        }
        return report


def compare_performance_baseline(
    report: dict[str, Any], baseline_path: Path
) -> dict[str, Any]:
    try:
        baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
        cold = float(
            baseline.get("cold", {}).get(
                "medianSeconds",
                statistics.median(
                    [
                        baseline["cold"]["first"]["seconds"],
                        baseline["cold"]["second"]["seconds"],
                    ]
                ),
            )
        )
        warm = float(
            baseline.get("warm", {}).get(
                "medianSeconds",
                statistics.median(
                    item["seconds"] for item in baseline["warm"]["samples"]
                ),
            )
        )
        neutral = float(baseline["factNeutral"]["seconds"])
        rss_values = [
            value
            for section in (
                baseline.get("cold", {}).get("first", {}),
                baseline.get("cold", {}).get("second", {}),
                baseline.get("factNeutral", {}),
                baseline.get("semanticEdit", {}),
                baseline.get("forced", {}),
                baseline.get("alternateCheckout", {}),
                baseline.get("restore", {}),
                *baseline.get("warm", {}).get("samples", []),
            )
            if (value := section.get("rssBytes")) is not None
        ]
        baseline_rss = max(rss_values) if rss_values else None
    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        raise QualificationError(
            f"invalid universal-language performance baseline {baseline_path}: {error}"
        ) from error

    gates = baseline.get("performanceGates", {})
    cold_limit = max(cold * float(gates.get("coldMultiplier", 1.10)), cold + 1.0)
    warm_limit = max(warm * float(gates.get("warmMultiplier", 1.15)), warm + 0.1)
    neutral_limit = max(
        neutral * float(gates.get("warmMultiplier", 1.15)),
        neutral + float(gates.get("factNeutralAdditiveSeconds", 0.25)),
    )
    rss_limit = None
    if baseline_rss is not None and report.get("peakRssBytes") is not None:
        rss_limit = max(
            baseline_rss * float(gates.get("peakRssMultiplier", 1.15)),
            baseline_rss + int(gates.get("peakRssAdditiveBytes", 32 * 1024 * 1024)),
        )
    observed = {
        "coldSeconds": report["cold"]["seconds"],
        "warmMedianSeconds": report["warm"]["medianSeconds"],
        "factNeutralSeconds": report["factNeutral"]["seconds"],
        "peakRssBytes": report.get("peakRssBytes"),
    }
    limits = {
        "coldSeconds": cold_limit,
        "warmMedianSeconds": warm_limit,
        "factNeutralSeconds": neutral_limit,
        "peakRssBytes": rss_limit,
    }
    passed = (
        observed["coldSeconds"] <= cold_limit
        and observed["warmMedianSeconds"] <= warm_limit
        and observed["factNeutralSeconds"] <= neutral_limit
        and (
            rss_limit is None
            or observed["peakRssBytes"] is None
            or observed["peakRssBytes"] <= rss_limit
        )
    )
    return {
        "baseline": str(baseline_path),
        "baselineRevision": baseline.get("compassRevision"),
        "passed": passed,
        "limits": limits,
        "observed": observed,
    }


def quality_audit_mode(
    audit_manifest: Path | None,
    graph: Path | None,
    corpus: Path | None,
) -> dict[str, Any]:
    if audit_manifest is None or graph is None or corpus is None:
        raise QualificationError("quality-audit mode requires --audit-manifest, --graph, and --corpus")
    for path, label in ((audit_manifest, "audit manifest"), (graph, "graph"), (corpus, "corpus")):
        if not path.exists():
            raise QualificationError(f"{label} does not exist: {path}")
    completed = subprocess.run(
        [
            sys.executable,
            str(ROOT / "benchmarks" / "performance" / "harness.py"),
            "audit",
            "--manifest",
            str(audit_manifest.resolve()),
            "--graph",
            str(graph.resolve()),
            "--corpus",
            str(corpus.resolve()),
        ],
        cwd=ROOT,
        check=False,
        text=True,
        capture_output=True,
    )
    lines = [line for line in completed.stdout.splitlines() if line.strip()]
    if not lines:
        raise QualificationError(
            "quality-audit evaluator emitted no result: "
            f"{completed.stderr.strip()}"
        )
    try:
        result = json.loads(lines[-1])
    except json.JSONDecodeError as error:
        raise QualificationError(f"quality-audit evaluator emitted invalid JSON: {error}") from error
    if result.get("schema") != "compass.quality-audit-result/2":
        raise QualificationError(f"unexpected quality-audit result schema: {result.get('schema')!r}")
    return {"mode": "quality-audit", "audit": result, "evaluatorExitCode": completed.returncode}


def run_cli(
    argv: list[str],
    *,
    language: str,
    manifest_path: Path,
    oracle_path: Path,
    fixture_root: Path,
) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=("fixture", "pinned", "quality-audit", "performance"), default="fixture")
    parser.add_argument("--root", type=Path, default=fixture_root)
    parser.add_argument("--manifest", type=Path, default=manifest_path)
    parser.add_argument("--oracle", type=Path, default=oracle_path)
    parser.add_argument("--repository", action="append", default=[], metavar="NAME=PATH")
    parser.add_argument("--audit-manifest", type=Path)
    parser.add_argument("--graph", type=Path)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--compass", type=Path)
    parser.add_argument(
        "--baseline",
        type=Path,
        default=ROOT / "tests/qualification" / f"{language}-universal-baseline.json",
        help="established-path performance baseline (use an explicit path to override)",
    )
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        schema = f"compass.{language}-universal-qualification/1"
        manifest = load_manifest(args.manifest, schema)
        if manifest.get("language") != language:
            raise QualificationError("manifest language does not match entry point")
        if args.mode == "fixture":
            report = fixture_mode(manifest, args.oracle, args.root)
        elif args.mode == "pinned":
            report = pinned_mode(args.manifest, manifest, args.oracle, parse_overrides(args.repository))
        elif args.mode == "quality-audit":
            report = quality_audit_mode(args.audit_manifest, args.graph, args.corpus)
        else:
            if args.compass is None:
                raise QualificationError("--compass is required for performance mode")
            report = performance_mode(args.root, args.compass.resolve(), args.samples)
            if args.baseline is not None:
                comparison = compare_performance_baseline(report, args.baseline.resolve())
                report["baselineComparison"] = comparison
                if not comparison["passed"]:
                    raise QualificationError(
                        f"performance gates failed for {language}: {comparison}"
                    )
        encoded = canonical_bytes({"schema": schema, **report})
        if args.output:
            args.output.write_bytes(encoded)
        else:
            sys.stdout.buffer.write(encoded)
        if args.mode == "quality-audit":
            audit = report.get("audit", {})
            return int(not (audit.get("passed") is True and audit.get("eligibleForQualityClaim") is True))
        return 0
    except (OSError, QualificationError, subprocess.SubprocessError) as error:
        print(f"{language} qualification failed: {error}", file=sys.stderr)
        return 1
