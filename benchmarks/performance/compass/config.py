"""Strict TOML configuration loading."""

from __future__ import annotations

import hashlib
from pathlib import Path
import re
import tomllib
from typing import Any

from . import SUITE_SCHEMA
from .model import (
    QueryEdgeOracle,
    QueryNodeOracle,
    QueryOracle,
    QuerySourceAnchorOracle,
    RepositorySpec,
    Suite,
)

_TOP_LEVEL_KEYS = {"schema", "repository"}
_REPOSITORY_KEYS = {"name", "url", "commit", "mutation_suffix", "query"}
_QUERY_KEYS = {
    "question",
    "required",
    "forbidden",
    "expectedSeeds",
    "acceptableSeeds",
    "forbiddenSeeds",
    "relevantNodes",
    "expectedEdges",
    "expectedDirection",
    "expectedAmbiguous",
    "allowNoMatch",
    "judgmentSource",
    "judgmentReason",
}
_SEED_KEYS = {"qualifiedName", "source"}
_SOURCE_KEYS = {"file", "startLine"}
_EDGE_KEYS = {"source", "relation", "target", "direction", "site"}
_HTTPS_GIT = re.compile(r"^https://github\.com/[^/]+/[^/]+\.git$")


def _unknown(record: dict[str, Any], allowed: set[str], context: str) -> None:
    extra = sorted(set(record) - allowed)
    if extra:
        raise ValueError(f"{context} contains unknown fields: {', '.join(extra)}")


def _strings(value: Any, context: str, *, allow_empty: bool = False) -> tuple[str, ...]:
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise ValueError(f"{context} must be an array of strings")
    cleaned = tuple(item.strip() for item in value)
    if not allow_empty and (not cleaned or any(not item for item in cleaned)):
        raise ValueError(f"{context} must contain nonempty strings")
    if any(not item for item in cleaned):
        raise ValueError(f"{context} cannot contain an empty string")
    return cleaned


def _nodes(
    value: Any, context: str, *, require_source: bool = True
) -> tuple[QueryNodeOracle, ...]:
    if not isinstance(value, list):
        raise ValueError(f"{context} must be an array of tables")
    nodes: list[QueryNodeOracle] = []
    for index, record in enumerate(value):
        if not isinstance(record, dict):
            raise ValueError(f"{context}[{index}] must be a table")
        _unknown(record, _SEED_KEYS, f"{context}[{index}]")
        qualified_name = record.get("qualifiedName")
        source = record.get("source")
        if not isinstance(qualified_name, str) or not qualified_name.strip():
            raise ValueError(f"{context}[{index}].qualifiedName must be nonempty")
        if source is None and not require_source:
            nodes.append(QueryNodeOracle(qualified_name.strip(), None))
            continue
        if not isinstance(source, dict):
            raise ValueError(f"{context}[{index}].source must be an anchor table")
        _unknown(source, _SOURCE_KEYS, f"{context}[{index}].source")
        file = source.get("file")
        start_line = source.get("startLine")
        if not isinstance(file, str) or not file.strip():
            raise ValueError(f"{context}[{index}].source.file must be nonempty")
        if start_line is not None and (not isinstance(start_line, int) or start_line < 1):
            raise ValueError(f"{context}[{index}].source.startLine must be positive")
        nodes.append(
            QueryNodeOracle(
                qualified_name.strip(), QuerySourceAnchorOracle(file.strip(), start_line)
            )
        )
    return tuple(nodes)


def _edges(value: Any, context: str) -> tuple[QueryEdgeOracle, ...]:
    if not isinstance(value, list):
        raise ValueError(f"{context} must be an array of tables")
    edges: list[QueryEdgeOracle] = []
    for index, record in enumerate(value):
        if not isinstance(record, dict):
            raise ValueError(f"{context}[{index}] must be a table")
        _unknown(record, _EDGE_KEYS, f"{context}[{index}]")
        values = [record.get(key) for key in ("source", "relation", "target", "direction")]
        if not all(isinstance(item, str) and item.strip() for item in values):
            raise ValueError(f"{context}[{index}] requires source/relation/target/direction")
        direction = str(values[3]).strip()
        if direction not in {"incoming", "outgoing"}:
            raise ValueError(f"{context}[{index}].direction must be incoming or outgoing")
        site = record.get("site")
        if site is not None and (not isinstance(site, str) or not site.strip()):
            raise ValueError(f"{context}[{index}].site must be nonempty when present")
        edges.append(
            QueryEdgeOracle(
                str(values[0]).strip(),
                str(values[1]).strip(),
                str(values[2]).strip(),
                direction,
                site.strip() if isinstance(site, str) else None,
            )
        )
    return tuple(edges)


