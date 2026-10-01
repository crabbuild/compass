#!/usr/bin/env python3
"""Offline source-oracle qualification. Python is a qualification-only dependency.

The supplied native graph remains the authority; this script never creates graph
facts or supplies a runtime fallback. Source observations and resolved targets
are measured separately. Checkouts are read-only, and output is explicit.
"""
import argparse
import ast
from collections import defaultdict
import json
from pathlib import Path
import random
import subprocess

MAX_GRAPH_BYTES = 1024 * 1024 * 1024
MAX_SOURCE_BYTES = 2 * 1024 * 1024
SEED = 20260930
MAX_SOURCE_FILES = 20_000


def source_path(root, relative):
    candidate = (root / relative).resolve()
    candidate.relative_to(root.resolve())
    if not candidate.is_file() or candidate.stat().st_size > MAX_SOURCE_BYTES:
        raise ValueError("source file is missing or exceeds its qualification bound")
    return candidate


def bounded_read(path, limit):
    if not path.is_file() or path.stat().st_size > limit:
        raise ValueError("qualification input is missing or exceeds its bound")
    with path.open("rb") as source:
        data = source.read(limit + 1)
    if len(data) > limit:
        raise ValueError("qualification input grew past its bound")
    return data


def load_graph(path):
    return json.loads(bounded_read(path, MAX_GRAPH_BYTES))


def anchor_key(anchor):
    return (anchor["file"], anchor["startByte"], anchor["endByte"])


def query(binary, graph, operation, symbol, batch):
    if batch is not None:
        return batch[(operation, symbol)]
    result = subprocess.run([str(binary), operation, symbol, "--graph", str(graph),
        "--format", "json", "--max-nodes", "2000", "--max-edges", "10000",
        "--max-depth", "8" if operation == "callees" else "1", "--max-response-bytes", "33554432"], capture_output=True, timeout=120, check=True)
    if len(result.stdout) > 32 * 1024 * 1024:
        raise ValueError("qualification response exceeds 32 MiB")
    return json.loads(result.stdout)


