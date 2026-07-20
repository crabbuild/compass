#!/usr/bin/env python3
"""Compare deterministic graph topology while repairing one legacy path-ID defect."""

from __future__ import annotations

import json
import hashlib
import sys
from pathlib import Path


def canonical(
    path: Path,
) -> tuple[set[str], list[tuple[str, str, str, str, str]], set[str]]:
    document = json.loads(path.read_text(encoding="utf-8"))
    nodes = document.get("nodes", [])
    node_ids = {str(node["id"]) for node in nodes}
    file_ids = {
        str(node.get("source_file")): str(node["id"])
        for node in nodes
        if node.get("source_file")
        and str(node.get("label", "")) == Path(str(node["source_file"])).name
    }
    source_files = {
        str(node["source_file"])
        for node in nodes
        if node.get("source_file")
    }
    edges = []
    for edge in document.get("links", document.get("edges", [])):
        source = str(edge.get("source", ""))
        if source not in node_ids:
            source = file_ids.get(str(edge.get("source_file", "")), source)
        edges.append(
            (
                source,
                str(edge.get("target", "")),
                str(edge.get("relation", "")),
                str(edge.get("context", "")),
                str(edge.get("confidence", "")),
            )
        )
    return node_ids, sorted(edges), source_files


def canonical_hash(
    nodes: set[str], edges: list[tuple[str, str, str, str, str]]
) -> str:
    payload = json.dumps(
        [sorted(nodes), edges], ensure_ascii=False, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def main() -> int:
    csv_output = len(sys.argv) == 4 and sys.argv[1] == "--csv"
    args = sys.argv[2:] if csv_output else sys.argv[1:]
    if len(args) != 2:
        print(
            "usage: compare_phase1_graphs.py [--csv] PYTHON_GRAPH TRAIL_GRAPH",
            file=sys.stderr,
        )
        return 2
    python_graph = canonical(Path(args[0]))
    trail_graph = canonical(Path(args[1]))
    if python_graph[:2] == trail_graph[:2]:
        digest = canonical_hash(trail_graph[0], trail_graph[1])
        if csv_output:
            print(
                f"true,{len(trail_graph[0])},{len(trail_graph[1])},"
                f"{len(trail_graph[2])},{digest}"
            )
        else:
            print(
                f"correct: {len(trail_graph[0])} nodes, "
                f"{len(trail_graph[1])} edges, sha256={digest}"
            )
        return 0
    python_nodes, python_edges, _ = python_graph
    trail_nodes, trail_edges, _ = trail_graph
    print(
        "mismatch: "
        f"nodes python={len(python_nodes)} trail={len(trail_nodes)}, "
        f"edges python={len(python_edges)} trail={len(trail_edges)}",
        file=sys.stderr,
    )
    print(f"python-only nodes: {sorted(python_nodes - trail_nodes)[:20]}", file=sys.stderr)
    print(f"trail-only nodes: {sorted(trail_nodes - python_nodes)[:20]}", file=sys.stderr)
    print(f"python-only edges: {sorted(set(python_edges) - set(trail_edges))[:20]}", file=sys.stderr)
    print(f"trail-only edges: {sorted(set(trail_edges) - set(python_edges))[:20]}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
