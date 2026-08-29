#!/usr/bin/env python3
"""Run a bounded raw-JSON traversal baseline over qualification tasks."""

from __future__ import annotations

import argparse
from collections import deque
import json
from pathlib import Path
import sys
import time

if __package__ in {None, ""}:
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from benchmarks.performance.compass.jsonstream import iter_top_level_array
from benchmarks.qualification.io import atomic_write_text

SCHEMA = "compass.qualification-raw-traversal/1"
MAX_TASK_BYTES = 1024 * 1024
MAX_TASKS = 100
TASK_LIMIT_KEYS = {
    "maxDepth",
    "maxEdges",
    "maxNodes",
    "maxResultsPerTask",
    "timeoutSeconds",
}
EXPECTED_KEYS = {
    "status",
    "containsNodeIds",
    "minimumEdgeCount",
    "firstNodeId",
    "lastNodeId",
}


class LimitExceeded(RuntimeError):
    pass


def _required_text(record: dict[str, object], key: str, *, label: str) -> str:
    value = record.get(key)
    if not isinstance(value, str) or not value:
        raise ValueError(f"{label} {key} must be a non-empty string")
    return value


def _deadline(timeout_seconds: float) -> float:
    if timeout_seconds <= 0:
        raise ValueError("timeout must be positive")
    return time.monotonic() + timeout_seconds


def _check_time(deadline: float) -> None:
    if time.monotonic() > deadline:
        raise LimitExceeded("raw traversal time limit exceeded")


def load_graph(
    path: Path,
    *,
    max_graph_bytes: int,
    max_nodes: int,
    max_edges: int,
    deadline: float,
) -> tuple[
    dict[str, tuple[str, str]],
    dict[str, list[tuple[str, str]]],
    dict[str, list[tuple[str, str]]],
]:
    size = path.stat().st_size
    if size > max_graph_bytes:
        raise LimitExceeded(f"graph is {size} bytes; maximum is {max_graph_bytes}")
    nodes: dict[str, tuple[str, str]] = {}
    for record in iter_top_level_array(path, "nodes"):
        _check_time(deadline)
        if not isinstance(record, dict):
            raise ValueError("node records must be objects")
        if len(nodes) >= max_nodes:
            raise LimitExceeded(f"node count exceeds maximum {max_nodes}")
        node_id = _required_text(record, "id", label="node")
        if node_id in nodes:
            raise ValueError(f"duplicate node ID {node_id!r}")
        nodes[node_id] = (
            _required_text(record, "name", label=f"node {node_id!r}"),
            _required_text(record, "qualifiedName", label=f"node {node_id!r}"),
        )
    outgoing: dict[str, list[tuple[str, str]]] = {}
    incoming: dict[str, list[tuple[str, str]]] = {}
    edge_ids: set[str] = set()
    edge_count = 0
    for record in iter_top_level_array(path, "links"):
        _check_time(deadline)
        if not isinstance(record, dict):
            raise ValueError("edge records must be objects")
        if edge_count >= max_edges:
            raise LimitExceeded(f"edge count exceeds maximum {max_edges}")
        edge_id = _required_text(record, "id", label="edge")
        source = _required_text(record, "source", label=f"edge {edge_id!r}")
        target = _required_text(record, "target", label=f"edge {edge_id!r}")
        if source not in nodes or target not in nodes:
            raise ValueError(f"edge {edge_id!r} has invalid endpoints")
        if edge_id in edge_ids:
            raise ValueError(f"duplicate edge ID {edge_id!r}")
        edge_ids.add(edge_id)
        outgoing.setdefault(source, []).append((target, edge_id))
        incoming.setdefault(target, []).append((source, edge_id))
        edge_count += 1
    for adjacency in (outgoing, incoming):
        for values in adjacency.values():
            values.sort()
    return nodes, outgoing, incoming


