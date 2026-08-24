#!/usr/bin/env python3
"""Executable semantic oracle for the published ``compass.graph/1`` contract."""

from __future__ import annotations

import hashlib
import json
import re
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable

# The graph wire contract remains compass.graph/1.  This expectation schema is
# independently versioned so adding frontend vocabulary cannot make an older
# oracle silently accept a newer manifest.
SCHEMA = "compass.code-graph-qualification/2"
GRAPH_SCHEMA = "compass.graph/1"
NODE_KINDS = (
    "file", "module", "package", "namespace", "class", "struct", "interface",
    "trait", "protocol", "enum", "enum_member", "type_alias", "function",
    "method", "constructor", "closure", "property", "field", "variable", "constant",
    "parameter", "import", "export", "macro", "annotation", "route",
    "component", "event", "message", "topic", "queue", "job", "resource",
    "schema", "query", "migration", "config_key", "database",
    "database_schema", "database_table", "database_view", "database_column",
    "database_index", "database_constraint", "database_procedure",
    "database_trigger",
)
EDGE_KINDS = (
    "contains", "calls", "imports", "exports", "extends", "implements", "mixes_in",
    "references", "type_of", "returns", "instantiates", "overrides",
    "decorates", "routes_to", "reads", "writes", "aliases", "registers",
    "handles", "publishes", "subscribes", "produces", "consumes", "schedules",
    "triggers", "tests", "depends_on", "documents", "maps_to",
    "renders",
)
DETAIL_TYPES = {
    "file": {"file"},
    "symbol": set(NODE_KINDS[1:25]) | {"migration"},
    "import_export": {"import", "export"},
    "route": {"route"},
    "render": {"function", "method", "class", "component", "variable", "property"},
    "component": {"component"},
    "resource": {"resource"},
    "messaging": {"event", "message", "topic", "queue"},
    "job": {"job"},
    "schema": {"schema"},
    "query": {"query"},
    "config": {"config_key"},
    "database": set(NODE_KINDS[37:]),
}
TRUSTED_ORIGINS = {"ast", "config", "convention", "artifact"}
ALL_ORIGINS = TRUSTED_ORIGINS | {"heuristic"}
CONFIDENCES = {"exact", "inferred", "ambiguous"}
RESOLUTIONS = {"exact", "ambiguous", "unresolved"}
STAGES = {
    "handler", "middleware", "layout", "template", "loading", "default",
    "error_boundary", "not_found", "boundary", "loader", "action",
    "data_loader", "route_component",
}
ENTERPRISE_KINDS = {
    "event", "message", "topic", "queue", "job", "resource", "schema", "query",
    "migration", "config_key", "database", "database_schema", "database_table",
    "database_view", "database_column", "database_index", "database_constraint",
    "database_procedure", "database_trigger",
}
KNOWN_PRODUCER = re.compile(
    r"^compass\.(?:languages|frameworks|resolve|graph|postgres|semantic)\.[a-z0-9_.-]+$"
)

TYPE_KINDS = {"class", "struct", "interface", "trait", "protocol", "enum", "type_alias"}
CALLABLE = {"function", "method", "constructor", "closure", "database_procedure"}
CONTAINER = {
    "file", "module", "package", "namespace", "class", "struct", "interface",
    "trait", "protocol", "enum", "component", "resource", "schema", "database",
    "database_schema", "database_table", "database_view",
}
CONTAINS_FILE_TARGETS = set(NODE_KINDS[1:37]) | {"database"}
CONTAINS_SCOPE_TARGETS = set(NODE_KINDS[:37])
CONTAINS_TYPE_TARGETS = {
    "class", "struct", "interface", "trait", "protocol", "enum", "enum_member",
    "type_alias", "function", "method", "constructor", "closure", "property", "field",
    "variable", "constant", "parameter", "macro", "annotation", "component",
}
CONTAINS_CALLABLE_TARGETS = {
    "class", "struct", "interface", "trait", "protocol", "enum", "type_alias",
    "function", "method", "constructor", "closure", "property", "field", "variable",
    "constant", "parameter",
}
EXECUTABLE = CALLABLE | {"component", "job", "query", "database_trigger"}
DATA = {
    "property", "field", "variable", "constant", "parameter", "resource",
    "schema", "query", "config_key", "database", "database_schema",
    "database_table", "database_view", "database_column",
}
VALUE_KINDS = {"property", "field", "variable", "constant", "parameter", "import", "export", "type_alias"}
MESSAGE_KINDS = {"event", "message", "topic", "queue"}


class QualificationError(ValueError):
    """A bounded, identity-bearing qualification failure."""


def fail(invariant: str, identity: str, message: str) -> None:
    raise QualificationError(f"{invariant} [{identity}]: {message}")


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode()


