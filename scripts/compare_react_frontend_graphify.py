#!/usr/bin/env python3
"""Compare an external Graphify diagnostic with a Compass frontend graph.

Graphify is intentionally an external, qualification-only diagnostic input.
This command never runs Graphify and is not imported by Compass runtime code.
It reports whether Compass provides a strictly richer, typed, source-anchored
directed graph while optionally checking the independent TypeScript oracle.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
MAX_INPUT_BYTES = 512 * 1024 * 1024


def fail(message: str) -> None:
    raise SystemExit(f"react frontend Graphify comparison failed: {message}")


def load(path: Path) -> dict[str, Any]:
    try:
        if path.stat().st_size > MAX_INPUT_BYTES:
            fail(f"input exceeds the {MAX_INPUT_BYTES}-byte limit: {path}")
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"cannot read {path}: {error}")
    if not isinstance(document, dict):
        fail(f"graph must be an object: {path}")
    nodes = document.get("nodes")
    links = document.get("links")
    if not isinstance(nodes, list) or not isinstance(links, list):
        fail(f"graph must contain node and link arrays: {path}")
    return document


def anchor(node_or_edge: dict[str, Any]) -> tuple[str, int, int] | None:
    source = node_or_edge.get("source")
    if isinstance(source, dict) and isinstance(source.get("file"), str):
        start = source.get("startByte")
        end = source.get("endByte")
        if isinstance(start, int) and isinstance(end, int) and 0 <= start <= end:
            return source["file"], start, end
    site = node_or_edge.get("relationshipSite")
    if isinstance(site, dict) and isinstance(site.get("file"), str):
        start = site.get("startByte")
        end = site.get("endByte")
        if isinstance(start, int) and isinstance(end, int) and 0 <= start <= end:
            return site["file"], start, end
    # Graphify's diagnostic envelope exposes line anchors rather than byte
    # ranges.  Preserve that source identity as a zero-width comparable
    # anchor; Compass is still required to retain exact byte anchors.
    graphify_file = node_or_edge.get("source_file")
    if isinstance(graphify_file, str) and isinstance(node_or_edge.get("source_location"), str):
        return graphify_file, 0, 0
    return None


def graph_metrics(graph: dict[str, Any]) -> dict[str, Any]:
    nodes = [node for node in graph["nodes"] if isinstance(node, dict)]
    links = [link for link in graph["links"] if isinstance(link, dict)]
    node_ids = {node.get("id") for node in nodes}
    dangling = sum(
        1
        for link in links
        if link.get("source") not in node_ids or link.get("target") not in node_ids
    )
    anchored_nodes = sum(anchor(node) is not None for node in nodes)
    anchored_links = sum(anchor(link) is not None for link in links)
    framework_nodes = sum(
        isinstance(node.get("framework"), str)
        or isinstance(node.get("attributes"), dict)
        and isinstance(node["attributes"].get("framework"), str)
        for node in nodes
    )
    return {
        "nodes": len(nodes),
        "links": len(links),
        "anchoredNodes": anchored_nodes,
        "anchoredLinks": anchored_links,
        "frameworkNodes": framework_nodes,
        "danglingTargets": dangling,
        "directed": graph.get("directed") is True,
        "multigraph": graph.get("multigraph") is True,
    }


def source_score(compass_path: Path, oracle_path: Path) -> dict[str, Any]:
    scripts = str(ROOT / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    try:
        from qualify_react_frontend_graph import load_source_oracle, match_source_facts
    except ImportError as error:
        fail(f"cannot load independent frontend scorer: {error}")
    compass = load(compass_path)
    return match_source_facts(compass, load_source_oracle(oracle_path))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compass", type=Path, required=True)
    parser.add_argument("--graphify", type=Path, required=True)
    parser.add_argument("--source-oracle", type=Path)
    args = parser.parse_args()
    compass = load(args.compass)
    graphify = load(args.graphify)
    compass_metrics = graph_metrics(compass)
    graphify_metrics = graph_metrics(graphify)
    comparisons = {
        "nodeCount": compass_metrics["nodes"] > graphify_metrics["nodes"],
        "linkCount": compass_metrics["links"] > graphify_metrics["links"],
        "anchoredNodes": compass_metrics["anchoredNodes"] >= graphify_metrics["anchoredNodes"],
        "anchoredLinks": compass_metrics["anchoredLinks"] >= graphify_metrics["anchoredLinks"],
        "typedDirected": compass_metrics["directed"] and compass_metrics["multigraph"],
        "noDanglingTargets": compass_metrics["danglingTargets"] == 0,
    }
    result: dict[str, Any] = {
        "schema": "compass.react-frontend-graphify-comparison/1",
        "compass": compass_metrics,
        "graphify": graphify_metrics,
        "comparisons": comparisons,
    }
    if args.source_oracle:
        scorecard = source_score(args.compass, args.source_oracle)
        result["sourceOracle"] = scorecard["aggregate"]
        comparisons["sourceOracleNoFabricatedTargets"] = scorecard["aggregate"].get(
            "zeroFabricatedTargets"
        ) is True
    result["surpassesGraphify"] = all(comparisons.values())
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    if not result["surpassesGraphify"]:
        fail(f"Compass did not satisfy every comparison: {comparisons}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
