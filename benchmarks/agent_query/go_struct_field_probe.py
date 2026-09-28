"""Compare frozen graph declaration identities with registered Go source fields."""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def source_location(node: dict, tool: str) -> tuple[str | None, int | None]:
    if tool == "compass":
        source = node.get("source", {})
        return source.get("file"), source.get("startLine")
    location = node.get("source_location")
    line = int(location[1:]) if isinstance(location, str) and location.startswith("L") and location[1:].isdigit() else None
    return node.get("source_file"), line


def compare(graph: dict, tool: str, declarations: list[dict]) -> dict:
    if not isinstance(graph.get("directed"), bool):
        raise ValueError(f"{tool} graph lacks directedness")
    nodes = graph["nodes"]
    links = graph["links"]
    by_id = {node["id"]: node for node in nodes}
    if len(by_id) != len(nodes):
        raise ValueError(f"{tool} duplicate node IDs")
    outgoing = {}
    contexts = Counter()
    for link in links:
        outgoing.setdefault(link["source"], []).append(link)
        if link.get("relation", link.get("kind")) == "references" and link.get("context") == "field":
            contexts[(link.get("source"), link.get("source_file"), link.get("source_location"))] += 1
    rows = []
    for item in declarations:
        file, owner_name, owner_line = item["file"], item["struct"], item["structLine"]
        owner_candidates = [node for node in nodes
                            if node.get("name", node.get("label")) == owner_name
                            and source_location(node, tool) == (file, owner_line)
                            and (tool != "compass" or node.get("kind") == "struct")]
        owner_ids = sorted(node["id"] for node in owner_candidates)
        field_ids = set()
        if len(owner_ids) == 1:
            for link in outgoing.get(owner_ids[0], []):
                if link.get("relation", link.get("kind")) not in ("contains", "defines"):
                    continue
                member = by_id.get(link["target"])
                if (member is not None and member.get("name", member.get("label")) == item["field"]
                        and source_location(member, tool) == (file, item["fieldLine"])
                        and (tool != "compass" or member.get("kind") == "field")):
                    field_ids.add(member["id"])
        status = ("unavailable-owner" if not owner_ids else "ambiguous-owner" if len(owner_ids) > 1
                  else "matched" if len(field_ids) == 1 else "ambiguous-field" if len(field_ids) > 1
                  else "missing-field")
        rows.append({**item, "status": status, "ownerIds": owner_ids,
                     "fieldIds": sorted(field_ids),
                     "fieldTypeContextRecords": contexts[(owner_ids[0], file, f"L{item['fieldLine']}")] if len(owner_ids) == 1 else 0})
    return {"tool": tool, "directed": graph["directed"],
            "nodeCount": len(nodes), "edgeRecordCount": len(links),
            "statusCounts": dict(sorted(Counter(row["status"] for row in rows).items())),
            "fieldTypeContextRecords": sum(row["fieldTypeContextRecords"] for row in rows),
            "rows": rows}


def run(args: argparse.Namespace) -> None:
    registration = json.loads(args.registration.read_text())
    if registration["schema"] != "compass.go-struct-field-registration/1":
        raise ValueError("unsupported registration")
    if sha256(args.source_run) != registration["priorSourceRunSha256"]:
        raise ValueError("prior source run changed")
    if sha256(args.prior_hub_registration) != registration["priorHubRegistrationSha256"]:
        raise ValueError("prior hub registration changed")
    source_run = json.loads(args.source_run.read_text())
    source_record = next(item for item in source_run["repositories"] if item["repository"] == registration["sourceRepository"])
    root = Path(source_record["source"])
    commit = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
    dirty = subprocess.run(["git", "-C", str(root), "status", "--porcelain=v1"], check=True, capture_output=True, text=True).stdout.strip()
    if commit != registration["sourceCommit"] or dirty:
        raise ValueError("source checkout changed")
    for record in registration["sourceFiles"]:
        path = root / record["path"]
        if path.stat().st_size != record["bytes"] or path.stat().st_size > registration["bounds"]["maxSourceBytesPerFile"] or sha256(path) != record["sha256"]:
            raise ValueError(f"source file changed: {record['path']}")
    oracle = json.loads(args.oracle.read_text())
    if oracle["schema"] != "compass.go-struct-field-oracle/1":
        raise ValueError("unsupported Go oracle")
    declarations = oracle["declarations"]
    if len(declarations) > registration["bounds"]["maxNamedFields"] or len(oracle["embedded"]) > registration["bounds"]["maxNamedFields"]:
        raise ValueError("source declaration bound exceeded")
    structs = {(item["file"], item["struct"], item["structLine"]) for item in declarations + oracle["embedded"]}
    if len(structs) > registration["bounds"]["maxStructs"]:
        raise ValueError("struct bound exceeded")
    results = []
    for record in registration["frozenGraphs"]:
        tool = record["tool"]
        path = Path(source_record[f"{tool}Graph"])
        if path.stat().st_size != record["bytes"] or path.stat().st_size > registration["bounds"]["maxGraphBytes"] or sha256(path) != record["sha256"]:
            raise ValueError(f"frozen {tool} graph changed")
        graph = json.loads(path.read_text())
        if len(graph["nodes"]) > registration["bounds"]["maxGraphNodes"] or len(graph["links"]) > registration["bounds"]["maxGraphEdges"]:
            raise ValueError(f"{tool} graph item bound exceeded")
        result = compare(graph, tool, declarations)
        result["graphSha256"] = record["sha256"]
        results.append(result)
    report = {"schema": "compass.go-struct-field-review/1",
              "registrationSha256": sha256(args.registration),
              "oracleSha256": sha256(args.oracle),
              "sourceRunSha256": sha256(args.source_run),
              "sourceCommit": commit,
              "sourceStructCount": len(structs),
              "sourceNamedFieldCount": len(declarations),
              "sourceEmbeddedFieldCount": len(oracle["embedded"]),
              "embedded": oracle["embedded"],
              "results": results,
              "limitations": registration["limitations"]}
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    for result in results:
        print(result["tool"], result["statusCounts"], flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--prior-hub-registration", type=Path, required=True)
    parser.add_argument("--source-run", type=Path, required=True)
    parser.add_argument("--oracle", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    run(parser.parse_args())
