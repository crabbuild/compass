"""Streaming graph invariants, canonical digests, and shared-fact comparison."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path
import re
import sqlite3
from typing import Any, Iterator

from .jsonstream import iter_top_level_array, iter_top_level_object_array
from .model import CorrectnessResult
from .occurrences import SourceOccurrenceOracle

CODE_TYPE_KINDS = {
    "class",
    "struct",
    "interface",
    "trait",
    "protocol",
    "enum",
    "type_alias",
}

MAX_CONTAINMENT_PATH_DEPTH = 8
MAX_CONTAINMENT_PATH_STATES = 4096


@dataclass(frozen=True)
class GraphSummary:
    tool: str
    nodes: int
    edges: int
    validation_errors: int
    digest: str


@dataclass(frozen=True)
class Coverage:
    status: str
    reason: str
    compass_fact: str | None


@dataclass(frozen=True)
class NodeFact:
    identifier: str
    label: str
    normalized_label: str
    kind: str
    source_file: str
    source_location: str
    fact_key: str
    qualified_name: str
    language: str
    module: str
    placeholder: bool
    anchored_definition: bool
    callable: bool


@dataclass(frozen=True)
class EdgeFact:
    source: str
    target: str
    relation: str
    original_relation: str
    context: str
    source_fact_key: str
    target_fact_key: str
    occurrence_file: str
    occurrence_location: str
    payload_sha256: str


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


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _normalized_label(record: dict[str, object], identifier: str) -> tuple[str, str]:
    source = record.get("source")
    nested = source if isinstance(source, dict) else {}
    label = _text(record.get("name", record.get("label", identifier))).strip()
    details = record.get("details")
    detail_data = details.get("data") if isinstance(details, dict) else None
    resource_kind = (
        detail_data.get("resourceKind") if isinstance(detail_data, dict) else None
    )
    rationale = record.get("file_type") == "rationale" or resource_kind == "rationale"
    normalized = "rationale" if rationale else label.casefold().lstrip(".")
    normalized = re.sub(r"\(\)$", "", normalized)
    if "/" in normalized or "\\" in normalized:
        normalized = Path(normalized.replace("\\", "/")).name
    return label, normalized


def _source_fact(record: dict[str, object], identifier: str) -> tuple[str, str, str, str]:
    source = record.get("source")
    nested = source if isinstance(source, dict) else {}
    label, normalized_label = _normalized_label(record, identifier)
    source_file = _text(record.get("source_file", nested.get("file"))).replace("\\", "/")
    source_location = _text(record.get("source_location"))
    if not source_location:
        start_line = nested.get("startLine", nested.get("start_line"))
        if isinstance(start_line, int) and start_line > 0:
            source_location = f"L{start_line}"
    fact_key = _hash((normalized_label, source_file, source_location))
    return label, source_file, source_location, fact_key


def _language(record: dict[str, object], source_file: str) -> str:
    explicit = _text(record.get("language", record.get("lang"))).casefold()
    if explicit:
        return explicit
    extension = Path(source_file).suffix.casefold().lstrip(".")
    return {
        "cjs": "javascript",
        "cts": "typescript",
        "go": "go",
        "js": "javascript",
        "jsx": "javascript",
        "mjs": "javascript",
        "mts": "typescript",
        "py": "python",
        "rs": "rust",
        "ts": "typescript",
        "tsx": "typescript",
    }.get(extension, extension)


def _module(record: dict[str, object], source_file: str) -> str:
    explicit = _text(
        record.get(
            "module",
            record.get("namespace", record.get("package", record.get("package_name"))),
        )
    )
    if explicit:
        return explicit.replace("\\", "/").casefold()
    if not source_file:
        return ""
    return str(Path(source_file).parent).replace("\\", "/").casefold().strip(".")


def _qualified_name(record: dict[str, object]) -> str:
    return _text(
        record.get(
            "qualified_name",
            record.get("qualifiedName", record.get("semantic_scope")),
        )
    ).casefold()


def _edge_occurrence(record: dict[str, object]) -> tuple[str, str]:
    site = record.get("relationshipSite", record.get("relationship_site"))
    nested = site if isinstance(site, dict) else {}
    source_file = _text(record.get("source_file", nested.get("file"))).replace("\\", "/")
    source_location = _text(record.get("source_location"))
    if not source_location:
        start_line = nested.get("startLine", nested.get("start_line"))
        if isinstance(start_line, int) and start_line > 0:
            source_location = f"L{start_line}"
    return source_file, source_location


def _shared_relation(tool: str, relation: str) -> str:
    if tool == "compass":
        return {"documents": "rationale_for", "instantiates": "calls"}.get(relation, relation)
    return {
        "case_of": "contains",
        "defines": "contains",
        "indirect_call": "calls",
        "inherits": "extends",
        "imports_from": "imports",
        "method": "contains",
        "re_exports": "exports",
        "uses": "references",
    }.get(relation, relation)


def _create_schema(database: sqlite3.Connection) -> None:
    schema_version = 2
    current = int(database.execute("PRAGMA user_version").fetchone()[0])
    if current != schema_version:
        database.executescript(
            """
            DROP TABLE IF EXISTS edges;
            DROP TABLE IF EXISTS nodes;
            DROP TABLE IF EXISTS summaries;
            DROP TABLE IF EXISTS artifacts;
            """
        )
    database.executescript(
        """
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        PRAGMA temp_store=FILE;
        CREATE TABLE IF NOT EXISTS nodes (
            tool TEXT NOT NULL,
            id TEXT NOT NULL,
            label TEXT NOT NULL,
            normalized_label TEXT NOT NULL,
            kind TEXT NOT NULL,
            source_file TEXT NOT NULL,
            source_location TEXT NOT NULL,
            fact_key TEXT NOT NULL,
            qualified_name TEXT NOT NULL,
            language TEXT NOT NULL,
            module TEXT NOT NULL,
            placeholder INTEGER NOT NULL,
            anchored_definition INTEGER NOT NULL,
            callable INTEGER NOT NULL,
            payload_sha256 TEXT NOT NULL,
            PRIMARY KEY (tool, id)
        );
        CREATE TABLE IF NOT EXISTS edges (
            tool TEXT NOT NULL,
            source TEXT NOT NULL,
            target TEXT NOT NULL,
            relation TEXT NOT NULL,
            original_relation TEXT NOT NULL,
            context TEXT NOT NULL,
            source_fact_key TEXT NOT NULL,
            target_fact_key TEXT NOT NULL,
            occurrence_file TEXT NOT NULL,
            occurrence_location TEXT NOT NULL,
            payload_sha256 TEXT NOT NULL,
            UNIQUE (tool, source, target, relation, payload_sha256)
        );
        CREATE INDEX IF NOT EXISTS edges_source ON edges(tool, source);
        CREATE INDEX IF NOT EXISTS edges_target ON edges(tool, target);
        CREATE INDEX IF NOT EXISTS edges_fact
          ON edges(tool, relation, source_fact_key, target_fact_key);
        CREATE INDEX IF NOT EXISTS nodes_fact_key ON nodes(tool, fact_key);
        CREATE INDEX IF NOT EXISTS nodes_semantic
          ON nodes(tool, normalized_label, language, module);
        CREATE TABLE IF NOT EXISTS summaries (
            tool TEXT PRIMARY KEY,
            nodes INTEGER NOT NULL,
            edges INTEGER NOT NULL,
            validation_errors INTEGER NOT NULL,
            digest TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS artifacts (
            tool TEXT PRIMARY KEY,
            file_sha256 TEXT NOT NULL
        );
        """
    )
    database.execute(f"PRAGMA user_version={schema_version}")


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
        diagnostics = iter_top_level_object_array(path, "graph", "diagnostics")
        return sum(
            str(item.get("severity", "")).lower() == "error"
            for item in diagnostics
        )
    except KeyError:
        return 0


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
    database.execute("DELETE FROM artifacts WHERE tool = ?", (tool,))
    artifact_digest = _file_sha256(graph_path)

    node_count = 0
    for record in _records(graph_path, "nodes"):
        identifier = record.get("id")
        if not isinstance(identifier, str) or not identifier:
            raise ValueError(f"{tool} node has an invalid id")
        label, source_file, source_location, fact_key = _source_fact(record, identifier)
        normalized_label = _normalized_label(record, identifier)[1]
        kind = _text(
            record.get(
                "kind",
                record.get("symbol_kind", record.get("type", record.get("node_type"))),
            )
        ).casefold()
        qualified_name = _qualified_name(record)
        language = _language(record, source_file)
        module = _module(record, source_file)
        callable_fact = kind in {"function", "method", "constructor"} or label.rstrip().endswith(
            "()"
        )
        explicit_placeholder = any(
            record.get(key) is True
            for key in ("placeholder", "unresolved", "external")
        ) or _text(record.get("resolution")).casefold() in {"deferred", "unresolved"}
        placeholder = (
            explicit_placeholder
            or not source_file
            or kind in {"import", "export"}
            or (tool == "graphify" and not kind)
        )
        anchored_definition = bool(
            source_file
            and not placeholder
            and kind
            not in {
                "file",
                "import",
                "export",
                "resource",
                "rationale",
            }
        )
        payload = _hash(
            (
                identifier,
                label,
                normalized_label,
                kind,
                source_file,
                source_location,
                fact_key,
                qualified_name,
                language,
                module,
                placeholder,
                anchored_definition,
                callable_fact,
            )
        )
        existing = database.execute(
            "SELECT payload_sha256 FROM nodes WHERE tool = ? AND id = ?",
            (tool, identifier),
        ).fetchone()
        if existing is not None and existing[0] != payload:
            raise ValueError(f"{tool} node id has conflicting payloads: {identifier}")
        if existing is None:
            database.execute(
                "INSERT INTO nodes VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    tool,
                    identifier,
                    label,
                    normalized_label,
                    kind,
                    source_file,
                    source_location,
                    fact_key,
                    qualified_name,
                    language,
                    module,
                    int(placeholder),
                    int(anchored_definition),
                    int(callable_fact),
                    payload,
                ),
            )
            node_count += 1

    node_facts = dict(
        database.execute("SELECT id,fact_key FROM nodes WHERE tool = ?", (tool,))
    )
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
        original_relation = relation.lower()
        relation = _shared_relation(tool, original_relation)
        context = _text(record.get("context"))
        source_fact_key = node_facts.get(source, "")
        target_fact_key = node_facts.get(target, "")
        occurrence_file, occurrence_location = _edge_occurrence(record)
        confidence = record.get("confidence")
        if isinstance(confidence, float) and not math.isfinite(confidence):
            raise ValueError(f"{tool} edge has non-finite confidence")
        payload = _hash(
            (
                source,
                target,
                relation,
                original_relation,
                context,
                _text(confidence),
                occurrence_file,
                occurrence_location,
            )
        )
        before = database.total_changes
        database.execute(
            "INSERT OR IGNORE INTO edges VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                tool,
                source,
                target,
                relation,
                original_relation,
                context,
                source_fact_key,
                target_fact_key,
                occurrence_file,
                occurrence_location,
                payload,
            ),
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
    database.execute(
        "INSERT INTO artifacts VALUES (?, ?)",
        (tool, artifact_digest),
    )
    database.commit()
    return GraphSummary(tool, node_count, edge_count, validation_errors, digest)


def canonical_graph_digest(database: sqlite3.Connection, tool: str) -> str:
    digest = hashlib.sha256()
    digest.update(b"compass.performance-canonical-graph/1\0")
    for table, query in (
        (
            "node",
            "SELECT id,label,normalized_label,kind,source_file,source_location,fact_key,"
            "qualified_name,language,module,placeholder,anchored_definition,callable,"
            "payload_sha256 "
            "FROM nodes WHERE tool = ? ORDER BY id",
        ),
        (
            "edge",
            "SELECT source,target,relation,occurrence_file,occurrence_location,payload_sha256 "
            "FROM edges "
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


def _node_facts(database: sqlite3.Connection, tool: str) -> dict[str, NodeFact]:
    rows = database.execute(
        """
        SELECT id,label,normalized_label,kind,source_file,source_location,fact_key,
               qualified_name,language,module,placeholder,anchored_definition,callable
        FROM nodes WHERE tool = ? ORDER BY id
        """,
        (tool,),
    )
    return {
        str(row[0]): NodeFact(
            identifier=str(row[0]),
            label=str(row[1]),
            normalized_label=str(row[2]),
            kind=str(row[3]),
            source_file=str(row[4]),
            source_location=str(row[5]),
            fact_key=str(row[6]),
            qualified_name=str(row[7]),
            language=str(row[8]),
            module=str(row[9]),
            placeholder=bool(row[10]),
            anchored_definition=bool(row[11]),
            callable=bool(row[12]),
        )
        for row in rows
    }


def _edge_facts(database: sqlite3.Connection, tool: str) -> list[EdgeFact]:
    return [
        EdgeFact(
            source=str(row[0]),
            target=str(row[1]),
            relation=str(row[2]),
            original_relation=str(row[3]),
            context=str(row[4]),
            source_fact_key=str(row[5]),
            target_fact_key=str(row[6]),
            occurrence_file=str(row[7]),
            occurrence_location=str(row[8]),
            payload_sha256=str(row[9]),
        )
        for row in database.execute(
            """
            SELECT source,target,relation,original_relation,context,
                   source_fact_key,target_fact_key,occurrence_file,
                   occurrence_location,payload_sha256
            FROM edges WHERE tool = ?
            ORDER BY source,target,relation,occurrence_file,occurrence_location,payload_sha256
            """,
            (tool,),
        )
    ]


def _compatible_definition(graphify: NodeFact, compass: NodeFact) -> bool:
    if graphify.normalized_label != compass.normalized_label:
        return False
    if graphify.language and compass.language and graphify.language != compass.language:
        return False
    if graphify.module and compass.module and graphify.module != compass.module:
        return False
    if (
        graphify.qualified_name
        and compass.qualified_name
        and graphify.qualified_name != compass.qualified_name
    ):
        return False
    return True


def _identifier_carries_module(identifier: str, module: str) -> bool:
    normalized_module = re.sub(r"[^a-z0-9]+", "_", module.casefold()).strip("_")
    normalized_identifier = re.sub(r"[^a-z0-9]+", "_", identifier.casefold()).strip("_")
    return bool(
        normalized_module
        and normalized_identifier.startswith(f"{normalized_module}_")
    )


def _terminal_symbol(normalized_label: str) -> str:
    return re.split(r"::|\.", normalized_label)[-1]


def _rust_generic_owner_name(node: NodeFact) -> str | None:
    if node.language != "rust" or "<" not in node.label:
        return None
    owner = node.label.split("<", 1)[0].strip()
    if not owner or owner.startswith(("&", "[", "(")):
        return None
    normalized = re.sub(r"\(\)$", "", owner.casefold()).lstrip(".")
    return _terminal_symbol(normalized) or None


def _rust_generic_parameter_placeholder(node: NodeFact) -> bool:
    return bool(
        node.language == "rust"
        and node.placeholder
        and not node.kind
        and node.source_file
        and re.fullmatch(r"[A-Z]", node.label)
    )


def _qualified_name_has_owner(qualified_name: str) -> bool:
    return "." in qualified_name or "::" in qualified_name


def _constructor_matches_name(constructor: NodeFact, normalized_label: str) -> bool:
    qualified_parts = [
        part for part in re.split(r"::|\.", constructor.qualified_name) if part
    ]
    return bool(
        len(qualified_parts) >= 2
        and qualified_parts[-1] in {"<init>", "new"}
        and qualified_parts[-2] == _terminal_symbol(normalized_label)
    )


def _unique_nearest_definition(
    candidates: list[NodeFact], source_line: int, *, maximum_distance: int = 8
) -> NodeFact | None:
    nearby = [
        (abs(candidate_line - source_line), candidate)
        for candidate in candidates
        if (candidate_line := _line_number(candidate.source_location)) is not None
        and abs(candidate_line - source_line) <= maximum_distance
    ]
    if not nearby:
        return None
    minimum_distance = min(distance for distance, _ in nearby)
    nearest = [candidate for distance, candidate in nearby if distance == minimum_distance]
    return nearest[0] if len(nearest) == 1 else None


def _unverifiable_placeholder(node: NodeFact) -> bool:
    return bool(
        node.placeholder
        and not node.source_file
        and not node.kind
        and not node.qualified_name
        and not node.language
        and not node.module
    )


def _classify_nodes(
    graphify_nodes: dict[str, NodeFact],
    compass_nodes: dict[str, NodeFact],
) -> tuple[dict[str, Coverage], dict[str, str]]:
    compass_by_fact: dict[str, list[NodeFact]] = {}
    compass_by_label: dict[str, list[NodeFact]] = {}
    compass_callables_by_site: dict[tuple[str, str, str], list[NodeFact]] = {}
    compass_callables_by_file_name: dict[tuple[str, str], list[NodeFact]] = {}
    compass_constructors_by_site: dict[tuple[str, str], list[NodeFact]] = {}
    compass_constructors_by_file: dict[str, list[NodeFact]] = {}
    compass_definitions_by_site: dict[tuple[str, str, str], list[NodeFact]] = {}
    compass_definitions_by_file_name: dict[tuple[str, str], list[NodeFact]] = {}
    for node in compass_nodes.values():
        compass_by_fact.setdefault(node.fact_key, []).append(node)
        if node.anchored_definition and not node.callable:
            compass_by_label.setdefault(node.normalized_label, []).append(node)
            definition_name = _terminal_symbol(node.normalized_label)
            compass_definitions_by_site.setdefault(
                (
                    node.source_file,
                    node.source_location,
                    definition_name,
                ),
                [],
            ).append(node)
            compass_definitions_by_file_name.setdefault(
                (node.source_file, definition_name), []
            ).append(node)
        if node.anchored_definition and node.callable:
            callable_name = _terminal_symbol(node.normalized_label)
            compass_callables_by_site.setdefault(
                (
                    node.source_file,
                    node.source_location,
                    callable_name,
                ),
                [],
            ).append(node)
            compass_callables_by_file_name.setdefault(
                (node.source_file, callable_name), []
            ).append(node)
            if node.kind == "constructor":
                compass_constructors_by_site.setdefault(
                    (node.source_file, node.source_location), []
                ).append(node)
                compass_constructors_by_file.setdefault(node.source_file, []).append(node)

    coverage: dict[str, Coverage] = {}
    mapping: dict[str, str] = {}
    excluded_kinds = {"file", "import", "export", "resource", "rationale"}
    for identifier, graphify in graphify_nodes.items():
        exact = compass_by_fact.get(graphify.fact_key, [])
        exact_compatible = [
            candidate
            for candidate in exact
            if (not graphify.kind or not candidate.kind or graphify.kind == candidate.kind)
            and (
                not graphify.language
                or not candidate.language
                or graphify.language == candidate.language
            )
            and (
                not _qualified_name_has_owner(graphify.qualified_name)
                or not _qualified_name_has_owner(candidate.qualified_name)
                or graphify.qualified_name == candidate.qualified_name
            )
        ]
        if len(exact_compatible) == 1 and not _unverifiable_placeholder(graphify):
            coverage[identifier] = Coverage(
                "exact", "source_fact", exact_compatible[0].identifier
            )
            mapping[identifier] = exact_compatible[0].identifier
            continue
        if _rust_generic_parameter_placeholder(graphify):
            coverage[identifier] = Coverage(
                "missing", "no_compatible_anchored_definition", None
            )
            continue
        if (
            graphify.source_file
            and (generic_owner := _rust_generic_owner_name(graphify)) is not None
        ):
            generic_owner_candidates = [
                candidate
                for candidate in compass_definitions_by_file_name.get(
                    (graphify.source_file, generic_owner), []
                )
                if candidate.kind in CODE_TYPE_KINDS
                and (
                    not graphify.language
                    or not candidate.language
                    or graphify.language == candidate.language
                )
            ]
            if len(generic_owner_candidates) == 1:
                coverage[identifier] = Coverage(
                    "dominated",
                    "canonical_rust_generic_owner",
                    generic_owner_candidates[0].identifier,
                )
                mapping[identifier] = generic_owner_candidates[0].identifier
                continue
        if graphify.callable and graphify.source_file and graphify.source_location:
            callable_candidates = [
                candidate
                for candidate in compass_callables_by_site.get(
                    (
                        graphify.source_file,
                        graphify.source_location,
                        _terminal_symbol(graphify.normalized_label),
                    ),
                    [],
                )
                if not graphify.language
                or not candidate.language
                or graphify.language == candidate.language
            ]
            if len(callable_candidates) == 1:
                coverage[identifier] = Coverage(
                    "dominated",
                    "canonical_callable_owner",
                    callable_candidates[0].identifier,
                )
                mapping[identifier] = callable_candidates[0].identifier
                continue
            graphify_line = _line_number(graphify.source_location)
            constructor_candidates = [
                candidate
                for candidate in compass_constructors_by_site.get(
                    (graphify.source_file, graphify.source_location), []
                )
                if _constructor_matches_name(candidate, graphify.normalized_label)
            ]
            if len(constructor_candidates) == 1:
                coverage[identifier] = Coverage(
                    "dominated",
                    "canonical_constructor_anchor",
                    constructor_candidates[0].identifier,
                )
                mapping[identifier] = constructor_candidates[0].identifier
                continue
            nearest_constructor = (
                _unique_nearest_definition(
                    [
                        candidate
                        for candidate in compass_constructors_by_file.get(
                            graphify.source_file, []
                        )
                        if _constructor_matches_name(
                            candidate, graphify.normalized_label
                        )
                    ],
                    graphify_line,
                )
                if graphify_line is not None
                else None
            )
            if nearest_constructor is not None:
                coverage[identifier] = Coverage(
                    "dominated",
                    "nearby_constructor_declaration",
                    nearest_constructor.identifier,
                )
                mapping[identifier] = nearest_constructor.identifier
                continue
            nearby_candidates = [
                candidate
                for candidate in compass_callables_by_file_name.get(
                    (
                        graphify.source_file,
                        _terminal_symbol(graphify.normalized_label),
                    ),
                    [],
                )
                if (
                    not graphify.language
                    or not candidate.language
                    or graphify.language == candidate.language
                )
            ]
            nearest_candidate = (
                _unique_nearest_definition(nearby_candidates, graphify_line)
                if graphify_line is not None
                else None
            )
            if nearest_candidate is not None:
                coverage[identifier] = Coverage(
                    "dominated",
                    "nearby_callable_declaration",
                    nearest_candidate.identifier,
                )
                mapping[identifier] = nearest_candidate.identifier
                continue
        if (
            not graphify.callable
            and graphify.source_file
            and graphify.source_location
            and Path(graphify.source_file).name.casefold()
            != graphify.normalized_label.casefold()
        ):
            definition_candidates = [
                candidate
                for candidate in compass_definitions_by_site.get(
                    (
                        graphify.source_file,
                        graphify.source_location,
                        _terminal_symbol(graphify.normalized_label),
                    ),
                    [],
                )
                if (not graphify.kind or not candidate.kind or graphify.kind == candidate.kind)
                and (
                    not graphify.language
                    or not candidate.language
                    or graphify.language == candidate.language
                )
            ]
            if len(definition_candidates) == 1:
                coverage[identifier] = Coverage(
                    "dominated",
                    "canonical_definition_anchor",
                    definition_candidates[0].identifier,
                )
                mapping[identifier] = definition_candidates[0].identifier
                continue
            graphify_line = _line_number(graphify.source_location)
            nearby_definitions = [
                candidate
                for candidate in compass_definitions_by_file_name.get(
                    (
                        graphify.source_file,
                        _terminal_symbol(graphify.normalized_label),
                    ),
                    [],
                )
                if (not graphify.kind or not candidate.kind or graphify.kind == candidate.kind)
                and (
                    not graphify.language
                    or not candidate.language
                    or graphify.language == candidate.language
                )
            ]
            nearest_definition = (
                _unique_nearest_definition(nearby_definitions, graphify_line)
                if graphify_line is not None
                else None
            )
            if nearest_definition is not None:
                coverage[identifier] = Coverage(
                    "dominated",
                    "nearby_definition_anchor",
                    nearest_definition.identifier,
                )
                mapping[identifier] = nearest_definition.identifier
                continue
        if exact and not _unverifiable_placeholder(graphify):
            coverage[identifier] = Coverage("exact", "source_fact", exact[0].identifier)
            continue
        if _unverifiable_placeholder(graphify):
            generated_owner = [
                candidate
                for candidate in compass_by_label.get(graphify.normalized_label, [])
                if _compatible_definition(graphify, candidate)
                and candidate.kind in CODE_TYPE_KINDS
                and candidate.module
                and _identifier_carries_module(graphify.identifier, candidate.module)
            ]
            if len(generated_owner) == 1:
                coverage[identifier] = Coverage(
                    "dominated",
                    "qualified_generated_owner",
                    generated_owner[0].identifier,
                )
                mapping[identifier] = generated_owner[0].identifier
            else:
                coverage[identifier] = Coverage(
                    "rejected", "unverifiable_placeholder", None
                )
            continue
        if (
            not graphify.placeholder
            or graphify.callable
            or graphify.kind in excluded_kinds
            or not graphify.normalized_label
        ):
            coverage[identifier] = Coverage("missing", "no_exact_fact", None)
            continue
        candidates = [
            candidate
            for candidate in compass_by_label.get(graphify.normalized_label, [])
            if _compatible_definition(graphify, candidate)
        ]
        reason = "canonical_owner" if graphify.source_file else "resolved_definition"
        if len(candidates) > 1:
            case_exact = [
                candidate for candidate in candidates if candidate.label == graphify.label
            ]
            if len(case_exact) == 1:
                candidates = case_exact
                reason = "case_exact_owner"
        if len(candidates) > 1 and not graphify.module:
            module_candidates = [
                candidate
                for candidate in candidates
                if candidate.module
                and _identifier_carries_module(graphify.identifier, candidate.module)
            ]
            if len(module_candidates) == 1:
                candidates = module_candidates
                reason = "qualified_generated_owner"
        if len(candidates) == 1:
            coverage[identifier] = Coverage(
                "dominated", reason, candidates[0].identifier
            )
            mapping[identifier] = candidates[0].identifier
        elif len(candidates) > 1:
            coverage[identifier] = Coverage(
                "ambiguous", "multiple_anchored_definitions", None
            )
        else:
            coverage[identifier] = Coverage(
                "missing", "no_compatible_anchored_definition", None
            )
    return coverage, mapping


def _canonical_compass_endpoints(
    compass_nodes: dict[str, NodeFact],
) -> dict[str, str]:
    anchored_by_label: dict[str, list[NodeFact]] = {}
    for node in compass_nodes.values():
        if node.anchored_definition and not node.callable:
            anchored_by_label.setdefault(node.normalized_label, []).append(node)
    canonical: dict[str, str] = {}
    for node in compass_nodes.values():
        if not node.placeholder or node.callable or not node.normalized_label:
            continue
        candidates = [
            candidate
            for candidate in anchored_by_label.get(node.normalized_label, [])
            if _compatible_definition(node, candidate)
        ]
        if len(candidates) > 1:
            case_exact = [candidate for candidate in candidates if candidate.label == node.label]
            if len(case_exact) == 1:
                candidates = case_exact
        if len(candidates) == 1:
            canonical[node.identifier] = candidates[0].identifier
    return canonical


def _occurrence_match(
    graphify: EdgeFact,
    compass: EdgeFact,
    oracle: SourceOccurrenceOracle | None,
) -> str | None:
    if graphify.occurrence_file and graphify.occurrence_file != compass.occurrence_file:
        return None
    if (
        graphify.occurrence_location
        and graphify.occurrence_location != compass.occurrence_location
    ):
        if (
            oracle is None
            or not graphify.occurrence_file
            or not compass.occurrence_file
            or not oracle.same_statement(
                graphify.relation,
                graphify.occurrence_file,
                graphify.occurrence_location,
                compass.occurrence_location,
            )
        ):
            return None
        return "source_statement"
    if not (
        graphify.occurrence_file
        or graphify.occurrence_location
        or not (compass.occurrence_file or compass.occurrence_location)
    ):
        return None
    return "exact"


def _line_number(location: str) -> int | None:
    match = re.fullmatch(r"L([1-9][0-9]*)", location)
    return int(match.group(1)) if match is not None else None


def _precise_inheritance_occurrence(
    graphify: EdgeFact,
    compass: EdgeFact,
    graphify_source: NodeFact | None,
) -> bool:
    if graphify_source is None:
        return False
    if (
        graphify.occurrence_file != graphify_source.source_file
        or compass.occurrence_file != graphify.occurrence_file
    ):
        return False
    declaration_line = _line_number(graphify.occurrence_location)
    source_line = _line_number(graphify_source.source_location)
    base_line = _line_number(compass.occurrence_location)
    return bool(
        source_line is not None
        and declaration_line is not None
        and base_line is not None
        and source_line <= declaration_line <= base_line <= source_line + 8
    )


def _bounded_unique_containment_path(
    containment: dict[str, set[str]],
    compass_nodes: dict[str, NodeFact],
    source: str,
    target: str,
) -> tuple[tuple[str, ...] | None, str | None]:
    """Return one unique bounded path, or report ambiguity/search exhaustion."""
    target_node = compass_nodes.get(target)
    target_file = target_node.source_file if target_node is not None else ""
    stack = [(source, (source,))]
    witness: tuple[str, ...] | None = None
    explored = 0
    while stack:
        current, path = stack.pop()
        explored += 1
        if explored > MAX_CONTAINMENT_PATH_STATES:
            return None, "bounded_containment_search_limit"
        if len(path) - 1 >= MAX_CONTAINMENT_PATH_DEPTH:
            continue
        for candidate in sorted(containment.get(current, set()), reverse=True):
            if candidate in path:
                continue
            candidate_path = (*path, candidate)
            if candidate == target:
                if witness is not None:
                    return None, "multiple_containment_paths"
                witness = candidate_path
                continue
            candidate_node = compass_nodes.get(candidate)
            if candidate_node is None:
                continue
            if target_file and candidate_node.source_file != target_file:
                continue
            stack.append((candidate, candidate_path))
    return witness, None


def _classify_edges(
    graphify_edges: list[EdgeFact],
    compass_edges: list[EdgeFact],
    graphify_nodes: dict[str, NodeFact],
    compass_nodes: dict[str, NodeFact],
    node_coverage: dict[str, Coverage],
    node_mapping: dict[str, str],
    occurrence_oracle: SourceOccurrenceOracle | None,
) -> list[Coverage]:
    exact_index: dict[tuple[str, str, str], list[EdgeFact]] = {}
    direct_index: dict[tuple[str, str, str], list[EdgeFact]] = {}
    occurrence_target_index: dict[tuple[str, str, str, str], list[EdgeFact]] = {}
    occurrence_source_index: dict[tuple[str, str, str, str], list[EdgeFact]] = {}
    qualified_external_targets: dict[tuple[str, str, str, str], set[str]] = {}
    qualified_external_imports: dict[tuple[str, str], set[str]] = {}
    imported_symbol_targets: dict[tuple[str, str, str, str], list[EdgeFact]] = {}
    reexport_occurrence_targets: dict[
        tuple[str, str, str, str], list[EdgeFact]
    ] = {}
    reexport_targets: dict[tuple[str, str, str], set[str]] = {}
    exact_occurrence_targets: dict[
        tuple[str, str, str, str, str], set[str]
    ] = {}
    inheritance_occurrence_targets: dict[
        tuple[str, str, str, str], set[str]
    ] = {}
    typed_reference_index: dict[tuple[str, str], list[EdgeFact]] = {}
    typed_references_by_source: dict[str, list[tuple[str, EdgeFact]]] = {}
    field_type_references: dict[tuple[str, str], list[EdgeFact]] = {}
    value_references: dict[tuple[str, str, str, str], list[EdgeFact]] = {}
    containment: dict[str, set[str]] = {}
    containment_parents: dict[str, set[str]] = {}
    import_occurrences: set[tuple[str, str, str]] = set()
    for edges in (graphify_edges, compass_edges):
        for edge in edges:
            if (
                edge.relation == "imports"
                and edge.target_fact_key
                and edge.occurrence_file
                and edge.occurrence_location
            ):
                import_occurrences.add(
                    (
                        edge.target_fact_key,
                        edge.occurrence_file,
                        edge.occurrence_location,
                    )
                )
    canonical_endpoints = _canonical_compass_endpoints(compass_nodes)
    for edge in compass_edges:
        exact_index.setdefault(
            (edge.relation, edge.source_fact_key, edge.target_fact_key), []
        ).append(edge)
        source = canonical_endpoints.get(edge.source, edge.source)
        target = canonical_endpoints.get(edge.target, edge.target)
        direct_index.setdefault((edge.relation, source, target), []).append(edge)
        if edge.relation in {"returns", "type_of"}:
            typed_reference_index.setdefault((source, target), []).append(edge)
        if edge.relation == "type_of":
            typed_references_by_source.setdefault(source, []).append((target, edge))
        if (
            edge.relation == "references"
            and edge.context in {"argument", "collection"}
            and edge.occurrence_file
            and edge.occurrence_location
        ):
            value_references.setdefault(
                (source, edge.context, edge.occurrence_file, edge.occurrence_location),
                [],
            ).append(edge)
        source_node = compass_nodes.get(edge.source)
        if (
            edge.relation in {"extends", "implements"}
            and source_node is not None
            and source_node.source_file == edge.occurrence_file
            and source_node.source_location
        ):
            inheritance_occurrence_targets.setdefault(
                (
                    edge.relation,
                    source,
                    source_node.source_file,
                    source_node.source_location,
                ),
                set(),
            ).add(target)
        if edge.occurrence_file and edge.occurrence_location:
            occurrence_source_index.setdefault(
                (
                    edge.relation,
                    source,
                    edge.occurrence_file,
                    edge.occurrence_location,
                ),
                [],
            ).append(edge)
            occurrence_target_index.setdefault(
                (
                    edge.relation,
                    edge.target_fact_key,
                    edge.occurrence_file,
                    edge.occurrence_location,
                ),
                [],
            ).append(edge)
            target_node = compass_nodes.get(edge.target)
            if (
                edge.relation
                in {
                    "calls",
                    "references",
                    "embeds",
                    "extends",
                    "implements",
                    "imports",
                    "exports",
                }
                and target_node is not None
                and target_node.normalized_label
            ):
                exact_occurrence_targets.setdefault(
                    (
                        edge.relation,
                        source,
                        edge.occurrence_file,
                        edge.occurrence_location,
                        _terminal_symbol(target_node.normalized_label),
                    ),
                    set(),
                ).add(target)
            if (
                edge.relation == "imports"
                and target_node is not None
                and target_node.placeholder
                and target_node.qualified_name
            ):
                qualified_external_imports.setdefault(
                    (edge.occurrence_file, edge.occurrence_location),
                    set(),
                ).add(edge.target)
            if (
                target_node is not None
                and target_node.placeholder
                and _qualified_name_has_owner(target_node.qualified_name)
            ):
                qualified_external_targets.setdefault(
                    (
                        edge.relation,
                        edge.occurrence_file,
                        edge.occurrence_location,
                        target_node.normalized_label,
                    ),
                    set(),
                ).add(edge.target)
        if edge.relation == "contains":
            containment.setdefault(source, set()).add(target)
            containment_parents.setdefault(target, set()).add(source)
        if (
            edge.relation == "exports"
            and edge.occurrence_file
            and edge.occurrence_location
        ):
            reexport_occurrence_targets.setdefault(
                (
                    source,
                    edge.target_fact_key,
                    edge.occurrence_file,
                    edge.occurrence_location,
                ),
                [],
            ).append(edge)
            reexport_targets.setdefault(
                (source, edge.occurrence_file, edge.occurrence_location), set()
            ).add(target)
        if edge.relation == "imports":
            target_node = compass_nodes.get(edge.target)
            if (
                target_node is not None
                and target_node.source_file
                and edge.occurrence_file
                and edge.occurrence_location
            ):
                imported_symbol_targets.setdefault(
                    (
                        source,
                        target_node.source_file,
                        edge.occurrence_file,
                        edge.occurrence_location,
                    ),
                    [],
                ).append(edge)

    for owner, children in containment.items():
        for child in children:
            for target, edge in typed_references_by_source.get(child, []):
                field_type_references.setdefault((owner, target), []).append(edge)

    output: list[Coverage] = []
    for graphify in graphify_edges:
        if (
            graphify.relation == "references"
            and (
                graphify.target_fact_key,
                graphify.occurrence_file,
                graphify.occurrence_location,
            )
            in import_occurrences
        ):
            output.append(
                Coverage("rejected", "module_import_projected_to_symbol", None)
            )
            continue

        source_coverage = node_coverage.get(graphify.source)
        target_coverage = node_coverage.get(graphify.target)
        unverifiable_endpoint = any(
            fact is not None
            and fact.status == "rejected"
            and fact.reason == "unverifiable_placeholder"
            for fact in (source_coverage, target_coverage)
        )
        exact = [
            (edge, occurrence)
            for edge in exact_index.get(
                (
                    graphify.relation,
                    graphify.source_fact_key,
                    graphify.target_fact_key,
                ),
                [],
            )
            if (occurrence := _occurrence_match(graphify, edge, occurrence_oracle))
            is not None
        ]
        if exact and not unverifiable_endpoint:
            edge, occurrence = exact[0]
            output.append(
                Coverage(
                    "exact" if occurrence == "exact" else "dominated",
                    "relationship_fact"
                    if occurrence == "exact"
                    else "source_statement_occurrence",
                    edge.payload_sha256,
                )
            )
            continue

        graphify_target = graphify_nodes.get(graphify.target)
        external_imports = qualified_external_imports.get(
            (graphify.occurrence_file, graphify.occurrence_location),
            set(),
        )
        if (
            graphify.relation == "imports"
            and graphify_target is not None
            and graphify_target.source_file
            and len(external_imports) == 1
        ):
            output.append(
                Coverage(
                    "rejected",
                    "qualified_external_import_rebound_to_local",
                    next(iter(external_imports)),
                )
            )
            continue
        corrected_targets = qualified_external_targets.get(
            (
                graphify.relation,
                graphify.occurrence_file,
                graphify.occurrence_location,
                graphify_target.normalized_label if graphify_target is not None else "",
            ),
            set(),
        )
        if graphify_target is not None and len(corrected_targets) == 1:
            status = "rejected" if graphify_target.source_file else "dominated"
            reason = (
                "qualified_external_target_rebound_to_local"
                if graphify_target.source_file
                else "qualified_external_binding"
            )
            output.append(
                Coverage(
                    status,
                    reason,
                    next(iter(corrected_targets)),
                )
            )
            continue

        precise_owner = occurrence_target_index.get(
            (
                graphify.relation,
                graphify.target_fact_key,
                graphify.occurrence_file,
                graphify.occurrence_location,
            ),
            [],
        )
        if (
            graphify.relation in {"calls", "references", "imports", "rationale_for"}
            and len(precise_owner) == 1
        ):
            output.append(
                Coverage(
                    "dominated",
                    "precise_occurrence_owner",
                    precise_owner[0].payload_sha256,
                )
            )
            continue

        source = node_mapping.get(graphify.source)
        target = node_mapping.get(graphify.target)
        graphify_source = graphify_nodes.get(graphify.source)
        compass_source = compass_nodes.get(source) if source is not None else None
        compass_target = compass_nodes.get(target) if target is not None else None
        if (
            graphify.relation == "references"
            and graphify.context in {"field", "generic_arg"}
            and source is not None
            and target is not None
        ):
            precise_field_types = [
                edge
                for edge in field_type_references.get((source, target), [])
                if _occurrence_match(graphify, edge, occurrence_oracle) is not None
            ]
            if len(precise_field_types) == 1:
                output.append(
                    Coverage(
                        "dominated",
                        "precise_field_type",
                        precise_field_types[0].payload_sha256,
                    )
                )
                continue
            if len(precise_field_types) > 1:
                output.append(Coverage("ambiguous", "multiple_field_types", None))
                continue
        if (
            graphify.relation
            in {"calls", "references", "extends", "implements"}
            and compass_source is not None
            and compass_target is not None
            and compass_source.anchored_definition
            and compass_target.anchored_definition
            and compass_source.language
            and compass_target.language
            and compass_source.language != compass_target.language
            and graphify.occurrence_file == compass_source.source_file
        ):
            output.append(Coverage("rejected", "cross_language_target", None))
            continue
        if (
            graphify.original_relation == "indirect_call"
            and source is not None
            and target is not None
        ):
            references = [
                edge
                for edge in value_references.get(
                    (
                        source,
                        graphify.context,
                        graphify.occurrence_file,
                        graphify.occurrence_location,
                    ),
                    [],
                )
                if _occurrence_match(graphify, edge, occurrence_oracle) is not None
            ]
            corrected_targets = {
                canonical_endpoints.get(edge.target, edge.target) for edge in references
            }
            if len(corrected_targets) == 1 and target not in corrected_targets:
                output.append(
                    Coverage(
                        "rejected",
                        "argument_reference_target_conflict",
                        references[0].payload_sha256,
                    )
                )
                continue
            if len(corrected_targets) == 1:
                output.append(
                    Coverage(
                        "rejected",
                        "value_reference_not_indirect_call",
                        references[0].payload_sha256,
                    )
                )
                continue
            if len(corrected_targets) > 1:
                output.append(
                    Coverage("ambiguous", "multiple_argument_reference_targets", None)
                )
                continue
        if graphify.relation == "imports" and source is not None:
            exact_reexports = reexport_occurrence_targets.get(
                (
                    source,
                    graphify.target_fact_key,
                    graphify.occurrence_file,
                    graphify.occurrence_location,
                ),
                [],
            )
            if len(exact_reexports) == 1:
                output.append(
                    Coverage(
                        "dominated",
                        "symbol_reexport",
                        exact_reexports[0].payload_sha256,
                    )
                )
                continue
            if len(exact_reexports) > 1:
                output.append(Coverage("ambiguous", "multiple_symbol_reexports", None))
                continue
            occurrence_reexports = reexport_targets.get(
                (source, graphify.occurrence_file, graphify.occurrence_location), set()
            )
            if occurrence_reexports and target not in occurrence_reexports:
                output.append(
                    Coverage(
                        "rejected",
                        "reexport_target_conflict",
                        sorted(occurrence_reexports)[0],
                    )
                )
                continue
        inheritance_targets = (
            inheritance_occurrence_targets.get(
                (
                    graphify.relation,
                    source,
                    graphify.occurrence_file,
                    graphify.occurrence_location,
                ),
                set(),
            )
            if source is not None
            and graphify.relation in {"extends", "implements"}
            and graphify_source is not None
            and graphify.occurrence_file == graphify_source.source_file
            and graphify.occurrence_location == graphify_source.source_location
            else set()
        )
        if inheritance_targets:
            placeholder_matches = (
                {
                    candidate
                    for candidate in inheritance_targets
                    if graphify_target is not None
                    and candidate in compass_nodes
                    and _terminal_symbol(compass_nodes[candidate].normalized_label)
                    == _terminal_symbol(graphify_target.normalized_label)
                }
                if target is None
                else set()
            )
            if len(placeholder_matches) == 1:
                output.append(
                    Coverage(
                        "dominated",
                        "precise_inheritance_occurrence",
                        next(iter(placeholder_matches)),
                    )
                )
                continue
            if len(placeholder_matches) > 1:
                output.append(
                    Coverage("ambiguous", "multiple_inheritance_occurrences", None)
                )
                continue
            if target not in inheritance_targets:
                output.append(
                    Coverage(
                        "rejected",
                        "exact_inheritance_target_conflict",
                        sorted(inheritance_targets)[0],
                    )
                )
                continue
        occurrence_targets = (
            exact_occurrence_targets.get(
                (
                    graphify.relation,
                    source,
                    graphify.occurrence_file,
                    graphify.occurrence_location,
                    _terminal_symbol(graphify_target.normalized_label)
                    if graphify_target is not None
                    else "",
                ),
                set(),
            )
            if source is not None
            else set()
        )
        if occurrence_targets and target not in occurrence_targets:
            resolved_target = sorted(occurrence_targets)[0]
            status = (
                "rejected"
                if graphify_target is not None and graphify_target.source_file
                else "dominated"
            )
            output.append(
                Coverage(
                    status,
                    "exact_occurrence_target_conflict",
                    resolved_target,
                )
            )
            continue
        if (
            graphify.relation == "imports"
            and source is not None
            and graphify_target is not None
            and graphify_target.source_file
        ):
            imported_symbols = imported_symbol_targets.get(
                (
                    source,
                    graphify_target.source_file,
                    graphify.occurrence_file,
                    graphify.occurrence_location,
                ),
                [],
            )
            if imported_symbols:
                output.append(
                    Coverage(
                        "dominated",
                        "imported_symbol_definition",
                        imported_symbols[0].payload_sha256,
                    )
                )
                continue
        if unverifiable_endpoint:
            output.append(
                Coverage("rejected", "unverifiable_placeholder_endpoint", None)
            )
            continue
        if source is None or target is None:
            status = (
                "ambiguous"
                if any(
                    fact is not None and fact.status == "ambiguous"
                    for fact in (source_coverage, target_coverage)
                )
                else "missing"
            )
            output.append(Coverage(status, "uncovered_endpoint", None))
            continue

        if (
            graphify.relation == "calls"
            and compass_target is not None
            and compass_target.language == "go"
            and compass_target.kind in CODE_TYPE_KINDS
        ):
            conversions = [
                edge
                for edge in direct_index.get(("references", source, target), [])
                if _occurrence_match(graphify, edge, occurrence_oracle) is not None
            ]
            if len(conversions) == 1:
                output.append(
                    Coverage(
                        "rejected",
                        "go_type_conversion_not_call",
                        conversions[0].payload_sha256,
                    )
                )
                continue
            if len(conversions) > 1:
                output.append(
                    Coverage("ambiguous", "multiple_go_type_conversions", None)
                )
                continue

        if graphify.relation == "references":
            typed_references = [
                edge
                for edge in typed_reference_index.get((source, target), [])
                if _occurrence_match(graphify, edge, occurrence_oracle) is not None
            ]
            if len(typed_references) == 1:
                output.append(
                    Coverage(
                        "dominated",
                        "precise_typed_reference",
                        typed_references[0].payload_sha256,
                    )
                )
                continue
            if len(typed_references) > 1:
                output.append(
                    Coverage("ambiguous", "multiple_typed_reference_facts", None)
                )
                continue

        direct = direct_index.get((graphify.relation, source, target), [])
        if graphify.relation in {"extends", "implements"}:
            precise_inheritance = [
                edge
                for edge in direct
                if _precise_inheritance_occurrence(graphify, edge, graphify_source)
            ]
            if len(precise_inheritance) == 1:
                output.append(
                    Coverage(
                        "dominated",
                        "precise_inheritance_occurrence",
                        precise_inheritance[0].payload_sha256,
                    )
                )
                continue
            if len(precise_inheritance) > 1:
                output.append(
                    Coverage("ambiguous", "multiple_inheritance_occurrences", None)
                )
                continue
        if (
            graphify.relation == "references"
            and graphify_source is not None
            and graphify.occurrence_file == graphify_source.source_file
            and graphify.occurrence_location == graphify_source.source_location
        ):
            precise_references = [
                edge
                for edge in direct
                if edge.occurrence_file == graphify.occurrence_file
                and edge.occurrence_location != graphify.occurrence_location
            ]
            if precise_references:
                output.append(
                    Coverage(
                        "dominated",
                        "precise_declaration_reference_occurrence",
                        precise_references[0].payload_sha256,
                    )
                )
                continue
        if graphify.relation == "contains":
            containment_path, containment_failure = _bounded_unique_containment_path(
                containment, compass_nodes, source, target
            )
            if containment_path is not None:
                output.append(
                    Coverage(
                        "dominated",
                        "containment_path",
                        containment_path[1],
                    )
                )
            elif containment_failure is not None:
                output.append(Coverage("ambiguous", containment_failure, None))
            else:
                exact_parents = containment_parents.get(target, set())
                specific_parents = {
                    parent
                    for parent in exact_parents
                    if not any(
                        other != parent and other in containment.get(parent, set())
                        for other in exact_parents
                    )
                }
                specific_parent = (
                    next(iter(specific_parents))
                    if len(specific_parents) == 1
                    else None
                )
                if (
                    specific_parent is not None
                    and source != specific_parent
                    and compass_nodes.get(source) is not None
                    and compass_nodes[source].kind in CODE_TYPE_KINDS
                    and compass_nodes.get(specific_parent) is not None
                    and compass_nodes[specific_parent].kind in CODE_TYPE_KINDS
                    and graphify_source is not None
                    and graphify_source.source_file
                    and graphify_target is not None
                    and graphify_target.source_file
                ):
                    output.append(
                        Coverage(
                            "rejected",
                            "exact_containment_owner_conflict",
                            specific_parent,
                        )
                    )
                else:
                    output.append(
                        Coverage("missing", "no_bounded_containment_path", None)
                    )
            continue

        if not (graphify.occurrence_file or graphify.occurrence_location):
            output.append(Coverage("missing", "missing_relationship_occurrence", None))
            continue
        matching = [
            (edge, occurrence)
            for edge in direct
            if (occurrence := _occurrence_match(graphify, edge, occurrence_oracle))
            is not None
        ]
        if len(matching) == 1:
            edge, occurrence = matching[0]
            reason = "source_statement_occurrence"
            if occurrence != "source_statement":
                if (
                    source_coverage is not None
                    and source_coverage.reason == "canonical_rust_generic_owner"
                ):
                    reason = "exact_rust_impl_owner"
                elif (
                    target_coverage is not None
                    and target_coverage.reason == "canonical_owner"
                ):
                    reason = "canonical_owner"
                else:
                    reason = "resolved_endpoint"
            output.append(Coverage("dominated", reason, edge.payload_sha256))
        elif len(matching) > 1:
            output.append(Coverage("ambiguous", "multiple_relationship_facts", None))
        else:
            output.append(Coverage("missing", "no_matching_relationship_occurrence", None))
    return output


def _coverage_metrics(prefix: str, coverage: list[Coverage]) -> dict[str, int | str]:
    metrics: dict[str, int | str] = {}
    for status in ("exact", "dominated", "rejected", "ambiguous", "missing"):
        metrics[f"{status}_graphify_{prefix}"] = sum(
            fact.status == status for fact in coverage
        )
    reasons: dict[str, int] = {}
    for fact in coverage:
        reasons[f"{fact.status}:{fact.reason}"] = reasons.get(
            f"{fact.status}:{fact.reason}", 0
        ) + 1
    metrics[f"graphify_{prefix}_coverage_reasons"] = _canonical(reasons)
    return metrics


def _coverage_examples(
    facts: list[tuple[str, Coverage]], status: str, limit: int = 10
) -> str:
    examples = [
        f"{identifier} [{coverage.reason}]"
        for identifier, coverage in facts
        if coverage.status == status
    ]
    return ", ".join(examples[:limit])


def compare_graphs(
    database: sqlite3.Connection,
    source_root: Path | None = None,
) -> CorrectnessResult:
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

    if "graphify" in tools and compass is not None:
        graphify_summary = database.execute(
            "SELECT nodes,edges,validation_errors,digest FROM summaries "
            "WHERE tool = 'graphify'"
        ).fetchone()
        assert graphify_summary is not None
        metrics.update(
            graphify_nodes=int(graphify_summary[0]),
            graphify_edges=int(graphify_summary[1]),
            graphify_validation_errors=int(graphify_summary[2]),
            graphify_digest=str(graphify_summary[3]),
        )
        if int(graphify_summary[2]):
            failures.append(
                f"Graphify graph reports {int(graphify_summary[2])} validation errors"
            )

        compass_nodes = _node_facts(database, "compass")
        graphify_nodes = _node_facts(database, "graphify")
        node_coverage, node_mapping = _classify_nodes(graphify_nodes, compass_nodes)
        node_facts = [
            (identifier, node_coverage[identifier])
            for identifier in sorted(node_coverage)
        ]
        node_metrics = _coverage_metrics(
            "nodes", [coverage for _, coverage in node_facts]
        )
        metrics.update(node_metrics)
        metrics["missing_graphify_nodes"] = node_metrics["missing_graphify_nodes"]
        metrics["mismatched_shared_nodes"] = 0

        graphify_edges = _edge_facts(database, "graphify")
        compass_edges = _edge_facts(database, "compass")
        edge_coverage = _classify_edges(
            graphify_edges,
            compass_edges,
            graphify_nodes,
            compass_nodes,
            node_coverage,
            node_mapping,
            SourceOccurrenceOracle(source_root) if source_root is not None else None,
        )
        edge_metrics = _coverage_metrics("edges", edge_coverage)
        metrics.update(edge_metrics)
        metrics["missing_graphify_edges"] = edge_metrics["missing_graphify_edges"]

        for prefix, facts in (
            ("nodes", node_facts),
            (
                "edges",
                [
                    (
                        f"{edge.source}|{edge.relation}|{edge.target}",
                        coverage,
                    )
                    for edge, coverage in zip(graphify_edges, edge_coverage, strict=True)
                ],
            ),
        ):
            for status in ("ambiguous", "missing"):
                count = int(metrics[f"{status}_graphify_{prefix}"])
                if count:
                    failures.append(
                        f"{status}_graphify_{prefix}: {count}; examples: "
                        f"{_coverage_examples(facts, status)}"
                    )

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
