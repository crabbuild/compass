#!/usr/bin/env python3
"""Record a deterministic, source-bounded universal-language baseline.

The recorder is qualification-only and accepts a caller-provided Compass
binary.  It runs cold, warm, forced, alternate-checkout, edit, and restore
publications, records graph/evidence digests, relation counts, diagnostics,
omissions, identity collisions, and child peak RSS, and never edits the
caller-provided source root.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import resource
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from typing import Any


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _rss_bytes() -> int | None:
    value = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    if value <= 0:
        return None
    return int(value if sys.platform == "darwin" else value * 1024)


def _active_graph(output: Path) -> Path:
    pointer = output / "compass-out" / "current-snapshot"
    snapshot = pointer.read_text(encoding="utf-8").strip()
    if not snapshot.startswith("snapshot-") or "/" in snapshot or "\\" in snapshot:
        raise RuntimeError(f"invalid Compass snapshot pointer: {snapshot!r}")
    graph = output / "compass-out" / "snapshots" / snapshot / "graph.json"
    if not graph.is_file():
        raise RuntimeError(f"Compass graph is missing: {graph}")
    return graph


def _publish(
    compass: Path,
    root: Path,
    output: Path,
    *,
    force: bool = False,
) -> tuple[float, Path, int | None]:
    command = [
        str(compass),
        "update",
        str(root),
        "--out",
        str(output),
        "--no-viz",
        "--no-cluster",
        "--inference-level",
        "max",
    ]
    if force:
        command.append("--force")
    started = time.perf_counter()
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
    )
    elapsed = time.perf_counter() - started
    if completed.returncode:
        raise RuntimeError(completed.stderr.strip() or completed.stdout.strip())
    return elapsed, _active_graph(output), _rss_bytes()


def _summary(graph: Path) -> dict[str, Any]:
    document = json.loads(graph.read_text(encoding="utf-8"))
    nodes = document.get("nodes", [])
    links = document.get("links", document.get("edges", []))
    relation_counts: dict[str, int] = {}
    for link in links:
        relation = link.get("kind", link.get("relation"))
        if isinstance(relation, str):
            relation_counts[relation] = relation_counts.get(relation, 0) + 1
    diagnostics = 0
    identity_collisions = 0
    omitted = 0
    for node in nodes:
        values = node.get("diagnostics", [])
        if isinstance(values, list):
            diagnostics += len(values)
            identity_collisions += sum(
                isinstance(item, dict)
                and item.get("code") in {"identity_collision", "ambiguous_identity"}
                for item in values
            )
    metadata = document.get("graph")
    if isinstance(metadata, dict):
        for key in ("omitted", "omittedFacts", "omissions"):
            value = metadata.get(key)
            if isinstance(value, int):
                omitted += value
            elif isinstance(value, list):
                omitted += len(value)
    return {
        "graphSha256": _sha256(graph),
        "evidenceDigest": (
            _sha256(graph.parent / "ast-fact-digests.json")
            if (graph.parent / "ast-fact-digests.json").is_file()
            else None
        ),
        "nodeCount": len(nodes),
        "edgeCount": len(links),
        "relationCounts": dict(sorted(relation_counts.items())),
        "diagnostics": diagnostics,
        "omittedFacts": omitted,
        "identityCollisions": identity_collisions,
    }


def _source_files(root: Path, language: str) -> list[Path]:
    suffixes = {
        "swift": {".swift"},
        "dart": {".dart"},
        "scala": {".scala"},
        "groovy": {".groovy", ".gradle"},
    }[language]
    return sorted(
        path
        for path in root.rglob("*")
        if path.is_file() and path.suffix.casefold() in suffixes
    )


def _semantic_marker(language: str) -> bytes:
    return {
        "swift": b"\n\nstruct CompassQualificationMarker {}\n",
        "dart": b"\n\nclass CompassQualificationMarker {}\n",
        "scala": b"\n\nclass CompassQualificationMarker\n",
        "groovy": b"\n\nclass CompassQualificationMarker {}\n",
    }[language]


def _neutral_marker(language: str) -> bytes:
    return {
        "swift": b"\n\n// compass-language-neutral\n",
        "dart": b"\n\n// compass-language-neutral\n",
        "scala": b"\n\n// compass-language-neutral\n",
        "groovy": b"\n\n// compass-language-neutral\n",
    }[language]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--language", required=True, choices=("swift", "dart", "scala", "groovy"))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--compass", type=Path, required=True)
    parser.add_argument(
        "--compass-revision",
        required=True,
        help="immutable source revision used to build the supplied binary",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=2)
    parser.add_argument(
        "--baseline-kind",
        choices=("established-pre-cutover", "post-cutover-fixture"),
        default="established-pre-cutover",
        help="provenance label for this baseline artifact",
    )
    parser.add_argument(
        "--baseline-status",
        default="reproduced from the caller-provided Compass binary",
        help="provenance statement stored in the artifact",
    )
    args = parser.parse_args()
    if args.samples < 2 or args.samples > 20:
        parser.error("--samples must be between 2 and 20")
    if not args.root.is_dir() or not args.compass.is_file():
        parser.error("--root and --compass must exist")
    with tempfile.TemporaryDirectory(prefix=f"compass-{args.language}-baseline-") as directory:
        base = Path(directory)
        workload = base / "workload"
        shutil.copytree(args.root.resolve(), workload, symlinks=True)
        source_files = _source_files(workload, args.language)
        if not source_files:
            raise RuntimeError(f"baseline root contains no {args.language} source files")

        first_output = base / "first"
        first_time, first_graph, first_rss = _publish(args.compass, workload, first_output)
        first_summary = _summary(first_graph)
        warm: list[dict[str, Any]] = []
        for _ in range(args.samples):
            elapsed, graph, rss = _publish(args.compass, workload, first_output)
            warm.append({"seconds": elapsed, "rssBytes": rss, **_summary(graph)})

        edited = source_files[0]
        original = edited.read_bytes()
        edited.write_bytes(original + _neutral_marker(args.language))
        neutral_time, neutral_graph, neutral_rss = _publish(
            args.compass, workload, first_output
        )
        neutral_summary = _summary(neutral_graph)

        edited.write_bytes(original + _semantic_marker(args.language))
        semantic_time, semantic_graph, semantic_rss = _publish(
            args.compass, workload, first_output
        )
        semantic_summary = _summary(semantic_graph)
        if semantic_summary["graphSha256"] == first_summary["graphSha256"]:
            raise RuntimeError("semantic edit did not change the published graph")

        edited.write_bytes(original)
        restore_time, restore_graph, restore_rss = _publish(
            args.compass, workload, first_output
        )
        restore_summary = _summary(restore_graph)
        if restore_summary["graphSha256"] != first_summary["graphSha256"]:
            raise RuntimeError("restored graph is not byte-identical to the cold graph")

        forced_output = base / "forced"
        forced_time, forced_graph, forced_rss = _publish(
            args.compass, workload, forced_output, force=True
        )
        forced_summary = _summary(forced_graph)
        if forced_summary["graphSha256"] != first_summary["graphSha256"]:
            raise RuntimeError("forced rebuild graph is not byte-identical to the cold graph")

        alternate_workload = base / "alternate-workload"
        alternate_output = base / "alternate"
        shutil.copytree(args.root.resolve(), alternate_workload, symlinks=True)
        alternate_time, alternate_graph, alternate_rss = _publish(
            args.compass, alternate_workload, alternate_output
        )
        alternate_summary = _summary(alternate_graph)
        if alternate_summary["graphSha256"] != first_summary["graphSha256"]:
            raise RuntimeError("alternate checkout graph is not byte-identical to the cold graph")

        second_output = base / "second"
        second_time, second_graph, second_rss = _publish(args.compass, workload, second_output)
        second_summary = _summary(second_graph)
        if first_summary["graphSha256"] != second_summary["graphSha256"]:
            raise RuntimeError("cold rebuild graph digest changed between independent outputs")
        if any(item["graphSha256"] != first_summary["graphSha256"] for item in warm):
            raise RuntimeError("warm graph digest changed")
        result = {
            "schema": "compass.universal-language-baseline/1",
            "language": args.language,
            "root": args.root.as_posix(),
            "baselineKind": args.baseline_kind,
            "baselineStatus": args.baseline_status,
            "compassRevision": args.compass_revision,
            "productionRoute": f"compass.{args.language}/1",
            "sourceFile": edited.relative_to(workload).as_posix(),
            "cold": {
                "medianSeconds": statistics.median((first_time, second_time)),
                "first": {"seconds": first_time, "rssBytes": first_rss, **first_summary},
                "second": {"seconds": second_time, "rssBytes": second_rss, **second_summary},
            },
            "factNeutral": {
                "seconds": neutral_time,
                "rssBytes": neutral_rss,
                **neutral_summary,
            },
            "semanticEdit": {
                "seconds": semantic_time,
                "rssBytes": semantic_rss,
                **semantic_summary,
            },
            "forced": {
                "seconds": forced_time,
                "rssBytes": forced_rss,
                **forced_summary,
            },
            "alternateCheckout": {
                "seconds": alternate_time,
                "rssBytes": alternate_rss,
                **alternate_summary,
            },
            "restore": {
                "seconds": restore_time,
                "rssBytes": restore_rss,
                **restore_summary,
            },
            "warm": {
                "samples": warm,
                "medianSeconds": statistics.median(
                    [item["seconds"] for item in warm]
                ),
                "graphSha256": first_summary["graphSha256"],
            },
            "editRestore": {
                "neutralGraphSha256": neutral_summary["graphSha256"],
                "semanticGraphSha256": semantic_summary["graphSha256"],
                "restoreGraphSha256": restore_summary["graphSha256"],
                "deterministic": True,
            },
            "performanceGates": {
                "coldMultiplier": 1.10,
                "warmMultiplier": 1.15,
                "factNeutralAdditiveSeconds": 0.25,
                "peakRssMultiplier": 1.15,
                "peakRssAdditiveBytes": 32 * 1024 * 1024,
            },
        }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"schema": result["schema"], "output": str(args.output), "graphSha256": result["cold"]["first"]["graphSha256"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
