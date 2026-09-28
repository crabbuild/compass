#!/usr/bin/env python3
"""Join the preregistered Go self-field oracle to a published Compass graph.

This is a development audit. It validates exact direct receiver contacts only;
other field edges are retained for review, not counted as true positives.
"""

import argparse
import collections
import hashlib
import json
from pathlib import Path


# Post-capture read-bound amendment recorded in go_field_access_registration.json.
MAX_GRAPH_BYTES = 24 * 1024 * 1024
MAX_ORACLE_BYTES = 8 * 1024 * 1024


def load(path: Path, limit: int):
    data = path.read_bytes()
    if len(data) > limit:
        raise ValueError(f"{path}: {len(data)} bytes exceeds {limit}")
    return json.loads(data), hashlib.sha256(data).hexdigest(), len(data)


def source(node):
    return node.get("source") or {}


def site(link):
    anchor = link.get("relationshipSite") or {}
    return anchor.get("file"), anchor.get("startLine"), anchor.get("startByte")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    oracle, oracle_sha, oracle_bytes = load(args.oracle, MAX_ORACLE_BYTES)
    baseline, baseline_sha, baseline_bytes = load(args.baseline, MAX_GRAPH_BYTES)
    candidate, candidate_sha, candidate_bytes = load(args.candidate, MAX_GRAPH_BYTES)
    if oracle.get("schema") != "compass.go-field-access-oracle/1":
        raise ValueError("unexpected oracle schema")
    contacts = oracle["contacts"]
    if len(contacts) > 100_000:
        raise ValueError("oracle contact bound exceeded")
    for graph in (baseline, candidate):
        if len(graph["nodes"]) > 100_000 or len(graph["links"]) > 100_000:
            raise ValueError("graph record bound exceeded")

    old_nodes = {node["id"]: node for node in baseline["nodes"]}
    nodes = {node["id"]: node for node in candidate["nodes"]}
    old_links = {link["id"]: link for link in baseline["links"]}
    links = {link["id"]: link for link in candidate["links"]}
    if len(old_nodes) != len(baseline["nodes"]) or len(nodes) != len(candidate["nodes"]):
        raise ValueError("duplicate node ID")
    if len(old_links) != len(baseline["links"]) or len(links) != len(candidate["links"]):
        raise ValueError("duplicate link ID")

    structs = collections.defaultdict(list)
    qualified = collections.defaultdict(list)
    for node in nodes.values():
        qualified[node.get("qualifiedName")].append(node)
        if node.get("kind") == "struct":
            structs[(node.get("name"), source(node).get("file"),
                     source(node).get("startLine"))].append(node)
    containment = {(link["source"], link["target"]) for link in links.values()
                   if link.get("kind") == "contains"}
    accesses = collections.defaultdict(list)
    for link in links.values():
        if link.get("kind") == "references" and str(link.get("occurrenceRule", "")).startswith(
                "universal-member-access"):
            if nodes[link["target"]].get("kind") != "field":
                raise ValueError("member-access link has a non-field target")
            accesses[(link["source"], link["target"], *site(link))].append(link)

    outcomes = []
    credited_ids = set()
    for contact in contacts:
        owner = structs[(contact["struct"], contact["structFile"], contact["structLine"])]
        method = field = []
        if len(owner) == 1:
            prefix = owner[0]["qualifiedName"] + "::"
            method = [node for node in qualified[prefix + contact["method"]]
                      if source(node).get("file") == contact["file"]
                      and source(node).get("startLine") == contact["methodLine"]]
            field = [node for node in qualified[prefix + contact["field"]]
                     if source(node).get("file") == contact["fieldFile"]
                     and source(node).get("startLine") == contact["fieldLine"]]
        status = "missing_or_ambiguous_declaration"
        edge_ids = []
        if len(owner) == len(method) == len(field) == 1:
            if (owner[0]["id"], field[0]["id"]) not in containment:
                status = "missing_field_ownership"
            else:
                matched = accesses[(method[0]["id"], field[0]["id"], contact["file"],
                                    contact["line"], contact["byteOffset"])]
                edge_ids = [link["id"] for link in matched]
                status = "exact" if len(matched) == 1 else "missing_or_ambiguous_edge"
                if status == "exact":
                    credited_ids.add(matched[0]["id"])
        outcomes.append({"contact": contact, "status": status, "edgeIds": edge_ids,
                         "ownerCount": len(owner), "methodCount": len(method),
                         "fieldCount": len(field)})

    exclusions = collections.defaultdict(set)
    for excluded in oracle["exclusions"]:
        exclusions[(excluded["file"], excluded["line"], excluded["byteOffset"])].add(
            excluded["category"])
    uncredited = []
    categories = collections.Counter()
    for edge_group in accesses.values():
        for link in edge_group:
            if link["id"] in credited_ids:
                continue
            edge_site = site(link)
            category = ",".join(sorted(exclusions.get(edge_site, ()))) or "outside_oracle_scope"
            categories[category] += 1
            uncredited.append({"edgeId": link["id"], "site": edge_site,
                               "source": nodes[link["source"]].get("qualifiedName"),
                               "target": nodes[link["target"]].get("qualifiedName"),
                               "oracleCategory": category})

    added = [link for link_id, link in links.items() if link_id not in old_links]
    output = {
        "schema": "compass.go-field-access-review/1",
        "scope": "known Litestream development repository; direct receiver-field contacts only",
        "inputs": {"oracle": {"sha256": oracle_sha, "bytes": oracle_bytes},
                   "baseline": {"sha256": baseline_sha, "bytes": baseline_bytes},
                   "candidate": {"sha256": candidate_sha, "bytes": candidate_bytes}},
        "summary": {"oracleContacts": len(contacts), "exact": len(credited_ids),
                    "outcomes": dict(sorted(collections.Counter(
                        item["status"] for item in outcomes).items())),
                    "baselineNodes": len(old_nodes), "candidateNodes": len(nodes),
                    "removedNodeIds": len(old_nodes.keys() - nodes.keys()),
                    "addedNodeIds": len(nodes.keys() - old_nodes.keys()),
                    "baselineLinks": len(old_links), "candidateLinks": len(links),
                    "removedLinks": len(old_links.keys() - links.keys()),
                    "changedOldLinks": sum(links.get(key) != link
                                           for key, link in old_links.items()),
                    "addedLinks": len(added), "addedMemberAccessLinks": sum(
                        link.get("kind") == "references"
                        and str(link.get("occurrenceRule", "")).startswith(
                            "universal-member-access") for link in added),
                    "uncreditedAccessLinks": len(uncredited),
                    "uncreditedOracleCategories": dict(sorted(categories.items()))},
        "outcomes": outcomes,
        "uncreditedAccessLinks": sorted(uncredited, key=lambda item: item["edgeId"]),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2, sort_keys=True) + "\n")
    print(json.dumps(output["summary"], sort_keys=True))


if __name__ == "__main__":
    main()
