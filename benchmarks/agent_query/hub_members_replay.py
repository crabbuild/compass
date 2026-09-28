"""Replay additive hub-member evidence on frozen three-language MCP graphs."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess

from benchmarks.agent_query.mcp_transport import StdioMcp


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def capture(args: argparse.Namespace) -> None:
    registration = json.loads(args.registration.read_text())
    source_run = json.loads((args.source_run / "run.json").read_text())
    prior_run = json.loads((args.prior_hubs / "run.json").read_text())
    if registration["schema"] != "compass.source-first-hub-three-language-registration/1":
        raise ValueError("unsupported registration")
    if source_run["suiteDigest"] != registration["sourceSuiteSha256"] or not prior_run["complete"]:
        raise ValueError("source or prior hub run changed")
    prepared = []
    for registered in registration["repositories"]:
        name = registered["name"]
        source = next(item for item in source_run["repositories"] if item["repository"] == name)
        prior = next(item for item in prior_run["results"] if item["repository"] == name and item["tool"] == "compass")
        root = Path(source["source"])
        head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"],
                              check=True, capture_output=True, text=True).stdout.strip()
        dirty = subprocess.run(["git", "-C", str(root), "status", "--porcelain=v1"],
                               check=True, capture_output=True, text=True).stdout.strip()
        graph = Path(source["compassGraph"])
        if head != registered["commit"] or dirty or sha256(graph) != registered["graphSha256"]["compass"]:
            raise ValueError(f"{name} source or graph changed")
        if not prior["executionSucceeded"] or prior["arguments"] != {"top_n": 100}:
            raise ValueError(f"{name} prior public hub response unavailable")
        prepared.append((registered, root, graph, prior))
    args.output_dir.mkdir(parents=True, exist_ok=False)
    report = {"schema": "compass.hub-members-three-language-replay/1",
              "scope": "Post-change public MCP development replay. Prior identities, ranks, degrees, anchors and connectivity must remain identical. Source roles are not defect labels.",
              "registrationSha256": sha256(args.registration),
              "sourceRunSha256": sha256(args.source_run / "run.json"),
              "priorHubRunSha256": sha256(args.prior_hubs / "run.json"),
              "binarySha256": sha256(args.candidate),
              "repetitions": 2, "repositories": []}
    for registered, root, graph, prior in prepared:
        name = registered["name"]
        old = prior["response"]["result"]["structuredContent"]["result"]["nodes"]
        observed = []
        for repetition in (1, 2):
            directory = args.output_dir / "raw" / name / str(repetition)
            with StdioMcp([str(args.candidate), "serve", "--graph", str(graph),
                           "--transport", "stdio"], root, directory,
                          timeout=registration["toolRequest"]["timeoutSeconds"],
                          max_bytes=registration["toolRequest"]["maxResponseBytes"]) as session:
                session.initialize()
                listing = session.send("tools/list", {})
                if "god_nodes" not in {tool["name"] for tool in listing["result"]["tools"]}:
                    raise ValueError("candidate did not advertise god_nodes")
                response = session.send("tools/call", {"name": "god_nodes", "arguments": {"top_n": 100}})
            if "error" in response or response["result"].get("isError", False):
                raise ValueError(f"{name} hub call failed")
            rows = response["result"]["structuredContent"]["result"]["nodes"]
            if len(rows) != len(old) or [{key: value for key, value in row.items()
                                         if key != "memberEvidence"} for row in rows] != old:
                raise ValueError(f"{name} legacy hub projection changed")
            for row in rows:
                members = row["memberEvidence"]
                if members is not None and (members["schema"] != "compass.hub-members/1"
                                            or members["sourceCoverage"] != "unverified"):
                    raise ValueError("unsupported member evidence")
            observed.append(rows)
        if observed[0] != observed[1]:
            raise ValueError(f"{name} repeated hub output differs")
        rows = observed[0]
        witnesses = []
        for witness in registered["witnesses"]:
            matches = [row for row in rows if row["label"] == witness["symbol"]]
            witnesses.append({"symbol": witness["symbol"], "returnedLabelMatches": len(matches),
                              "rank": matches[0]["rank"] if len(matches) == 1 else None,
                              "memberEvidence": matches[0]["memberEvidence"] if len(matches) == 1 else None})
        report["repositories"].append({"repository": name, "graphSha256": sha256(graph),
                                       "unchangedLegacyRows": len(rows),
                                       "memberEvidenceRows": sum(row["memberEvidence"] is not None for row in rows),
                                       "witnesses": witnesses})
        print(name, "unchanged rows", len(rows), "member evidence rows",
              report["repositories"][-1]["memberEvidenceRows"], flush=True)
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--source-run", type=Path, required=True)
    parser.add_argument("--prior-hubs", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    capture(parser.parse_args())
