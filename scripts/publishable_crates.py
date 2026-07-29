#!/usr/bin/env python3
"""Print publishable Compass workspace crates in dependency-first order."""

from __future__ import annotations

import json
import sys


def main() -> int:
    metadata = json.load(sys.stdin)
    workspace_members = set(metadata["workspace_members"])
    workspace_packages = {
        package["name"]: package
        for package in metadata["packages"]
        if package["id"] in workspace_members
    }
    internal = {
        name
        for name, package in workspace_packages.items()
        if package["publish"] == []
    }
    publishable = {
        name: package
        for name, package in workspace_packages.items()
        if name.startswith("compass-")
        and name != "compass-tree-sitter-language-pack"
        and package["publish"] != []
    }

    for name, package in publishable.items():
        dependencies = {
            dependency["name"]
            for dependency in package["dependencies"]
            if dependency["kind"] != "dev"
        }
        forbidden = dependencies & internal
        if forbidden:
            joined = ", ".join(sorted(forbidden))
            raise SystemExit(f"{name} depends on internal crate(s): {joined}")

    remaining = set(publishable)
    ordered: list[str] = []
    while remaining:
        ready = sorted(
            name
            for name in remaining
            if not (
                {
                    dependency["name"]
                    for dependency in publishable[name]["dependencies"]
                    if dependency["kind"] != "dev"
                }
                & remaining
            )
        )
        if not ready:
            joined = ", ".join(sorted(remaining))
            raise SystemExit(f"publishable workspace dependency cycle: {joined}")
        ordered.extend(ready)
        remaining.difference_update(ready)

    print("\n".join(ordered))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