def load_suite(path: Path) -> Suite:
    raw = path.read_bytes()
    document = tomllib.loads(raw.decode("utf-8"))
    _unknown(document, _TOP_LEVEL_KEYS, "suite")
    if document.get("schema") != SUITE_SCHEMA:
        raise ValueError(f"unsupported suite schema: {document.get('schema')!r}")
    records = document.get("repository")
    if not isinstance(records, list) or not 1 <= len(records) <= 8:
        raise ValueError("suite must declare between one and eight repositories")

    repositories: list[RepositorySpec] = []
    names: set[str] = set()
    for index, record in enumerate(records):
        if not isinstance(record, dict):
            raise ValueError(f"repository[{index}] must be a table")
        _unknown(record, _REPOSITORY_KEYS, f"repository[{index}]")
        name = record.get("name")
        url = record.get("url")
        suffix = record.get("mutation_suffix")
        commit = record.get("commit")
        if not isinstance(name, str) or not name or name in names:
            raise ValueError(f"repository[{index}] has an invalid or duplicate name")
        if not isinstance(url, str) or _HTTPS_GIT.fullmatch(url) is None:
            raise ValueError(f"repository {name} must use an HTTPS GitHub .git URL")
        if not isinstance(commit, str) or re.fullmatch(r"[0-9a-f]{40}", commit) is None:
            raise ValueError(f"repository {name} must pin a lowercase 40-character commit")
        if not isinstance(suffix, str) or not suffix.startswith(".") or len(suffix) < 2:
            raise ValueError(f"repository {name} has an invalid mutation suffix")
        query_records = record.get("query")
        if not isinstance(query_records, list) or len(query_records) < 2:
            raise ValueError(f"repository {name} must declare at least two query oracles")
        queries: list[QueryOracle] = []
        for query_index, query in enumerate(query_records):
            if not isinstance(query, dict):
                raise ValueError(f"repository {name} query[{query_index}] must be a table")
            _unknown(query, _QUERY_KEYS, f"repository {name} query[{query_index}]")
            question = query.get("question")
            if not isinstance(question, str) or not question.strip():
                raise ValueError(f"repository {name} query[{query_index}] needs a question")
            required = _strings(query.get("required", []), "required", allow_empty=True)
            forbidden = _strings(query.get("forbidden", []), "forbidden", allow_empty=True)
            expected_seeds = _nodes(query.get("expectedSeeds", []), "expectedSeeds")
            acceptable_seeds = _nodes(query.get("acceptableSeeds", []), "acceptableSeeds")
            forbidden_seeds = _nodes(
                query.get("forbiddenSeeds", []), "forbiddenSeeds", require_source=False
            )
            relevant_nodes = _nodes(query.get("relevantNodes", []), "relevantNodes")
            expected_edges = _edges(query.get("expectedEdges", []), "expectedEdges")
            expected_direction = query.get("expectedDirection", "both")
            if expected_direction not in {"incoming", "outgoing", "both"}:
                raise ValueError(
                    f"repository {name} query[{query_index}] has invalid expectedDirection"
                )
            expected_ambiguous = query.get("expectedAmbiguous", False)
            allow_no_match = query.get("allowNoMatch", False)
            if not isinstance(expected_ambiguous, bool) or not isinstance(allow_no_match, bool):
                raise ValueError("expectedAmbiguous and allowNoMatch must be booleans")
            if not expected_seeds and not acceptable_seeds and not allow_no_match:
                raise ValueError(
                    f"repository {name} query[{query_index}] needs expectedSeeds, "
                    "acceptableSeeds, or allowNoMatch"
                )
            if allow_no_match and (expected_seeds or acceptable_seeds):
                raise ValueError(
                    f"repository {name} query[{query_index}] cannot combine "
                    "allowNoMatch with expectedSeeds or acceptableSeeds"
                )
            judgment_source = query.get("judgmentSource")
            judgment_reason = query.get("judgmentReason")
            if judgment_source is not None and judgment_source not in {
                "manual_source_review",
                "compiler_oracle",
            }:
                raise ValueError(
                    f"repository {name} query[{query_index}] has invalid judgmentSource"
                )
            if judgment_reason is not None and (
                not isinstance(judgment_reason, str) or not judgment_reason.strip()
            ):
                raise ValueError(
                    f"repository {name} query[{query_index}] has invalid judgmentReason"
                )
            if (judgment_source is None) != (judgment_reason is None):
                raise ValueError(
                    f"repository {name} query[{query_index}] must declare judgmentSource "
                    "and judgmentReason together"
                )
            if allow_no_match and judgment_source is None:
                raise ValueError(
                    f"repository {name} query[{query_index}] no-match oracle requires "
                    "an independent judgmentSource and judgmentReason"
                )
            queries.append(
                QueryOracle(
                    question=question.strip(),
                    required=required,
                    forbidden=forbidden,
                    expected_seeds=expected_seeds,
                    acceptable_seeds=acceptable_seeds,
                    forbidden_seeds=forbidden_seeds,
                    relevant_nodes=relevant_nodes,
                    expected_edges=expected_edges,
                    expected_direction=expected_direction,
                    expected_ambiguous=expected_ambiguous,
                    allow_no_match=allow_no_match,
                    judgment_source=judgment_source,
                    judgment_reason=(
                        judgment_reason.strip() if isinstance(judgment_reason, str) else None
                    ),
                )
            )
        names.add(name)
        repositories.append(RepositorySpec(name, url, suffix, tuple(queries), commit))

    return Suite(SUITE_SCHEMA, tuple(repositories), hashlib.sha256(raw).hexdigest())
