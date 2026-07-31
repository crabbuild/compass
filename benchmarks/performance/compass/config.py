"""Strict TOML configuration loading."""

from __future__ import annotations

import hashlib
from pathlib import Path
import re
import tomllib
from typing import Any

from . import SUITE_SCHEMA
from .model import QueryOracle, RepositorySpec, Suite

_TOP_LEVEL_KEYS = {"schema", "repository"}
_REPOSITORY_KEYS = {"name", "url", "mutation_suffix", "query"}
_QUERY_KEYS = {"question", "required", "forbidden"}
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


def load_suite(path: Path) -> Suite:
    raw = path.read_bytes()
    document = tomllib.loads(raw.decode("utf-8"))
    _unknown(document, _TOP_LEVEL_KEYS, "suite")
    if document.get("schema") != SUITE_SCHEMA:
        raise ValueError(f"unsupported suite schema: {document.get('schema')!r}")
    records = document.get("repository")
    if not isinstance(records, list) or len(records) != 8:
        raise ValueError("suite must declare exactly eight repositories")

    repositories: list[RepositorySpec] = []
    names: set[str] = set()
    for index, record in enumerate(records):
        if not isinstance(record, dict):
            raise ValueError(f"repository[{index}] must be a table")
        _unknown(record, _REPOSITORY_KEYS, f"repository[{index}]")
        name = record.get("name")
        url = record.get("url")
        suffix = record.get("mutation_suffix")
        if not isinstance(name, str) or not name or name in names:
            raise ValueError(f"repository[{index}] has an invalid or duplicate name")
        if not isinstance(url, str) or _HTTPS_GIT.fullmatch(url) is None:
            raise ValueError(f"repository {name} must use an HTTPS GitHub .git URL")
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
            required = _strings(query.get("required"), "required")
            forbidden = _strings(query.get("forbidden", []), "forbidden", allow_empty=True)
            queries.append(QueryOracle(question.strip(), required, forbidden))
        names.add(name)
        repositories.append(RepositorySpec(name, url, suffix, tuple(queries)))

    return Suite(SUITE_SCHEMA, tuple(repositories), hashlib.sha256(raw).hexdigest())
