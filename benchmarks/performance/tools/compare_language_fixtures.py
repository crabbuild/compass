#!/usr/bin/env python3
"""Compare language fixture graphs with the shared correctness classifier."""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import json
from pathlib import Path
import sqlite3
from typing import Sequence

from benchmarks.performance.compass.correctness import (
    _classify_edges,
    _classify_nodes,
    _edge_facts,
    _node_facts,
    index_graph,
)


SCHEMA = "compass.language-fixture-comparison/1"
FIXTURE_SCHEMA = "compass.language-fixture/1"
STATUSES = ("exact", "dominated", "rejected", "ambiguous", "missing")


@dataclass(frozen=True)
class FixtureSpec:
    language: str
    fixture: str
    compass_graph: Path
    graphify_graph: Path


@dataclass(frozen=True)
class CoverageRow:
    language: str
    fixture: str
    relation: str
    graphify_total: int
    exact: int
    dominated: int
    rejected: int
    ambiguous: int
    missing: int

    @property
    def handled(self) -> int:
        return self.exact + self.dominated + self.rejected


@dataclass(frozen=True)
class FixtureResult:
    language: str
    fixture: str
    compass_nodes: int
    compass_edges: int
    compass_digest: str
    graphify_nodes: int
    graphify_edges: int
    graphify_digest: str
    rows: tuple[CoverageRow, ...]


def _required_text(document: dict[str, object], key: str, manifest: Path) -> str:
    value = document.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"fixture manifest {manifest} has invalid {key!r}")
    return value.strip()


def load_fixture_spec(manifest: Path) -> FixtureSpec:
    try:
        document = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read fixture manifest {manifest}: {error}") from error
    if not isinstance(document, dict):
        raise ValueError(f"fixture manifest {manifest} must be a JSON object")
    if document.get("schema") != FIXTURE_SCHEMA:
        raise ValueError(
            f"fixture manifest {manifest} must use schema {FIXTURE_SCHEMA!r}"
        )
    root = manifest.resolve().parent
    compass_graph = root / _required_text(document, "compass_graph", manifest)
    graphify_graph = root / _required_text(document, "graphify_graph", manifest)
    for tool, path in (("Compass", compass_graph), ("Graphify", graphify_graph)):
        if not path.is_file():
            raise ValueError(f"{tool} graph does not exist for {manifest}: {path}")
    return FixtureSpec(
        language=_required_text(document, "language", manifest).casefold(),
        fixture=_required_text(document, "fixture", manifest),
        compass_graph=compass_graph,
        graphify_graph=graphify_graph,
    )


def compare_fixture(spec: FixtureSpec) -> FixtureResult:
    database = sqlite3.connect(":memory:")
    try:
        compass_summary = index_graph("compass", spec.compass_graph, database)
        graphify_summary = index_graph("graphify", spec.graphify_graph, database)
        compass_nodes = _node_facts(database, "compass")
        graphify_nodes = _node_facts(database, "graphify")
        node_coverage, node_mapping = _classify_nodes(graphify_nodes, compass_nodes)
        compass_edges = _edge_facts(database, "compass")
        graphify_edges = _edge_facts(database, "graphify")
        edge_coverage = _classify_edges(
            graphify_edges,
            compass_edges,
            graphify_nodes,
            compass_nodes,
            node_coverage,
            node_mapping,
        )
    finally:
        database.close()

    relations: dict[str, dict[str, int]] = {}
    for edge, coverage in zip(graphify_edges, edge_coverage, strict=True):
        counts = relations.setdefault(
            edge.relation, {status: 0 for status in STATUSES}
        )
        counts[coverage.status] += 1
    rows = tuple(
        CoverageRow(
            language=spec.language,
            fixture=spec.fixture,
            relation=relation,
            graphify_total=sum(counts.values()),
            **counts,
        )
        for relation, counts in sorted(relations.items())
    )
    return FixtureResult(
        language=spec.language,
        fixture=spec.fixture,
        compass_nodes=compass_summary.nodes,
        compass_edges=compass_summary.edges,
        compass_digest=compass_summary.digest,
        graphify_nodes=graphify_summary.nodes,
        graphify_edges=graphify_summary.edges,
        graphify_digest=graphify_summary.digest,
        rows=rows,
    )


def build_report(manifests: Sequence[Path]) -> dict[str, object]:
    if not manifests:
        raise ValueError("at least one --fixture manifest is required")
    specs = sorted(
        (load_fixture_spec(path) for path in manifests),
        key=lambda spec: (spec.language, spec.fixture),
    )
    identities = [(spec.language, spec.fixture) for spec in specs]
    if len(identities) != len(set(identities)):
        raise ValueError("fixture language/name pairs must be unique")
    results = [compare_fixture(spec) for spec in specs]
    rows = [asdict(row) | {"handled": row.handled} for result in results for row in result.rows]
    return {
        "schema": SCHEMA,
        "fixtures": [
            {
                "language": result.language,
                "fixture": result.fixture,
                "compass": {
                    "nodes": result.compass_nodes,
                    "edges": result.compass_edges,
                    "digest": result.compass_digest,
                },
                "graphify": {
                    "nodes": result.graphify_nodes,
                    "edges": result.graphify_edges,
                    "digest": result.graphify_digest,
                },
            }
            for result in results
        ],
        "coverage": rows,
    }


def render_markdown(report: dict[str, object]) -> str:
    lines = [
        "# Language fixture comparison",
        "",
        "| Language | Fixture | Relation | Graphify | Exact | Dominated | Rejected | Ambiguous | Missing |",
        "|---|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for row in report["coverage"]:
        assert isinstance(row, dict)
        lines.append(
            "| {language} | {fixture} | {relation} | {graphify_total} | "
            "{exact} | {dominated} | {rejected} | {ambiguous} | {missing} |".format(
                **row
            )
        )
    return "\n".join(lines) + "\n"


def write_report(
    manifests: Sequence[Path], json_output: Path, markdown_output: Path
) -> dict[str, object]:
    report = build_report(manifests)
    json_output.parent.mkdir(parents=True, exist_ok=True)
    markdown_output.parent.mkdir(parents=True, exist_ok=True)
    json_output.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    markdown_output.write_text(render_markdown(report), encoding="utf-8")
    return report


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--fixture", type=Path, action="append", required=True, metavar="MANIFEST"
    )
    parser.add_argument("--json-output", type=Path, required=True)
    parser.add_argument("--markdown-output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        write_report(args.fixture, args.json_output, args.markdown_output)
    except ValueError as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
