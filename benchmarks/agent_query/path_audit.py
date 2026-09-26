"""Audit captured path responses against graphs and reviewed source witnesses.

This diagnoses the checked-in development cases. It does not estimate held-out
accuracy, accept endpoint echoes as paths, or choose among ambiguous labels.
Run with ``python3 -m benchmarks.agent_query.path_audit --help``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re

from benchmarks.agent_query.runner import _node_anchor, _sha256_file, _verify_source, load_suite

MAX_GRAPH_BYTES = 256 * 1024 * 1024
MAX_TEXT_BYTES = 16 * 1024 * 1024
MAX_HOPS = 64
HEADER = re.compile(r"(?:Best path \(weighted, (\d+) hops, weight [0-9.]+\)|Shortest path \((\d+) hops\)):")
EDGE = re.compile(r"--([a-z_]+) \[([A-Z_]+)\]-->|<--([a-z_]+) \[([A-Z_]+)\]--")
SAFE_KEY = re.compile(r"[a-z0-9][a-z0-9_-]*")


def read_bounded(path: Path, limit: int = MAX_TEXT_BYTES) -> bytes:
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"input exceeds {limit} bytes: {path}")
    return data


def parse_path(text: str) -> tuple[list[str], list[dict]]:
    lines = text.splitlines()
    headers = [(index, match) for index, line in enumerate(lines)
               if (match := HEADER.fullmatch(line))]
    if len(headers) != 1:
        raise ValueError("expected exactly one rendered path header")
    index, header = headers[0]
    hops = int(header[1] or header[2])
    if hops > MAX_HOPS or index + 1 >= len(lines):
        raise ValueError("invalid path length or missing path body")
    body = lines[index + 1].strip()
    labels, steps, offset = [], [], 0
    for edge in EDGE.finditer(body):
        labels.append(body[offset:edge.start()].strip())
        steps.append({"relation": edge[1] or edge[3],
                      "direction": "forward" if edge[1] else "reverse",
                      "confidence": edge[2] or edge[4]})
        offset = edge.end()
    labels.append(body[offset:].strip())
    if len(steps) != hops or any(not label or len(label) > 512 for label in labels):
        raise ValueError("printed hop count does not match the path body")
    if any(line.strip() for line in lines[index + 2:]):
        raise ValueError("unexpected text after rendered path")
    return labels, steps


def check_source(root: Path, anchor: dict) -> None:
    relative = Path(anchor["file"])
    path = (root / relative).resolve()
    if relative.is_absolute() or ".." in relative.parts or not path.is_relative_to(root.resolve()):
        raise ValueError("source witness escapes its root")
    lines = read_bounded(path).decode("utf-8").splitlines()
    line = anchor["line"]
    if type(line) is not int or not 1 <= line <= len(lines):
        raise ValueError("source witness line is outside the file")
    if anchor["text"] not in lines[line - 1]:
        raise ValueError(f"source witness changed at {relative}:{line}")


def audit_path(witness: dict, tool: str, graph: dict, output: str, source_root: Path) -> dict:
    if tool not in ("compass", "graphify"):
        raise ValueError("unsupported path audit tool")
    for anchor in witness["nodes"]:
        check_source(source_root, anchor)
    for step in witness["steps"]:
        if "site" in step:
            check_source(source_root, step["site"])
        for site in step.get("sitesByRelation", {}).values():
            check_source(source_root, site)
    result = {"tool": tool, "category": witness["category"], "matched": False,
              "failures": [], "nodeIds": [], "steps": []}
    try:
        labels, steps = parse_path(output)
    except ValueError as error:
        result["failures"].append(str(error))
        return result
    if len(labels) != len(witness["nodes"]) or len(steps) != len(witness["steps"]):
        result["failures"].append("route length differs from reviewed witness")
        return result
    nodes = graph.get("nodes", [])
    node_ids = [node.get("id") for node in nodes]
    if any(not isinstance(identity, str) or not identity for identity in node_ids):
        raise ValueError("graph node IDs must be nonempty strings")
    if len(set(node_ids)) != len(node_ids):
        raise ValueError("graph node IDs must be unique")
    # Match exactly what the renderer prints. Never resolve a collision using
    # the expected answer; doing so would conceal an ambiguous response.
    for label, expected in zip(labels, witness["nodes"]):
        candidates = [node for node in nodes
                      if node.get("name" if tool == "compass" else "label") == label]
        if len(candidates) != 1:
            result["failures"].append(f"unverified identity for {label!r}: {len(candidates)} matches")
            return result
        node = candidates[0]
        file, line, _ = _node_anchor(node, tool)
        if label not in expected["labels"] or (file, line) != (expected["file"], expected["line"]):
            result["failures"].append(f"wrong source declaration for {label!r}")
            return result
        result["nodeIds"].append(node["id"])
    edges = graph.get("edges") or graph.get("links") or []
    for index, (actual, expected) in enumerate(zip(steps, witness["steps"])):
        if actual["direction"] != expected["direction"] or actual["relation"] not in expected["relations"]:
            result["failures"].append(f"hop {index + 1} has an unreviewed direction or relation")
            continue
        left, right = result["nodeIds"][index:index + 2]
        source, target = (left, right) if actual["direction"] == "forward" else (right, left)
        matches = [edge for edge in edges if (
            edge.get("_src", edge.get("source")), edge.get("_tgt", edge.get("target")),
            edge.get("kind", edge.get("relation"))) == (source, target, actual["relation"])]
        if not matches:
            result["failures"].append(f"hop {index + 1} is absent from graph in printed direction")
        site = expected.get("sitesByRelation", {}).get(actual["relation"], expected.get("site"))
        grounded = None
        if site is not None:
            grounded = any(_edge_site(edge, tool) == (site["file"], site["line"]) for edge in matches)
            if not grounded:
                result["failures"].append(f"hop {index + 1} lacks the reviewed occurrence anchor")
        result["steps"].append({**actual, "matchingEdges": len(matches),
                                "reviewedSite": site, "reviewedSiteSupported": grounded})
    result["matched"] = not result["failures"]
    return result


def _edge_site(edge: dict, tool: str) -> tuple[object, object]:
    if tool == "compass":
        site = edge.get("relationshipSite") or {}
        line = site.get("startLine")
        return site.get("file"), line if type(line) is int and line > 0 else None
    location = edge.get("source_location", "")
    match = re.fullmatch(r"L([0-9]+)", location) if isinstance(location, str) else None
    return edge.get("source_file"), int(match[1]) if match else None


def execute(args: argparse.Namespace) -> None:
    root = args.run.resolve()
    run = json.loads(read_bounded(root / "run.json"))
    if run.get("schema") != "compass.agent-query-run/2":
        raise ValueError("path audit requires a provenance-recorded v2 run")
    if _sha256_file(root / "suite.toml") != run["suiteDigest"]:
        raise ValueError("captured suite digest mismatch")
    suite = load_suite(root / "suite.toml")
    manifest_bytes = read_bounded(args.witnesses)
    manifest = json.loads(manifest_bytes)
    if manifest.get("schema") != "compass.agent-path-witnesses/1":
        raise ValueError("unsupported path witness schema")
    if not 1 <= len(manifest["witnesses"]) <= 1000:
        raise ValueError("witness count outside audit limit")
    repositories = {record["repository"]: record for record in run["repositories"]}
    observations = {(row["repository"], row["question"], row["tool"]): row
                    for row in run["observations"]}
    results = []
    for witness in manifest["witnesses"]:
        repository, question = witness["repository"], witness["question"]
        if not SAFE_KEY.fullmatch(repository) or not SAFE_KEY.fullmatch(question):
            raise ValueError("unsafe witness identifier")
        record = repositories[repository]
        source = Path(record["source"])
        pinned = suite.repository(repository)
        if witness["commit"] != pinned.commit or record["commit"] != pinned.commit:
            raise ValueError("witness or captured source commit mismatch")
        _verify_source(pinned, source)
        for tool in ("compass", "graphify"):
            graph_path = Path(record[f"{tool}Graph"]).resolve()
            if not graph_path.is_relative_to(root):
                raise ValueError("captured graph escapes the run directory")
            graph_bytes = read_bounded(graph_path, MAX_GRAPH_BYTES)
            if hashlib.sha256(graph_bytes).hexdigest() != record[f"{tool}GraphSha256"]:
                raise ValueError("captured graph digest mismatch")
            observation = observations[repository, question, tool]
            if observation["exitCode"] != 0 or observation["timedOut"] or observation["followUps"]:
                raise ValueError("path audit requires a successful single-response execution")
            raw = read_bounded(root / "raw" / repository / f"{question}.{tool}.0.stdout")
            if len(raw) != observation["stdoutBytes"]:
                raise ValueError("captured response length mismatch")
            result = audit_path(witness, tool, json.loads(graph_bytes), raw.decode("utf-8"), source)
            results.append({"repository": repository, "question": question,
                            "stdoutSha256": hashlib.sha256(raw).hexdigest(), **result})
        _verify_source(pinned, source)
    report = {"schema": "compass.agent-path-audit/1", "runId": run["runId"],
              "scope": manifest["scope"], "witnessDigest": hashlib.sha256(manifest_bytes).hexdigest(),
              "auditorDigest": _sha256_file(Path(__file__)), "results": results}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(report, indent=2, sort_keys=True) + "\n")
    for result in results:
        print(result["repository"], result["tool"], result["matched"], result["failures"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--witnesses", type=Path, default=Path(__file__).with_name("path_witnesses.json"))
    parser.add_argument("--output", type=Path, required=True)
    execute(parser.parse_args())