def _resolve(
    nodes: dict[str, tuple[str, str]], value: str, *, deadline: float
) -> str:
    if value in nodes:
        return value
    matches = []
    for index, (node_id, (name, qualified_name)) in enumerate(nodes.items()):
        if index % 1024 == 0:
            _check_time(deadline)
        if value == name or value == qualified_name:
            matches.append(node_id)
    matches.sort()
    if len(matches) != 1:
        state = "missing" if not matches else "ambiguous"
        raise ValueError(f"symbol {value!r} is {state}")
    return matches[0]


def validate_task(task: object, *, index: int) -> dict[str, object]:
    if not isinstance(task, dict):
        raise ValueError(f"task at index {index} must be an object")
    task_id = task.get("id")
    if not isinstance(task_id, str) or not task_id:
        raise ValueError(f"task at index {index} requires a non-empty string id")
    operation = task.get("operation")
    if operation not in {"search", "callers", "callees", "impact", "path"}:
        raise ValueError(f"task {task_id!r} has unsupported operation {operation!r}")
    required = ("query",) if operation == "search" else ("source",)
    if operation == "path":
        required = ("source", "target")
    for key in required:
        value = task.get(key)
        if not isinstance(value, str) or not value:
            raise ValueError(f"task {task_id!r} requires a non-empty string {key}")
    if "maxDepth" in task:
        value = task["maxDepth"]
        if isinstance(value, bool) or not isinstance(value, int):
            raise ValueError(f"task {task_id!r} maxDepth must be an integer")
    expected = task.get("expected")
    if not isinstance(expected, dict):
        raise ValueError(f"task {task_id!r} requires an expected evidence object")
    unknown_expected = sorted(set(expected) - EXPECTED_KEYS)
    if unknown_expected:
        raise ValueError(
            f"task {task_id!r} has unsupported expected fields: "
            + ", ".join(unknown_expected)
        )
    if isinstance(expected, dict):
        status = expected.get("status")
        if status not in {"complete", "empty"}:
            raise ValueError(
                f"task {task_id!r} expected status must be 'complete' or 'empty'"
            )
        contains = expected.get("containsNodeIds", [])
        if not isinstance(contains, list) or any(
            not isinstance(value, str) or not value for value in contains
        ):
            raise ValueError(
                f"task {task_id!r} expected containsNodeIds must contain non-empty strings"
            )
        minimum_edges = expected.get("minimumEdgeCount")
        if minimum_edges is not None and (
            isinstance(minimum_edges, bool)
            or not isinstance(minimum_edges, int)
            or minimum_edges < 0
        ):
            raise ValueError(
                f"task {task_id!r} expected minimumEdgeCount must be a non-negative integer"
            )
        for key in ("firstNodeId", "lastNodeId"):
            value = expected.get(key)
            if value is not None and (not isinstance(value, str) or not value):
                raise ValueError(
                    f"task {task_id!r} expected {key} must be a non-empty string"
                )
    return task


def validate_declared_limits(value: object, args: argparse.Namespace) -> None:
    if value is None:
        return
    if not isinstance(value, dict) or set(value) != TASK_LIMIT_KEYS:
        raise ValueError(
            "tasks limits must contain exactly maxDepth, maxEdges, maxNodes, "
            "maxResultsPerTask, and timeoutSeconds"
        )
    effective: dict[str, int | float] = {
        "maxDepth": args.max_depth,
        "maxEdges": args.max_edges,
        "maxNodes": args.max_nodes,
        "maxResultsPerTask": args.max_results,
        "timeoutSeconds": args.timeout_seconds,
    }
    for key, actual in value.items():
        if key == "timeoutSeconds":
            valid_type = not isinstance(actual, bool) and isinstance(
                actual, (int, float)
            )
        else:
            valid_type = not isinstance(actual, bool) and isinstance(actual, int)
        if not valid_type:
            expected_type = "numeric" if key == "timeoutSeconds" else "an integer"
            raise ValueError(f"tasks limit {key} must be {expected_type}")
        if actual <= 0:
            raise ValueError(f"tasks limit {key} must be positive")
        if actual != effective[key]:
            raise ValueError(
                f"tasks limit {key}={actual} does not match effective "
                f"value {effective[key]}"
            )


