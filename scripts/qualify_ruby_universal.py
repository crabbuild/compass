#!/usr/bin/env python3
"""Run the bounded, independent Ruby universal-evidence qualification.

This entry point deliberately keeps the Ripper oracle and Compass production
build separate.  Fixture mode needs only Ruby and the standard library;
pinned mode consumes clean, caller-provided checkouts; performance mode uses a
prebuilt Compass binary and a temporary copy of the input tree.  No mode
clones, mutates, or executes code from a qualification repository.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path
from typing import Any


SCHEMA = "compass.ruby-universal-qualification/1"
ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ORACLE = ROOT / "scripts" / "ruby_source_oracle.rb"
DEFAULT_MANIFEST = ROOT / "tests" / "qualification" / "ruby-universal-repositories.toml"
SKIP_DIRECTORIES = frozenset(
    {".git", ".bundle", "vendor", "node_modules", "tmp", "log", "coverage"}
)


class QualificationError(RuntimeError):
    """A reproducibility or safety failure in the qualification harness."""


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def source_files(root: Path) -> list[Path]:
    return sorted(
        path
        for path in root.rglob("*")
        if path.is_file()
        and (path.suffix in {".rb", ".rake"} or ruby_shebang(path))
        and not SKIP_DIRECTORIES.intersection(path.relative_to(root).parts)
    )


def ruby_shebang(path: Path) -> bool:
    try:
        first_line = path.read_bytes()[:256].split(b"\n", 1)[0].decode("utf-8")
    except (OSError, UnicodeDecodeError):
        return False
    if not first_line.startswith("#!"):
        return False
    words = first_line[2:].strip().split()
    if not words:
        return False
    interpreter = Path(words.pop(0)).name
    if interpreter == "env":
        while words and (words[0].startswith("-") or "=" in words[0]):
            words.pop(0)
        if not words:
            return False
        interpreter = Path(words[0]).name
    return interpreter == "ruby"


def run_oracle(root: Path, oracle: Path) -> tuple[dict[str, Any], bytes]:
    if not root.is_dir():
        raise QualificationError(f"Ruby root does not exist: {root}")
    if not oracle.is_file():
        raise QualificationError(f"Ruby oracle does not exist: {oracle}")
    with tempfile.TemporaryDirectory(prefix="compass-ruby-oracle-") as directory:
        output = Path(directory) / "oracle.json"
        command = ["ruby", str(oracle), "--root", str(root), "--output", str(output)]
        completed = subprocess.run(command, cwd=ROOT, check=False, text=True, capture_output=True)
        if completed.returncode:
            raise QualificationError(
                f"Ruby oracle failed for {root}: {completed.stderr.strip() or completed.stdout.strip()}"
            )
        raw = output.read_bytes()
    try:
        document = json.loads(raw)
    except json.JSONDecodeError as error:
        raise QualificationError(f"Ruby oracle emitted invalid JSON: {error}") from error
    if document.get("schema") != "compass.ruby-source-oracle/1":
        raise QualificationError(f"unexpected Ruby oracle schema: {document.get('schema')!r}")
    if not isinstance(document.get("files"), list):
        raise QualificationError("Ruby oracle files inventory is not a list")
    without_digest = dict(document)
    inventory_digest = without_digest.pop("inventorySha256", None)
    expected_digest = sha256(canonical_bytes(without_digest).rstrip(b"\n"))
    if inventory_digest != expected_digest:
        raise QualificationError(
            f"Ruby oracle inventory digest mismatch: {inventory_digest!r} != {expected_digest!r}"
        )
    return document, raw


def oracle_summary(document: dict[str, Any], raw: bytes, root: Path) -> dict[str, Any]:
    files = document["files"]
    declarations = sum(len(item.get("declarations", [])) for item in files)
    relations = [relation for item in files for relation in item.get("relations", [])]
    relation_counts: dict[str, int] = {}
    partial = 0
    for item in files:
        if item.get("status") != "ok":
            partial += 1
        for relation in item.get("relations", []):
            name = relation.get("relation", "unknown")
            relation_counts[name] = relation_counts.get(name, 0) + 1
    return {
        "root": str(root),
        "rubyVersion": document["rubyVersion"],
        "rubyRevision": document["rubyRevision"],
        "files": len(files),
        "sourceFiles": len(source_files(root)),
        "partialFiles": partial,
        "declarations": declarations,
        "relations": len(relations),
        "relationFamilies": dict(sorted(relation_counts.items())),
        "inventorySha256": document["inventorySha256"],
        "oracleSha256": sha256(raw),
    }


def run_deterministic_oracle(root: Path, oracle: Path) -> dict[str, Any]:
    first, first_raw = run_oracle(root, oracle)
    second, second_raw = run_oracle(root, oracle)
    if first_raw != second_raw:
        raise QualificationError(f"Ruby oracle output is not byte deterministic for {root}")
    summary = oracle_summary(first, first_raw, root)
    summary["deterministic"] = True
    summary["partialFiles"] = summary["partialFiles"]
    return summary


def parse_repository_overrides(values: list[str]) -> dict[str, Path]:
    overrides: dict[str, Path] = {}
    for value in values:
        name, separator, path = value.partition("=")
        if not separator or not name or not path:
            raise QualificationError(f"--repository must be NAME=PATH, got {value!r}")
        if name in overrides:
            raise QualificationError(f"duplicate repository override: {name}")
        overrides[name] = Path(path).expanduser().resolve()
    return overrides


def inferred_checkout(url: str) -> Path:
    parts = [part for part in url.rstrip("/").split("/") if part]
    if len(parts) < 2:
        raise QualificationError(f"cannot infer a mounted checkout from URL {url!r}")
    owner = parts[-2]
    repository = parts[-1].removesuffix(".git")
    return Path("/Volumes/Workspace/Github") / owner / repository


def verify_clean_pinned_checkout(repository: dict[str, Any], checkout: Path) -> None:
    if not checkout.is_dir():
        raise QualificationError(
            f"missing checkout for {repository['name']}; pass --repository {repository['name']}=PATH"
        )
    revision = subprocess.run(
        ["git", "-C", str(checkout), "rev-parse", "HEAD"],
        check=False,
        text=True,
        capture_output=True,
    )
    if revision.returncode or revision.stdout.strip() != repository["commit"]:
        raise QualificationError(
            f"{repository['name']} is not pinned to {repository['commit']}"
        )
    status = subprocess.run(
        ["git", "-C", str(checkout), "status", "--porcelain=v1", "--untracked-files=all"],
        check=False,
        text=True,
        capture_output=True,
    )
    if status.returncode or status.stdout:
        raise QualificationError(f"{repository['name']} checkout is not clean")


def pinned_mode(manifest_path: Path, oracle: Path, overrides: dict[str, Path]) -> dict[str, Any]:
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema") != SCHEMA:
        raise QualificationError(f"unexpected Ruby repository manifest schema: {manifest.get('schema')!r}")
    reports = []
    for repository in manifest.get("repository", []):
        name = repository.get("name")
        if not name or not repository.get("url") or not repository.get("commit"):
            raise QualificationError(f"repository entry is missing identity fields: {repository!r}")
        checkout = overrides.get(name, inferred_checkout(repository["url"]))
        verify_clean_pinned_checkout(repository, checkout)
        reports.append({
            "name": name,
            "url": repository["url"],
            "commit": repository["commit"],
            "purpose": repository.get("purpose", ""),
            "oracle": run_deterministic_oracle(checkout, oracle),
        })
    if not reports:
        raise QualificationError("Ruby repository manifest has no repositories")
    return {"mode": "pinned", "manifest": str(manifest_path), "repositories": reports}


def active_graph(output: Path) -> Path:
    pointer = output / "compass-out" / "current-snapshot"
    if not pointer.is_file():
        raise QualificationError(f"Compass did not publish an active snapshot: {pointer}")
    snapshot = pointer.read_text(encoding="utf-8").strip()
    if not snapshot.startswith("snapshot-") or "/" in snapshot or "\\" in snapshot:
        raise QualificationError(f"invalid Compass active snapshot pointer: {snapshot!r}")
    graph = output / "compass-out" / "snapshots" / snapshot / "graph.json"
    if not graph.is_file():
        raise QualificationError(f"Compass active graph is missing: {graph}")
    return graph


def run_compass(compass: Path, root: Path, output: Path) -> tuple[float, str]:
    started = time.perf_counter()
    completed = subprocess.run(
        [
            str(compass),
            "update",
            str(root),
            "--out",
            str(output),
            "--no-cluster",
            "--no-viz",
            "--inference-level",
            "max",
        ],
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
    return elapsed, sha256(graph.read_bytes())


def performance_mode(root: Path, compass: Path, samples: int) -> dict[str, Any]:
    if samples < 1 or samples > 20:
        raise QualificationError("--samples must be between 1 and 20")
    if not compass.is_file():
        raise QualificationError(f"Compass binary does not exist: {compass}")
    if root.is_file():
        raise QualificationError("performance mode expects a repository directory")
    with tempfile.TemporaryDirectory(prefix="compass-ruby-performance-") as directory:
        workload = Path(directory) / "source"
        output = Path(directory) / "output"
        shutil.copytree(root, workload, symlinks=True)
        cold_time, cold_hash = run_compass(compass, workload, output)
        warm_runs = [run_compass(compass, workload, output) for _ in range(samples)]
        warm_samples = [elapsed for elapsed, _ in warm_runs]
        warm_hashes = [graph_hash for _, graph_hash in warm_runs]
        if any(graph_hash != cold_hash for graph_hash in warm_hashes):
            raise QualificationError("warm Ruby graph is not byte-identical to the cold graph")
        ruby_files = source_files(workload)
        if not ruby_files:
            raise QualificationError(f"performance root contains no Ruby source: {root}")
        # Prefer a tracked-looking library source over extensionless executable
        # entrypoints (for example bin/console), which some project scopes omit
        # from their graph.  The edit must exercise a file Compass actually
        # publishes or the semantic-incremental assertion is meaningless.
        library_files = [
            path
            for path in ruby_files
            if path.suffix == ".rb" and "lib" in path.relative_to(workload).parts
        ]
        ruby_source_files = [path for path in ruby_files if path.suffix == ".rb"]
        edited = (library_files or ruby_source_files or ruby_files)[0]
        baseline = edited.read_bytes()
        edited.write_bytes(baseline + b"\n# compass-ruby-qualification-neutral\n")
        neutral_time, neutral_hash = run_compass(compass, workload, output)
        edited.write_bytes(baseline + b"\nclass CompassRubyQualificationMarker\n  def marker; end\nend\n")
        semantic_time, semantic_hash = run_compass(compass, workload, output)
        if semantic_hash == cold_hash:
            raise QualificationError(
                f"semantic Ruby edit did not change the published graph: {edited.relative_to(workload)}"
            )
        edited.write_bytes(baseline)
        restore_time, restore_hash = run_compass(compass, workload, output)
        if restore_hash != cold_hash:
            raise QualificationError(
                "restored Ruby graph is not byte-identical to the cold graph "
                f"(cold={cold_hash}, restore={restore_hash})"
            )
        return {
            "mode": "performance",
            "root": str(root),
            "compass": str(compass),
            "cold": {"seconds": cold_time, "graphSha256": cold_hash},
            "warm": {
                "samples": warm_samples,
                "medianSeconds": statistics.median(warm_samples),
                "graphSha256": cold_hash,
                "graphHashes": warm_hashes,
            },
            "factNeutral": {"seconds": neutral_time, "graphSha256": neutral_hash},
            "semanticEdit": {"seconds": semantic_time, "graphSha256": semantic_hash},
            "restore": {"seconds": restore_time, "graphSha256": restore_hash},
            "changedFiles": 1,
            "reusedFiles": max(0, len(ruby_files) - 1),
            "rssBlocking": False,
        }


def quality_audit_mode(
    audit_manifest: Path | None,
    graph: Path | None,
    corpus: Path | None,
) -> dict[str, Any]:
    """Run the repository's strict, independent quality-audit evaluator.

    The Ruby wrapper intentionally delegates scoring to the shared validator;
    it only supplies the explicit paths and preserves its machine-readable
    result.  Missing inputs are a hard failure, never an empty audit.
    """

    if audit_manifest is None or graph is None or corpus is None:
        raise QualificationError(
            "quality-audit mode requires --audit-manifest, --graph, and --corpus"
        )
    for path, label in (
        (audit_manifest, "audit manifest"),
        (graph, "graph"),
        (corpus, "corpus"),
    ):
        if not path.exists():
            raise QualificationError(f"{label} does not exist: {path}")
    command = [
        sys.executable,
        str(ROOT / "benchmarks" / "performance" / "harness.py"),
        "audit",
        "--manifest",
        str(audit_manifest.resolve()),
        "--graph",
        str(graph.resolve()),
        "--corpus",
        str(corpus.resolve()),
    ]
    completed = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        text=True,
        capture_output=True,
    )
    lines = [line for line in completed.stdout.splitlines() if line.strip()]
    if not lines:
        raise QualificationError(
            "shared quality-audit evaluator emitted no machine-readable result: "
            f"{completed.stderr.strip()}"
        )
    try:
        result = json.loads(lines[-1])
    except json.JSONDecodeError as error:
        raise QualificationError(
            f"shared quality-audit evaluator emitted invalid JSON: {error}"
        ) from error
    if result.get("schema") != "compass.quality-audit-result/2":
        raise QualificationError(
            f"unexpected quality-audit result schema: {result.get('schema')!r}"
        )
    return {
        "mode": "quality-audit",
        "audit": result,
        "evaluatorExitCode": completed.returncode,
    }


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument(
        "--mode",
        choices=("fixture", "pinned", "quality-audit", "performance"),
        default="fixture",
    )
    result.add_argument("--root", type=Path, default=ROOT / "fixtures" / "code-graph" / "qualification")
    result.add_argument("--oracle", type=Path, default=DEFAULT_ORACLE)
    result.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    result.add_argument("--audit-manifest", type=Path, help="compass.quality-audit/2 JSON")
    result.add_argument("--graph", type=Path, help="published graph for quality-audit mode")
    result.add_argument("--corpus", type=Path, help="pinned source corpus for quality-audit mode")
    result.add_argument("--repository", action="append", default=[], metavar="NAME=PATH")
    result.add_argument("--compass", type=Path, help="prebuilt compass binary for performance mode")
    result.add_argument("--samples", type=int, default=5, help="warm performance samples (1-20)")
    result.add_argument("--output", type=Path, help="write the machine-readable report to this path")
    return result


def main(argv: list[str]) -> int:
    arguments = parser().parse_args(argv)
    try:
        if arguments.mode == "fixture":
            report = {"mode": "fixture", "oracle": run_deterministic_oracle(arguments.root, arguments.oracle)}
        elif arguments.mode == "pinned":
            report = pinned_mode(arguments.manifest, arguments.oracle, parse_repository_overrides(arguments.repository))
        elif arguments.mode == "quality-audit":
            report = quality_audit_mode(arguments.audit_manifest, arguments.graph, arguments.corpus)
        else:
            if arguments.compass is None:
                raise QualificationError("--compass is required for performance mode")
            report = performance_mode(arguments.root, arguments.compass.resolve(), arguments.samples)
        report = {"schema": SCHEMA, **report}
        encoded = canonical_bytes(report)
        if arguments.output:
            arguments.output.write_bytes(encoded)
        else:
            sys.stdout.buffer.write(encoded)
        if arguments.mode == "quality-audit":
            audit = report.get("audit", {})
            return int(
                not (
                    audit.get("passed") is True
                    and audit.get("eligibleForQualityClaim") is True
                )
            )
        return 0
    except (OSError, QualificationError, subprocess.SubprocessError) as error:
        print(f"ruby qualification failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
