"""Bounded relationship multiplicity and serialization auditing."""

from __future__ import annotations

from collections import Counter
import json
from pathlib import Path
from typing import Any

from .jsonstream import iter_top_level_array


MULTIPLICITY_SCHEMA = "compass.graph-multiplicity-audit/1"
DEFAULT_MAX_RELATIONSHIPS = 2_000_000


def _text(value: object, context: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{context} must be a non-empty string")
    return value


def _site(edge: dict[str, object]) -> tuple[str, int, int] | None:
    raw = edge.get("relationshipSite", edge.get("source_anchor"))
    if not isinstance(raw, dict):
        return None
    source_file = raw.get("file", raw.get("source_file"))
    start = raw.get("startByte", raw.get("start_byte"))
    end = raw.get("endByte", raw.get("end_byte"))
    if (
        not isinstance(source_file, str)
        or not source_file
        or isinstance(start, bool)
        or not isinstance(start, int)
        or isinstance(end, bool)
        or not isinstance(end, int)
        or start < 0
        or end <= start
    ):
        return None
    return (source_file, start, end)


def audit_multiplicity(
    graph: Path,
    *,
    max_relationships: int = DEFAULT_MAX_RELATIONSHIPS,
) -> dict[str, Any]:
    """Audit occurrence-preserving parallel edges without loading the graph."""

    if max_relationships <= 0:
        raise ValueError("max_relationships must be positive")
    if not graph.is_file():
        raise ValueError(f"graph does not exist: {graph}")

    edge_ids: Counter[str] = Counter()
    semantic_pairs: Counter[tuple[str, str, str]] = Counter()
    pair_sites: Counter[tuple[tuple[str, str, str], tuple[str, int, int] | None]] = (
        Counter()
    )
    edges_by_relation: Counter[str] = Counter()
    serialized_bytes_by_relation: Counter[str] = Counter()
    missing_relationship_sites = 0
    relationships = 0
    serialized_record_bytes = 0
    for index, edge in enumerate(iter_top_level_array(graph, "links")):
        relationships += 1
        if relationships > max_relationships:
            raise ValueError(
                f"relationship count exceeds max_relationships={max_relationships}"
            )
        source = _text(edge.get("source"), f"links[{index}].source")
        target = _text(edge.get("target"), f"links[{index}].target")
        relation = _text(
            edge.get("kind", edge.get("relation")),
            f"links[{index}].relation",
        )
        edge_id = edge.get("id")
        if isinstance(edge_id, str) and edge_id:
            edge_ids[edge_id] += 1
        pair = (source, relation, target)
        site = _site(edge)
        missing_relationship_sites += site is None
        semantic_pairs[pair] += 1
        pair_sites[(pair, site)] += 1
        edges_by_relation[relation] += 1
        record_bytes = len(
            json.dumps(edge, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        )
        serialized_record_bytes += record_bytes
        serialized_bytes_by_relation[relation] += record_bytes

    parallel_pairs = {
        pair: count for pair, count in semantic_pairs.items() if count > 1
    }
    parallel_pairs_by_relation: Counter[str] = Counter()
    for (_, relation, _), _count in parallel_pairs.items():
        parallel_pairs_by_relation[relation] += 1
    duplicate_edge_ids = sum(count - 1 for count in edge_ids.values())
    duplicate_pair_sites = sum(count - 1 for count in pair_sites.values())
    missing_edge_ids = relationships - sum(edge_ids.values())
    repeated_occurrences = sum(count - 1 for count in parallel_pairs.values())
    relationship_array_bytes = (
        serialized_record_bytes + max(0, relationships - 1) + 2
    )
    occurrence_integrity = (
        duplicate_edge_ids == 0
        and duplicate_pair_sites == 0
        and missing_edge_ids == 0
        and missing_relationship_sites == 0
    )
    return {
        "schema": MULTIPLICITY_SCHEMA,
        "passed": occurrence_integrity,
        "maxRelationships": max_relationships,
        "relationships": relationships,
        "semanticPairs": len(semantic_pairs),
        "parallelPairs": len(parallel_pairs),
        "parallelOccurrences": sum(parallel_pairs.values()),
        "repeatedOccurrences": repeated_occurrences,
        "maxPairMultiplicity": max(semantic_pairs.values(), default=0),
        "uniqueEdgeIds": len(edge_ids),
        "missingEdgeIds": missing_edge_ids,
        "duplicateEdgeIds": duplicate_edge_ids,
        "missingRelationshipSites": missing_relationship_sites,
        "duplicatePairSites": duplicate_pair_sites,
        "relationshipSerializedBytes": relationship_array_bytes,
        "meanRelationshipBytes": (
            relationship_array_bytes / relationships if relationships else 0.0
        ),
        "edgesByRelation": dict(sorted(edges_by_relation.items())),
        "parallelPairsByRelation": dict(sorted(parallel_pairs_by_relation.items())),
        "serializedBytesByRelation": dict(
            sorted(serialized_bytes_by_relation.items())
        ),
    }
