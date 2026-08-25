#!/usr/bin/env python3
"""Run the bounded React package/config precedence matrix with one binary.

The matrix is qualification-only input. It creates a small two-package
workspace, changes one package/config input at a time, and checks that the
affected package changes (or is invalidated) while the sibling package stays
byte-stable. No package manager or project script is executed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
MATRIX = ROOT / "tests/qualification/react-frontend-precedence-matrix.json"
MAX_OUTPUT_BYTES = 8 * 1024 * 1024
COMMAND_TIMEOUT_SECONDS = 180


class MatrixError(RuntimeError):
    pass


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest(value: Any) -> str:
    return hashlib.sha256(canonical(value)).hexdigest()


def load_matrix() -> dict[str, Any]:
    try:
        document = json.loads(MATRIX.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise MatrixError(f"cannot read precedence matrix: {error}") from error
    if document.get("schema") != "compass.react-frontend-precedence-matrix/1":
        raise MatrixError("precedence matrix has an unknown schema")
    cases = document.get("cases")
    if not isinstance(cases, list) or not cases:
        raise MatrixError("precedence matrix must contain cases")
    ids: set[str] = set()
    for case in cases:
        if not isinstance(case, dict):
            raise MatrixError("precedence matrix case must be an object")
        identifier = case.get("id")
        if not isinstance(identifier, str) or not identifier or identifier in ids:
            raise MatrixError(f"precedence matrix case ID is empty or duplicated: {identifier!r}")
        ids.add(identifier)
        for key in ("manager", "mutation", "input", "affected"):
            if not isinstance(case.get(key), str) or not case[key]:
                raise MatrixError(f"precedence matrix case {identifier} is missing {key}")
        if not isinstance(case.get("expectGraphChange"), bool):
            raise MatrixError(f"precedence matrix case {identifier} has invalid expectGraphChange")
    return document


def write_fixture(root: Path, manager: str) -> dict[str, bytes]:
    files: dict[str, str] = {
        "package.json": json.dumps(
            {"private": True, "workspaces": ["apps/*"], "packageManager": f"{manager}@10"},
            sort_keys=True,
        )
        + "\n",
        "tsconfig.base.json": '{"compilerOptions":{"jsx":"react-jsx","jsxImportSource":"react","baseUrl":"."}}\n',
        "apps/web/package.json": '{"dependencies":{"react":"19.0.0"}}\n',
        "apps/web/tsconfig.json": '{"extends":"../../tsconfig.base.json","references":[{"path":"../shared"}],"compilerOptions":{"paths":{"@/*":["./src/*"]}}}\n',
        "apps/web/src/App.tsx": 'import React from "react";\nimport Widget from "@/Widget";\nexport function App() { return <Widget />; }\n',
        "apps/web/src/Widget.tsx": 'export default function Widget() { return <main />; }\n',
        "apps/docs/package.json": '{"dependencies":{"preact":"10.0.0"}}\n',
        "apps/docs/tsconfig.json": '{"extends":"../../tsconfig.base.json","compilerOptions":{"jsxImportSource":"preact"}}\n',
        "apps/docs/src/App.tsx": 'import { h } from "preact";\nexport function Docs() { return <article />; }\n',
        "apps/shared/tsconfig.json": '{"compilerOptions":{"composite":true}}\n',
    }
    lockfiles = {
        "npm": ("package-lock.json", '{"lockfileVersion":3}\n'),
        "yarn": ("yarn.lock", "__metadata:\n  version: 6\n"),
        "pnpm": ("pnpm-lock.yaml", "lockfileVersion: '9.0'\n"),
    }
    lockfile, contents = lockfiles[manager]
    files[lockfile] = contents
    if manager == "pnpm":
        files["pnpm-workspace.yaml"] = "packages:\n  - apps/*\n"
    original: dict[str, bytes] = {}
    for relative, contents in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        encoded = contents.encode()
        path.write_bytes(encoded)
        original[relative] = encoded
    return original


def run_compass(binary: Path, project: Path, output: Path) -> tuple[dict[str, Any], dict[str, Any], str]:
    command = [
        str(binary),
        "update",
        str(project),
        "--out",
        str(output),
        "--no-cluster",
        "--no-viz",
        "--no-gitignore",
        "--inference-level",
        "max",
    ]
    try:
        completed = subprocess.run(
            command,
            cwd=ROOT,
            env={**__import__("os").environ, "COMPASS_OFFLINE": "1", "NO_COLOR": "1"},
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=COMMAND_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise MatrixError(f"Compass matrix command failed to start: {error}") from error
    if len(completed.stdout) + len(completed.stderr) > MAX_OUTPUT_BYTES:
        raise MatrixError("Compass matrix command exceeded its output limit")
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).decode("utf-8", errors="replace")
        raise MatrixError(f"Compass matrix update failed: {detail[-2000:]}")
    pointer = output / "compass-out" / "current-snapshot"
    snapshot = pointer.read_text(encoding="utf-8").strip()
    if not snapshot.startswith("snapshot-") or "/" in snapshot or "\\" in snapshot:
        raise MatrixError(f"invalid Compass snapshot pointer: {snapshot!r}")
    snapshot_root = output / "compass-out" / "snapshots" / snapshot
    graph = json.loads((snapshot_root / "graph.json").read_text(encoding="utf-8"))
    manifest = json.loads((snapshot_root / "manifest.json").read_text(encoding="utf-8"))
    fact_digests = json.loads((snapshot_root / "ast-fact-digests.json").read_text(encoding="utf-8"))
    project_evidence_digest = fact_digests.get("project_evidence_digest")
    if not isinstance(project_evidence_digest, str) or not project_evidence_digest:
        raise MatrixError("Compass did not publish a project-evidence digest")
    return graph, manifest, project_evidence_digest


def subgraph_digest(graph: dict[str, Any], prefix: str) -> str:
    prefix = prefix.rstrip("/") + "/"
    nodes = [
        node for node in graph.get("nodes", [])
        if isinstance(node, dict)
        and isinstance(node.get("source"), dict)
        and isinstance(node["source"].get("file"), str)
        and node["source"]["file"].startswith(prefix)
    ]
    node_ids = {node.get("id") for node in nodes}
    links = []
    for link in graph.get("links", []):
        if not isinstance(link, dict):
            continue
        site = link.get("relationshipSite")
        site_file = site.get("file") if isinstance(site, dict) else None
        if link.get("source") in node_ids or link.get("target") in node_ids or (
            isinstance(site_file, str) and site_file.startswith(prefix)
        ):
            links.append(link)
    return digest({"nodes": nodes, "links": links})


def manifest_input_digest(manifest: dict[str, Any], relative: str) -> str | None:
    value = manifest.get(relative)
    return value.get("ast_hash") if isinstance(value, dict) else None


def mutate(path: Path, mutation: str) -> bytes:
    original = path.read_bytes()
    source = original.decode("utf-8")
    if mutation == "web-package-runtime":
        source = source.replace('"react":"19.0.0"', '"react":"npm:preact@10.0.0"')
    elif mutation == "lockfile":
        source += "importers: {}\n"
    elif mutation == "root-tsconfig-runtime":
        source = source.replace('"jsxImportSource":"react"', '"jsxImportSource":"preact"')
    elif mutation == "web-jsx-import-source":
        source = source.replace('"compilerOptions":{"paths"', '"compilerOptions":{"jsxImportSource":"preact","paths"')
    elif mutation == "shared-reference":
        source = source.replace('"composite":true', '"composite":false')
    elif mutation == "web-alias":
        source = source.replace('"./src/*"', '"./missing/*"')
    else:
        raise MatrixError(f"unknown matrix mutation: {mutation}")
    if source == original.decode("utf-8"):
        raise MatrixError(f"matrix mutation did not change {path}")
    path.write_text(source, encoding="utf-8")
    return original


def run_case(binary: Path, case: dict[str, Any]) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"compass-react-matrix-{case['id']}-") as directory:
        root = Path(directory) / "project"
        original = write_fixture(root, case["manager"])
        output = Path(directory) / "out"
        before_graph, before_manifest, before_evidence = run_compass(binary, root, output)
        affected_before = subgraph_digest(before_graph, case["affected"])
        sibling_before = subgraph_digest(before_graph, "apps/docs")
        input_path = root / case["input"]
        original_input = mutate(input_path, case["mutation"])
        after_graph, after_manifest, after_evidence = run_compass(binary, root, output)
        affected_after = subgraph_digest(after_graph, case["affected"])
        sibling_after = subgraph_digest(after_graph, "apps/docs")
        input_before = manifest_input_digest(before_manifest, case["input"])
        input_after = manifest_input_digest(after_manifest, case["input"])
        evidence_invalidated = before_evidence != after_evidence
        if input_before is not None and input_after is not None and input_before == input_after:
            raise MatrixError(f"{case['id']} did not record the mutated input in the manifest")
        if not evidence_invalidated:
            raise MatrixError(f"{case['id']} did not invalidate project evidence")
        if (affected_before != affected_after) != case["expectGraphChange"]:
            raise MatrixError(
                f"{case['id']} affected-package graph change mismatch: "
                f"expected {case['expectGraphChange']}, got {affected_before != affected_after}"
            )
        if sibling_before != sibling_after:
            raise MatrixError(f"{case['id']} changed the unaffected sibling package")
        input_path.write_bytes(original_input)
        restored_graph, _, restored_evidence = run_compass(binary, root, output)
        if digest(restored_graph) != digest(before_graph) or restored_evidence != before_evidence:
            raise MatrixError(f"{case['id']} restore did not reproduce the original graph")
        return {
            "id": case["id"],
            "manager": case["manager"],
            "mutation": case["mutation"],
            "affectedGraphChanged": affected_before != affected_after,
            "siblingGraphStable": True,
            "inputInvalidated": evidence_invalidated,
            "restoredGraphStable": True,
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--compass", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.compass.name != "compass" or not args.compass.is_file():
            raise MatrixError(f"matrix requires an executable Compass binary: {args.compass}")
        matrix = load_matrix()
        results = [run_case(args.compass.resolve(), case) for case in matrix["cases"]]
        print(json.dumps({"schema": "compass.react-frontend-precedence-result/1", "cases": results}, sort_keys=True, separators=(",", ":")))
        return 0
    except (MatrixError, OSError, json.JSONDecodeError) as error:
        print(f"react frontend precedence matrix failed: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
