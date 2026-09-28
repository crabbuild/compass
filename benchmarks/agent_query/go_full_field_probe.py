"""Audit all pinned Litestream Go struct fields against three frozen graphs."""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess

from benchmarks.agent_query.go_struct_field_probe import compare, sha256


def source_key(row: dict) -> tuple[str, str, int, str, int]:
    return (row["file"], row["struct"], row["structLine"], row["field"], row["fieldLine"])


def canonical(item: dict, *, omit_community: bool = False) -> str:
    return json.dumps({key: value for key, value in item.items() if not (omit_community and key == "community")}, sort_keys=True)


def run(args: argparse.Namespace) -> None:
    registration = json.loads(args.registration.read_text())
    if registration["schema"] != "compass.go-full-field-registration/1":
        raise ValueError("unsupported full-field registration")
    if sha256(args.selected_registration) != registration["priorSelectedRegistrationSha256"]:
        raise ValueError("selected registration changed")
    root = args.source
    commit = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
    dirty = subprocess.run(["git", "-C", str(root), "status", "--porcelain=v1"], check=True, capture_output=True, text=True).stdout.strip()
    if commit != registration["sourceCommit"] or dirty:
        raise ValueError("source checkout changed")
    listing = subprocess.run(["git", "ls-files", "*.go"], cwd=root, check=True, capture_output=True).stdout
    paths = listing.decode().splitlines()
    if (len(paths) != registration["sourcePathCount"] or len(paths) > registration["bounds"]["maxSourcePaths"]
            or hashlib.sha256(listing).hexdigest() != registration["sourcePathListSha256"]):
        raise ValueError("tracked Go path list changed")
    if any((root / path).stat().st_size > registration["bounds"]["maxSourceBytesPerFile"] for path in paths):
        raise ValueError("source file size bound exceeded")
    oracle = json.loads(args.oracle.read_text())
    if oracle["schema"] != "compass.go-struct-field-oracle/1":
        raise ValueError("unsupported source oracle")
    fields = oracle["declarations"]
    structs = {(row["file"], row["struct"], row["structLine"]) for row in fields + oracle["embedded"] + oracle["blank"]}
    if (len(fields) > registration["bounds"]["maxNamedFields"] or len(structs) > registration["bounds"]["maxStructs"]
            or len({source_key(row) for row in fields}) != len(fields)):
        raise ValueError("source oracle exceeds bound or repeats identities")
    graph_paths = {"compass-baseline": args.baseline, "graphify": args.graphify, "compass-candidate": args.candidate}
    candidate_record = next(item for item in registration["graphs"] if item["tool"] == "compass-candidate")
    if sha256(args.candidate_binary) != candidate_record["binarySha256"]:
        raise ValueError("candidate binary differs from registration")
    graphs = {}
    results = []
    for record in registration["graphs"]:
        tool = record["tool"]
        path = graph_paths[tool]
        if (path.stat().st_size != record["bytes"] or path.stat().st_size > registration["bounds"]["maxGraphBytes"]
                or sha256(path) != record["sha256"]):
            raise ValueError(f"{tool} graph differs from registration")
        graph = json.loads(path.read_text())
        if len(graph["nodes"]) > registration["bounds"]["maxGraphNodes"] or len(graph["links"]) > registration["bounds"]["maxGraphEdges"]:
            raise ValueError(f"{tool} graph item bound exceeded")
        graphs[tool] = graph
        projection = compare(graph, "graphify" if tool == "graphify" else "compass", fields)
        projection["tool"] = tool
        projection["graphSha256"] = record["sha256"]
        results.append(projection)
    baseline = graphs["compass-baseline"]
    candidate = graphs["compass-candidate"]
    old_nodes = {node["id"]: node for node in baseline["nodes"]}
    new_nodes = {node["id"]: node for node in candidate["nodes"]}
    if len(old_nodes) != len(baseline["nodes"]) or len(new_nodes) != len(candidate["nodes"]):
        raise ValueError("duplicate Compass node ID")
    removed_ids = sorted(old_nodes.keys() - new_nodes.keys())
    changed_old = sorted(identifier for identifier in old_nodes.keys() & new_nodes.keys()
                         if canonical(old_nodes[identifier], omit_community=True)
                         != canonical(new_nodes[identifier], omit_community=True))
    old_edges = Counter(canonical(edge) for edge in baseline["links"])
    new_edges = Counter(canonical(edge) for edge in candidate["links"])
    removed_edges = sum((old_edges - new_edges).values())
    added_edges = sum((new_edges - old_edges).values())
    added_ids = sorted(new_nodes.keys() - old_nodes.keys())
    candidate_rows = next(item["rows"] for item in results if item["tool"] == "compass-candidate")
    matched_ids = {row["fieldIds"][0] for row in candidate_rows if row["status"] == "matched"}
    unexpected_additions = [new_nodes[identifier] for identifier in added_ids if identifier not in matched_ids]
    old_omissions = sorted(item.get("relatedIds", [""])[0] for item in baseline["graph"]["diagnostics"]
                           if item["code"] == "publication_omitted_node")
    new_omissions = sorted(item.get("relatedIds", [""])[0] for item in candidate["graph"]["diagnostics"]
                           if item["code"] == "publication_omitted_node")
    delta = {"removedNodeIds": removed_ids,
             "changedOldNodeIdsIgnoringCommunity": changed_old,
             "removedEdgeRecords": removed_edges,
             "addedEdgeRecords": added_edges,
             "addedNodeCount": len(added_ids),
             "addedNodeKinds": dict(sorted(Counter(new_nodes[identifier]["kind"] for identifier in added_ids).items())),
             "unexpectedAddedNodes": unexpected_additions,
             "publicationOmissionsUnchanged": old_omissions == new_omissions,
             "baselinePublicationOmittedNodes": len(old_omissions),
             "candidatePublicationOmittedNodes": len(new_omissions)}
    raw = {"schema": "compass.go-full-field-raw/1", "registrationSha256": sha256(args.registration),
           "oracleSha256": sha256(args.oracle), "sourceCommit": commit,
           "trackedGoPaths": paths, "sourceNamedFields": fields,
           "sourceEmbeddedFields": oracle["embedded"], "sourceBlankFields": oracle["blank"],
           "results": results, "delta": delta}
    args.raw.write_text(json.dumps(raw, indent=2, sort_keys=True) + "\n")
    compact = {"schema": "compass.go-full-field-review/1", "registrationSha256": sha256(args.registration),
               "oracleSha256": sha256(args.oracle), "rawSha256": sha256(args.raw),
               "sourceCommit": commit, "trackedGoFileCount": len(paths),
               "sourceStructCount": len(structs), "sourceNamedFieldCount": len(fields),
               "sourceEmbeddedFieldCount": len(oracle["embedded"]),
               "sourceBlankFieldCount": len(oracle["blank"]),
               "graphs": [{"tool": item["tool"], "sha256": item["graphSha256"],
                            "nodeCount": item["nodeCount"], "edgeRecordCount": item["edgeRecordCount"],
                            "directed": item["directed"], "statusCounts": item["statusCounts"],
                            "fieldTypeContextRecords": item["fieldTypeContextRecords"],
                            "nonmatchedCount": sum(row["status"] != "matched" for row in item["rows"]),
                            "nonmatchedExamples": [{"source": source_key(row), "status": row["status"]}
                                                   for row in item["rows"] if row["status"] != "matched"][:10]}
                           for item in results],
               "delta": {key: value for key, value in delta.items() if key != "unexpectedAddedNodes"},
               "unexpectedAddedNodes": [{"id": node["id"], "name": node.get("name"), "source": node.get("source")}
                                        for node in unexpected_additions],
               "limitations": registration["limitations"]}
    args.summary.write_text(json.dumps(compact, indent=2, sort_keys=True) + "\n")
    print("source", len(fields), "fields in", len(structs), "structs", flush=True)
    for item in compact["graphs"]:
        print(item["tool"], item["statusCounts"], flush=True)
    print("candidate unexpected additions", len(unexpected_additions), "removed edges", removed_edges, flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--selected-registration", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--oracle", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--graphify", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--candidate-binary", type=Path, required=True)
    parser.add_argument("--raw", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    run(parser.parse_args())
