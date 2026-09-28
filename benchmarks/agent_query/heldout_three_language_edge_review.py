#!/usr/bin/env python3
"""Source-site call-edge review for the registered three-language panel.

This reviewer was written after the first capture. Its 13 expected call sites
are the source facts named in heldout_three_language_suite.toml. Keep that
provenance separate from the suite's before-graph registration.
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess


# repository, call file, call line, source declaration file/line, target file/line
CALLS = (
    ("anyhow-heldout", "src/error.rs", 453, "src/error.rs", 452, "src/error.rs", 441),
    ("anyhow-heldout", "src/error.rs", 985, "src/error.rs", 984, "src/chain.rs", 28),
    ("markitdown-heldout", "packages/markitdown/src/markitdown/_markitdown.py", 347, "packages/markitdown/src/markitdown/_markitdown.py", 322, "packages/markitdown/src/markitdown/_markitdown.py", 471),
    ("markitdown-heldout", "packages/markitdown/src/markitdown/_markitdown.py", 349, "packages/markitdown/src/markitdown/_markitdown.py", 322, "packages/markitdown/src/markitdown/_markitdown.py", 368),
    ("markitdown-heldout", "packages/markitdown/src/markitdown/_markitdown.py", 352, "packages/markitdown/src/markitdown/_markitdown.py", 322, "packages/markitdown/src/markitdown/_markitdown.py", 368),
    ("markitdown-heldout", "packages/markitdown/src/markitdown/_markitdown.py", 355, "packages/markitdown/src/markitdown/_markitdown.py", 322, "packages/markitdown/src/markitdown/_markitdown.py", 533),
    ("markitdown-heldout", "packages/markitdown/src/markitdown/_markitdown.py", 362, "packages/markitdown/src/markitdown/_markitdown.py", 322, "packages/markitdown/src/markitdown/_markitdown.py", 405),
    ("markitdown-heldout", "packages/markitdown/src/markitdown/_markitdown.py", 403, "packages/markitdown/src/markitdown/_markitdown.py", 368, "packages/markitdown/src/markitdown/_markitdown.py", 606),
    ("markitdown-heldout", "packages/markitdown/src/markitdown/_markitdown.py", 450, "packages/markitdown/src/markitdown/_markitdown.py", 405, "packages/markitdown/src/markitdown/_markitdown.py", 606),
    ("markitdown-heldout", "packages/markitdown/src/markitdown/_markitdown.py", 604, "packages/markitdown/src/markitdown/_markitdown.py", 533, "packages/markitdown/src/markitdown/_markitdown.py", 606),
    ("vue-heldout", "src/compiler/index.ts", 14, "src/compiler/index.ts", 10, "src/compiler/parser/index.ts", 86),
    ("vue-heldout", "src/compiler/index.ts", 16, "src/compiler/index.ts", 10, "src/compiler/optimizer.ts", 20),
    ("vue-heldout", "src/compiler/index.ts", 18, "src/compiler/index.ts", 10, "src/compiler/codegen/index.ts", 57),
)

MAX_GRAPH_BYTES = 128 * 1024 * 1024
MAX_NODES = 300_000
MAX_EDGES = 500_000


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def graph_path(run: Path, repository: str, tool: str) -> Path:
    root = run / "artifacts" / repository / tool
    if tool == "graphify":
        return root / "graphify-out" / "graph.json"
    output = root / "compass-out"
    snapshot = (output / "current-snapshot").read_text().strip()
    if not snapshot.startswith("snapshot-") or Path(snapshot).name != snapshot:
        raise ValueError(f"invalid active Compass snapshot: {snapshot!r}")
    return output / "snapshots" / snapshot / "graph.json"


def read_graph(path: Path) -> dict:
    if path.stat().st_size > MAX_GRAPH_BYTES:
        raise ValueError(f"graph byte bound exceeded: {path}")
    graph = json.loads(path.read_text())
    if len(graph["nodes"]) > MAX_NODES or len(graph["links"]) > MAX_EDGES:
        raise ValueError(f"graph item bound exceeded: {path}")
    return graph


def node_site(node: dict, tool: str) -> tuple[str | None, int | None]:
    if tool == "compass":
        source = node.get("source") or {}
        return source.get("file"), source.get("startLine")
    location = node.get("source_location", "")
    if isinstance(location, str) and location.startswith("L") and location[1:].isdigit():
        return node.get("source_file"), int(location[1:])
    return node.get("source_file"), None


def edge_site(edge: dict, tool: str) -> tuple[str | None, int | None]:
    if tool == "compass":
        site = edge.get("relationshipSite") or {}
        return site.get("file"), site.get("startLine")
    location = edge.get("source_location", "")
    if isinstance(location, str) and location.startswith("L") and location[1:].isdigit():
        return edge.get("source_file"), int(location[1:])
    return edge.get("source_file"), None


def audit(graph: dict, tool: str, fact: tuple) -> dict:
    repository, call_file, call_line, source_file, source_line, target_file, target_line = fact
    nodes = graph["nodes"]
    sources = [node for node in nodes if node_site(node, tool) == (source_file, source_line)]
    targets = [node for node in nodes if node_site(node, tool) == (target_file, target_line)]
    if repository == "vue-heldout":
        # The callback's declaration shares a line with the assigned variable
        # and an export. Require its actual name, not just a matching line.
        sources = [node for node in sources
                   if (node.get("name") or node.get("label") or "").strip(".()") == "baseCompile"]
        targets = [node for node in targets
                   if (node.get("kind") or node.get("type")) != "export"]
    if len(sources) != 1 or len(targets) != 1:
        return {"status": "declaration_missing_or_ambiguous", "sourceCount": len(sources), "targetCount": len(targets)}
    source_id, target_id = sources[0]["id"], targets[0]["id"]
    call_edges = [edge for edge in graph["links"]
                  if (edge.get("kind") or edge.get("relation")) == "calls"
                  and edge.get("source") == source_id
                  and edge_site(edge, tool) == (call_file, call_line)]
    matches = [edge for edge in call_edges if edge.get("target") == target_id]
    return {
        "status": "exact" if len(matches) == 1 else "missing_or_ambiguous_edge",
        "sourceId": source_id,
        "targetId": target_id,
        "exactEdgeCount": len(matches),
        "otherCallTargetsAtSite": sorted({edge["target"] for edge in call_edges if edge["target"] != target_id}),
    }


def source_paths(values: list[str]) -> dict[str, Path]:
    result = {}
    for value in values:
        name, delimiter, path = value.partition("=")
        if not delimiter or not name or not path or name in result:
            raise ValueError(f"invalid --source {value!r}")
        result[name] = Path(path)
    return result


def run(args: argparse.Namespace) -> None:
    registration = json.loads(args.registration.read_text())
    if registration["schema"] != "compass.heldout-three-language-registration/1":
        raise ValueError("unexpected registration schema")
    if digest(args.suite) != registration["suiteSha256"]:
        raise ValueError("registered suite changed")
    sources = source_paths(args.source)
    runs = source_paths(args.run)
    rows = []
    graphs = {}
    for record in registration["repositories"]:
        name = record["name"]
        root = sources[name]
        commit = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"],
                                check=True, capture_output=True, text=True).stdout.strip()
        dirty = subprocess.run(["git", "-C", str(root), "status", "--porcelain=v1"],
                               check=True, capture_output=True, text=True).stdout.strip()
        if commit != record["sourceCommit"] or dirty:
            raise ValueError(f"source checkout changed: {name}")
        for relative, expected in record["reviewedSourceSha256"].items():
            if digest(root / relative) != expected:
                raise ValueError(f"reviewed source changed: {name}/{relative}")
        for tool in ("compass", "graphify"):
            path = graph_path(runs[name], name, tool)
            graphs[(name, tool)] = read_graph(path)
    for index, fact in enumerate(CALLS, 1):
        name, call_file, call_line, source_file, source_line, target_file, target_line = fact
        rows.append({"id": index, "repository": name, "callFile": call_file,
                     "callLine": call_line, "sourceDeclaration": [source_file, source_line],
                     "targetDeclaration": [target_file, target_line],
                     "compass": audit(graphs[(name, "compass")], "compass", fact),
                     "graphify": audit(graphs[(name, "graphify")], "graphify", fact)})
    counts = {tool: dict(sorted(Counter(row[tool]["status"] for row in rows).items()))
              for tool in ("compass", "graphify")}
    hashes = {name: {tool: digest(graph_path(runs[name], name, tool)) for tool in ("compass", "graphify")}
              for name in sources}
    report = {"schema": "compass.heldout-three-language-edge-review/1",
              "registrationSha256": digest(args.registration), "suiteSha256": digest(args.suite),
              "runs": {name: str(path) for name, path in sorted(runs.items())},
              "graphSha256": hashes, "counts": counts, "rows": rows,
              "limits": "Thirteen source-selected direct call occurrences only; no whole-graph precision or runtime call claim."}
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"counts": counts, "facts": len(rows)}, sort_keys=True))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--suite", type=Path, required=True)
    parser.add_argument("--run", action="append", required=True,
                        help="repository=run directory; repeat for each registered repository")
    parser.add_argument("--source", action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    run(parser.parse_args())


if __name__ == "__main__":
    main()
