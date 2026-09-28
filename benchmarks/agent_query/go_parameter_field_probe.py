"""Join the registered typed Go parameter-field source oracle to frozen graphs."""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import subprocess

from benchmarks.agent_query.go_struct_field_probe import compare, sha256


def read_bounded(path: Path, max_bytes: int) -> dict:
    if path.stat().st_size > max_bytes:
        raise ValueError(f"input byte bound exceeded: {path}")
    return json.loads(path.read_text())


def anchor(item: dict) -> dict:
    return item.get("source") or {}


def index_compass(graph: dict) -> dict:
    owners = defaultdict(list)
    fields = defaultdict(list)
    methods = defaultdict(list)
    containment = set()
    accesses = defaultdict(list)
    for node in graph["nodes"]:
        source = anchor(node)
        if node.get("kind") == "struct":
            owners[(node.get("name"), source.get("file"), source.get("startLine"))].append(node)
        if node.get("kind") == "field":
            fields[(node.get("qualifiedName"), source.get("file"), source.get("startLine"))].append(node)
        if node.get("kind") == "method":
            methods[(source.get("file"), source.get("startLine"))].append(node)
    for link in graph["links"]:
        if link.get("kind") == "contains":
            containment.add((link["source"], link["target"]))
        if link.get("kind") == "references" and str(link.get("occurrenceRule", "")).startswith(
            "universal-member-access"
        ):
            site = link.get("relationshipSite") or {}
            accesses[(link["source"], link["target"], site.get("file"),
                      site.get("startLine"), site.get("startByte"))].append(link)
    return {"owners": owners, "fields": fields, "methods": methods,
            "containment": containment, "accesses": accesses}


def join(index: dict, contact: dict) -> dict:
    owner = index["owners"][(contact["struct"], contact["structFile"], contact["structLine"])]
    if len(owner) != 1:
        return {"status": "missing_or_ambiguous_owner", "ownerCount": len(owner)}
    qualified = owner[0]["qualifiedName"] + "::" + contact["field"]
    field = index["fields"][(qualified, contact["fieldFile"], contact["fieldLine"])]
    method = [node for node in index["methods"][(contact["file"], contact["methodLine"])]
              if node.get("qualifiedName", "").endswith("::" + contact["method"])]
    if len(field) != 1 or len(method) != 1:
        return {"status": "missing_or_ambiguous_declaration", "fieldCount": len(field),
                "methodCount": len(method)}
    if (owner[0]["id"], field[0]["id"]) not in index["containment"]:
        return {"status": "missing_direct_ownership"}
    links = index["accesses"][(method[0]["id"], field[0]["id"], contact["file"],
                                contact["line"], contact["byteOffset"])]
    return {"status": "exact" if len(links) == 1 else "missing_or_ambiguous_edge",
            "edgeIds": [link["id"] for link in links]}


def run(args: argparse.Namespace) -> None:
    registration = read_bounded(args.registration, 64 * 1024)
    if registration["schema"] != "compass.go-parameter-field-registration/1":
        raise ValueError("unexpected registration schema")
    bounds = registration["bounds"]
    commit = subprocess.run(["git", "-C", str(args.source), "rev-parse", "HEAD"],
                            check=True, capture_output=True, text=True).stdout.strip()
    dirty = subprocess.run(["git", "-C", str(args.source), "status", "--porcelain=v1"],
                           check=True, capture_output=True, text=True).stdout.strip()
    if commit != registration["sourceCommit"] or dirty:
        raise ValueError("source checkout changed")
    paths = subprocess.run(["git", "ls-files", "*.go"], cwd=args.source,
                           check=True, capture_output=True, text=True).stdout.splitlines()
    oracle = read_bounded(args.oracle, 8 * 1024 * 1024)
    if oracle["schema"] != "compass.go-parameter-field-oracle/1" or len(paths) != 146:
        raise ValueError("oracle schema or selected path count changed")
    if len(paths) > bounds["maxSourcePaths"] or paths != [item["path"] for item in oracle["files"]]:
        raise ValueError("oracle source paths differ from tracked source")
    for record in oracle["files"]:
        path = args.source / record["path"]
        if (path.stat().st_size != record["bytes"] or record["bytes"] > bounds["maxSourceBytesPerFile"]
                or sha256(path) != record["sha256"]):
            raise ValueError(f"source changed: {record['path']}")
    if oracle["selectorCount"] > bounds["maxSelectors"]:
        raise ValueError("oracle selector bound exceeded")

    graphs = {}
    paths_by_tool = {"compass-before-access": args.baseline,
                     "compass-after-access": args.candidate, "graphify": args.graphify}
    for record in registration["frozenGraphs"]:
        path = paths_by_tool[record["tool"]]
        if (path.stat().st_size != record["bytes"] or record["bytes"] > bounds["maxGraphBytes"]
                or sha256(path) != record["sha256"]):
            raise ValueError(f"frozen graph changed: {record['tool']}")
        graph = read_bounded(path, bounds["maxGraphBytes"])
        if len(graph["nodes"]) > bounds["maxGraphNodes"] or len(graph["links"]) > bounds["maxGraphEdges"]:
            raise ValueError("graph item bound exceeded")
        graphs[record["tool"]] = graph

    unique_fields = sorted({(item["structFile"], item["struct"], item["structLine"],
                             item["field"], item["fieldLine"]) for item in oracle["contacts"]})
    declarations = [{"file": item[0], "struct": item[1], "structLine": item[2],
                     "field": item[3], "fieldLine": item[4]} for item in unique_fields]
    graphify_fields = compare(graphs["graphify"], "graphify", declarations)
    graphify_status = {(item["file"], item["struct"], item["structLine"],
                        item["field"], item["fieldLine"]): item["status"]
                       for item in graphify_fields["rows"]}
    before = index_compass(graphs["compass-before-access"])
    after = index_compass(graphs["compass-after-access"])
    rows = []
    for contact in oracle["contacts"]:
        field_key = (contact["structFile"], contact["struct"], contact["structLine"],
                     contact["field"], contact["fieldLine"])
        rows.append({"contact": contact, "before": join(before, contact),
                     "after": join(after, contact),
                     "graphifyFieldStatus": graphify_status[field_key]})
    counts = {tool: dict(sorted(Counter(row[tool]["status"] for row in rows).items()))
              for tool in ("before", "after")}
    counts["graphifyFieldStatus"] = dict(sorted(Counter(
        row["graphifyFieldStatus"] for row in rows).items()))
    credited = {edge_id for row in rows for edge_id in row["after"].get("edgeIds", [])
                if row["after"]["status"] == "exact"}
    other_edges = sorted(link["id"] for links in after["accesses"].values()
                         for link in links if link["id"] not in credited)
    raw = {"schema": "compass.go-parameter-field-probe/1",
           "registrationSha256": sha256(args.registration), "oracleSha256": sha256(args.oracle),
           "sourceCommit": commit, "trackedGoPaths": paths, "counts": counts,
           "uniqueFieldTargets": len(unique_fields),
           "graphifyDirected": graphs["graphify"]["directed"],
           "rows": rows, "uncreditedCandidateAccessEdgeIds": other_edges}
    args.output.write_text(json.dumps(raw, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"contacts": len(rows), "uniqueFields": len(unique_fields),
                      "counts": counts, "uncreditedCandidateEdges": len(other_edges)}, sort_keys=True))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--oracle", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--graphify", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    run(parser.parse_args())
