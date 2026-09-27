"""Source-defined task co-location in captured native community assignments.

This developer audit never invents an agent answer or a god-object label.
Cross-task co-location describes granularity; it is not a correctness failure.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
from itertools import combinations
import json
from pathlib import Path
import subprocess

from benchmarks.agent_query.runner import _node_anchor


MAX_JSON_BYTES = 256 * 1024 * 1024
MAX_SOURCE_BYTES = 4 * 1024 * 1024
MAX_DECLARATIONS = 256


def read_bounded(path: Path, limit: int) -> bytes:
    if type(limit) is not int or not 0 <= limit <= MAX_JSON_BYTES:
        raise ValueError("invalid input byte limit")
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"input exceeds {limit} bytes: {path}")
    return data


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def community_id(node: dict, tool: str) -> int | None:
    value = node.get("community")
    if value is None:
        return None
    if tool == "compass":
        if not isinstance(value, dict):
            raise ValueError("Compass community must be an object")
        value = value.get("id")
    elif tool != "graphify":
        raise ValueError(f"unsupported tool: {tool}")
    if type(value) is not int or value < 0:
        raise ValueError("community id must be a nonnegative integer")
    return value


def audit_graph(graph: dict, tool: str, tasks: list[dict], limits: dict) -> dict:
    if tool not in {"compass", "graphify"}:
        raise ValueError(f"unsupported tool: {tool}")
    if not isinstance(tasks, list) or not 1 <= len(tasks) <= 64:
        raise ValueError("invalid task count")
    task_ids = [task["id"] for task in tasks]
    if any(not isinstance(identifier, str) or not identifier for identifier in task_ids) or len(set(task_ids)) != len(task_ids):
        raise ValueError("task ids must be nonempty and unique")
    if sum(len(task["declarations"]) for task in tasks) > MAX_DECLARATIONS:
        raise ValueError("declaration limit exceeded")
    for key, cap in [("maxNodes", 200000), ("maxEdges", 1000000)]:
        if type(limits[key]) is not int or not 1 <= limits[key] <= cap:
            raise ValueError("invalid graph record limit")
    nodes = graph.get("nodes")
    edges = graph.get("links")
    if not isinstance(nodes, list) or not isinstance(edges, list):
        raise ValueError("graph must contain node and link arrays")
    if len(nodes) > limits["maxNodes"] or len(edges) > limits["maxEdges"]:
        raise ValueError("graph record limit exceeded")
    index: dict[tuple, set[str]] = defaultdict(set)
    by_id = {}
    sizes = Counter()
    files: dict[int, set[str]] = defaultdict(set)
    for node in nodes:
        if not isinstance(node, dict):
            raise ValueError("node must be an object")
        identifier = node.get("id")
        if not isinstance(identifier, str) or not identifier or identifier in by_id:
            raise ValueError("node ids must be nonempty and unique")
        by_id[identifier] = node
        file, line, names = _node_anchor(node, tool)
        if file is not None and line is not None:
            for name in names:
                index[file, line, name].add(identifier)
        group = community_id(node, tool)
        if group is not None:
            sizes[group] += 1
            if file is not None:
                files[group].add(file)

    declarations = []
    seen_ids = set()
    seen_anchors = set()
    for task in tasks:
        for declaration in task["declarations"]:
            identifier = declaration["id"]
            key = (declaration["file"], declaration["startLine"], declaration["symbol"])
            if (not isinstance(identifier, str) or not identifier
                    or not isinstance(key[0], str) or not key[0]
                    or type(key[1]) is not int or key[1] <= 0
                    or not isinstance(key[2], str) or not key[2]):
                raise ValueError("invalid source declaration identity")
            if identifier in seen_ids or key in seen_anchors:
                raise ValueError("repeated source declaration in registration")
            seen_ids.add(identifier)
            seen_anchors.add(key)
            matches = sorted(index.get(key, set()))
            row = dict(id=identifier, task=task["id"], file=declaration["file"],
                       startLine=declaration["startLine"], symbol=declaration["symbol"],
                       matchedNodeIds=matches, status="missing", community=None)
            if len(matches) > 1:
                row["status"] = "ambiguous"
            elif matches:
                group = community_id(by_id[matches[0]], tool)
                row["community"] = group
                row["status"] = "unassigned" if group is None else "resolved"
                if group is not None:
                    row["communityNodes"] = sizes[group]
                    row["communitySourceFiles"] = len(files[group])
            declarations.append(row)
    declarations.sort(key=lambda row: row["id"])
    pairs = []
    summaries = defaultdict(Counter)
    for left, right in combinations(declarations, 2):
        kind = "within_task" if left["task"] == right["task"] else "cross_task"
        file_kind = "same_file" if left["file"] == right["file"] else "cross_file"
        outcome = "unresolved"
        if left["status"] == right["status"] == "resolved":
            outcome = "same_community" if left["community"] == right["community"] else "different_community"
        pairs.append(dict(left=left["id"], right=right["id"], kind=kind,
                          fileKind=file_kind, outcome=outcome))
        for scope in [kind, kind + "/" + file_kind]:
            summaries[scope][outcome] += 1
    return dict(nodes=len(nodes), assignedNodes=sum(sizes.values()), communities=len(sizes),
                declarations=declarations, pairs=pairs,
                summaries={key: {outcome: counts[outcome] for outcome in
                                 ["same_community", "different_community", "unresolved"]}
                           for key, counts in sorted(summaries.items())})


def verify_source(repository: dict, source: Path) -> None:
    commit = subprocess.run(["git", "-C", str(source), "rev-parse", "HEAD"],
                            check=True, capture_output=True, text=True, timeout=10).stdout.strip()
    if commit != repository["commit"]:
        raise ValueError("source commit differs from registration")
    root = source.resolve()
    for task in repository["tasks"]:
        for declaration in task["declarations"]:
            relative = Path(declaration["file"])
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("source path must be repository-relative")
            path = (root / relative).resolve()
            path.relative_to(root)
            data = read_bounded(path, MAX_SOURCE_BYTES)
            if digest(data) != declaration["sourceFileSha256"]:
                raise ValueError("source file digest differs from registration")
            start, end = declaration["startLine"], declaration["witnessEndLine"]
            lines = data.decode("utf-8").splitlines()
            if type(start) is not int or type(end) is not int or not 1 <= start <= end <= len(lines):
                raise ValueError("invalid source witness range")
            if "\n".join(lines[start - 1:end]) != declaration["witness"]:
                raise ValueError("source witness differs from registration")


def execute(registration_path: Path, run_path: Path, output: Path) -> None:
    registration_bytes = read_bounded(registration_path, MAX_SOURCE_BYTES)
    registration = json.loads(registration_bytes)
    if registration.get("schema") != "compass.community-task-pairs/1":
        raise ValueError("unsupported registration schema")
    if not 1 <= len(registration["repositories"]) <= 32:
        raise ValueError("invalid repository count")
    run_bytes = read_bounded(run_path, MAX_JSON_BYTES)
    if digest(run_bytes) != registration["sourceRunSha256"]:
        raise ValueError("source run digest differs from registration")
    run = json.loads(run_bytes)
    sources = {row["repository"]: row for row in run["repositories"]}
    report = dict(schema="compass.community-task-audit/1",
                  scope=registration["scope"], registrationSha256=digest(registration_bytes),
                  sourceRunSha256=digest(run_bytes),
                  auditorSha256=digest(read_bounded(Path(__file__), MAX_SOURCE_BYTES)),
                  anchorMatcherSha256=digest(read_bounded(Path(__file__).with_name("runner.py"), MAX_SOURCE_BYTES)),
                  repositories=[])
    bounds = registration["policy"]["bounds"]
    for repository in registration["repositories"]:
        previous = sources[repository["repository"]]
        source = Path(previous["source"])
        verify_source(repository, source)
        row = dict(repository=repository["repository"], commit=repository["commit"], tools={})
        for tool in ["compass", "graphify"]:
            data = read_bounded(Path(previous[tool + "Graph"]), min(bounds["maxGraphBytes"], MAX_JSON_BYTES))
            graph_digest = digest(data)
            if graph_digest != repository["graphSha256"][tool]:
                raise ValueError("graph digest differs from registration")
            result = audit_graph(json.loads(data), tool, repository["tasks"], bounds)
            result["graphSha256"] = graph_digest
            row["tools"][tool] = result
        report["repositories"].append(row)
    # Never replace an earlier diagnostic capture.
    with output.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    execute(arguments.registration, arguments.run, arguments.output)
