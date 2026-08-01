#!/usr/bin/env python3
"""Compare language fixture graphs with the shared correctness classifier."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from dataclasses import asdict, dataclass
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
from typing import Iterator, Sequence

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
MAX_GRAPHIFY_COMPARISON_BYTES = 512 * 1024 * 1024


@dataclass(frozen=True)
class FixtureSpec:
    language: str
    fixture: str
    compass_graphs: tuple[Path, ...]
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
    compass_bytes_digest: str
    compass_occurrence_digest: str
    compass_runs: int
    graphify_nodes: int
    graphify_edges: int
    graphify_dangling_edges: int
    graphify_digest: str
    node_coverage: dict[str, int]
    rows: tuple[CoverageRow, ...]


def _required_text(document: dict[str, object], key: str, manifest: Path) -> str:
    value = document.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"fixture manifest {manifest} has invalid {key!r}")
    return value.strip()


def _compass_graphs(
    document: dict[str, object], root: Path, manifest: Path
) -> tuple[Path, ...]:
    repeated = document.get("compass_graphs")
    if repeated is None:
        return (root / _required_text(document, "compass_graph", manifest),)
    if (
        not isinstance(repeated, list)
        or len(repeated) < 3
        or not all(isinstance(item, str) and item.strip() for item in repeated)
    ):
        raise ValueError(
            f"fixture manifest {manifest} compass_graphs must contain at least three paths"
        )
    return tuple(root / item.strip() for item in repeated)


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _occurrence_sha256(edges: object) -> str:
    payload = json.dumps(
        [asdict(edge) for edge in edges],
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


@contextmanager
def _sanitized_graphify_graph(path: Path) -> Iterator[tuple[Path, int]]:
    try:
        size = path.stat().st_size
    except OSError as error:
        raise ValueError(f"cannot inspect Graphify graph {path}: {error}") from error
    if size > MAX_GRAPHIFY_COMPARISON_BYTES:
        raise ValueError(
            f"Graphify graph exceeds {MAX_GRAPHIFY_COMPARISON_BYTES} bytes: {path}"
        )
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read Graphify graph {path}: {error}") from error
    if not isinstance(document, dict) or not isinstance(document.get("nodes"), list):
        raise ValueError(f"Graphify graph has an invalid node inventory: {path}")
    edge_key = "links" if "links" in document else "edges"
    edges = document.get(edge_key)
    if not isinstance(edges, list):
        raise ValueError(f"Graphify graph has an invalid edge inventory: {path}")
    node_ids = {
        node.get("id")
        for node in document["nodes"]
        if isinstance(node, dict)
        and isinstance(node.get("id"), str)
        and node.get("id")
    }
    retained = []
    dangling = 0
    for edge in edges:
        if not isinstance(edge, dict):
            retained.append(edge)
            continue
        source = edge.get("source")
        target = edge.get("target")
        if (
            isinstance(source, str)
            and source
            and isinstance(target, str)
            and target
            and (source not in node_ids or target not in node_ids)
        ):
            dangling += 1
        else:
            retained.append(edge)
    if not dangling:
        yield path, 0
        return
    document[edge_key] = retained
    with tempfile.TemporaryDirectory(prefix="graphify-sanitized-", dir=path.parent) as directory:
        sanitized = Path(directory) / "graph.json"
        sanitized.write_text(
            json.dumps(document, separators=(",", ":")), encoding="utf-8"
        )
        yield sanitized, dangling


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
    compass_graphs = _compass_graphs(document, root, manifest)
    graphify_graph = root / _required_text(document, "graphify_graph", manifest)
    for tool, path in (
        *(("Compass", path) for path in compass_graphs),
        ("Graphify", graphify_graph),
    ):
        if not path.is_file():
            raise ValueError(f"{tool} graph does not exist for {manifest}: {path}")
    return FixtureSpec(
        language=_required_text(document, "language", manifest).casefold(),
        fixture=_required_text(document, "fixture", manifest),
        compass_graphs=compass_graphs,
        graphify_graph=graphify_graph,
    )


def compare_fixture(spec: FixtureSpec) -> FixtureResult:
    database = sqlite3.connect(":memory:")
    try:
        compass_summary = index_graph("compass", spec.compass_graphs[0], database)
        compass_bytes_digest = _file_sha256(spec.compass_graphs[0])
        compass_occurrence_digest = _occurrence_sha256(
            _edge_facts(database, "compass")
        )
        for run, graph in enumerate(spec.compass_graphs[1:], start=2):
            repeated_summary = index_graph("compass", graph, database)
            repeated_bytes_digest = _file_sha256(graph)
            repeated_occurrence_digest = _occurrence_sha256(
                _edge_facts(database, "compass")
            )
            if repeated_bytes_digest != compass_bytes_digest:
                raise ValueError(
                    f"{spec.fixture}: Compass run {run} graph bytes differ"
                )
            if repeated_summary.digest != compass_summary.digest:
                raise ValueError(
                    f"{spec.fixture}: Compass run {run} canonical graph differs"
                )
            if repeated_occurrence_digest != compass_occurrence_digest:
                raise ValueError(
                    f"{spec.fixture}: Compass run {run} occurrences differ"
                )
        with _sanitized_graphify_graph(spec.graphify_graph) as (
            graphify_graph,
            graphify_dangling_edges,
        ):
            graphify_summary = index_graph("graphify", graphify_graph, database)
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
            None,
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
        compass_bytes_digest=compass_bytes_digest,
        compass_occurrence_digest=compass_occurrence_digest,
        compass_runs=len(spec.compass_graphs),
        graphify_nodes=graphify_summary.nodes,
        graphify_edges=graphify_summary.edges,
        graphify_dangling_edges=graphify_dangling_edges,
        graphify_digest=graphify_summary.digest,
        node_coverage={
            status: sum(
                coverage.status == status for coverage in node_coverage.values()
            )
            for status in STATUSES
        },
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
                    "bytes_digest": result.compass_bytes_digest,
                    "occurrence_digest": result.compass_occurrence_digest,
                    "runs": result.compass_runs,
                },
                "graphify": {
                    "nodes": result.graphify_nodes,
                    "edges": result.graphify_edges,
                    "dangling_edges": result.graphify_dangling_edges,
                    "digest": result.graphify_digest,
                },
                "node_coverage": result.node_coverage,
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
