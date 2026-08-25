#!/usr/bin/env python3
"""Build a source-grounded Python framework ``compass.quality-audit/2`` manifest.

This qualification-only tool joins independently parsed stdlib-AST constructs
to an already-published Compass graph.  A graph edge enters the accepted pool
only when relation, exact source range, framework pack, and target identity all
agree.  Zero or multiple candidates remain explicit missing or ambiguous
source-oracle records.  Corpus code is never imported or executed.
"""

from __future__ import annotations

import argparse
import ast
from collections import Counter, defaultdict
from dataclasses import replace
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tomllib
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from benchmarks.performance.compass.audit import _edge_anchor, _target_cluster
from benchmarks.performance.compass.jsonstream import iter_top_level_array
from benchmarks.performance.compass.occurrences import (
    SourceConstruct,
    _qualification_glob_matches,
    independent_source_inventory,
    independent_source_provider_identity,
    source_construct_inventory_sha256,
)


PRODUCER = "python"
FRAMEWORK_PRODUCER = "python-frameworks"
EXTRACTOR_PACK = {
    "compass.frameworks.django": "django-python",
    "compass.frameworks.django-rest-framework": "django-rest-framework-python",
    "compass.frameworks.fastapi": "fastapi-python",
    "compass.frameworks.flask": "flask-python",
    "compass.frameworks.pydantic": "pydantic-python",
    "compass.frameworks.sqlalchemy": "sqlalchemy-python",
    "compass.frameworks.celery": "celery-python",
    "compass.frameworks.starlette": "starlette-python",
}
PACK_RELATION_CAPABILITY = {
    "django-python": {
        "routes_to": "http_routes",
        "subscribes": "messaging",
        "depends_on": "persistence",
    },
    "django-rest-framework-python": {
        "routes_to": "http_routes",
        "depends_on": "dependency_injection",
    },
    "fastapi-python": {
        "routes_to": "http_routes",
        "depends_on": "dependency_injection",
    },
    "starlette-python": {"routes_to": "http_routes"},
    "pydantic-python": {"depends_on": "data_modeling"},
    "flask-python": {"routes_to": "http_routes"},
    "sqlalchemy-python": {
        "depends_on": "data_modeling",
        "maps_to": "persistence",
    },
    "celery-python": {
        "produces": "messaging",
        "consumes": "messaging",
        "schedules": "scheduling",
        "triggers": "messaging",
    },
}
UNIVERSAL_RELATION_CAPABILITY = {
    "calls": "calls",
    "imports": "imports",
    "instantiates": "construction",
}
SOURCE_RELATION_ALIASES = {
    "calls": frozenset(("calls", "instantiates")),
}


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _commit(root: Path) -> str:
    completed = subprocess.run(
        ("git", "-C", str(root), "rev-parse", "HEAD"),
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    value = completed.stdout.strip()
    if completed.returncode or len(value) != 40 or any(
        character not in "0123456789abcdef" for character in value
    ):
        raise RuntimeError(f"could not identify pinned corpus revision at {root}")
    return value


def _graph_nodes(path: Path) -> dict[str, dict[str, Any]]:
    nodes: dict[str, dict[str, Any]] = {}
    for raw in iter_top_level_array(path, "nodes"):
        identifier = raw.get("id")
        if not isinstance(identifier, str) or not identifier:
            continue
        source = raw.get("source")
        nodes[identifier] = {
            "id": identifier,
            "language": raw.get("language") if isinstance(raw.get("language"), str) else "",
            "qualifiedName": raw.get("qualifiedName")
            if isinstance(raw.get("qualifiedName"), str)
            else "",
            "kind": raw.get("kind") if isinstance(raw.get("kind"), str) else "",
            "sourceFile": source.get("file") if isinstance(source, dict) else "",
        }
    return nodes


def _edge_pack(raw: dict[str, Any]) -> str | None:
    evidence = raw.get("evidence")
    if not isinstance(evidence, list):
        return None
    packs = {
        pack
        for item in evidence
        if isinstance(item, dict)
        and isinstance((extractor := item.get("extractor")), str)
        and (pack := _extractor_pack(extractor)) is not None
    }
    return next(iter(packs)) if len(packs) == 1 else None


def _extractor_pack(extractor: str) -> str | None:
    pack = EXTRACTOR_PACK.get(extractor)
    if pack is not None:
        return pack
    domain = extractor.removesuffix(".domain")
    return EXTRACTOR_PACK.get(domain) if domain != extractor else None


def _graph_edges(
    path: Path,
    nodes: dict[str, dict[str, Any]],
) -> list[dict[str, Any]]:
    edges: list[dict[str, Any]] = []
    for raw in iter_top_level_array(path, "links"):
        source = raw.get("source")
        target = raw.get("target")
        relation = raw.get("kind", raw.get("relation"))
        if not isinstance(source, str) or not isinstance(target, str) or not isinstance(relation, str):
            continue
        source_node = nodes.get(source)
        target_node = nodes.get(target)
        if source_node is None or target_node is None:
            continue
        pack = _edge_pack(raw)
        if pack is None and source_node["language"] != "python":
            continue
        if target_node["language"] not in {"", "python", "external"}:
            continue
        anchor = _edge_anchor(raw)
        if anchor is None:
            continue
        confidence = "published"
        evidence = raw.get("evidence")
        if isinstance(evidence, list):
            values = sorted(
                {
                    item.get("confidence")
                    for item in evidence
                    if isinstance(item, dict) and isinstance(item.get("confidence"), str)
                }
            )
            if len(values) == 1:
                confidence = values[0]
        edges.append(
            {
                "id": raw.get("id") if isinstance(raw.get("id"), str) else "",
                "source": source,
                "target": target,
                "relation": relation.casefold(),
                "anchor": anchor,
                "frameworkPack": pack,
                "confidence": confidence,
                "occurrenceRule": raw.get("occurrenceRule")
                if isinstance(raw.get("occurrenceRule"), str)
                else "",
                "evidence": raw.get("evidence")
                if isinstance(raw.get("evidence"), list)
                else [],
                "sourceNode": source_node,
                "targetNode": target_node,
            }
        )
    return sorted(
        edges,
        key=lambda item: (
            item["anchor"],
            item["relation"],
            item["source"],
            item["target"],
        ),
    )


def _target_matches(construct: SourceConstruct, edge: dict[str, Any]) -> bool:
    qualified = edge["targetNode"]["qualifiedName"].removesuffix("()")
    spelling = construct.target_spelling.removesuffix("()")
    if construct.relation == "imports":
        if qualified == spelling or qualified.endswith("." + spelling):
            return True
        qualifier = _absolute_import_qualifier(construct)
        if qualifier is None:
            return False
        spelling_terminal = spelling.rsplit(".", 1)[-1]
        qualified_module, qualified_terminal = _qualified_terminal(qualified)
        return (
            qualified_terminal == spelling_terminal
            and (
                qualified_module == qualifier
                or qualified_module.startswith(qualifier + ".")
            )
        )
    _, terminal = _qualified_terminal(spelling)
    terminal = terminal.rsplit(".", 1)[-1]
    _, qualified_terminal = _qualified_terminal(qualified)
    qualified_terminal = qualified_terminal.rsplit(".", 1)[-1]
    return qualified_terminal == terminal


def _exact_django_route_representation(edge: dict[str, Any]) -> dict[str, str] | None:
    edge_id = edge.get("id")
    rule = edge.get("occurrenceRule")
    if (
        edge.get("frameworkPack") != "django-python"
        or edge.get("relation") != "routes_to"
        or edge.get("confidence") != "exact"
        or not isinstance(edge_id, str)
        or not edge_id
        or not isinstance(rule, str)
        or not rule.startswith("framework-route-stage:handler:")
    ):
        return None
    exact_evidence = {
        (item.get("extractor"), item.get("rule"))
        for item in edge.get("evidence", ())
        if isinstance(item, dict)
        and item.get("origin") == "ast"
        and item.get("confidence") == "exact"
    }
    expected = ("compass.frameworks.django", rule)
    if exact_evidence != {expected}:
        return None
    return {
        "source": edge["source"],
        "target": edge["target"],
        "relation": edge["relation"],
        "edgeId": edge_id,
        "extractor": expected[0],
        "rule": rule,
    }


def _django_route_representations(
    constructs: tuple[SourceConstruct, ...],
    by_anchor: dict[
        tuple[str, str, int, int, str | None],
        list[dict[str, Any]],
    ],
) -> dict[tuple[str, str], tuple[dict[str, str], ...]]:
    candidates: dict[tuple[str, str], dict[str, dict[str, str]]] = defaultdict(dict)
    for construct in constructs:
        child_module = construct.qualifier
        if (
            construct.framework_pack != "django-python"
            or construct.relation != "routes_to"
            or child_module is None
        ):
            continue
        edges = [
            edge
            for edge in by_anchor.get(
                (
                    construct.relation,
                    construct.source_file,
                    construct.start_byte,
                    construct.end_byte,
                    construct.framework_pack,
                ),
                (),
            )
            if _target_matches(construct, edge)
        ]
        for edge in edges:
            representation = _exact_django_route_representation(edge)
            if representation is None:
                continue
            candidates[(child_module, construct.target_spelling)][
                representation["edgeId"]
            ] = representation
    return {
        key: tuple(by_id[edge_id] for edge_id in sorted(by_id))
        for key, by_id in candidates.items()
    }


def _django_route_representation_candidates(
    construct: SourceConstruct,
    has_exact_fact: bool,
    representations: dict[tuple[str, str], tuple[dict[str, str], ...]],
) -> tuple[dict[str, str], ...]:
    if (
        has_exact_fact
        or construct.framework_pack != "django-python"
        or construct.relation != "routes_to"
        or construct.qualifier is not None
    ):
        return ()
    return representations.get(
        (construct.owner_qualified_name, construct.target_spelling),
        (),
    )


def _absolute_import_qualifier(construct: SourceConstruct) -> str | None:
    qualifier = construct.qualifier
    if qualifier is None:
        return None
    if not qualifier.startswith("."):
        return qualifier
    level = len(qualifier) - len(qualifier.lstrip("."))
    suffix = qualifier[level:]
    relative = Path(construct.source_file).with_suffix("")
    module_parts = list(relative.parts)
    if module_parts and module_parts[-1] == "__init__":
        module_parts.pop()
        package = module_parts
    else:
        package = module_parts[:-1]
    parents = level - 1
    if parents > len(package):
        return None
    base = package[: len(package) - parents] if parents else package
    parts = [*base, *(part for part in suffix.split(".") if part)]
    return ".".join(parts) or None


def _qualified_terminal(value: str) -> tuple[str, str]:
    """Split Compass dotted or declaration-member qualified identities."""

    owner, separator, terminal = value.rpartition("::")
    if separator:
        return owner, terminal
    owner, separator, terminal = value.rpartition(".")
    return (owner, terminal) if separator else ("", value)


def _snippet(root: Path, construct: SourceConstruct) -> str | None:
    path = (root / construct.source_file).resolve()
    try:
        path.relative_to(root.resolve())
        contents = path.read_bytes()
    except (OSError, ValueError):
        return None
    if not 0 <= construct.start_byte < construct.end_byte <= len(contents):
        return None
    return hashlib.sha256(
        contents[construct.start_byte : construct.end_byte].replace(b"\r\n", b"\n")
    ).hexdigest()


def _record_id(*parts: object) -> str:
    encoded = json.dumps(parts, separators=(",", ":"), ensure_ascii=True).encode()
    return "python-audit-" + hashlib.sha256(encoded).hexdigest()[:24]


def _record(
    *,
    corpus: str,
    producer: str,
    construct: SourceConstruct,
    relation: str,
    capability: str,
    pool: str,
    source: str,
    target: str,
    target_language: str,
    target_cluster: str,
    judgment: str,
    reason: str,
    confidence: str,
    snippet_sha256: str,
    representation: dict[str, str] | None = None,
) -> dict[str, Any]:
    value: dict[str, Any] = {
        "id": _record_id(
            pool,
            corpus,
            producer,
            construct.framework_pack,
            relation,
            construct.source_file,
            construct.start_byte,
            construct.end_byte,
            source,
            target,
        ),
        "corpus": corpus,
        "pool": pool,
        "producer": producer,
        "capability": capability,
        "language": "python",
        "relation": relation,
        "confidence": confidence,
        "targetCluster": target_cluster,
        "source": {"nodeId": source, "language": "python"},
        "target": {"nodeId": target, "language": target_language or "python"},
        "occurrence": {
            "file": construct.source_file,
            "startByte": construct.start_byte,
            "endByte": construct.end_byte,
            "snippetSha256": snippet_sha256,
        },
        "judgment": judgment,
        "reason": reason,
    }
    if construct.framework_pack is not None:
        value["frameworkPack"] = construct.framework_pack
    if representation is not None:
        value["representation"] = representation
    return value


def _declared_names(
    root: Path,
    include_globs: tuple[str, ...],
) -> Counter[str]:
    names: Counter[str] = Counter()
    for path in sorted(root.rglob("*.py")):
        relative = path.relative_to(root).as_posix()
        if include_globs and not any(
            _qualification_glob_matches(relative, pattern) for pattern in include_globs
        ):
            continue
        try:
            tree = ast.parse(path.read_text(encoding="utf-8-sig"), filename=relative)
        except (OSError, SyntaxError, UnicodeError, ValueError):
            continue
        for node in ast.walk(tree):
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names[node.name] += 1
    return names


def _local_declared_names(
    root: Path,
    include_globs: tuple[str, ...],
) -> Counter[tuple[str, str]]:
    names: Counter[tuple[str, str]] = Counter()
    for path in sorted(root.rglob("*.py")):
        relative = path.relative_to(root).as_posix()
        if include_globs and not any(
            _qualification_glob_matches(relative, pattern) for pattern in include_globs
        ):
            continue
        try:
            tree = ast.parse(path.read_text(encoding="utf-8-sig"), filename=relative)
        except (OSError, SyntaxError, UnicodeError, ValueError):
            continue
        for node in ast.walk(tree):
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names[(relative, node.name)] += 1
    return names


def _source_target_is_exact(
    construct: SourceConstruct,
    declared_names: Counter[str],
    local_declared_names: Counter[tuple[str, str]],
) -> bool:
    terminal = construct.target_spelling.rsplit(".", 1)[-1]
    if construct.relation == "calls":
        return (
            construct.qualifier is None
            and local_declared_names[(construct.source_file, terminal)] == 1
        )
    return declared_names[terminal] == 1


def _parse_corpus(value: str) -> tuple[str, Path, Path]:
    name, separator, remainder = value.partition("=")
    root, separator, graph = remainder.partition("=")
    if not name or not separator or not root or not graph:
        raise ValueError("--corpus must be NAME=ROOT=GRAPH")
    return name, Path(root).resolve(), Path(graph).resolve()


def _load_repositories(path: Path) -> dict[str, dict[str, Any]]:
    raw = tomllib.loads(path.read_text(encoding="utf-8"))
    if raw.get("schema") != "compass.python-framework-qualification/1":
        raise RuntimeError("unexpected Python framework repository manifest schema")
    repositories = raw.get("repository")
    if not isinstance(repositories, list):
        raise RuntimeError("Python framework repository manifest is empty")
    return {item["name"]: item for item in repositories if isinstance(item, dict)}


def _cap_cluster_sample(records: list[dict[str, Any]], limit: int) -> list[dict[str, Any]]:
    selected: list[dict[str, Any]] = []
    by_cluster: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in sorted(records, key=lambda item: item["id"]):
        by_cluster[record["targetCluster"]].append(record)
    target = min(len(records), limit)
    per_cluster = max(1, target // 10)
    while len(selected) < target:
        progressed = False
        for cluster in sorted(by_cluster):
            values = by_cluster[cluster]
            if values and sum(item["targetCluster"] == cluster for item in selected) < per_cluster:
                selected.append(values.pop(0))
                progressed = True
                if len(selected) == target:
                    break
        if not progressed:
            break
    return selected


def _sample(records: list[dict[str, Any]], limit: int) -> list[dict[str, Any]]:
    by_identity: dict[tuple[str, str, str], list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        by_identity[
            (
                record["relation"],
                record.get("frameworkPack", ""),
                record["capability"],
            )
        ].append(record)
    if not by_identity:
        return []
    per_identity = max(1, limit // len(by_identity))
    selected: list[dict[str, Any]] = []
    for identity in sorted(by_identity):
        selected.extend(_cap_cluster_sample(by_identity[identity], per_identity))
    return selected[:limit]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qualification-manifest", type=Path, required=True)
    parser.add_argument("--corpus", action="append", required=True, metavar="NAME=ROOT=GRAPH")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-accepted", type=int, default=100_000)
    parser.add_argument("--max-source", type=int, default=100_000)
    args = parser.parse_args(argv)
    if args.max_accepted < 1 or args.max_source < 1:
        parser.error("sampling limits must be positive")

    repositories = _load_repositories(args.qualification_manifest)
    parsed = sorted((_parse_corpus(value) for value in args.corpus), key=lambda item: item[0])
    if len({item[0] for item in parsed}) != len(parsed):
        parser.error("duplicate corpus name")
    staging = args.output.parent / (args.output.stem + ".inputs")
    if staging.exists():
        raise RuntimeError(f"refusing to overwrite existing audit inputs: {staging}")
    (staging / "sources").mkdir(parents=True)
    (staging / "graphs").mkdir()

    corpora: list[dict[str, Any]] = []
    source_oracles: list[dict[str, Any]] = []
    accepted: list[dict[str, Any]] = []
    source_records: list[dict[str, Any]] = []
    coverage: list[dict[str, Any]] = []
    for name, root, graph in parsed:
        repository = repositories.get(name)
        if repository is None:
            raise RuntimeError(f"unknown pinned corpus {name!r}")
        if not root.is_dir() or not graph.is_file():
            raise RuntimeError(f"missing corpus root or graph for {name}")
        commit = _commit(root)
        if commit != repository.get("commit"):
            raise RuntimeError(f"{name} checkout is not at its pinned commit")
        source_link = staging / "sources" / name
        graph_link = staging / "graphs" / f"{name}.json"
        source_link.symlink_to(root, target_is_directory=True)
        graph_link.symlink_to(graph)
        include_globs = tuple(repository.get("source_globs", ()))
        inventories = (
            (PRODUCER, independent_source_inventory(root, PRODUCER, include_globs=include_globs)),
            (
                FRAMEWORK_PRODUCER,
                independent_source_inventory(
                    root,
                    FRAMEWORK_PRODUCER,
                    include_globs=include_globs,
                ),
            ),
        )
        nodes = _graph_nodes(graph)
        edges = _graph_edges(graph, nodes)
        by_anchor: dict[tuple[str, str, int, int, str | None], list[dict[str, Any]]] = defaultdict(list)
        for edge in edges:
            file, start, end = edge["anchor"]
            by_anchor[(edge["relation"], file, start, end, edge["frameworkPack"])].append(edge)
        django_route_representations = _django_route_representations(
            inventories[1][1].constructs,
            by_anchor,
        )
        declared_names = _declared_names(root, include_globs)
        local_declared_names = _local_declared_names(root, include_globs)
        accepted_part: list[dict[str, Any]] = []
        source_part: list[dict[str, Any]] = []
        for producer, inventory in inventories:
            source_oracles.append(
                {
                    "corpus": name,
                    "producer": producer,
                    "provider": independent_source_provider_identity(producer),
                    "scannedFiles": inventory.scanned_files,
                    "parsedFiles": inventory.parsed_files,
                    "rejectedFiles": list(inventory.rejected_files),
                    "inventorySha256": source_construct_inventory_sha256(producer, inventory),
                }
            )
            for construct in inventory.constructs:
                aliases = SOURCE_RELATION_ALIASES.get(
                    construct.relation,
                    frozenset((construct.relation,)),
                )
                candidates = [
                    edge
                    for relation in aliases
                    for edge in by_anchor.get(
                        (
                            relation,
                            construct.source_file,
                            construct.start_byte,
                            construct.end_byte,
                            construct.framework_pack,
                        ),
                        (),
                    )
                    if _target_matches(construct, edge)
                ]
                facts = {
                    (edge["source"], edge["target"], edge["relation"]): edge
                    for edge in candidates
                }
                snippet = _snippet(root, construct)
                if snippet is None:
                    continue
                if len(facts) == 1:
                    edge = next(iter(facts.values()))
                    relation = edge["relation"]
                    capability = (
                        PACK_RELATION_CAPABILITY[construct.framework_pack][relation]
                        if construct.framework_pack is not None
                        else UNIVERSAL_RELATION_CAPABILITY[relation]
                    )
                    matched = replace(
                        construct,
                        relation=relation,
                        capability=capability,
                    )
                    cluster = _target_cluster(
                        edge["targetNode"]["qualifiedName"] or edge["target"],
                        edge["target"],
                    )
                    common = {
                        "corpus": name,
                        "producer": producer,
                        "construct": matched,
                        "relation": relation,
                        "capability": capability,
                        "source": edge["source"],
                        "target": edge["target"],
                        "target_language": edge["targetNode"]["language"] or "python",
                        "target_cluster": cluster,
                        "snippet_sha256": snippet,
                    }
                    accepted_part.append(
                        _record(
                            **common,
                            pool="accepted",
                            judgment="correct",
                            reason="independent stdlib AST relation, exact source range, framework pack, and target identity agree",
                            confidence=edge["confidence"],
                        )
                    )
                    source_part.append(
                        _record(
                            **common,
                            pool="source_oracle",
                            judgment="correct",
                            reason="independent stdlib AST construct has one exact Compass graph fact",
                            confidence="source_ast",
                        )
                    )
                    continue

                local = _source_target_is_exact(
                    construct,
                    declared_names,
                    local_declared_names,
                )
                exact_static_domain = (
                    construct.framework_pack is not None
                    and construct.relation
                    in {"consumes", "maps_to", "produces", "schedules", "subscribes"}
                )
                representation_candidates = _django_route_representation_candidates(
                    construct,
                    bool(facts),
                    django_route_representations,
                )
                representation = (
                    representation_candidates[0]
                    if len(representation_candidates) == 1
                    else None
                )
                if representation is not None:
                    judgment = "represented_elsewhere"
                elif len(representation_candidates) > 1:
                    judgment = "ambiguous"
                else:
                    judgment = (
                        "missing"
                        if not facts and (local or exact_static_domain)
                        else "ambiguous"
                    )
                source_id = "oracle-source-" + hashlib.sha256(
                    f"{name}:{construct.source_file}:{construct.owner_qualified_name}".encode()
                ).hexdigest()[:16]
                target_id = "oracle-target-" + hashlib.sha256(
                    f"{name}:{construct.target_spelling}".encode()
                ).hexdigest()[:16]
                capability = construct.capability
                source_part.append(
                    _record(
                        corpus=name,
                        producer=producer,
                        construct=construct,
                        relation=construct.relation,
                        capability=capability,
                        pool="source_oracle",
                        source=source_id,
                        target=target_id,
                        target_language="python",
                        target_cluster=_target_cluster(construct.target_spelling, target_id),
                        judgment=judgment,
                        reason=(
                            "exact Django child route is represented by one source-proven parent include graph fact"
                            if judgment == "represented_elsewhere"
                            else (
                                "independent local stdlib AST construct has no exact Compass graph fact"
                                if judgment == "missing"
                                else "independent stdlib AST construct has zero or multiple safe target identities"
                            )
                        ),
                        confidence="source_ast",
                        snippet_sha256=snippet,
                        representation=representation,
                    )
                )
        accepted.extend(_sample(accepted_part, args.max_accepted))
        source_records.extend(_sample(source_part, args.max_source))
        corpora.append(
            {
                "name": name,
                "commit": commit,
                "path": f"sources/{name}",
                "graph": f"graphs/{name}.json",
                "graphSha256": _sha256(graph),
                "sourceGlobs": list(include_globs),
                "excludeGlobs": [],
            }
        )
        coverage.append(
            {
                "corpus": name,
                "graphEdges": len(edges),
                "accepted": len(accepted_part),
                "sourceOracle": len(source_part),
                "sourceProviders": [
                    {
                        "producer": producer,
                        "scannedFiles": inventory.scanned_files,
                        "parsedFiles": inventory.parsed_files,
                        "rejectedFiles": list(inventory.rejected_files),
                    }
                    for producer, inventory in inventories
                ],
            }
        )

    advertised = [
        {"producer": PRODUCER, "capability": capability}
        for capability in sorted(set(UNIVERSAL_RELATION_CAPABILITY.values()))
    ]
    advertised.extend(
        {
            "producer": FRAMEWORK_PRODUCER,
            "frameworkPack": pack,
            "capability": capability,
        }
        for pack, relations in sorted(PACK_RELATION_CAPABILITY.items())
        for capability in sorted(set(relations.values()))
    )
    advertised.sort(
        key=lambda item: (
            item["producer"],
            item.get("frameworkPack", ""),
            item["capability"],
        )
    )
    required_relations = sorted(
        set(UNIVERSAL_RELATION_CAPABILITY)
        | {
            relation
            for relations in PACK_RELATION_CAPABILITY.values()
            for relation in relations
        }
    )
    allowed_capabilities = {
        (item["producer"], item.get("frameworkPack"), item["capability"])
        for item in advertised
    }
    records = [
        record
        for record in accepted + source_records
        if (
            record["producer"],
            record.get("frameworkPack"),
            record["capability"],
        )
        in allowed_capabilities
        and record["relation"] in required_relations
    ]
    records = sorted({record["id"]: record for record in records}.values(), key=lambda item: item["id"])
    manifest = {
        "schema": "compass.quality-audit/2",
        "mode": "qualification",
        "corpora": sorted(corpora, key=lambda item: item["name"]),
        "sourceOracles": sorted(
            source_oracles,
            key=lambda item: (item["corpus"], item["producer"]),
        ),
        "advertisedCapabilities": advertised,
        "requiredRelations": required_relations,
        "records": records,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(manifest, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    report = {
        "schema": "compass.python-quality-audit-build/1",
        "manifest": str(args.output),
        "auditRoot": str(staging),
        "corpora": coverage,
        "accepted": sum(record["pool"] == "accepted" for record in records),
        "sourceOracle": sum(record["pool"] == "source_oracle" for record in records),
        "advertisedCapabilities": advertised,
        "requiredRelations": required_relations,
    }
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
