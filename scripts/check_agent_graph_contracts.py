#!/usr/bin/env python3
"""Lint the frozen Agent Graph v1 JSON fixtures without implementing semantics."""

from __future__ import annotations

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures" / "contracts" / "agent-graph"
EXPECTED_SCHEMAS = {
    "audit-v1.json": "compass.agent-graph.audit/1",
    "batch-v1.json": "compass.agent-graph.batch/1",
    "effective-v1.json": "compass.agent-graph.effective/1",
    "errors-v1.json": "compass.agent-graph.errors/1",
    "limits-v1.json": "compass.agent-graph.limits/1",
    "overlay-v1.json": "compass.agent-graph.overlay/1",
    "rebase-plan-v1.json": "compass.agent-graph.rebase-plan/1",
    "receipt-v1.json": "compass.agent-graph.receipt/1",
}


def main() -> int:
    problems: list[str] = []
    documents: dict[str, object] = {}
    for name, schema in EXPECTED_SCHEMAS.items():
        path = FIXTURES / name
        try:
            raw = path.read_text(encoding="utf-8")
            document = json.loads(raw)
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            problems.append(f"{name}: cannot read strict JSON: {error}")
            continue
        documents[name] = document
        if not isinstance(document, dict) or document.get("schema") != schema:
            problems.append(f"{name}: schema must be {schema}")
        formatted = json.dumps(document, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
        if raw != formatted:
            problems.append(f"{name}: JSON must use deterministic sorted pretty formatting")

    errors = documents.get("errors-v1.json")
    if isinstance(errors, dict):
        examples = errors.get("negativeExamples")
        ids = [item.get("id") for item in examples] if isinstance(examples, list) else []
        if not ids or any(not isinstance(item, str) or not item for item in ids):
            problems.append("errors-v1.json: every negative example requires a non-empty ID")
        elif len(ids) != len(set(ids)):
            problems.append("errors-v1.json: negative example IDs must be unique")

    for name, document in documents.items():
        encoded = json.dumps(document, ensure_ascii=False)
        if "GROUNDED" in encoded and name not in {"overlay-v1.json", "errors-v1.json"}:
            problems.append(f"{name}: GROUNDED is valid only in output or rejection fixtures")
        if name != "effective-v1.json" and '"directed"' in encoded:
            problems.append(f"{name}: contract fixture must not embed a complete Base Graph")

    if problems:
        for problem in sorted(problems):
            print(problem, file=sys.stderr)
        return 1
    print(f"validated {len(documents)} Agent Graph contract fixtures")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
