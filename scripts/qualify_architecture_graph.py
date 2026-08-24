#!/usr/bin/env python3
"""Fail closed when an architecture projection regresses its UX invariants."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

MAX_ARCHITECTURE_BYTES = 128 * 1024 * 1024


def fail(message: str) -> None:
    raise ValueError(message)


def projection(model: dict, scope: str) -> dict:
    matches = [item for item in model.get("projections", []) if item.get("scope") == scope]
    if len(matches) != 1:
        fail(f"expected exactly one {scope!r} projection, found {len(matches)}")
    return matches[0]


def check(path: Path) -> dict:
    size = path.stat().st_size
    if size > MAX_ARCHITECTURE_BYTES:
        fail(f"architecture payload is {size} bytes; limit is {MAX_ARCHITECTURE_BYTES}")
    with path.open("r", encoding="utf-8") as handle:
        model = json.load(handle)
    if model.get("schema") != "compass.viewer.architecture/1":
        fail(f"unsupported architecture schema {model.get('schema')!r}")
    nodes = {node["id"]: node for node in model.get("nodes", [])}
    relationships = model.get("relationships", [])
    allowed_classes = {"execution", "dependency", "type", "structure", "contextual", "unknown"}
    for relationship in relationships:
        if relationship.get("source") not in nodes or relationship.get("target") not in nodes:
            fail(f"relationship {relationship.get('id')} has a dangling endpoint")
        if relationship.get("relationClass") not in allowed_classes:
            fail(f"relationship {relationship.get('id')} has no closed relation class")

    production = projection(model, "production")
    all_code = projection(model, "all_code")
    for scope_projection in (production, all_code):
        groups_in_scope = scope_projection.get("groups", [])
        for item in scope_projection.get("memberships", []):
            node_index = item.get("nodeIndex")
            group_index = item.get("groupIndex")
            if not isinstance(node_index, int) or not 0 <= node_index < len(model["nodes"]):
                fail(f"{scope_projection['scope']} membership has an invalid node index")
            if not isinstance(group_index, int) or not 0 <= group_index < len(groups_in_scope):
                fail(f"{scope_projection['scope']} membership has an invalid group index")
    production_members = {
        model["nodes"][item["nodeIndex"]]["id"]
        for item in production.get("memberships", [])
    }
    leaked = sorted(
        node_id
        for node_id in production_members
        if nodes[node_id].get("sourceScope") != "production"
    )
    if leaked:
        fail(f"non-production nodes shaped Production: {leaked[:8]}")
    if len(all_code.get("memberships", [])) != len(nodes):
        fail("All-code does not retain every classified node")

    groups = production.get("groups", [])
    names = [group.get("name", {}).get("value", "").strip() for group in groups]
    if any(name.casefold() == "other" for name in names):
        fail("automatic Other group is forbidden")
    normalized_names = [name.casefold() for name in names]
    if len(normalized_names) != len(set(normalized_names)):
        fail("Production group names are not unique")

    coverage = production.get("coverage", {})
    if coverage.get("admitted") != sum(
        coverage.get(field, 0) for field in ("internal", "crossGroup", "unassigned")
    ):
        fail("Production relationship coverage does not sum to admitted")
    omissions = production.get("omissions", {})
    if omissions.get("totalGroups") != omissions.get("shownGroups", 0) + omissions.get("omittedGroups", 0):
        fail("overview group omissions do not sum to total groups")
    quality = production.get("quality", {})
    metrics = quality.get("metrics", {})
    if metrics.get("generatedVendorLeakage") != 0:
        fail("generated/vendor nodes leaked into Production")
    if metrics.get("duplicateNames") != 0:
        fail("Production quality reports duplicate names")
    if quality.get("status") == "insufficient":
        fail("Production architecture quality is insufficient")

    return {
        "schema": model["schema"],
        "nodes": len(nodes),
        "relationships": len(relationships),
        "productionGroups": len(groups),
        "shownGroups": omissions.get("shownGroups", 0),
        "omittedGroups": omissions.get("omittedGroups", 0),
        "quality": quality.get("status"),
        "unknownSourceFraction": metrics.get("unknownSourceFraction"),
        "largestGroupFraction": metrics.get("largestGroupFraction"),
        "fallbackNames": metrics.get("fallbackNames"),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("architecture_json", type=Path)
    args = parser.parse_args()
    try:
        summary = check(args.architecture_json)
    except (OSError, json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
        print(f"architecture qualification failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