def verify_expected(task: dict[str, object], result: dict[str, object]) -> None:
    expected = task.get("expected")
    if not isinstance(expected, dict):
        return
    task_id = str(task["id"])
    actual_status = result.get("status")
    if actual_status not in {"complete", "empty"}:
        raise ValueError(f"task {task_id!r} result requires an explicit status")
    if actual_status != expected.get("status"):
        raise ValueError(
            f"task {task_id!r} status {actual_status!r} does not match expected "
            f"{expected.get('status')!r}"
        )
    node_ids = result.get("nodeIds", [])
    edge_ids = result.get("edgeIds", [])
    if not isinstance(node_ids, list) or not isinstance(edge_ids, list):
        raise ValueError(f"task {task_id!r} result arrays are malformed")
    required_nodes = expected.get("containsNodeIds", [])
    if not isinstance(required_nodes, list) or not set(required_nodes).issubset(node_ids):
        raise ValueError(f"task {task_id!r} is missing expected node evidence")
    minimum_edges = expected.get("minimumEdgeCount")
    if minimum_edges is not None and len(edge_ids) < minimum_edges:
        raise ValueError(f"task {task_id!r} has fewer edges than expected")
    first = expected.get("firstNodeId")
    if first is not None and (not node_ids or node_ids[0] != first):
        raise ValueError(f"task {task_id!r} has the wrong first path node")
    last = expected.get("lastNodeId")
    if last is not None and (not node_ids or node_ids[-1] != last):
        raise ValueError(f"task {task_id!r} has the wrong last path node")


