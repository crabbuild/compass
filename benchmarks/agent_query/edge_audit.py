"""Check selected direct relationships and call occurrences against pinned source.

This is a diagnostic over explicitly reviewed endpoint pairs, not an estimate
of whole-graph precision or recall. Missing endpoints never pass a negative.
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path

from benchmarks.agent_query.path_audit import MAX_GRAPH_BYTES, _edge_site, check_source, read_bounded
from benchmarks.agent_query.runner import _node_anchor, _sha256_file, _verify_source, load_suite


def audit_edges(manifest: dict, tool: str, graph: dict, root: Path) -> list[dict]:
    if tool not in ("compass", "graphify"):
        raise ValueError("unsupported edge audit tool")
    nodes = graph.get("nodes", [])
    ids = [node.get("id") for node in nodes]
    if any(not isinstance(identity, str) or not identity for identity in ids) or len(set(ids)) != len(ids):
        raise ValueError("graph must have unique nonempty string node IDs")
    edges = graph.get("edges") or graph.get("links") or []
    results = []
    for witness in manifest["witnesses"]:
        expected = witness["expected"]
        if expected not in ("present", "absent"):
            raise ValueError("unsupported edge expectation")
        occurrences = witness["occurrences"]
        if (expected == "present") != bool(occurrences):
            raise ValueError("positive witnesses need occurrences; negatives must have none")
        for anchor in (witness["source"], witness["target"], *occurrences):
            check_source(root, anchor)
        endpoints = []
        for anchor in (witness["source"], witness["target"]):
            matches = []
            for node in nodes:
                file, line, names = _node_anchor(node, tool)
                if (file, line) == (anchor["file"], anchor["line"]) and anchor["symbol"] in names:
                    matches.append(node["id"])
            endpoints.append(matches)
        result = {"id": witness["id"], "tool": tool, "expected": expected,
                  "sourceCandidates": endpoints[0], "targetCandidates": endpoints[1],
                  "endpointIdentityVerified": all(len(matches) == 1 for matches in endpoints),
                  "expectedOccurrences": len(occurrences), "matchedOccurrences": 0,
                  "relationshipMatched": False, "matched": False}
        if not result["endpointIdentityVerified"]:
            result["reason"] = "missing or ambiguous declaration identity"
            results.append(result)
            continue
        source, target = endpoints[0][0], endpoints[1][0]
        matches = [edge for edge in edges if (
            edge.get("_src", edge.get("source")), edge.get("_tgt", edge.get("target")),
            edge.get("kind", edge.get("relation"))) == (source, target, witness["relation"])]
        result["matchingEdges"] = len(matches)
        result["relationshipMatched"] = bool(matches) if expected == "present" else not matches
        expected_sites = Counter((site["file"], site["line"]) for site in occurrences)
        observed_sites = Counter(_edge_site(edge, tool) for edge in matches)
        # Counter subtraction preserves multiplicity: duplicating one occurrence
        # cannot recover another missing call site.
        def records(sites: Counter) -> list[dict]:
            return [{"file": file, "line": line, "count": count}
                    for (file, line), count in sorted(sites.items(), key=lambda item: repr(item[0]))]
        result["expectedOccurrences"] = sum(expected_sites.values())
        result["matchedOccurrences"] = sum((expected_sites & observed_sites).values())
        result["missingOccurrences"] = records(expected_sites - observed_sites)
        result["unexpectedOccurrences"] = records(observed_sites - expected_sites)
        result["matched"] = (result["relationshipMatched"] and not result["missingOccurrences"]
                             and not result["unexpectedOccurrences"])
        results.append(result)
    return results


def execute(args: argparse.Namespace) -> None:
    run_root = args.run.resolve()
    code_files = [Path(__file__), Path(__file__).with_name("runner.py"),
                  Path(__file__).with_name("path_audit.py")]
    code_digests = {path.name: _sha256_file(path) for path in code_files}
    run_bytes = read_bounded(run_root / "run.json")
    run = json.loads(run_bytes)
    if run.get("schema") != "compass.agent-query-run/2":
        raise ValueError("edge audit requires a provenance-recorded v2 run")
    if _sha256_file(run_root / "suite.toml") != run["suiteDigest"]:
        raise ValueError("captured suite digest mismatch")
    manifest_bytes = read_bounded(args.witnesses)
    manifest = json.loads(manifest_bytes)
    if manifest.get("schema") != "compass.agent-edge-witnesses/1":
        raise ValueError("unsupported edge witness schema")
    if not 1 <= len(manifest["witnesses"]) <= 1000:
        raise ValueError("witness count outside audit limit")
    records = [record for record in run["repositories"] if record["repository"] == manifest["repository"]]
    if len(records) != 1:
        raise ValueError("expected one captured repository")
    record = records[0]
    repository = load_suite(run_root / "suite.toml").repository(manifest["repository"])
    if manifest["commit"] != repository.commit or record["commit"] != repository.commit:
        raise ValueError("witness or captured source commit mismatch")
    source = Path(record["source"])
    _verify_source(repository, source)
    results = []
    for tool in ("compass", "graphify"):
        graph_path = Path(record[f"{tool}Graph"]).resolve()
        if not graph_path.is_relative_to(run_root):
            raise ValueError("captured graph escapes the run directory")
        graph_bytes = read_bounded(graph_path, MAX_GRAPH_BYTES)
        if hashlib.sha256(graph_bytes).hexdigest() != record[f"{tool}GraphSha256"]:
            raise ValueError("captured graph digest mismatch")
        results.extend(audit_edges(manifest, tool, json.loads(graph_bytes), source))
    _verify_source(repository, source)
    if code_digests != {path.name: _sha256_file(path) for path in code_files}:
        raise ValueError("audit code changed during execution")
    report = {"schema": "compass.agent-edge-audit/1", "runId": run["runId"],
              "scope": manifest["scope"], "codeDigests": code_digests,
              "runDigest": hashlib.sha256(run_bytes).hexdigest(),
              "witnessDigest": hashlib.sha256(manifest_bytes).hexdigest(), "results": results}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(report, indent=2, sort_keys=True) + "\n")
    for row in results:
        print(row["id"], row["tool"], "identity", row["endpointIdentityVerified"],
              "relationship", row["relationshipMatched"], "all occurrences", row["matched"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--witnesses", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    execute(parser.parse_args())
