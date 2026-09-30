"""Run the registered class-only degree diagnostic on frozen Java graphs.

Each graph is processed in a separate bounded process. This is a development
retrieval diagnostic, not a god-object classifier or native tool comparison.
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def graph_worker(path: Path, record: dict, rules: dict, cases: list[dict]) -> dict:
    if path.stat().st_size != record["graphBytes"] or path.stat().st_size > rules["graphByteLimit"]:
        raise ValueError("graph size differs from registered bound")
    if digest(path) != record["graphSha256"]:
        raise ValueError("graph digest differs from registration")
    graph = json.loads(path.read_bytes())
    nodes, edges = graph["nodes"], graph["links"]
    if len(nodes) > rules["maxNodes"] or len(edges) > rules["maxEdges"]:
        raise ValueError("graph item bound exceeded")
    ids = [node["id"] for node in nodes]
    if any(not isinstance(identifier, str) for identifier in ids) or len(ids) != len(set(ids)):
        raise ValueError("invalid or repeated graph node ID")
    by_id = {node["id"]: node for node in nodes}
    pairs = {(edge["source"], edge["target"]) for edge in edges}
    degrees = Counter()
    dangling = 0
    for source, target in pairs:
        dangling += source not in by_id or target not in by_id
        degrees[source] += 1
        degrees[target] += 1
    tool = record["tool"]
    if tool == "compass":
        def eligible(node: dict) -> bool:
            source = node.get("source")
            return (node.get("kind") == "class" and isinstance(source, dict)
                    and isinstance(source.get("file"), str) and bool(source["file"])
                    and type(source.get("startLine")) is int and source["startLine"] > 0)
    elif tool == "graphify":
        def eligible(node: dict) -> bool:
            return (node.get("_callable_class") is True
                    and isinstance(node.get("source_file"), str) and bool(node["source_file"])
                    and isinstance(node.get("source_location"), str)
                    and re.fullmatch(r"L[1-9][0-9]*", node["source_location"]) is not None)
    else:
        raise ValueError("unsupported tool")
    pool = [node for node in nodes if eligible(node)]
    if len(pool) > rules["maxClassCandidates"]:
        raise ValueError("class candidate bound exceeded")
    pool.sort(key=lambda node: (-degrees[node["id"]], node["id"]))
    ranks = {node["id"]: rank for rank, node in enumerate(pool, 1)}
    selected = []
    for case in cases:
        matches = case["graphIds"]
        if len(matches) != 1:
            selected.append({"sampleId": case["sampleId"], "classification": case["classification"],
                             "status": "unavailable-prior-identity", "graphIds": matches})
            continue
        identifier = matches[0]
        node = by_id.get(identifier)
        if node is None or identifier not in ranks:
            selected.append({"sampleId": case["sampleId"], "classification": case["classification"],
                             "status": "unavailable-not-eligible", "graphId": identifier})
            continue
        selected.append({"sampleId": case["sampleId"], "classification": case["classification"],
                         "status": "ranked", "graphId": identifier, "rank": ranks[identifier],
                         "degree": degrees[identifier]})
    return {"repository": record["repository"], "tool": tool,
            "graphSha256": record["graphSha256"], "directed": graph["directed"],
            "nodeCount": len(nodes), "edgeRecordCount": len(edges),
            "distinctEndpointPairs": len(pairs), "danglingPairs": dangling,
            "classCandidates": len(pool),
            "top100": [{"rank": rank, "id": node["id"],
                        "label": node.get("name", node.get("label", "")),
                        "degree": degrees[node["id"]]}
                       for rank, node in enumerate(pool[:100], 1)],
            "cases": selected}


def main(args: argparse.Namespace) -> None:
    registration = json.loads(args.registration.read_text())
    if registration["schema"] != "compass.mlcq-class-degree-registration/1":
        raise ValueError("unsupported registration")
    if digest(args.inputs) != registration["priorInputsSha256"] or digest(args.prior_review) != registration["priorReleaseReviewSha256"]:
        raise ValueError("prior input or review changed")
    inputs = json.loads(args.inputs.read_text())
    prior = json.loads(args.prior_review.read_text())
    if not inputs["complete"]:
        raise ValueError("prior capture incomplete")
    expected = {(r["repository"], r["tool"], c["sampleId"]): c["graphIds"]
                for r in prior["verification"]["results"] for c in r["cases"]}
    registered = {(r["repository"], r["tool"], r["sampleId"]): r["graphIds"]
                  for r in registration["cases"]}
    if expected != registered or len(registered) != 16:
        raise ValueError("registered primary identities differ from prior review")
    args.raw.mkdir(parents=True, exist_ok=False)
    results = []
    for index, record in enumerate(registration["graphs"]):
        repository, tool = record["repository"], record["tool"]
        build = next(r for r in inputs["builds"][repository]["results"] if r["tool"] == tool)
        if any(build[key] != record[key] for key in ("graphBytes", "graphSha256")):
            raise ValueError("registered graph differs from prior input")
        cases = [case for case in registration["cases"]
                 if case["repository"] == repository and case["tool"] == tool]
        command = [sys.executable, "-m", "benchmarks.agent_query.mlcq_class_degree_probe",
                   "--worker", "--graph", build["graphPath"],
                   "--registration", str(args.registration), "--graph-index", str(index)]
        try:
            process = subprocess.run(command, check=False, capture_output=True, text=True,
                                     timeout=registration["maxSecondsPerGraph"])
            if process.returncode != 0 or len(process.stdout.encode()) > 1024 * 1024 or len(process.stderr.encode()) > 1024 * 1024:
                raise ValueError(f"graph worker failed or exceeded output bound: {process.returncode}: {process.stderr[:500]}")
            result = json.loads(process.stdout)
        except subprocess.TimeoutExpired as error:
            raise ValueError(f"graph worker timed out: {repository}/{tool}") from error
        if (result["repository"], result["tool"], result["graphSha256"]) != (repository, tool, record["graphSha256"]):
            raise ValueError("worker returned a different graph")
        raw_path = args.raw / f"{index:02}-{tool}.json"
        raw_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        result["rawSha256"] = digest(raw_path)
        results.append(result)
        print(repository, tool, [(c["sampleId"], c.get("rank")) for c in result["cases"]], flush=True)
    totals = {}
    for tool in ("compass", "graphify"):
        for cutoff in registration["method"]["cutoffs"]:
            cases = [case for result in results if result["tool"] == tool for case in result["cases"]]
            totals[f"{tool}@{cutoff}"] = {
                label: sum(case.get("rank", cutoff + 1) <= cutoff and case["classification"] == label
                           for case in cases)
                for label in ("unanimous-major-critical", "unanimous-none")}
    output = {"schema": "compass.mlcq-class-degree-review/1",
              "registrationSha256": digest(args.registration),
              "priorReviewSha256": digest(args.prior_review),
              "probeSha256": digest(Path(__file__)),
              "scope": __doc__, "results": results, "cutoffs": totals}
    args.output.write_text(json.dumps(output, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--inputs", type=Path)
    parser.add_argument("--prior-review", type=Path)
    parser.add_argument("--raw", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--graph", type=Path)
    parser.add_argument("--graph-index", type=int)
    args = parser.parse_args()
    if args.worker:
        registration = json.loads(args.registration.read_text())
        record = registration["graphs"][args.graph_index]
        cases = [case for case in registration["cases"] if
                 case["repository"] == record["repository"] and case["tool"] == record["tool"]]
        print(json.dumps(graph_worker(args.graph, record, registration, cases), sort_keys=True))
    else:
        if any(value is None for value in (args.inputs, args.prior_review, args.raw, args.output)):
            parser.error("--inputs, --prior-review, --raw and --output are required")
        main(args)
