#!/usr/bin/env python3
"""Evaluate pinned concept questions against native query_batch responses."""
import argparse
import json
from pathlib import Path

MAX_BYTES = 64 * 1024 * 1024


def read(path):
    with path.open("rb") as source:
        data = source.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise ValueError(f"{path}: input exceeds 64 MiB")
    return json.loads(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--responses", type=Path)
    args = parser.parse_args()
    corpus = read(args.corpus)
    cases = corpus["cases"]
    if corpus["schema"] != "compass.concept-qualification/1" or not 1 <= len(cases) <= 32:
        raise ValueError("unsupported corpus schema or question count")
    if args.responses is None:
        print(json.dumps([{"operation": "ask", "symbol": case["question"]} for case in cases]))
        return 0
    responses = read(args.responses)
    if len(responses) != len(cases):
        raise ValueError("response count does not match pinned question count")
    rows = []
    for case, response in zip(cases, responses):
        if response["schema"] != "compass.query/1":
            raise ValueError("unsupported native response schema")
        nodes = {node["id"]: node for node in response["nodes"]}
        paths = []
        for hit in response["results"]:
            node = nodes[hit["nodeId"]]
            source = node.get("source")
            if source is None:
                continue
            path = source["file"]
            if any(part in {"tests", "test", "fixtures", "mocks"} for part in Path(path).parts):
                continue
            if path not in paths:
                paths.append(path)
            if len(paths) == 3:
                break
        correct = [any(path.startswith(prefix) for prefix in case["expected_paths"]) for path in paths]
        rows.append({"question": case["question"], "top_paths": paths, "top_one": bool(correct and correct[0]), "top_three": any(correct), "truncated": response["truncated"]})
    report = {"schema": "compass.concept-qualification-report/1", "repository": corpus["repository"], "commit": corpus["commit"], "correct_top_one": sum(row["top_one"] for row in rows), "correct_top_three": sum(row["top_three"] for row in rows), "questions": len(rows), "minimum_correct": corpus["minimum_correct"], "cases": rows}
    print(json.dumps(report, indent=2))
    return 0 if report["correct_top_three"] >= corpus["minimum_correct"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