def digest_bytes(data: bytes) -> str:
    return f"sha256:{hashlib.sha256(data).hexdigest()}"


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def manifest_fingerprint(paths: Iterable[Path]) -> str:
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(path.name.encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return f"sha256:{digest.hexdigest()}"


def _require(record: dict[str, Any], fields: set[str], identity: str) -> None:
    missing = sorted(field for field in fields if field not in record)
    if missing:
        fail("manifest_missing_field", identity, f"missing {', '.join(missing)}")


def load_manifest(
    path: Path,
    fixture_root: Path,
    declared_sources: set[str] | None = None,
) -> dict[str, Any]:
    manifest = load_json(path)
    if not isinstance(manifest, dict) or manifest.get("schema") != SCHEMA:
        fail("manifest_schema", str(path), f"expected {SCHEMA}")
    allowed = {
        "schema", "flows", "negatives", "nodeProducers", "edgeProducers",
        "languages", "occurrences", "coverage", "limits",
    }
    unknown = sorted(set(manifest) - allowed)
    if unknown:
        fail("manifest_unknown_field", str(path), f"unknown {', '.join(unknown)}")
    for name in allowed - {"schema"}:
        if name not in manifest:
            fail("manifest_missing_field", str(path), f"missing {name}")

    ids: set[str] = set()
    selectors: set[tuple[Any, ...]] = set()
    flow_fields = {
        "id", "framework", "routeFramework", "operation", "path", "routeSource",
        "handler", "handlerSource", "relationship", "stage", "position",
        "handlerKind", "handlerLanguage", "resolution", "origins", "producer",
        "rules", "allowHeuristic", "candidates",
    }
    for flow in manifest["flows"]:
        identity = str(flow.get("id", "<missing>"))
        _require(flow, flow_fields, identity)
        if set(flow) != flow_fields:
            fail("manifest_unknown_field", identity, f"flow fields {sorted(set(flow) - flow_fields)}")
        _unique_id(ids, identity)
        if flow["relationship"] != "routes_to":
            fail("manifest_enum", identity, "relationship must be routes_to")
        if flow["stage"] not in STAGES or flow["resolution"] not in RESOLUTIONS:
            fail("manifest_enum", identity, "unknown stage or resolution")
        if flow["handlerKind"] not in NODE_KINDS:
            fail("manifest_enum", identity, f"unknown handler kind {flow['handlerKind']}")
        if not set(flow["origins"]) <= ALL_ORIGINS or not flow["origins"]:
            fail("manifest_enum", identity, "unknown or empty origins")
        selector = (
            flow["routeFramework"], flow["operation"], flow["path"],
            flow["routeSource"], flow["stage"], flow["position"],
        )
        if selector in selectors:
            fail("manifest_duplicate_selector", identity, repr(selector))
        selectors.add(selector)
        _source_exists(fixture_root, flow["routeSource"], identity, declared_sources)
        _source_exists(fixture_root, flow["handlerSource"], identity, declared_sources)
        if not isinstance(flow["handler"], dict) or set(flow["handler"]) not in (
            {"qualifiedName"}, {"qualifiedNameTemplate"},
        ):
            fail("manifest_handler_identity", identity, "handler must contain exactly one qualified identity")

    negative_fields = {"id", "framework", "source", "routeFramework"}
    for item in manifest["negatives"]:
        identity = str(item.get("id", "<missing>"))
        _require(item, negative_fields, identity)
        if set(item) != negative_fields:
            fail("manifest_unknown_field", identity, "negative fields differ from contract")
        _unique_id(ids, identity)
        _source_exists(fixture_root, item["source"], identity, declared_sources)

    producer_fields = {
        "id", "kind", "source", "qualifiedName", "producer", "origins",
        "detailType",
    }
    for group, vocabulary in (("nodeProducers", NODE_KINDS), ("edgeProducers", EDGE_KINDS)):
        seen_kinds: set[str] = set()
        for item in manifest[group]:
            identity = str(item.get("id", "<missing>"))
            _require(item, producer_fields, identity)
            if set(item) != producer_fields:
                fail("manifest_unknown_field", identity, f"{group} fields differ from contract")
            _unique_id(ids, identity)
            if item["kind"] not in vocabulary:
                fail("manifest_enum", identity, f"unknown {group} kind {item['kind']}")
            if item["kind"] in seen_kinds:
                fail("manifest_duplicate_kind", identity, item["kind"])
            seen_kinds.add(item["kind"])
            _source_exists(fixture_root, item["source"], identity, declared_sources)
        missing = sorted(set(vocabulary) - seen_kinds)
        if missing:
            fail("manifest_missing_vocabulary", group, ", ".join(missing))
    language_contract = manifest["languages"]
    if not isinstance(language_contract, dict) or set(language_contract) != {
        "source", "producerVersion",
    } or language_contract["source"] != "corpus":
        fail("manifest_language_contract", str(path), "languages must bind to corpus")
    if not isinstance(manifest["occurrences"], list):
        fail("manifest_occurrences", str(path), "occurrences must be an array")
    occurrence_fields = {
        "id", "kind", "source", "sourceQualifiedName", "targetQualifiedName",
        "minimum",
    }
    for item in manifest["occurrences"]:
        identity = str(item.get("id", "<missing>"))
        _require(item, occurrence_fields, identity)
        if set(item) != occurrence_fields or item["kind"] not in EDGE_KINDS:
            fail("manifest_occurrences", identity, "invalid occurrence expectation")
        _unique_id(ids, identity)
        _source_exists(fixture_root, item["source"], identity, declared_sources)
        if not isinstance(item["minimum"], int) or item["minimum"] < 2:
            fail("manifest_occurrences", identity, "minimum must be at least two")
    if not isinstance(manifest["coverage"], list) or not manifest["coverage"]:
        fail("manifest_coverage", str(path), "coverage expectations must be non-empty")
    for item in manifest["coverage"]:
        identity = str(item.get("id", "<missing>"))
        _unique_id(ids, identity)
        _source_exists(fixture_root, item.get("source", ""), identity, declared_sources)
        expected = (
            {"id", "source", "forbidCompleteWhen"},
            {"id", "source", "extractionStatus"},
            {"id", "source", "diagnosticCode"},
        )
        if set(item) not in expected:
            fail("manifest_coverage", identity, "invalid coverage expectation")
    if not isinstance(manifest["limits"], dict) or set(manifest["limits"]) != {"maxDiagnostics"}:
        fail("manifest_limits", str(path), "limits must contain maxDiagnostics")
    return manifest


def _unique_id(ids: set[str], identity: str) -> None:
    if not identity or identity == "<missing>" or identity in ids:
        fail("manifest_duplicate_id", identity, "ID is empty or duplicated")
    ids.add(identity)


def _source_exists(
    root: Path,
    relative: str,
    identity: str,
    declared_sources: set[str] | None,
) -> None:
    path = Path(relative)
    declared = declared_sources is not None and relative in declared_sources
    if path.is_absolute() or ".." in path.parts or (not declared and not (root / path).is_file()):
        fail("manifest_fixture_source", identity, f"missing or unsafe source {relative}")


def _detail_type(record: dict[str, Any]) -> str | None:
    details = record.get("details")
    return details.get("type") if isinstance(details, dict) else None


def _anchor(anchor: Any, files: dict[str, dict[str, Any]], owner: str) -> None:
    if not isinstance(anchor, dict):
        fail("invalid_anchor", owner, "anchor is not an object")
    fields = ("file", "startByte", "endByte", "startLine", "startColumn", "endLine", "endColumn")
    if any(field not in anchor for field in fields):
        fail("invalid_anchor", owner, "anchor fields are incomplete")
    file = anchor["file"]
    if file not in files:
        fail("invalid_anchor", owner, f"unknown file {file!r}")
    values = [anchor[field] for field in fields[1:]]
    if any(not isinstance(value, int) or value < 0 for value in values):
        fail("invalid_anchor", owner, "anchor coordinates must be non-negative integers")
    if anchor["endByte"] < anchor["startByte"] or anchor["endByte"] > files[file]["byteSize"]:
        fail("invalid_anchor", owner, f"byte range outside {file}")
    start = (anchor["startLine"], anchor["startColumn"])
    end = (anchor["endLine"], anchor["endColumn"])
    if start > end or anchor["startLine"] == 0:
        fail("invalid_anchor", owner, "line/column range is invalid")


def _evidence(
    records: Any,
    files: dict[str, dict[str, Any]],
    owner: str,
    *,
    allow_empty_direct: bool = False,
) -> None:
    if not isinstance(records, list) or not records:
        fail("unknown_producer", owner, "missing provenance")
    for index, evidence in enumerate(records):
        identity = f"{owner}:evidence:{index}"
        producer = evidence.get("extractor", "")
        if producer.endswith(".unknown") or not KNOWN_PRODUCER.fullmatch(producer):
            fail("unknown_producer", identity, repr(producer))
        origin = evidence.get("origin")
        confidence = evidence.get("confidence")
        if origin not in ALL_ORIGINS or confidence not in CONFIDENCES:
            fail("unsupported_provenance", identity, f"{origin}/{confidence}")
        anchors = evidence.get("anchors", [])
        wiring = evidence.get("wiringSite")
        if origin in {"ast", "config", "artifact"} and not anchors:
            fail("invalid_anchor", identity, "direct evidence requires an anchor")
        if origin == "convention" and (not anchors or not evidence.get("rule")):
            fail("unsupported_provenance", identity, "convention requires rule and anchor")
        if origin == "heuristic" and (not evidence.get("rule") or not wiring):
            fail("heuristic_wiring", identity, "rule and exact wiring site required")
        for anchor in anchors:
            _anchor(anchor, files, identity)
            if (
                origin in {"ast", "config", "artifact"}
                and anchor["startByte"] == anchor["endByte"]
                and not allow_empty_direct
            ):
                fail("invalid_anchor", identity, "direct anchor is empty")
        if wiring is not None:
            _anchor(wiring, files, identity)
            if wiring["startByte"] == wiring["endByte"]:
                fail("heuristic_wiring", identity, "wiring site is empty")
        candidates = evidence.get("candidates", [])
        if len(candidates) > 20:
            fail("candidate_bound", identity, "more than 20 candidates")
        candidate_ids = [candidate.get("nodeId") for candidate in candidates]
        if candidate_ids != sorted(set(candidate_ids)):
            fail("candidate_order", identity, "candidate IDs are not unique and sorted")


def endpoint_allowed(source: dict[str, Any], edge: dict[str, Any], target: dict[str, Any]) -> bool:
    s, kind, t = source["kind"], edge["kind"], target["kind"]
    if kind == "contains":
        return (
            (s == "schema" and t == "config_key")
            or (s == "config_key" and t == "config_key")
            # Framework route hierarchy is an explicit parent-route to
            # child-route containment relation. Keep this allowance narrow;
            # do not turn every route into a generic container endpoint.
            or (s == "route" and t == "route")
            # Object-valued JavaScript/TypeScript bindings expose their
            # literal members as properties. This is a declaration-level
            # containment edge, not an inferred type relationship.
            or (s == "variable" and t == "property")
            or (s == "file" and t in CONTAINS_FILE_TARGETS)
            or (s in {"module", "package", "namespace"} and t in CONTAINS_SCOPE_TARGETS)
            or (s in TYPE_KINDS | {"component", "schema"} and t in CONTAINS_TYPE_TARGETS)
            or (s in CALLABLE | {"type_alias"} and t in CONTAINS_CALLABLE_TARGETS)
            or (s == "resource" and t in {"file", "resource", "config_key"})
            or (
                s == "database"
                and t in {
                    "database_schema", "database_table", "database_view",
                    "database_index", "database_trigger",
                }
            )
            or (
                s == "database_schema"
                and t in {
                    "database_table", "database_view", "database_procedure",
                    "database_trigger",
                }
            )
            or (
                s in {"database_table", "database_view"}
                and t in {
                    "database_column", "database_index", "database_constraint",
                    "database_trigger",
                }
            )
        )
    if kind == "calls":
        return s in CALLABLE | TYPE_KINDS | {"file", "module", "variable"} and t in CALLABLE | {"variable", "import", "type_alias"}
    if kind == "imports":
        return (s in {"file", "module", "package", "namespace", "import"} | CALLABLE and t in CONTAINER | CALLABLE | TYPE_KINDS | {"import", "export", "type_alias", "variable", "constant", "resource", "config_key"}) or (s == "config_key" and t == "resource")
    if kind == "exports":
        return s in CONTAINER | {"export"} and t in CONTAINER | CALLABLE | TYPE_KINDS | {"import", "export", "type_alias", "variable", "constant"}
    if kind == "extends":
        return s in TYPE_KINDS and t in TYPE_KINDS
    if kind == "implements":
        return (
            s in TYPE_KINDS
            and (
                t in {"interface", "trait", "protocol"}
                or (
                    source.get("language") == "dart"
                    and target.get("language") == "dart"
                    and t == "class"
                )
            )
        )
    if kind == "mixes_in":
        return s in TYPE_KINDS and t in TYPE_KINDS
    if kind == "type_of":
        return s in VALUE_KINDS and t in TYPE_KINDS | {"parameter"}
    if kind == "returns":
        return s in CALLABLE and t in TYPE_KINDS | {
            "variable", "parameter", "import", "schema", "database_table",
            "database_view",
        }
    if kind == "instantiates":
        return (
            s in CALLABLE | TYPE_KINDS | {"file", "module", "variable"}
            and (
                t in {"class", "struct", "enum", "component", "database_procedure"}
                or (
                    t == "enum_member"
                    and target.get("language") == "rust"
                )
            )
        )
    if kind == "overrides":
        return s in CALLABLE and t in CALLABLE
    if kind == "decorates":
        return s in {"annotation", "macro"} and t in CALLABLE | TYPE_KINDS | VALUE_KINDS | {"component", "route", "resource"}
    if kind == "routes_to":
        return s == "route" and t in {"file", "function", "method", "class", "component"}
    if kind == "renders":
        # Top-level JSX/createElement has no callable owner; production uses
        # the smallest source module/file as the conservative renderer.
        return s in EXECUTABLE | {"file", "module", "variable"} and t in {
            "function", "method", "class", "component", "variable", "property",
        }
    if kind == "maps_to":
        return s in {"class", "struct", "schema", "database_table", "database_view"} and t in {"database_table", "database_view"}
    if kind == "reads":
        return (s in EXECUTABLE and t in DATA) or (s == "database_view" and t == "database_table")
    if kind == "writes":
        return s in EXECUTABLE and t in DATA
    if kind == "aliases":
        return s in {"import", "export", "type_alias"} and t in CALLABLE | TYPE_KINDS | {"import", "export", "type_alias", "variable", "constant"}
    if kind == "registers":
        return s in EXECUTABLE | CONTAINER and t in CALLABLE | CONTAINER | {"component", "route", "event", "message", "topic", "queue", "job"}
    if kind in {"handles", "publishes", "produces"}:
        return s in EXECUTABLE and t in MESSAGE_KINDS
    if kind in {"subscribes", "consumes"}:
        return s in {"function", "method", "component", "job", "queue"} and t in MESSAGE_KINDS
    if kind in {"schedules", "triggers"}:
        return (s in EXECUTABLE and t in {"function", "method", "job", "event", "database_trigger"}) or (kind == "triggers" and s == "database_trigger" and t == "database_table")
    if kind == "tests":
        return (
            s in {"file", "function", "method", "class"}
            and "test" in source.get("roles", [])
            and (
                t in CALLABLE
                or t in TYPE_KINDS
                or t in CONTAINER
                or t in {
                    "route", "component", "event", "message", "topic", "queue",
                    "job", "query", "migration", "database_procedure",
                    "database_trigger",
                }
            )
        )
    if kind == "documents":
        return s == "resource" and (
            t in CALLABLE
            or t in TYPE_KINDS
            or t in CONTAINER
            or t in {
                "route", "component", "event", "message", "topic", "queue",
                "job", "schema", "query", "migration", "database_procedure",
                "database_trigger",
            }
        )
    if kind == "references":
        reference_source = CONTAINER | CALLABLE | TYPE_KINDS | {
            "file", "property", "field", "variable", "constant", "import",
            "export", "enum_member", "annotation", "macro", "type_alias",
            "parameter", "resource", "schema", "query", "config_key",
            "database_table", "database_view", "database_column",
            "database_procedure", "database_trigger",
        }
        reference_target = reference_source | {
            "parameter", "database", "database_schema", "database_index",
            "database_constraint",
        }
        return s in reference_source and t in reference_target
    if kind == "depends_on":
        dependency = CONTAINER | CALLABLE | TYPE_KINDS | {
            "file", "import", "export", "type_alias", "resource", "schema",
            "query", "config_key", "database", "database_schema",
            "database_table", "database_view", "database_procedure",
            "database_trigger",
        }
        return s in dependency and t in dependency
    return False


def validate_graph(graph: dict[str, Any], manifest: dict[str, Any]) -> dict[str, Any]:
    if graph.get("directed") is not True or graph.get("multigraph") is not True:
        fail("graph_envelope", "graph", "directed multigraph required")
    metadata = graph.get("graph", {})
    if metadata.get("schema") != GRAPH_SCHEMA:
        fail("graph_schema", "graph", f"expected {GRAPH_SCHEMA}")
    files = {item["path"]: item for item in metadata.get("files", [])}
    nodes = graph.get("nodes")
    edges = graph.get("links")
    if not isinstance(nodes, list) or not isinstance(edges, list):
        fail("graph_envelope", "graph", "nodes and links must be arrays")
    node_index: dict[str, dict[str, Any]] = {}
    edge_ids: set[str] = set()
    for node in nodes:
        identity = node.get("id", "<missing>")
        if identity in node_index:
            fail("duplicate_node_id", identity, "durable node ID repeated")
        if node.get("kind") not in NODE_KINDS:
            fail("node_kind", identity, repr(node.get("kind")))
        detail = _detail_type(node)
        if detail is not None and node["kind"] not in DETAIL_TYPES.get(detail, set()):
            fail("typed_details", identity, f"{detail} is incompatible with {node['kind']}")
        if node.get("source") is not None:
            _anchor(node["source"], files, identity)
        source_file = (node.get("source") or {}).get("file")
        allow_empty = (
            node.get("kind") == "file"
            and source_file in files
            and files[source_file].get("byteSize") == 0
        )
        _evidence(node.get("evidence"), files, identity, allow_empty_direct=allow_empty)
        node_index[identity] = node
    pair_sites: dict[tuple[str, str, str], set[tuple[Any, ...]]] = defaultdict(set)
    for edge in edges:
        identity = edge.get("id", "<missing>")
        if identity in edge_ids or identity != edge.get("key"):
            fail("duplicate_edge_id", identity, "edge ID duplicated or differs from key")
        edge_ids.add(identity)
        if edge.get("kind") not in EDGE_KINDS:
            fail("edge_kind", identity, repr(edge.get("kind")))
        if edge.get("source") not in node_index or edge.get("target") not in node_index:
            fail("dangling_endpoint", identity, f"{edge.get('source')} -> {edge.get('target')}")
        if edge["source"] == edge["target"] and edge["kind"] != "calls":
            fail("non_recursive_self_loop", identity, edge["kind"])
        source, target = node_index[edge["source"]], node_index[edge["target"]]
        if not endpoint_allowed(source, edge, target):
            fail("impossible_endpoint", identity, f"{source['kind']} -{edge['kind']}-> {target['kind']}")
        if edge.get("relationshipSite") is not None:
            _anchor(edge["relationshipSite"], files, identity)
        _evidence(edge.get("evidence"), files, identity)
        site = edge.get("relationshipSite") or {}
        pair_sites[(edge["source"], edge["target"], edge["kind"])].add(
            (site.get("file"), site.get("startByte"), site.get("endByte"), edge.get("occurrenceRule"))
        )
    _validate_coverage(metadata, files, manifest)
    _validate_external_placeholders(nodes, edges, node_index)
    _validate_global_hubs(nodes, edges, node_index)
    summary = Counter()
    summary.update({"invariants": len(nodes) + len(edges)})
    return {
        "node_index": node_index,
        "files": files,
        "pair_sites": pair_sites,
        "invariant_assertions": summary["invariants"],
    }


def _validate_coverage(metadata: dict[str, Any], files: dict[str, dict[str, Any]], manifest: dict[str, Any]) -> None:
    diagnostics = metadata.get("diagnostics", [])
    limit = manifest["limits"]["maxDiagnostics"]
    if len(diagnostics) > limit:
        fail("diagnostic_bound", "graph", f"{len(diagnostics)} > {limit}")
    coverage = metadata.get("coverage", [])
    by_file = defaultdict(list)
    for record in coverage:
        by_file[record.get("fileId")].append(record)
    for path, file in files.items():
        if file.get("extractionStatus") in {"partial", "parse_failure", "unsupported", "excluded"}:
            if any(record.get("status") == "complete" for record in by_file[file.get("id")]):
                fail("false_coverage", path, f"{file.get('extractionStatus')} file marked complete")


def _validate_global_hubs(nodes: list[dict[str, Any]], edges: list[dict[str, Any]], index: dict[str, dict[str, Any]]) -> None:
    incident = defaultdict(set)
    for edge in edges:
        for endpoint, other in ((edge["source"], edge["target"]), (edge["target"], edge["source"])):
            node = index[other]
            source = (node.get("source") or {}).get("file")
            incident[endpoint].add((node.get("language"), source))
    for node in nodes:
        if node.get("source") is None and node.get("name") == node.get("qualifiedName"):
            scopes = incident[node["id"]]
            languages = {language for language, _ in scopes if language}
            files = {source for _, source in scopes if source}
            if len(languages) > 1 or len(files) > 1:
                fail("global_unresolved_hub", node["id"], f"{len(languages)} languages/{len(files)} files")


def _validate_external_placeholders(
    nodes: list[dict[str, Any]],
    edges: list[dict[str, Any]],
    index: dict[str, dict[str, Any]],
) -> None:
    scopes: set[tuple[Any, ...]] = set()
    for node in nodes:
        records = [
            item for item in node.get("evidence", [])
            if item.get("rule") == "external-symbol-placeholder"
        ]
        if not records:
            continue
        if len(records) != 1:
            fail("external_placeholder", node["id"], "requires one placeholder provenance")
        evidence = records[0]
        wiring = evidence.get("wiringSite") or {}
        scope = (
            node.get("language"), wiring.get("file"), wiring.get("startByte"),
            wiring.get("endByte"), node.get("qualifiedName"),
        )
        if None in scope or scope in scopes:
            fail("external_placeholder_scope", node["id"], repr(scope))
        scopes.add(scope)
        if (
            evidence.get("origin") != "heuristic"
            or evidence.get("confidence") != "inferred"
            or not KNOWN_PRODUCER.fullmatch(evidence.get("extractor", ""))
            or not isinstance(node.get("details"), dict)
        ):
            fail("external_placeholder", node["id"], "not typed inferred heuristic evidence")
        incident = [
            edge for edge in edges
            if node["id"] in (edge["source"], edge["target"])
        ]
        if not incident:
            fail("external_placeholder", node["id"], "orphan placeholder")
        for edge in incident:
            edge_file = (edge.get("relationshipSite") or {}).get("file")
            if edge.get("deferred") is not True or edge_file != wiring.get("file"):
                fail(
                    "external_placeholder_deferred",
                    edge["id"],
                    f"deferred={edge.get('deferred')} scope={edge_file!r}",
                )


def _expand_identity(identity: dict[str, str], fixture_root: Path) -> str:
    if "qualifiedName" in identity:
        return identity["qualifiedName"]
    return identity["qualifiedNameTemplate"].replace("{fixtureRoot}", fixture_root.as_posix())


def assert_flows(graph: dict[str, Any], manifest: dict[str, Any], fixture_root: Path) -> dict[str, int]:
    nodes = graph["nodes"]
    index = {node["id"]: node for node in nodes}
    edges = graph["links"]
    by_framework = Counter()
    resolutions = Counter()
    for flow in manifest["flows"]:
        identity = flow["id"]
        matches = []
        for node in nodes:
            data = (node.get("details") or {}).get("data", {})
            source = (node.get("source") or {}).get("file")
            if (
                node.get("kind") == "route"
                and node.get("framework") == flow["routeFramework"]
                and data.get("operation") == flow["operation"]
                and data.get("path") == flow["path"]
                and source == flow["routeSource"]
            ):
                matches.append(node)
        if len(matches) != 1:
            fail("flow_route_selector", identity, f"matched {len(matches)} route occurrences")
        route = matches[0]
        data = route["details"]["data"]
        if data.get("resolution") != flow["resolution"]:
            fail("flow_resolution", identity, f"{data.get('resolution')} != {flow['resolution']}")
        stage_matches = [
            stage for stage in data.get("stages", [])
            if stage.get("stage") == flow["stage"] and stage.get("position") == flow["position"]
        ]
        if len(stage_matches) != 1:
            fail("flow_stage", identity, f"matched {len(stage_matches)} stages")
        stage = stage_matches[0]
        expected_candidates = flow["candidates"]
        actual_candidates = sorted(candidate["nodeId"] for candidate in stage.get("candidates", []))
        if expected_candidates and actual_candidates != sorted(expected_candidates):
            fail("flow_candidates", identity, f"{actual_candidates!r}")
        route_edges = [
            edge for edge in edges
            if edge["source"] == route["id"]
            and edge["kind"] == "routes_to"
            and (edge.get("details") or {}).get("data", {}).get("stage") == flow["stage"]
            and (edge.get("details") or {}).get("data", {}).get("position") == flow["position"]
        ]
        if len(route_edges) != 1:
            fail("flow_edge_selector", identity, f"matched {len(route_edges)} routes_to edges")
        edge = route_edges[0]
        target = index[edge["target"]]
        expected_name = _expand_identity(flow["handler"], fixture_root)
        actual_source = (target.get("source") or {}).get("file")
        if target.get("qualifiedName") != expected_name or actual_source != flow["handlerSource"]:
            fail("flow_target_mismatch", identity, f"{target.get('qualifiedName')} @ {actual_source}")
        if target.get("kind") != flow["handlerKind"] or target.get("language") != flow["handlerLanguage"]:
            fail(
                "flow_target_mismatch",
                identity,
                f"{target.get('kind')}/{target.get('language')}",
            )
        if stage.get("target") not in (None, edge["target"]):
            fail("flow_stage", identity, f"stage target {stage.get('target')} != {edge['target']}")
        evidence = edge["evidence"]
        if not any(
            item["extractor"] == flow["producer"]
            and item["origin"] in flow["origins"]
            and item.get("rule") in flow["rules"]
            for item in evidence
        ):
            fail("flow_provenance", identity, "producer/origin/rule mismatch")
        if not flow["allowHeuristic"] and any(item["origin"] == "heuristic" for item in evidence):
            fail("flow_provenance", identity, "heuristic evidence forbidden")
        if flow["resolution"] == "exact" and not any(
            item["confidence"] == "exact" and item["origin"] in TRUSTED_ORIGINS for item in evidence
        ):
            fail("false_exact", identity, edge["id"])
        by_framework[flow["framework"]] += 1
        resolutions[flow["resolution"]] += 1
    return {
        "flows": sum(by_framework.values()),
        "frameworks": len(by_framework),
        **{f"resolution_{key}": value for key, value in sorted(resolutions.items())},
    }


def assert_negatives(graph: dict[str, Any], manifest: dict[str, Any]) -> dict[str, int]:
    count = 0
    for item in manifest["negatives"]:
        route_ids = {
            node["id"] for node in graph["nodes"]
            if node.get("kind") == "route"
            and node.get("framework") == item["routeFramework"]
            and (node.get("source") or {}).get("file") == item["source"]
            and (node.get("details") or {}).get("data", {}).get("resolution") == "exact"
        }
        edge_ids = [
            edge["id"] for edge in graph["links"]
            if edge["kind"] == "routes_to" and edge["source"] in route_ids
            and any(evidence.get("confidence") == "exact" for evidence in edge.get("evidence", []))
        ]
        if route_ids or edge_ids:
            fail("framework_negative", item["id"], f"unexpected nodes {sorted(route_ids)} edges {edge_ids}")
        count += 1
    return {"negatives": count}


def assert_vocabulary(graph: dict[str, Any], manifest: dict[str, Any]) -> dict[str, int]:
    counts = {}
    for group, records, key in (
        ("nodeProducers", graph["nodes"], "node_kinds"),
        ("edgeProducers", graph["links"], "edge_kinds"),
    ):
        passed = 0
        for expectation in manifest[group]:
            candidates = []
            for record in records:
                source = (
                    (record.get("source") or {}).get("file")
                    if group == "nodeProducers"
                    else (record.get("relationshipSite") or {}).get("file")
                )
                qualified = record.get("qualifiedName") if group == "nodeProducers" else record.get("id")
                if (
                    record.get("kind") == expectation["kind"]
                    and source == expectation["source"]
                    and (expectation["qualifiedName"] == "*" or qualified == expectation["qualifiedName"])
                    and _detail_type(record) == expectation["detailType"]
                    and any(
                        evidence.get("extractor") == expectation["producer"]
                        and evidence.get("origin") in expectation["origins"]
                        for evidence in record.get("evidence", [])
                    )
                ):
                    candidates.append(record)
            if not candidates:
                fail("runtime_vocabulary_producer", expectation["id"], expectation["kind"])
            passed += 1
        counts[key] = passed
    if {node["kind"] for node in graph["nodes"]}.isdisjoint(ENTERPRISE_KINDS):
        fail("enterprise_kinds", "graph", "no enterprise/domain nodes")
    missing_enterprise = sorted(ENTERPRISE_KINDS - {node["kind"] for node in graph["nodes"]})
    if missing_enterprise:
        fail("enterprise_kinds", "graph", ", ".join(missing_enterprise))
    return counts


def assert_languages(graph: dict[str, Any], manifest: dict[str, Any]) -> dict[str, int]:
    files = {item["path"]: item for item in graph["graph"].get("files", [])}
    passed = 0
    for item in manifest.get("_languageExpectations", []):
        file = files.get(item["source"])
        if file is None:
            fail("language_matrix", item["id"], "file absent from inventory")
        if file.get("language") != item["language"]:
            fail("language_matrix", item["id"], f"{file.get('language')} != {item['language']}")
        if item["producerVersion"] not in file.get("extractorVersions", []):
            fail("producer_version", item["id"], repr(file.get("extractorVersions")))
        passed += 1
    return {"languages": passed}


def assert_occurrences(graph: dict[str, Any], manifest: dict[str, Any]) -> dict[str, int]:
    passed = 0
    index = {node["id"]: node for node in graph["nodes"]}
    for item in manifest["occurrences"]:
        matches = [
            edge for edge in graph["links"]
            if edge["kind"] == item["kind"]
            and index[edge["source"]].get("qualifiedName") == item["sourceQualifiedName"]
            and index[edge["target"]].get("qualifiedName") == item["targetQualifiedName"]
            and (edge.get("relationshipSite") or {}).get("file") == item["source"]
        ]
        sites = {
            (
                edge["relationshipSite"]["startByte"],
                edge["relationshipSite"]["endByte"],
                edge.get("occurrenceRule"),
            )
            for edge in matches if edge.get("relationshipSite")
        }
        if len(matches) < item["minimum"] or len(sites) < item["minimum"]:
            fail("repeated_occurrence_loss", item["id"], f"{len(matches)} edges/{len(sites)} sites")
        passed += 1
    return {"occurrences": passed}


def assert_coverage(graph: dict[str, Any], manifest: dict[str, Any]) -> dict[str, int]:
    metadata = graph["graph"]
    files = {item["path"]: item for item in metadata.get("files", [])}
    diagnostics = metadata.get("diagnostics", [])
    passed = 0
    for item in manifest["coverage"]:
        identity = item["id"]
        file = files.get(item["source"])
        if "extractionStatus" in item:
            if file is None or file.get("extractionStatus") != item["extractionStatus"]:
                fail(
                    "coverage_expectation",
                    identity,
                    f"{None if file is None else file.get('extractionStatus')} != {item['extractionStatus']}",
                )
        elif "forbidCompleteWhen" in item:
            if file is None:
                fail("coverage_expectation", identity, "file absent from inventory")
            status = file.get("extractionStatus")
            if status in item["forbidCompleteWhen"]:
                records = [
                    record for record in metadata.get("coverage", [])
                    if record.get("fileId") == file.get("id")
                ]
                if any(record.get("status") == "complete" for record in records):
                    fail("false_coverage", identity, f"{status} file marked complete")
        else:
            matches = [
                diagnostic for diagnostic in diagnostics
                if diagnostic.get("code") == item["diagnosticCode"]
                and (
                    diagnostic.get("file") == item["source"]
                    or (diagnostic.get("anchor") or {}).get("file") == item["source"]
                    or item["source"] in str(diagnostic)
                )
            ]
            if not matches:
                fail("coverage_diagnostic", identity, item["diagnosticCode"])
        passed += 1
    return {"coverage_expectations": passed}


def qualify_graph(graph: dict[str, Any], manifest: dict[str, Any], fixture_root: Path) -> dict[str, Any]:
    invariants = validate_graph(graph, manifest)
    summary: dict[str, Any] = {}
    summary.update(assert_flows(graph, manifest, fixture_root))
    summary.update(assert_negatives(graph, manifest))
    summary.update(assert_vocabulary(graph, manifest))
    summary.update(assert_languages(graph, manifest))
    summary.update(assert_occurrences(graph, manifest))
    summary.update(assert_coverage(graph, manifest))
    summary["invariants"] = invariants["invariant_assertions"]
    summary["coverage_records"] = len(graph["graph"].get("coverage", []))
    summary["diagnostics"] = len(graph["graph"].get("diagnostics", []))
    return dict(sorted(summary.items()))


def qualification_summary(
    *,
    compass_revision: str,
    manifest_digest: str,
    graph_bytes: bytes,
    graph: dict[str, Any],
    assertions: dict[str, Any],
    comparisons: dict[str, bool],
) -> dict[str, Any]:
    resolutions = Counter(
        (node.get("details") or {}).get("data", {}).get("resolution", "unknown")
        for node in graph["nodes"] if node.get("kind") == "route"
    )
    return {
        "schema": "compass.code-graph-qualification-summary/1",
        "compassRevision": compass_revision,
        "fixtureManifestFingerprint": manifest_digest,
        "graphDigest": digest_bytes(graph_bytes),
        "runMode": "fixtures-only",
        "assertions": dict(sorted(assertions.items())),
        "routeResolutions": dict(sorted(resolutions.items())),
        "coverage": {
            "records": len(graph["graph"].get("coverage", [])),
            "diagnostics": len(graph["graph"].get("diagnostics", [])),
        },
        "byteComparisons": dict(sorted(comparisons.items())),
    }
