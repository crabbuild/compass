#!/usr/bin/env python3
"""Verify the registered three-language run against source-bound call edges.

The runner's text checks deliberately remain unchanged. This review gives a
path credit only when its actual output is a one-hop call between the pinned
declarations, rather than a structural path that happens to name both symbols.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import tomllib
from pathlib import Path


FACTS = {
    "litestream": (
        ("store.go", 715, "sendHeartbeatIfNeeded", "heartbeat.go", 68, "ShouldPing", 721),
        ("store.go", 715, "sendHeartbeatIfNeeded", "heartbeat.go", 80, "RecordPing", 735),
        ("store.go", 715, "sendHeartbeatIfNeeded", "heartbeat.go", 45, "Ping", 737),
    ),
    "fastapi": (
        ("fastapi/routing.py", 1225, "get_route_handler", "fastapi/routing.py", 375, "get_request_handler", 1232),
    ),
    "celld": (
        ("crates/celld/peer_auth.rs", 137, "signed_headers_parts", "crates/celld/peer_auth.rs", 341, "now_ms", 145),
        ("crates/celld/peer_auth.rs", 137, "signed_headers_parts", "crates/celld/peer_auth.rs", 352, "sha256_hex_parts", 149),
    ),
}

QUESTION_FACTS = {
    "litestream-callees-heartbeat": (0, 1, 2),
    "litestream-callers-should-ping": (0,),
    "litestream-path-heartbeat": (0,),
    "fastapi-callees-route-handler": (0,),
    "fastapi-callers-request-handler": (0,),
    "fastapi-path-route-request": (0,),
    "celld-callees-signed-headers": (0, 1),
    "celld-callers-sha-parts": (1,),
    "celld-path-signed-sha": (1,),
}

DIRECT_PATH_LINES = {
    "litestream-path-heartbeat": "  .sendHeartbeatIfNeeded() --calls [EXTRACTED]--> .ShouldPing()",
    "fastapi-path-route-request": "  .get_route_handler() --calls [EXTRACTED]--> get_request_handler()",
    "celld-path-signed-sha": "  .signed_headers_parts() --calls [EXTRACTED]--> sha256_hex_parts()",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def line_number(value: object) -> int | None:
    if isinstance(value, int):
        return value
    if isinstance(value, str) and re.fullmatch(r"L\d+", value):
        return int(value[1:])
    return None


def node_location(node: dict, tool: str) -> tuple[str | None, int | None]:
    if tool == "compass":
        source = node.get("source") or {}
        return source.get("file"), line_number(source.get("startLine"))
    return node.get("source_file"), line_number(node.get("source_location"))


def node_name(node: dict, tool: str) -> str:
    if tool == "compass":
        return node.get("qualifiedName", "").split("::")[-1].split(".")[-1]
    return node.get("label", "").lstrip(".").removesuffix("()")


def exact_node(graph: dict, tool: str, file: str, line: int, symbol: str) -> dict:
    matches = [
        node for node in graph["nodes"]
        if node_location(node, tool) == (file, line)
        and node_name(node, tool) == symbol
    ]
    if len(matches) != 1:
        raise ValueError(f"{tool}: expected one {file}:{line} {symbol}, found {len(matches)}")
    return matches[0]


def call_sites(graph: dict, tool: str, fact: tuple) -> list[dict]:
    source_file, source_line, source_symbol, target_file, target_line, target_symbol, call_line = fact
    source = exact_node(graph, tool, source_file, source_line, source_symbol)
    target = exact_node(graph, tool, target_file, target_line, target_symbol)
    return [
        edge for edge in graph["links"]
        if edge["source"] == source["id"]
        and edge["target"] == target["id"]
        and edge.get("kind", edge.get("relation")) == "calls"
        and (
            (edge.get("relationshipSite") or {}).get("file", edge.get("source_file")),
            line_number((edge.get("relationshipSite") or {}).get("startLine", edge.get("source_location"))),
        ) == (source_file, call_line)
    ]


def direct_call_output(text: str, tool: str, identifier: str) -> bool:
    header = "Best path (weighted, 1 hops" if tool == "compass" else "Shortest path (1 hops):"
    return header in text and DIRECT_PATH_LINES[identifier] in text.splitlines()


def review(suite_path: Path, run_root: Path, sources: dict[str, Path]) -> dict:
    suite = tomllib.loads(suite_path.read_text())
    run_path = run_root / "run.json"
    run = json.loads(run_path.read_text())
    if sha256(suite_path) != run["suiteDigest"] or sha256(run_root / "suite.toml") != run["suiteDigest"]:
        raise ValueError("suite bytes changed after registration")
    if sha256(run_root / "runner.py") != run["runnerDigest"]:
        raise ValueError("captured runner digest differs")
    if set(sources) != set(FACTS):
        raise ValueError("supply exactly the three registered source roots")
    for tool in run["tools"]:
        if sha256(Path(tool["binary"])) != tool["binarySha256"]:
            raise ValueError(f"{tool['name']} binary digest changed")

    source_pins = []
    graphs = {}
    graph_pins = []
    for repository in run["repositories"]:
        name = repository["repository"]
        root = sources[name]
        registered = next(item for item in suite["repository"] if item["name"] == name)
        if git(root, "rev-parse", "HEAD") != registered["commit"] or git(root, "status", "--porcelain=v1"):
            raise ValueError(f"{name} checkout is not clean and pinned")
        files = {fact[0] for fact in FACTS[name]} | {fact[3] for fact in FACTS[name]}
        files |= {anchor["file"] for anchor in registered["anchor"]}
        for file in sorted(files):
            source_pins.append({"repository": name, "file": file, "sha256": sha256(root / file)})
        for anchor in registered["anchor"]:
            line = (root / anchor["file"]).read_text().splitlines()[anchor["line"] - 1]
            if anchor["symbol"] not in line:
                raise ValueError(f"{name} source anchor changed: {anchor}")
        for tool in ("compass", "graphify"):
            path = Path(repository[f"{tool}Graph"])
            digest = repository[f"{tool}GraphSha256"]
            if sha256(path) != digest:
                raise ValueError(f"{name} {tool} graph digest changed")
            graphs[(name, tool)] = json.loads(path.read_text())
            graph_pins.append({"repository": name, "tool": tool, "sha256": digest})

    relationship_facts = []
    for name, facts in FACTS.items():
        root = sources[name]
        for index, fact in enumerate(facts):
            source_file, _, _, _, _, target_symbol, call_line = fact
            line = (root / source_file).read_text().splitlines()[call_line - 1]
            if target_symbol not in line:
                raise ValueError(f"{name} call token absent at {source_file}:{call_line}")
            counts = {tool: len(call_sites(graphs[(name, tool)], tool, fact)) for tool in ("compass", "graphify")}
            relationship_facts.append({
                "repository": name,
                "factIndex": index,
                "sourceFile": source_file,
                "callLine": call_line,
                "caller": fact[2],
                "callee": target_symbol,
                "edgeCounts": counts,
            })

    fact_counts = {(fact["repository"], fact["factIndex"]): fact["edgeCounts"] for fact in relationship_facts}
    expected_questions = {
        (repository["name"], question["id"], tool)
        for repository in suite["repository"]
        for question in repository["question"]
        for tool in ("compass", "graphify")
    }
    actual_questions = [(item["repository"], item["question"], item["tool"]) for item in run["observations"]]
    if len(actual_questions) != len(expected_questions) or set(actual_questions) != expected_questions:
        raise ValueError("run observations differ from the registered question/tool pairs")
    questions = []
    totals = {"compass": 0, "graphify": 0}
    for observation in run["observations"]:
        name = observation["repository"]
        tool = observation["tool"]
        identifier = observation["question"]
        raw = run_root / "raw" / name / f"{identifier}.{tool}.0.stdout"
        text = raw.read_text()
        if len(text.encode()) != observation["stdoutBytes"]:
            raise ValueError(f"raw stdout byte count changed for {identifier} {tool}")
        facts = QUESTION_FACTS.get(identifier, ())
        all_edges = all(fact_counts[(name, index)][tool] == 1 for index in facts)
        direct_path = None
        if observation["kind"] == "path":
            direct_path = direct_call_output(text, tool, identifier)
        supported = bool(observation["passed"] and all_edges and (direct_path is not False))
        totals[tool] += supported
        questions.append({
            "repository": name,
            "question": identifier,
            "tool": tool,
            "stdoutSha256": sha256(raw),
            "registeredTextPass": observation["passed"],
            "requiredCallEdgesPresent": all_edges,
            "renderedDirectCallPath": direct_path,
            "sourceSupportedPass": supported,
        })
    return {
        "schema": "compass.source-first-three-language-review/1",
        "scope": "Selected source-first Go/Python/Rust development questions; source-grounded static calls, not runtime reachability, authored explanations, or whole-graph accuracy",
        "registrationCommit": "ab93cd34",
        "suiteSha256": run["suiteDigest"],
        "runnerSha256": run["runnerDigest"],
        "runJsonSha256": sha256(run_path),
        "sourcePins": source_pins,
        "graphPins": graph_pins,
        "relationshipFacts": relationship_facts,
        "questions": questions,
        "totals": {"registeredText": {tool: sum(o["passed"] for o in run["observations"] if o["tool"] == tool) for tool in totals}, "sourceSupported": totals},
        "caveat": "A source-backed edge does not validate every other returned relationship or explanation claim. Short-name ambiguity remains a valid refusal; structural paths do not answer direct-call requests.",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", type=Path, required=True)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--source", action="append", required=True, metavar="NAME=PATH")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    sources = {}
    for entry in args.source:
        name, separator, path = entry.partition("=")
        if not separator or name in sources:
            raise ValueError("--source needs distinct NAME=PATH values")
        sources[name] = Path(path)
    result = review(args.suite, args.run, sources)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result["totals"], sort_keys=True))


if __name__ == "__main__":
    main()