def owned_calls(function):
    pending = list(function.body)
    while pending:
        node = pending.pop()
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            continue
        if isinstance(node, ast.Call):
            yield node.func
        pending.extend(ast.iter_child_nodes(node))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, required=True, help="graph's source root")
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--compass", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--batch-runner", type=Path, help="native query_batch example; reuses one engine")
    args = parser.parse_args()
    graph = load_graph(args.graph)
    nodes = {node["id"]: node for node in graph["nodes"]}
    by_class = defaultdict(list)
    for node in nodes.values():
        if node["kind"] == "class" and node.get("source"):
            by_class[(node["source"]["file"], node["name"])].append(node)
    inventory = defaultdict(set)
    resolved = defaultdict(set)
    inferred = defaultdict(set)
    for node in nodes.values():
        for site in node.get("details", {}).get("data", {}).get("callSites", {}).get("sites", []):
            inventory[node["id"]].add(anchor_key(site))
    for edge in graph["links"]:
        if edge["kind"] not in ("calls", "instantiates") or not edge.get("relationshipSite"):
            continue
        site = anchor_key(edge["relationshipSite"])
        exact = bool(edge["evidence"]) and all(item["confidence"] == "exact" and len(item.get("candidates", [])) <= 1 for item in edge["evidence"])
        (resolved if exact else inferred)[edge["source"]].add(site)
    classes = []
    paths = []
    for path in (args.repository / "services").rglob("*.py"):
        paths.append(path)
        if len(paths) > MAX_SOURCE_FILES:
            raise ValueError("service-source file bound exceeded")
    for path in sorted(paths):
        if "tests" in path.parts or path.stat().st_size > MAX_SOURCE_BYTES:
            continue
        source = bounded_read(source_path(args.repository, path.relative_to(args.repository)), MAX_SOURCE_BYTES)
        tree = ast.parse(source, filename=str(path))
        relative = path.relative_to(args.repository).as_posix()
        starts = [0]
        for line in source.splitlines(keepends=True):
            starts.append(starts[-1] + len(line))
        for definition in tree.body:
            if not isinstance(definition, ast.ClassDef) or not definition.name.endswith("Service"):
                continue
            matches = by_class[(relative, definition.name)]
            expected = set()
            for function in definition.body:
                if not isinstance(function, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue
                for call in owned_calls(function):
                    expected.add((relative, starts[call.lineno - 1] + call.col_offset,
                        starts[call.end_lineno - 1] + call.end_col_offset))
            if len(expected) >= 5:
                classes.append((relative, definition.name, matches[0] if len(matches) == 1 else None, expected))
    if len(classes) < 10:
        raise ValueError("qualification requires at least ten nontrivial service classes")
    chosen = sorted(random.Random(SEED).sample(classes, 10), key=lambda item: item[:2])
    for file, name, node, _ in chosen:
        if node is None:
            raise ValueError(f"{file}:{name}: source-sampled service has no unique graph class")
    incoming = defaultdict(set)
    incident = defaultdict(set)
    for edge in graph["links"]:
        incoming[edge["target"]].add(edge["source"])
        incident[edge["target"]].add(("incoming", edge["source"]))
        incident[edge["source"]].add(("outgoing", edge["target"]))
    symbols = sorted((node for node in nodes.values() if node["kind"] == "class"
        and node.get("source") and len(incoming[node["id"]]) >= 5),
        key=lambda node: (-len(incoming[node["id"]]), node["id"]))[:5]
    if len(symbols) != 5:
        raise ValueError("qualification requires five widely used source symbols")
    batch = None
    if args.batch_runner:
        requests = [{"operation": "callees", "symbol": node["id"]} for _, _, node, _ in chosen]
        requests.extend({"operation": "impact", "symbol": node["id"]} for node in symbols)
        request_path = args.report.with_suffix(".requests.json")
        response_path = args.report.with_suffix(".responses.json")
        request_path.write_text(json.dumps(requests))
        subprocess.run([str(args.batch_runner), str(args.graph), str(request_path), str(response_path)],
            check=True, timeout=1200)
        if response_path.stat().st_size > 64 * 1024 * 1024:
            raise ValueError("batch response exceeds 64 MiB")
        responses = json.loads(bounded_read(response_path, 64 * 1024 * 1024))
        if len(responses) != len(requests):
            raise ValueError("batch response count mismatch")
        batch = {(request["operation"], request["symbol"]): response
            for request, response in zip(requests, responses)}
    calls = []
    for file, name, node, expected in chosen:
        # Independently ask the public class query; it must aggregate methods.
        response = query(args.compass, args.graph, "callees", node["id"], batch)
        observed = set()
        exact = set()
        possible = set()
        for result_node in response["nodes"]:
            observed.update(inventory[result_node["id"]])
            exact.update(resolved[result_node["id"]])
            possible.update(inferred[result_node["id"]])
        summary = response.get("callSummary")
        if not summary or summary["observedCalls"] == 0 or summary["unresolvedCalls"] is None:
            raise ValueError(f"{name}: missing honest source-call coverage")
        calls.append({"file": file, "class": name, "source_calls": len(expected),
            "observed_calls": len(expected & observed), "resolved_calls": len(expected & exact),
            "inferred_edges": len(expected & possible), "query_summary": summary,
            "missing_source_sites": sorted(expected - observed)})
    impacts = []
    for node in symbols:
        response = query(args.compass, args.graph, "impact", node["id"], batch)
        summary = response["impactSummary"]
        known = len(incident[node["id"]])
        if summary["observedDirectConnections"] != known or not summary["directCoverageComplete"]:
            raise ValueError(f'{node["qualifiedName"]}: incomplete direct-connection audit')
        if len(summary["directDependents"]) + len(summary["outgoingContext"]) + summary["excludedDirectConnections"] != known:
            raise ValueError("direct-connection audit has unaccounted neighbors")
        samples = []
        for edge in graph["links"]:
            if edge["target"] != node["id"] or edge["source"] not in summary["directDependents"]:
                continue
            site = edge.get("relationshipSite")
            if not site or len(samples) >= 5:
                continue
            lines = bounded_read(source_path(args.repository, site["file"]), MAX_SOURCE_BYTES).decode().splitlines()
            samples.append({"relation": edge["kind"], "source": nodes[edge["source"]]["qualifiedName"],
                "file": site["file"], "line": site["startLine"],
                "source_line": lines[site["startLine"] - 1].strip()})
        impacts.append({"symbol": node["qualifiedName"], "source": node["source"],
            "known_direct_connections": known, "summary": summary, "hand_check_samples": samples})
    total = sum(item["source_calls"] for item in calls)
    observed = sum(item["observed_calls"] for item in calls)
    report = {"schema": "compass.qualification.python-services/1", "random_seed": SEED,
        "source_commit": graph["graph"]["build"].get("sourceCommit"),
        "eligible_source_services": len(classes),
        "service_count": len(calls), "source_calls": total, "observed_calls": observed,
        "source_capture_ratio": observed / total, "services": calls, "impact_symbols": impacts}
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    if observed / total < 0.9:
        raise SystemExit("source-call capture below 90%; see report")
    print(f"{observed}/{total} source calls captured across ten services; five direct-impact audits complete")


if __name__ == "__main__":
    main()
