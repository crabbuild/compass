#!/usr/bin/env python3
"""Compare Phase 1 read-command output with only documented normalization.

The graph comparator repairs Python's legacy dangling path-ID source on raw
indirect-call edges. Querying the Python graph materializes that dangling ID as
an extra NetworkX node, while Compass correctly rewires it to the existing file
node. Apply the same narrow repair here so the read gate measures command
semantics rather than requiring Compass to reproduce a malformed graph endpoint.
"""

from __future__ import annotations

from collections import Counter
import json
from pathlib import Path
import re
import sys


def query_parts(path: Path) -> tuple[str, Counter[str]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    if len(lines) < 2 or lines[1] != "":
        raise ValueError(f"invalid query output shape: {path}")
    if any(line.startswith("... (truncated") for line in lines):
        raise ValueError(f"query parity requires an untruncated result: {path}")
    return lines[0], Counter(lines[2:])


def legacy_dangling_sources(path: Path) -> set[str]:
    document = json.loads(path.read_text(encoding="utf-8"))
    nodes = document.get("nodes", [])
    node_ids = {str(node.get("id", "")) for node in nodes}
    return {
        str(edge.get("source", ""))
        for edge in document.get("links", document.get("edges", []))
        if edge.get("source") and str(edge.get("source")) not in node_ids
    }


def repair_legacy_query_output(
    header: str, lines: Counter[str], dangling_sources: set[str]
) -> tuple[str, Counter[str]]:
    repaired = lines.copy()
    removed_nodes = 0
    for source in dangling_sources:
        node_prefix = f"NODE {source} ["
        for line, count in list(repaired.items()):
            if line.startswith(node_prefix):
                removed_nodes += count
                del repaired[line]
            elif line.startswith(f"EDGE {source} --") or line.endswith(f"--> {source}"):
                del repaired[line]
    if removed_nodes:
        match = re.search(r" \| (\d+) nodes found$", header)
        if match is None:
            raise ValueError(f"invalid query header: {header}")
        count = int(match.group(1)) - removed_nodes
        header = f"{header[:match.start()]} | {count} nodes found"
    return header, repaired


def main() -> int:
    if len(sys.argv) not in (4, 6):
        print(
            "usage: compare_phase1_read_outputs.py CASE PYTHON COMPASS "
            "[PYTHON_GRAPH COMPASS_GRAPH]",
            file=sys.stderr,
        )
        return 2
    case, python_path, compass_path = sys.argv[1:4]
    python = Path(python_path)
    compass = Path(compass_path)
    if case != "query":
        if python.read_bytes() != compass.read_bytes():
            print(f"{case} output differs", file=sys.stderr)
            return 1
        return 0

    try:
        python_header, python_lines = query_parts(python)
        compass_header, compass_lines = query_parts(compass)
        if len(sys.argv) == 6:
            python_header, python_lines = repair_legacy_query_output(
                python_header, python_lines, legacy_dangling_sources(Path(sys.argv[4]))
            )
            compass_header, compass_lines = repair_legacy_query_output(
                compass_header, compass_lines, legacy_dangling_sources(Path(sys.argv[5]))
            )
    except ValueError as error:
        print(error, file=sys.stderr)
        return 1
    if python_header != compass_header:
        print("query header differs", file=sys.stderr)
        print(f"python: {python_header}", file=sys.stderr)
        print(f"compass:  {compass_header}", file=sys.stderr)
        return 1
    if python_lines != compass_lines:
        print("query result contents differ", file=sys.stderr)
        print(
            f"python-only: {list((python_lines - compass_lines).elements())[:20]}",
            file=sys.stderr,
        )
        print(
            f"compass-only: {list((compass_lines - python_lines).elements())[:20]}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
