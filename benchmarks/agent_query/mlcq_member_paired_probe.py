"""Compare frozen Java source declarations with both stored graph formats.

This checks selected declaration presence and direct owner links. It does not
validate field-access targets, responsibility cohesion, or god-object labels.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


def sha256(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def graph_line(node: dict, tool: str) -> tuple[str | None, int | None, int | None]:
    if tool == "compass":
        source = node.get("source", {})
        if not isinstance(source, dict):
            return None, None, None
        return source.get("file"), source.get("startLine"), source.get("endLine")
    location = node.get("source_location")
    match = re.fullmatch(r"L([1-9][0-9]*)", location) if isinstance(location, str) else None
    return node.get("source_file"), int(match[1]) if match else None, int(match[1]) if match else None


def display_name(node: dict) -> str | None:
    label = node.get("name", node.get("label"))
    if not isinstance(label, str):
        return None
    if label.startswith(".") and label.endswith("()"):
        return label[1:-2]
    return label


def candidate(node: dict, member: dict, tool: str) -> bool:
    kind = member["kind"]
    if tool == "compass":
        if node.get("kind") != kind:
            return False
    elif tool == "graphify":
        if node.get("_callable_class") is True:
            return False
        if (node.get("_callable") is True) != (kind == "method"):
            return False
    else:
        raise ValueError("unsupported tool")
    file, start, end = graph_line(node, tool)
    return (file == member["file"] and type(start) is int and type(end) is int
            and start <= member["endLine"] and member["startLine"] <= end
            and display_name(node) == member["name"])


def worker(graph_path: Path, registered: dict, cases: list[dict], source_cases: dict,
           bounds: dict) -> dict:
    if graph_path.stat().st_size != registered["graphBytes"] or graph_path.stat().st_size > bounds["graphBytes"]:
        raise ValueError("graph bytes differ from registration")
    if sha256(graph_path) != registered["graphSha256"]:
        raise ValueError("graph digest differs from registration")
    graph = json.loads(graph_path.read_bytes())
    nodes, links = graph["nodes"], graph["links"]
    if len(nodes) > bounds["graphNodes"] or len(links) > bounds["graphEdges"]:
        raise ValueError("graph item bound exceeded")
    by_id = {node["id"]: node for node in nodes}
    if len(by_id) != len(nodes):
        raise ValueError("duplicate graph node ID")
    by_file = defaultdict(list)
    for node in nodes:
        file, _, _ = graph_line(node, registered["tool"])
        if isinstance(file, str):
            by_file[file].append(node)
    direct = defaultdict(set)
    for edge in links:
        direct[(edge["source"], edge["target"])].add(edge.get("kind" if registered["tool"] == "compass" else "relation"))
    outputs = []
    for case in cases:
        source_case = source_cases[case["sampleId"]]
        class_id = case["classGraphIds"][registered["tool"]]
        if class_id not in by_id:
            raise ValueError("prior verified class identity missing")
        rows = []
        for member in source_case["sourceMembers"]:
            if member["kind"] not in ("method", "field"):
                continue
            matches = [node["id"] for node in by_file[member["file"]]
                       if candidate(node, member, registered["tool"])]
            required = ({"contains"} if registered["tool"] == "compass" else
                        {"method"} if member["kind"] == "method" else {"contains", "field"})
            links_found = {identifier: sorted(direct[class_id, identifier])
                           for identifier in matches}
            rows.append({"kind": member["kind"], "name": member["name"],
                         "file": member["file"], "startLine": member["startLine"],
                         "endLine": member["endLine"], "startUtf16": member["startUtf16"],
                         "candidateIds": sorted(matches), "directOwnerRelations": links_found,
                         "directOwnerMatch": len(matches) == 1 and
                         bool(required.intersection(links_found[matches[0]]))})
        outputs.append({"sampleId": case["sampleId"], "classification": case["classification"],
                        "sourceRole": case["sourceRole"], "classId": class_id,
                        "members": rows})
    return {"repository": registered["repository"], "tool": registered["tool"],
            "graphSha256": registered["graphSha256"], "directed": graph["directed"],
            "nodeCount": len(nodes), "edgeCount": len(links), "cases": outputs}


def main(args: argparse.Namespace) -> None:
    registration = json.loads(args.registration.read_text())
    if registration["schema"] != "compass.mlcq-member-paired-registration/1":
        raise ValueError("unsupported registration")
    for path, key in [(args.inputs, "priorInputsSha256"),
                      (args.prior_review, "priorReleaseReviewSha256"),
                      (args.member_review, "priorMemberReviewSha256")]:
        if sha256(path) != registration[key]:
            raise ValueError(f"registered input changed: {key}")
    if sha256(Path(__file__).with_name("mlcq_god_source_witnesses.json")) != registration["sourceWitnessesSha256"]:
        raise ValueError("source witness manifest changed")
    inputs = json.loads(args.inputs.read_text())
    prior = json.loads(args.prior_review.read_text())
    source_review = json.loads(args.member_review.read_text())
    source_cases = {case["sampleId"]: case for case in source_review["fullCases"]}
    if len(source_cases) != 8 or {case["sampleId"] for case in registration["cases"]} != set(source_cases):
        raise ValueError("source review denominator differs")
    prior_ids = {(row["repository"], row["tool"], case["sampleId"]): case["graphIds"]
                 for row in prior["verification"]["results"] for case in row["cases"]}
    for case in registration["cases"]:
        source = source_cases[case["sampleId"]]
        if any(source[key] != case[value] for key, value in
               (("repository", "repository"), ("classification", "classification"),
                ("sourceRole", "sourceRole"), ("file", "sourceFile"),
                ("sourceFileSha256", "sourceFileSha256"))):
            raise ValueError("registered source case differs from independent census")
        for tool in ("compass", "graphify"):
            if prior_ids[case["repository"], tool, case["sampleId"]] != [case["classGraphIds"][tool]]:
                raise ValueError("registered class ID differs from prior source-assisted review")
    for repo in registration["repositories"]:
        root = args.github_root / repo["name"]
        head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"],
                              check=True, capture_output=True, text=True).stdout.strip()
        dirty = subprocess.run(["git", "-C", str(root), "status", "--porcelain=v1"],
                               check=True, capture_output=True, text=True).stdout.strip()
        if head != repo["commit"] or dirty:
            raise ValueError("source checkout differs from registration")
        for case in registration["cases"]:
            if case["repository"] != repo["name"]:
                continue
            relative = Path(case["sourceFile"])
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("invalid registered source path")
            path = root / relative
            if path.stat().st_size > registration["bounds"]["sourceBytesPerFile"] or sha256(path) != case["sourceFileSha256"]:
                raise ValueError("source file differs from independent census")
    args.raw.mkdir(parents=True, exist_ok=False)
    results = []
    for repo in registration["repositories"]:
        for tool in ("compass", "graphify"):
            registered = {"repository": repo["name"], "tool": tool, **repo["graphs"][tool]}
            build = next(row for row in inputs["builds"][repo["name"]]["results"] if row["tool"] == tool)
            if build["graphSha256"] != registered["graphSha256"] or build["graphBytes"] != registered["graphBytes"]:
                raise ValueError("graph producer manifest changed")
            command = [sys.executable, "-m", "benchmarks.agent_query.mlcq_member_paired_probe",
                       "--worker", "--registration", str(args.registration),
                       "--member-review", str(args.member_review),
                       "--repository", repo["name"], "--tool", tool,
                       "--graph", build["graphPath"]]
            try:
                process = subprocess.run(command, check=False, capture_output=True, text=True,
                                         timeout=registration["bounds"]["secondsPerGraph"])
            except subprocess.TimeoutExpired as error:
                raise ValueError(f"graph worker timed out: {repo['name']}/{tool}") from error
            if process.returncode != 0 or len(process.stdout.encode()) > 1024 * 1024 or len(process.stderr.encode()) > 1024 * 1024:
                raise ValueError(f"graph worker failed or exceeded output bound: {process.returncode}: {process.stderr[:500]}")
            result = json.loads(process.stdout)
            if (result["repository"], result["tool"], result["graphSha256"]) != (repo["name"], tool, registered["graphSha256"]):
                raise ValueError("worker returned a different graph")
            raw_path = args.raw / f"{repo['name'].replace('/', '-')}-{tool}.json"
            raw_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
            result["rawSha256"] = sha256(raw_path)
            results.append(result)
    summaries = []
    for tool in ("compass", "graphify"):
        members = [member for result in results if result["tool"] == tool
                   for case in result["cases"] for member in case["members"]]
        reused = Counter(identifier for member in members for identifier in member["candidateIds"])
        for kind in ("method", "field"):
            rows = [member for member in members if member["kind"] == kind]
            summaries.append({"tool": tool, "kind": kind, "sourceDeclarations": len(rows),
                              "uniqueNodeMatches": sum(len(row["candidateIds"]) == 1 for row in rows),
                              "missingNodes": sum(not row["candidateIds"] for row in rows),
                              "ambiguousNodes": sum(len(row["candidateIds"]) > 1 for row in rows),
                              "directOwnerMatches": sum(row["directOwnerMatch"] for row in rows),
                              "distinctIdentityMatches": sum(len(row["candidateIds"]) == 1 and
                                                             reused[row["candidateIds"][0]] == 1 for row in rows)})
    output = {"schema": "compass.mlcq-member-paired-review/1",
              "scope": __doc__, "registrationSha256": sha256(args.registration),
              "probeSha256": sha256(Path(__file__)), "summaries": summaries, "results": results}
    args.output.write_text(json.dumps(output, indent=2, sort_keys=True) + "\n")
    for item in summaries:
        print(item)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--inputs", type=Path)
    parser.add_argument("--prior-review", type=Path)
    parser.add_argument("--member-review", type=Path, required=True)
    parser.add_argument("--github-root", type=Path)
    parser.add_argument("--raw", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--repository")
    parser.add_argument("--tool")
    parser.add_argument("--graph", type=Path)
    args = parser.parse_args()
    if args.worker:
        registration = json.loads(args.registration.read_text())
        source = {case["sampleId"]: case for case in json.loads(args.member_review.read_text())["fullCases"]}
        repo = next(item for item in registration["repositories"] if item["name"] == args.repository)
        record = {"repository": args.repository, "tool": args.tool, **repo["graphs"][args.tool]}
        cases = [case for case in registration["cases"] if case["repository"] == args.repository]
        print(json.dumps(worker(args.graph, record, cases, source, registration["bounds"]), sort_keys=True))
    else:
        if any(value is None for value in (args.inputs, args.prior_review, args.github_root, args.raw, args.output)):
            parser.error("--inputs, --prior-review, --github-root, --raw and --output are required")
        main(args)
