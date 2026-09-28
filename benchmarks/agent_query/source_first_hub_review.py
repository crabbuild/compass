"""Review registered source witnesses against captured public hub responses.

This checks retrieval and consistency on each tool's own stored graph. Source
roles are not independent labels for god-object defects.
"""

from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path

from benchmarks.agent_query.mcp_audit import audit
from benchmarks.agent_query.path_audit import MAX_GRAPH_BYTES, read_bounded
from benchmarks.agent_query.runner import _node_anchor, _sha256_file
from benchmarks.agent_query.source_first_hub_capture import checked_source


def load(path: Path, limit: int = 16 * 1024 * 1024) -> dict:
    return json.loads(read_bounded(path, limit))


def check_response(run: Path, row: dict) -> None:
    directory = run / "raw" / row["repository"] / row["tool"]
    expected = {"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {"name": "god_nodes", "arguments": row["arguments"]}}
    request = load(directory / "04.request.json", 4096)
    if request != expected:
        raise ValueError("raw hub request differs from recorded request")
    packets = [json.loads(line) for line in
               read_bounded(directory / "04.response.jsonl", 16 * 1024 * 1024).splitlines()]
    if packets != [row["response"]]:
        raise ValueError("raw hub response differs from recorded response")


def review(args: argparse.Namespace) -> None:
    run = load(args.run / "run.json", 32 * 1024 * 1024)
    registration = load(args.registration)
    source = load(args.source_run / "run.json")
    if run["schema"] != "compass.source-first-hub-three-language-capture/1" or not run["complete"]:
        raise ValueError("incomplete or unsupported hub capture")
    if registration["schema"] != "compass.source-first-hub-three-language-registration/1":
        raise ValueError("unsupported registration")
    if run["registrationSha256"] != _sha256_file(args.registration) or _sha256_file(args.run / "registration.json") != run["registrationSha256"]:
        raise ValueError("registration bytes changed")
    if run["sourceRunJsonSha256"] != _sha256_file(args.source_run / "run.json") or source["suiteDigest"] != registration["sourceSuiteSha256"]:
        raise ValueError("source capture changed")
    if run["collectorSha256"] != _sha256_file(args.run / "collector.py"):
        raise ValueError("captured collector changed")
    if run["transportSha256"] != _sha256_file(Path(__file__).with_name("mcp_transport.py")) or run["binarySha256"] != registration["binarySha256"]:
        raise ValueError("captured transport or binary manifest changed")
    if len(run["results"]) != 2 * len(registration["repositories"]):
        raise ValueError("unexpected result count")
    pairs = [(row["repository"], row["tool"]) for row in run["results"]]
    expected_pairs = [(repo["name"], tool) for repo in registration["repositories"]
                      for tool in ("compass", "graphify")]
    if pairs != expected_pairs:
        raise ValueError("missing, duplicate or reordered hub result")
    results = []
    for row in run["results"]:
        registered = next(r for r in registration["repositories"] if r["name"] == row["repository"])
        captured = next(r for r in source["repositories"] if r["repository"] == row["repository"])
        tool = row["tool"]
        if tool not in ("compass", "graphify") or row["method"] != "god_nodes" or row["arguments"] != registration["toolRequest"]["arguments"]:
            raise ValueError("unregistered hub request")
        if row["graphSha256"] != registered["graphSha256"][tool]:
            raise ValueError("recorded graph digest differs")
        root = Path(captured["source"])
        checked_source(root, registered)
        graph_path = Path(captured[f"{tool}Graph"])
        if _sha256_file(graph_path) != row["graphSha256"]:
            raise ValueError("graph bytes changed")
        graph = json.loads(read_bounded(graph_path, MAX_GRAPH_BYTES))
        if row["executionSucceeded"]:
            check_response(args.run, row)
        checked = audit({**row, "question": "hubs"}, graph)
        hubs = checked.get("hubs", [])
        if row["executionSucceeded"] and (len(hubs) != 100 or [h["rank"] for h in hubs] != list(range(1, 101))):
            raise ValueError("successful response lacks 100 ordered hubs")
        degree = Counter()
        for source_id, target_id in {(e["source"], e["target"]) for e in graph["links"]}:
            degree[source_id] += 1
            degree[target_id] += 1
        witnesses = []
        for witness in registered["witnesses"]:
            matches = [node for node in graph["nodes"] if
                       (lambda anchor: anchor[0] == witness["file"] and
                        anchor[1] == witness["line"] and
                        witness["symbol"] in anchor[2])(_node_anchor(node, tool))]
            result = {"symbol": witness["symbol"], "sourceFile": witness["file"],
                      "sourceLine": witness["line"], "sourceRole": witness["sourceRole"],
                      "graphDeclarationMatches": len(matches)}
            if len(matches) == 1:
                identifier = matches[0]["id"]
                result.update(graphId=identifier, storedDegree=degree[identifier])
                hits = [hub for hub in hubs if hub.get("id") == identifier]
                if len(hits) > 1:
                    raise ValueError("duplicate returned hub identity")
                if hits:
                    hit = hits[0]
                    result.update(returnedRank=hit["rank"], returnedDegree=hit["degree"],
                                  degreeMatches=hit["degreeMatches"],
                                  sourceAnchorMatches=hit.get("sourceAnchorMatches"))
                result["top10"] = bool(hits and hits[0]["rank"] <= 10)
                result["top50"] = bool(hits and hits[0]["rank"] <= 50)
                result["top100"] = bool(hits)
            witnesses.append(result)
        results.append({"repository": row["repository"], "tool": tool,
                        "executionSucceeded": row["executionSucceeded"],
                        "returned": checked.get("returned", 0),
                        "verifiedIdentities": checked.get("verifiedIdentities", 0),
                        "matchingDegrees": checked.get("matchingDegrees", 0),
                        "matchingSourceAnchors": checked.get("matchingSourceAnchors", 0),
                        "witnesses": witnesses})
    output = {"schema": "compass.source-first-hub-three-language-review/1",
              "scope": __doc__, "registrationSha256": run["registrationSha256"],
              "runJsonSha256": _sha256_file(args.run / "run.json"),
              "results": results}
    args.output.write_text(json.dumps(output, indent=2, sort_keys=True) + "\n")
    for result in results:
        print(result["repository"], result["tool"],
              [(w["symbol"], w.get("returnedRank")) for w in result["witnesses"]])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--source-run", type=Path, required=True)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    review(parser.parse_args())