def execute_task(
    task: dict[str, object],
    nodes: dict[str, tuple[str, str]],
    outgoing: dict[str, list[tuple[str, str]]],
    incoming: dict[str, list[tuple[str, str]]],
    *,
    max_depth: int,
    max_results: int,
    deadline: float,
) -> dict[str, object]:
    operation = str(task.get("operation", ""))
    if operation == "search":
        query = str(task["query"]).casefold()
        matches = []
        for index, (node_id, values) in enumerate(nodes.items()):
            if index % 1024 == 0:
                _check_time(deadline)
            if query in values[0].casefold() or query in values[1].casefold():
                matches.append(node_id)
        matches.sort()
        if len(matches) > max_results:
            raise LimitExceeded(f"search results exceed maximum {max_results}")
        return {"status": "complete" if matches else "empty", "nodeIds": matches}
    source = _resolve(nodes, str(task["source"]), deadline=deadline)
    if operation in {"callers", "callees"}:
        adjacency = incoming if operation == "callers" else outgoing
        pairs = adjacency.get(source, [])
        if len(pairs) > max_results:
            raise LimitExceeded(f"{operation} results exceed maximum {max_results}")
        return {
            "status": "complete" if pairs else "empty",
            "nodeIds": [node for node, _ in pairs],
            "edgeIds": [edge for _, edge in pairs],
        }
    if operation == "impact":
        requested_depth = int(task.get("maxDepth", max_depth))
        if requested_depth < 0 or requested_depth > max_depth:
            raise LimitExceeded(f"impact depth exceeds maximum {max_depth}")
        seen = {source}
        queue = deque([(source, 0)])
        edge_ids: list[str] = []
        while queue:
            _check_time(deadline)
            current, depth = queue.popleft()
            if depth >= requested_depth:
                continue
            for target, edge_id in incoming.get(current, []):
                edge_ids.append(edge_id)
                if target not in seen:
                    seen.add(target)
                    queue.append((target, depth + 1))
                if len(seen) > max_results or len(edge_ids) > max_results:
                    raise LimitExceeded(f"impact results exceed maximum {max_results}")
        return {
            "status": "complete",
            "nodeIds": sorted(seen),
            "edgeIds": sorted(set(edge_ids)),
        }
    if operation == "path":
        target = _resolve(nodes, str(task["target"]), deadline=deadline)
        queue = deque([(source, 0)])
        previous: dict[str, tuple[str, str] | None] = {source: None}
        while queue:
            _check_time(deadline)
            current, depth = queue.popleft()
            if current == target:
                break
            if depth >= max_depth:
                continue
            for next_node, edge_id in outgoing.get(current, []):
                if next_node in previous:
                    continue
                previous[next_node] = (current, edge_id)
                queue.append((next_node, depth + 1))
                if len(previous) > max_results:
                    raise LimitExceeded(f"path expansion exceeds maximum {max_results}")
        if target not in previous:
            return {"status": "empty", "nodeIds": [], "edgeIds": []}
        node_ids = [target]
        edge_ids = []
        cursor = target
        while previous[cursor] is not None:
            parent, edge_id = previous[cursor]
            edge_ids.append(edge_id)
            node_ids.append(parent)
            cursor = parent
        node_ids.reverse()
        edge_ids.reverse()
        return {"status": "complete", "nodeIds": node_ids, "edgeIds": edge_ids}
    raise ValueError(f"unsupported operation {operation!r}")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--tasks", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--max-graph-bytes", type=int, default=2 * 1024**3)
    parser.add_argument("--max-nodes", type=int, default=1_000_000)
    parser.add_argument("--max-edges", type=int, default=2_500_000)
    parser.add_argument("--max-depth", type=int, default=32)
    parser.add_argument("--max-results", type=int, default=10_000)
    parser.add_argument("--timeout-seconds", type=float, default=120.0)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    deadline = _deadline(args.timeout_seconds)
    with args.tasks.open("rb") as task_stream:
        task_contents = task_stream.read(MAX_TASK_BYTES + 1)
    if len(task_contents) > MAX_TASK_BYTES:
        raise LimitExceeded(
            f"tasks document exceeds maximum {MAX_TASK_BYTES} bytes"
        )
    tasks_document = json.loads(task_contents.decode("utf-8"))
    if not isinstance(tasks_document, dict):
        raise ValueError("tasks document must be an object")
    validate_declared_limits(tasks_document.get("limits"), args)
    tasks = tasks_document.get("tasks")
    if not isinstance(tasks, list):
        raise ValueError("tasks document requires a tasks array")
    if not tasks or len(tasks) > MAX_TASKS:
        raise LimitExceeded(f"task count must be in 1..={MAX_TASKS}")
    validated_tasks = [validate_task(task, index=index) for index, task in enumerate(tasks)]
    started = time.monotonic_ns()
    nodes, outgoing, incoming = load_graph(
        args.graph,
        max_graph_bytes=args.max_graph_bytes,
        max_nodes=args.max_nodes,
        max_edges=args.max_edges,
        deadline=deadline,
    )
    results = []
    for task in validated_tasks:
        _check_time(deadline)
        task_started = time.monotonic_ns()
        result = execute_task(
            task,
            nodes,
            outgoing,
            incoming,
            max_depth=args.max_depth,
            max_results=args.max_results,
            deadline=deadline,
        )
        verify_expected(task, result)
        results.append(
            {
                "id": task.get("id"),
                "elapsedMicroseconds": (time.monotonic_ns() - task_started) // 1_000,
                "result": result,
            }
        )
    payload = {
        "schema": SCHEMA,
        "oracleStatus": "PASS",
        "graph": str(args.graph),
        "nodeCount": len(nodes),
        "edgeCount": sum(len(items) for items in outgoing.values()),
        "elapsedMicroseconds": (time.monotonic_ns() - started) // 1_000,
        "limits": {
            "maxDepth": args.max_depth,
            "maxEdges": args.max_edges,
            "maxGraphBytes": args.max_graph_bytes,
            "maxNodes": args.max_nodes,
            "maxResults": args.max_results,
            "timeoutSeconds": args.timeout_seconds,
        },
        "results": results,
    }
    encoded = json.dumps(payload, indent=2, sort_keys=True) + "\n"
    if args.output is None:
        sys.stdout.write(encoded)
    else:
        atomic_write_text(args.output, encoded)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (LimitExceeded, OSError, TypeError, ValueError) as error:
        print(f"raw traversal failed: {error}", file=sys.stderr)
        raise SystemExit(2) from error
