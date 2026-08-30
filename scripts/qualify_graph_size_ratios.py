#!/usr/bin/env python3
"""Measure admitted-source to canonical-graph expansion without loading graphs."""

from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path


SCHEMA = "compass.qualification.graph-size-ratios/1"
DEFAULT_METADATA_LIMIT = 256 * 1024 * 1024
NODES_MARKER = b',"nodes":['
BYTE_SIZE = re.compile(rb'"byteSize":([0-9]+)')


@dataclass(frozen=True)
class Estate:
    name: str
    graph: Path


def parse_estate(values: list[str]) -> Estate:
    name, graph = values
    if not name or any(character.isspace() for character in name):
        raise argparse.ArgumentTypeError("estate NAME must be non-empty and contain no whitespace")
    return Estate(name=name, graph=Path(graph).resolve())


def read_metadata_prefix(path: Path, maximum: int) -> bytes:
    prefix = bytearray()
    with path.open("rb") as stream:
        while len(prefix) <= maximum:
            block = stream.read(min(1024 * 1024, maximum + 1 - len(prefix)))
            if not block:
                break
            prefix.extend(block)
            marker = prefix.find(NODES_MARKER)
            if marker >= 0:
                return bytes(prefix[:marker])
    raise ValueError(
        f"{path}: canonical graph metadata did not end within the "
        f"{maximum}-byte qualification bound"
    )


def measure(estate: Estate, maximum: int) -> dict[str, object]:
    if not estate.graph.is_file():
        raise ValueError(f"{estate.graph}: graph artifact does not exist")
    prefix = read_metadata_prefix(estate.graph, maximum)
    sizes = [int(match.group(1)) for match in BYTE_SIZE.finditer(prefix)]
    if not sizes:
        raise ValueError(f"{estate.graph}: canonical graph metadata has no admitted files")
    source_bytes = sum(sizes)
    graph_bytes = estate.graph.stat().st_size
    if source_bytes <= 0 or graph_bytes <= 0:
        raise ValueError(f"{estate.graph}: byte measurements must be positive")
    return {
        "estate": estate.name,
        "graph": str(estate.graph),
        "admittedFiles": len(sizes),
        "admittedSourceBytes": source_bytes,
        "canonicalGraphBytes": graph_bytes,
        "graphToSourceRatio": graph_bytes / source_bytes,
    }


def render_markdown(report: dict[str, object]) -> str:
    rows = report["estates"]
    summary = report["summary"]
    lines = [
        "# Canonical graph size qualification",
        "",
        "The ratio is measured from each canonical artifact's admitted file inventory; "
        "it is diagnostic evidence, not a publication predictor.",
        "",
        "| Estate | Files | Admitted source bytes | Canonical graph bytes | Ratio |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for row in rows:
        lines.append(
            f"| {row['estate']} | {row['admittedFiles']:,} | "
            f"{row['admittedSourceBytes']:,} | {row['canonicalGraphBytes']:,} | "
            f"{row['graphToSourceRatio']:.6f}x |"
        )
    lines.extend(
        [
            "",
            f"Distribution: minimum **{summary['minimumRatio']:.6f}x**, "
            f"median **{summary['medianRatio']:.6f}x**, and "
            f"maximum **{summary['maximumRatio']:.6f}x**.",
            "",
        ]
    )
    return "\n".join(lines)


def write_atomic(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def main() -> int:
    parser = argparse.ArgumentParser(
        description="measure canonical graph bytes per admitted source byte"
    )
    parser.add_argument(
        "--estate",
        nargs=2,
        action="append",
        metavar=("NAME", "GRAPH_JSON"),
        required=True,
        help="estate label and its completed canonical graph artifact; repeat exactly five times",
    )
    parser.add_argument("--json-output", type=Path)
    parser.add_argument("--markdown-output", type=Path)
    parser.add_argument(
        "--max-metadata-bytes", type=int, default=DEFAULT_METADATA_LIMIT
    )
    arguments = parser.parse_args()

    estates = [parse_estate(values) for values in arguments.estate]
    if len(estates) != 5:
        parser.error(f"expected exactly five --estate entries, found {len(estates)}")
    names = [estate.name for estate in estates]
    if len(set(names)) != len(names):
        parser.error("estate names must be unique")
    if arguments.max_metadata_bytes <= 0:
        parser.error("--max-metadata-bytes must be positive")

    try:
        rows = sorted(
            (measure(estate, arguments.max_metadata_bytes) for estate in estates),
            key=lambda row: str(row["estate"]),
        )
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    ratios = [float(row["graphToSourceRatio"]) for row in rows]
    report: dict[str, object] = {
        "schema": SCHEMA,
        "estates": rows,
        "summary": {
            "minimumRatio": min(ratios),
            "medianRatio": statistics.median(ratios),
            "maximumRatio": max(ratios),
        },
    }
    json_payload = (json.dumps(report, indent=2, sort_keys=False) + "\n").encode()
    markdown_payload = render_markdown(report).encode()
    if arguments.json_output:
        write_atomic(arguments.json_output.resolve(), json_payload)
    if arguments.markdown_output:
        write_atomic(arguments.markdown_output.resolve(), markdown_payload)
    if not arguments.json_output and not arguments.markdown_output:
        sys.stdout.buffer.write(markdown_payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
