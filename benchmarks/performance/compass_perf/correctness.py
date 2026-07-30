"""Streaming graph invariants, canonical digests, and shared-fact comparison."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path
import sqlite3
from typing import Any, Iterator

from .jsonstream import iter_top_level_array, read_top_level_value
from .model import CorrectnessResult


@dataclass(frozen=True)
class GraphSummary:
    tool: str
    nodes: int
    edges: int
    validation_errors: int
    digest: str


def _text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def _canonical(value: object) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def _hash(value: object) -> str:
    return hashlib.sha256(_canonical(value).encode("utf-8")).hexdigest()


def _create_schema(database: sqlite3.Connection) -> None:
    database.executescript(
        """
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        PRAGMA temp_store=FILE;
        CREATE TABLE IF NOT EXISTS nodes (
            tool TEXT NOT NULL,
            id TEXT NOT NULL,
            label TEXT NOT NULL,
            kind TEXT NOT NULL,
            source_file TEXT NOT NULL,
            source_location TEXT NOT NULL,
            payload_sha256 TEXT NOT NULL,
            PRIMARY KEY (tool, id)
        );
        CREATE TABLE IF NOT EXISTS edges (
            tool TEXT NOT NULL,
            source TEXT NOT NULL,
            target TEXT NOT NULL,
            relation TEXT NOT NULL,
            payload_sha256 TEXT NOT NULL,
            UNIQUE (tool, source, target, relation, payload_sha256)
        );
        CREATE INDEX IF NOT EXISTS edges_source ON edges(tool, source);
        CREATE INDEX IF NOT EXISTS edges_target ON edges(tool, target);
        CREATE TABLE IF NOT EXISTS summaries (
            tool TEXT PRIMARY KEY,
            nodes INTEGER NOT NULL,
            edges INTEGER NOT NULL,
            validation_errors INTEGER NOT NULL,
            digest TEXT NOT NULL
        );
        """
    )


def _records(path: Path, preferred: str, fallback: str | None = None) -> Iterator[dict[str, object]]:
    try:
        yield from iter_top_level_array(path, preferred)
    except KeyError:
        if fallback is None:
            raise ValueError(f"graph is missing required array {preferred!r}: {path}") from None
        try:
            yield from iter_top_level_array(path, fallback)
        except KeyError:
            raise ValueError(
                f"graph is missing required array {preferred!r} or {fallback!r}: {path}"
            ) from None


def _validation_errors(path: Path) -> int:
    try:
        metadata = read_top_level_value(path, "graph")
    except KeyError:
        return 0
    if not isinstance(metadata, dict):
        return 0
    diagnostics = metadata.get("diagnostics", [])
    if not isinstance(diagnostics, list):
        return 1
    return sum(
        isinstance(item, dict) and str(item.get("severity", "")).lower() == "error"
        for item in diagnostics
    )


def index_graph(
    tool: str,
    graph_path: Path,
    database: sqlite3.Connection,
) -> GraphSummary:
    if tool not in {"compass", "graphify"}:
        raise ValueError(f"unsupported graph tool: {tool}")
    _create_schema(database)
    database.execute("DELETE FROM edges WHERE tool = ?", (tool,))
    database.execute("DELETE FROM nodes WHERE tool = ?", (tool,))
    database.execute("DELETE FROM summaries WHERE tool = ?", (tool,))

    node_count = 0
    for record in _records(graph_path, "nodes"):
        identifier = record.get("id")
        if not isinstance(identifier, str) or not identifier:
            raise ValueError(f"{tool} node has an invalid id")
        label = _text(record.get("label", identifier))
        kind = _text(record.get("kind", record.get("type", record.get("node_type"))))
        source_file = _text(record.get("source_file")).replace("\\", "/")
        source_location = _text(record.get("source_location"))
        payload = _hash((identifier, label, kind, source_file, source_location))
        existing = database.execute(
            "SELECT payload_sha256 FROM nodes WHERE tool = ? AND id = ?",
            (tool, identifier),
        ).fetchone()
        if existing is not None and existing[0] != payload:
            raise ValueError(f"{tool} node id has conflicting payloads: {identifier}")
        if existing is None:
            database.execute(
                "INSERT INTO nodes VALUES (?, ?, ?, ?, ?, ?, ?)",
                (tool, identifier, label, kind, source_file, source_location, payload),
            )
            node_count += 1

    edge_count = 0
    for record in _records(graph_path, "links", "edges"):
        source = record.get("source")
        target = record.get("target")
        relation = record.get("relation", record.get("kind"))
        if not isinstance(source, str) or not source:
            raise ValueError(f"{tool} edge has an invalid source")
        if not isinstance(target, str) or not target:
            raise ValueError(f"{tool} edge has an invalid target")
        if not isinstance(relation, str) or not relation:
            raise ValueError(f"{tool} edge has an invalid relation")
        relation = relation.lower()
        confidence = record.get("confidence")
        if isinstance(confidence, float) and not math.isfinite(confidence):
            raise ValueError(f"{tool} edge has non-finite confidence")
        payload = _hash(
            (
                source,
                target,
                relation,
                _text(confidence),
                _text(record.get("source_file")).replace("\\", "/"),
                _text(record.get("source_location")),
            )
        )
        before = database.total_changes
        database.execute(
            "INSERT OR IGNORE INTO edges VALUES (?, ?, ?, ?, ?)",
            (tool, source, target, relation, payload),
        )
        if database.total_changes > before:
            edge_count += 1

    dangling = database.execute(
        """
        SELECT COUNT(*)
        FROM edges AS edge
        LEFT JOIN nodes AS source
          ON source.tool = edge.tool AND source.id = edge.source
        LEFT JOIN nodes AS target
          ON target.tool = edge.tool AND target.id = edge.target
        WHERE edge.tool = ? AND (source.id IS NULL OR target.id IS NULL)
        """,
        (tool,),
    ).fetchone()[0]
    if dangling:
        raise ValueError(f"{tool} graph has {dangling} dangling edges")
    validation_errors = _validation_errors(graph_path)
    if tool == "compass" and validation_errors:
        raise ValueError(f"Compass graph reports {validation_errors} validation errors")
    digest = canonical_graph_digest(database, tool)
    database.execute(
        "INSERT INTO summaries VALUES (?, ?, ?, ?, ?)",
        (tool, node_count, edge_count, validation_errors, digest),
    )
    database.commit()
    return GraphSummary(tool, node_count, edge_count, validation_errors, digest)


def canonical_graph_digest(database: sqlite3.Connection, tool: str) -> str:
    digest = hashlib.sha256()
    digest.update(b"compass.performance-canonical-graph/1\0")
    for table, query in (
        (
            "node",
            "SELECT id,label,kind,source_file,source_location,payload_sha256 "
            "FROM nodes WHERE tool = ? ORDER BY id",
        ),
        (
            "edge",
            "SELECT source,target,relation,payload_sha256 FROM edges "
            "WHERE tool = ? ORDER BY source,target,relation,payload_sha256",
        ),
    ):
        for row in database.execute(query, (tool,)):
            encoded = _canonical((table, *row)).encode("utf-8")
            digest.update(len(encoded).to_bytes(8, "big"))
            digest.update(encoded)
    return digest.hexdigest()


def _examples(database: sqlite3.Connection, query: str, limit: int = 10) -> tuple[str, ...]:
    return tuple(str(row[0]) for row in database.execute(f"{query} LIMIT {limit}"))


def compare_graphs(database: sqlite3.Connection) -> CorrectnessResult:
    tools = {row[0] for row in database.execute("SELECT tool FROM summaries")}
    failures: list[str] = []
    metrics: dict[str, int | str | bool] = {}
    compass = database.execute(
        "SELECT nodes,edges,digest FROM summaries WHERE tool = 'compass'"
    ).fetchone()
    if compass is None:
        failures.append("Compass graph was not indexed")
    else:
        metrics.update(
            compass_nodes=int(compass[0]),
            compass_edges=int(compass[1]),
            compass_digest=str(compass[2]),
        )

    if "graphify" in tools:
        missing_nodes_query = """
            SELECT graphify.id
            FROM nodes AS graphify
            LEFT JOIN nodes AS compass
              ON compass.tool = 'compass' AND compass.id = graphify.id
            WHERE graphify.tool = 'graphify' AND compass.id IS NULL
        """
        mismatch_query = """
            SELECT graphify.id
            FROM nodes AS graphify
            JOIN nodes AS compass
              ON compass.tool = 'compass' AND compass.id = graphify.id
            WHERE graphify.tool = 'graphify'
              AND (
                graphify.label != compass.label
                OR graphify.kind != compass.kind
                OR graphify.source_file != compass.source_file
                OR graphify.source_location != compass.source_location
              )
        """
        missing_edges_query = """
            SELECT graphify.source || '|' || graphify.relation || '|' || graphify.target
            FROM edges AS graphify
            LEFT JOIN edges AS compass
              ON compass.tool = 'compass'
              AND compass.source = graphify.source
              AND compass.target = graphify.target
              AND compass.relation = graphify.relation
            WHERE graphify.tool = 'graphify' AND compass.source IS NULL
        """
        for name, query in (
            ("missing_graphify_nodes", missing_nodes_query),
            ("mismatched_shared_nodes", mismatch_query),
            ("missing_graphify_edges", missing_edges_query),
        ):
            count = int(database.execute(f"SELECT COUNT(*) FROM ({query})").fetchone()[0])
            metrics[name] = count
            if count:
                examples = ", ".join(_examples(database, query))
                failures.append(f"{name}: {count}; examples: {examples}")

    payload = {
        "failures": failures,
        "metrics": metrics,
    }
    return CorrectnessResult(
        passed=not failures,
        digest=_hash(payload),
        failures=tuple(failures),
        metrics=metrics,
    )

