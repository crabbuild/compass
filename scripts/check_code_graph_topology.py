#!/usr/bin/env python3
"""Validate evidence-backed code-graph topology against a v1 regression policy."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from code_graph_v1_oracle import (
    QualificationError,
    canonical_bytes,
    digest_bytes,
    load_topology_policy,
    topology_report,
)

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_POLICY = ROOT / "tests/qualification/code-graph-v1-topology.json"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    args = parser.parse_args()

    graph_bytes = args.graph.read_bytes()
    graph = json.loads(graph_bytes)
    policy = load_topology_policy(args.policy)
    report = topology_report(
        graph,
        policy,
        graph_digest=digest_bytes(graph_bytes),
    )
    print(canonical_bytes(report).decode(), end="")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, json.JSONDecodeError, QualificationError, ValueError) as error:
        print(f"code-graph topology qualification failed: {error}", file=sys.stderr)
        raise SystemExit(1)
