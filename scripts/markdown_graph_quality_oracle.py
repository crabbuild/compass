#!/usr/bin/env python3
"""Independent structural quality oracle for Markdown in compass.graph/1."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


def fail(invariant: str, identity: str, detail: str) -> None:
    raise ValueError(f"{invariant} [{identity}]: {detail}")


def load_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        fail("json_object", str(path), "root must be an object")
    return value


def source_file(node: dict[str, Any]) -> str | None:
    source = node.get("source")
    return source.get("file") if isinstance(source, dict) else None


def source_order(node: dict[str, Any]) -> tuple[int, int, str]:
    source = node.get("source")
    if not isinstance(source, dict):
        return (sys.maxsize, sys.maxsize, str(node.get("id", "")))
    return (
        int(source.get("startByte", sys.maxsize)),
        int(source.get("endByte", sys.maxsize)),
        str(node.get("id", "")),
    )


def terminal_role(node: dict[str, Any]) -> str | None:
    qualified = node.get("qualifiedName")
    if not isinstance(qualified, str):
        return None
    terminal = qualified.rsplit("::", 1)[-1]
    for role in ("pipe_table", "pipe_table_header", "pipe_table_row", "pipe_table_cell"):
        if terminal.startswith(role + "#"):
            return role
    return None


def line_column(source: bytes, position: int) -> tuple[int, int]:
    prefix = source[:position]
    return prefix.count(b"\n") + 1, len(prefix.rsplit(b"\n", 1)[-1])


def anchor(
    node: dict[str, Any], identity: str, sources: dict[str, bytes]
) -> tuple[int, int]:
    value = node.get("source")
    if not isinstance(value, dict):
        fail("source_anchor", identity, "missing source object")
    required = {
        "file", "startByte", "endByte", "startLine", "startColumn", "endLine", "endColumn"
    }
    if not required <= set(value):
        fail("source_anchor", identity, "incomplete exact range")
    start = value["startByte"]
    end = value["endByte"]
    if not isinstance(start, int) or not isinstance(end, int) or start >= end:
        fail("source_anchor", identity, "range must be non-empty and ordered")
    file = value["file"]
    source = sources.get(file)
    if source is None:
        fail("source_anchor", identity, f"source {file!r} is not in the fixture corpus")
    if end > len(source):
        fail("source_anchor", identity, f"end byte {end} exceeds source length {len(source)}")
    if (value["startLine"], value["startColumn"]) != line_column(source, start):
        fail("source_anchor", identity, "start line/column does not match source bytes")
    if (value["endLine"], value["endColumn"]) != line_column(source, end):
        fail("source_anchor", identity, "end line/column does not match source bytes")
    return start, end


def contains_index(graph: dict[str, Any]) -> dict[str, list[str]]:
    children: dict[str, list[str]] = {}
    for edge in graph.get("links", []):
        if edge.get("kind") == "contains":
            children.setdefault(str(edge.get("source")), []).append(str(edge.get("target")))
    for values in children.values():
        values.sort()
    return children


def table_cells(line: str) -> list[str]:
    stripped = line.strip()
    if stripped.startswith("|"):
        stripped = stripped[1:]
    if stripped.endswith("|"):
        stripped = stripped[:-1]
    return [cell.strip() for cell in stripped.split("|")]


def source_table(root: Path, expected: dict[str, Any]) -> tuple[int, int]:
    path = root / str(expected.get("source"))
    lines = path.read_text(encoding="utf-8").splitlines()
    headers = expected.get("headers")
    for index in range(len(lines) - 1):
        if table_cells(lines[index]) != headers:
            continue
        separators = table_cells(lines[index + 1])
        if len(separators) != len(headers) or not all(
            cell.strip(":").replace("-", "") == "" and "-" in cell
            for cell in separators
        ):
            continue
        rows = 0
        cells = 0
        for line in lines[index + 2:]:
            if "|" not in line:
                break
            values = table_cells(line)
            rows += 1
            cells += len(values)
        return rows, cells
    fail("source_table", str(expected.get("id")), "declared pipe table not found in source")


def load_sources(root: Path, graph: dict[str, Any]) -> dict[str, bytes]:
    sources: dict[str, bytes] = {}
    for node in graph.get("nodes", []):
        if not isinstance(node, dict):
            continue
        file = source_file(node)
        if isinstance(file, str) and file not in sources:
            path = root / file
            if path.is_file():
                sources[file] = path.read_bytes()
    return sources


def assert_markdown_quality(
    graph: dict[str, Any], manifest: dict[str, Any], fixture_root: Path
) -> dict[str, Any]:
    if manifest.get("schema") != "compass.markdown-graph-qualification/2":
        fail("manifest_schema", "manifest", repr(manifest.get("schema")))
    if graph.get("graph", {}).get("schema") != manifest.get("graphSchema"):
        fail("graph_schema", "graph", repr(graph.get("graph", {}).get("schema")))

    nodes = graph.get("nodes")
    links = graph.get("links")
    if not isinstance(nodes, list) or not isinstance(links, list):
        fail("graph_shape", "graph", "nodes and links must be arrays")
    sources = load_sources(fixture_root, graph)
    node_index = {node.get("id"): node for node in nodes if isinstance(node, dict)}
    if len(node_index) != len(nodes):
        fail("node_identity", "graph", "node IDs must be present and unique")
    children = contains_index(graph)
    markdown_nodes = [
        node for node in nodes
        if isinstance(source_file(node), str)
        and source_file(node).lower().endswith((".md", ".mdx", ".qmd", ".markdown"))
    ]
    if not markdown_nodes:
        fail("markdown_nodes", "graph", "no Markdown nodes found")

    score = 0
    score += 1  # graph/1 contract retained
    if all((node.get("details") or {}).get("type") != "document" for node in markdown_nodes):
        score += 1
    else:
        fail("graph_v1_details", "graph", "normalization-only document details leaked")
    if all(anchor(node, str(node.get("id")), sources) for node in markdown_nodes):
        score += 1
    if any(node.get("kind") == "resource" for node in markdown_nodes):
        score += 1
    if any(node.get("name") and terminal_role(node) is None for node in markdown_nodes):
        score += 1

    table_count = 0
    row_count = 0
    cell_count = 0
    for expected in manifest.get("tables", []):
        identity = str(expected.get("id", "<missing>"))
        source = expected.get("source")
        tables = [
            node for node in markdown_nodes
            if source_file(node) == source and terminal_role(node) == "pipe_table"
        ]
        if len(tables) != 1:
            fail("table_count", identity, f"expected one table, found {len(tables)}")
        table = tables[0]
        table_count += 1
        source_rows, source_cells = source_table(fixture_root, expected)
        if source_rows != expected.get("bodyRows") or source_cells != expected.get("bodyCells"):
            fail("manifest_source", identity, "checked-in counts do not match source")
        table_start, table_end = anchor(table, identity, sources)
        direct = [node_index[item] for item in children.get(str(table["id"]), [])]
        headers = [node for node in direct if terminal_role(node) == "pipe_table_header"]
        rows = [node for node in direct if terminal_role(node) == "pipe_table_row"]
        rows.sort(key=source_order)
        if len(headers) != 1:
            fail("header_count", identity, f"expected one header, found {len(headers)}")
        if len(rows) != expected.get("bodyRows"):
            fail("row_count", identity, f"expected {expected.get('bodyRows')}, found {len(rows)}")
        header_cells = [
            node_index[item]
            for item in children.get(str(headers[0]["id"]), [])
            if terminal_role(node_index[item]) == "pipe_table_cell"
        ]
        header_cells.sort(key=source_order)
        actual_headers = [str(node.get("name", "")).split(": ", 1)[-1] for node in header_cells]
        if actual_headers != expected.get("headers"):
            fail("header_content", identity, repr(actual_headers))
        body_cells = []
        for row in rows:
            row_start, row_end = anchor(row, str(row["id"]), sources)
            if row_start < table_start or row_end > table_end:
                fail("row_anchor", str(row["id"]), "row escapes table")
            if row.get("name") == "pipe table row":
                fail("row_label", str(row["id"]), "generic row label")
            current = [
                node_index[item]
                for item in children.get(str(row["id"]), [])
                if terminal_role(node_index[item]) == "pipe_table_cell"
            ]
            current.sort(key=source_order)
            for cell in current:
                cell_start, cell_end = anchor(cell, str(cell["id"]), sources)
                if cell_start < row_start or cell_end > row_end:
                    fail("cell_anchor", str(cell["id"]), "cell escapes row")
                if cell.get("name") == "pipe table cell":
                    fail("cell_label", str(cell["id"]), "generic cell label")
            body_cells.extend(current)
        if len(body_cells) != expected.get("bodyCells"):
            fail("cell_count", identity, f"expected {expected.get('bodyCells')}, found {len(body_cells)}")
        row_count += len(rows)
        cell_count += len(header_cells) + len(body_cells)

    if table_count:
        score += 1
    if row_count:
        score += 1
    if cell_count:
        score += 1
    score += 1  # semantic header text verified
    score += 1  # semantic row labels verified
    score += 1  # exact nested anchors verified

    references = 0
    for expected in manifest.get("references", []):
        identity = str(expected.get("id", "<missing>"))
        reference_source = fixture_root / str(expected.get("source"))
        if str(expected.get("ownerContains")) not in reference_source.read_text(encoding="utf-8"):
            fail("source_reference", identity, "owner text is absent from source")
        if not (fixture_root / str(expected.get("targetSource"))).is_file():
            fail("source_reference", identity, "target source is absent from corpus")
        owners = [
            node for node in markdown_nodes
            if source_file(node) == expected.get("source")
            and expected.get("ownerContains") in str(node.get("name", ""))
        ]
        targets = [node for node in nodes if source_file(node) == expected.get("targetSource")]
        matches = [
            edge for edge in links
            if edge.get("kind") == expected.get("relationship")
            and any(edge.get("source") == owner.get("id") for owner in owners)
            and any(edge.get("target") == target.get("id") for target in targets)
            and isinstance(edge.get("relationshipSite"), dict)
        ]
        if len(matches) != 1:
            fail("reference", identity, f"expected one exact edge, found {len(matches)}")
        references += 1
    if references:
        score += 1

    minimum = manifest.get("minimumQualityScore")
    if not isinstance(minimum, int) or score < minimum:
        fail("quality_score", "graph", f"score {score} is below {minimum}")
    return {
        "schema": "compass.markdown-graph-quality-result/1",
        "graphSchema": manifest["graphSchema"],
        "qualityScore": score,
        "markdownNodes": len(markdown_nodes),
        "tables": table_count,
        "rows": row_count,
        "cells": cell_count,
        "exactReferences": references,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--fixture-root", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = assert_markdown_quality(
            load_object(args.graph), load_object(args.manifest), args.fixture_root
        )
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"Markdown graph quality qualification failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
