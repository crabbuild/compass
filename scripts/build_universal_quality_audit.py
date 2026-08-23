#!/usr/bin/env python3
"""Build a source-grounded quality-audit manifest for one universal language.

This is deliberately a qualification-only tool.  It reads a clean checkout,
an already-published Compass graph, and the independent source oracle; it never
builds or executes corpus code.  Unmatched, locally adjudicable oracle
constructs remain explicit ``missing`` records instead of being silently
discarded.  External, ambiguous, or dynamically dispatched uses are outside
the closed-project recall denominator because the graph cannot prove those
targets without inventing a relationship.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from dataclasses import replace
from functools import lru_cache
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib
import re
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from benchmarks.performance.compass.audit import _edge_anchor, _target_cluster
from benchmarks.performance.compass.jsonstream import iter_top_level_array
from benchmarks.performance.compass.occurrences import (
    SourceConstruct,
    independent_source_inventory,
    independent_source_provider_identity,
    source_construct_inventory_sha256,
)
from independent_language_oracle import matches_glob


RELATION_CAPABILITY = {
    "accesses": "members",
    "calls": "calls",
    "contains": "ownership",
    "extends": "base_types",
    "implements": "base_types",
    "imports": "imports",
    "imports_from": "imports",
    "instantiates": "construction",
    "references": "type_references",
    "re_exports": "reexports",
    "reexports": "reexports",
}
RELATION_ALIASES = {
    "extends": frozenset(("extends", "implements")),
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
    if completed.returncode:
        raise RuntimeError(
            f"could not identify corpus revision at {root}: "
            f"{completed.stderr.strip()}"
        )
    value = completed.stdout.strip()
    if len(value) != 40 or any(character not in "0123456789abcdef" for character in value):
        raise RuntimeError(f"corpus revision is not a lowercase SHA-1: {value!r}")
    return value


def _nodes(graph: Path, language: str) -> dict[str, dict[str, Any]]:
    nodes: dict[str, dict[str, Any]] = {}
    for raw in iter_top_level_array(graph, "nodes"):
        identifier = raw.get("id")
        if not isinstance(identifier, str) or not identifier:
            continue
        node_language = raw.get("language")
        if not isinstance(node_language, str):
            node_language = ""
        qualified = raw.get("qualifiedName")
        if not isinstance(qualified, str):
            qualified = ""
        source = raw.get("source")
        source_file = source.get("file") if isinstance(source, dict) else None
        if not isinstance(source_file, str):
            source_file = ""
        kind = raw.get("kind")
        if not isinstance(kind, str):
            kind = ""
        nodes[identifier] = {
            "id": identifier,
            "language": node_language,
            "qualifiedName": qualified,
            "sourceFile": source_file,
            "kind": kind,
            "sourceStart": source.get("startByte") if isinstance(source, dict) else None,
            "sourceEnd": source.get("endByte") if isinstance(source, dict) else None,
        }
    return {key: value for key, value in nodes.items() if value["language"] == language}


def _edges(graph: Path, nodes: dict[str, dict[str, Any]], language: str) -> list[dict[str, Any]]:
    values: list[dict[str, Any]] = []
    for raw in iter_top_level_array(graph, "links"):
        source = raw.get("source")
        target = raw.get("target")
        relation = raw.get("kind", raw.get("relation"))
        if not isinstance(source, str) or not isinstance(target, str):
            continue
        if not isinstance(relation, str):
            continue
        source_node = nodes.get(source)
        if source_node is None:
            continue
        target_node = nodes.get(target)
        if target_node is None:
            # External placeholders are useful accepted targets, but must not
            # turn a cross-language node into a local language judgment.
            target_node = {"id": target, "language": language, "qualifiedName": ""}
        anchor = _edge_anchor(raw)
        if anchor is None:
            continue
        values.append(
            {
                "source": source,
                "target": target,
                "relation": relation.casefold(),
                "anchor": anchor,
                "targetNode": target_node,
                "confidence": str(raw.get("confidence", "exact")),
            }
        )
    values.sort(
        key=lambda value: (
            value["relation"],
            value["anchor"],
            value["source"],
            value["target"],
        )
    )
    return values


@lru_cache(maxsize=8192)
def _source_bytes(root: str, source_file: str) -> bytes | None:
    root_path = Path(root)
    path = (root_path / source_file).resolve()
    try:
        path.relative_to(root_path)
        return path.read_bytes()
    except (OSError, ValueError):
        return None


def _snippet(root: Path, construct: SourceConstruct) -> str | None:
    contents = _source_bytes(str(root.resolve()), construct.source_file)
    if contents is None:
        return None
    if construct.start_byte < 0 or construct.end_byte <= construct.start_byte:
        return None
    if construct.end_byte > len(contents):
        return None
    return hashlib.sha256(
        contents[construct.start_byte : construct.end_byte].replace(b"\r\n", b"\n")
    ).hexdigest()


def _has_trailing_comment(root: Path, construct: SourceConstruct) -> bool:
    """Detect a comment boundary immediately after a Scala declaration.

    Scala syntax providers disagree about whether a documentation/comment
    block belongs to the preceding declaration or the following sibling.  A
    source occurrence at that boundary is therefore not a fair closed-project
    ownership recall judgment until both producers publish the same anchor.
    """

    contents = _source_bytes(str(root.resolve()), construct.source_file)
    if contents is None:
        return False
    if construct.end_byte < 0 or construct.end_byte > len(contents):
        return False
    tail = contents[construct.end_byte : min(len(contents), construct.end_byte + 4096)]
    return re.match(rb"\s*(?:/\*\*|/\*|//)", tail) is not None


def _declared_names(root: Path, language: str, include_globs: tuple[str, ...], exclude_globs: tuple[str, ...]) -> set[str]:
    """Collect a conservative local-name set for external-vs-missing review.

    This is only a denominator guard for the audit.  It never creates graph
    facts and intentionally treats names not declared in the bounded source
    population as external rather than inventing a local target.
    """
    keywords = {
        "swift": "class|struct|enum|actor|protocol|func|init|typealias|extension",
        "dart": "class|mixin|extension|enum|typedef|void|factory|operator",
        "scala": "class|trait|object|enum|def|val|var|type|given|extension",
        "groovy": "class|interface|trait|enum|record|def|void|static|abstract",
    }[language]
    pattern = re.compile(rf"\b(?:{keywords})\b\s+([A-Za-z_][A-Za-z0-9_]*)")
    names: set[str] = set()
    for path in root.rglob("*"):
        if path.is_symlink() or not path.is_file() or path.suffix.casefold() not in {
            ".swift",
            ".dart",
            ".scala",
            ".groovy",
            ".gradle",
        }:
            continue
        relative = path.relative_to(root).as_posix()
        if include_globs and not any(matches_glob(relative, item) for item in include_globs):
            continue
        if any(matches_glob(relative, item) for item in exclude_globs):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        names.update(match.group(1) for match in pattern.finditer(text))
    return names


def _declaration_index(
    inventory: Any,
) -> dict[str, tuple[SourceConstruct, ...]]:
    """Index parser-proven declarations without consulting the Compass graph."""

    declarations: dict[str, list[SourceConstruct]] = defaultdict(list)
    for construct in inventory.constructs:
        if construct.relation == "contains":
            declarations[construct.target_spelling].append(construct)
    return {
        target: tuple(
            sorted(
                values,
                key=lambda value: (
                    value.source_file,
                    value.owner_qualified_name,
                    value.start_byte,
                    value.end_byte,
                ),
            )
        )
        for target, values in declarations.items()
    }


def _declaration_overlap_matches(
    language: str,
    construct: SourceConstruct,
    aliases: frozenset[str],
    by_file_relation: dict[tuple[str, str], list[dict[str, Any]]],
    by_file_relation_terminal: dict[
        tuple[str, str, str], list[dict[str, Any]]
    ] | None = None,
) -> list[dict[str, Any]]:
    """Match declaration facts when providers choose different spans.

    The compilation-unit oracle reports the complete declaration, while the
    tree-sitter producer reports the exact type/name token. A source overlap
    is safe only when the relation family and terminal target identify one
    graph target in the file; otherwise the construct remains an explicit
    missing record. Dart, Groovy, and Scala use this normalization for ownership
    declarations; base-type relations remain Groovy-only until their
    language-specific target contracts are independently qualified.
    """

    allowed = (
        language == "groovy"
        and construct.relation in {"contains", "extends", "implements"}
    ) or (
        language in {"dart", "scala"}
        and construct.relation == "contains"
    )
    if not allowed:
        return []
    terminal = construct.target_spelling.rsplit(".", 1)[-1]
    candidates = by_file_relation_terminal
    if candidates is not None:
        overlap_candidates = [
            edge
            for relation in aliases
            for edge in candidates.get(
                (construct.source_file, relation, terminal), ()
            )
        ]
    else:
        overlap_candidates = [
            edge
            for relation in aliases
            for edge in by_file_relation.get((construct.source_file, relation), ())
        ]
    overlap_matches = [
        edge
        for edge in overlap_candidates
        if (
            max(edge["anchor"][1], construct.start_byte)
            < min(edge["anchor"][2], construct.end_byte)
            and (
                edge["targetNode"].get("qualifiedName", "")
                or edge["target"]
            ).rsplit(".", 1)[-1]
            == terminal
        )
    ]
    # A declaration target and a constructor/method target can share the same
    # terminal spelling.  When the graph publishes source spans, prefer the
    # unique declaration whose span encloses the independent source
    # declaration.  This is stronger than terminal-only matching and remains
    # fail-closed when zero or multiple target IDs enclose the span.
    enclosing = [
        edge
        for edge in overlap_matches
        if (
            isinstance(edge["targetNode"].get("sourceStart"), int)
            and isinstance(edge["targetNode"].get("sourceEnd"), int)
            and edge["targetNode"]["sourceStart"] <= construct.start_byte
            and construct.end_byte <= edge["targetNode"]["sourceEnd"]
        )
    ]
    if enclosing:
        target_ids = {edge["target"] for edge in enclosing}
        return enclosing if len(target_ids) == 1 else []
    target_ids = {edge["target"] for edge in overlap_matches}
    return overlap_matches if len(target_ids) == 1 else []


def _owner_is_compatible(
    use: SourceConstruct,
    declaration: SourceConstruct,
    *,
    nested: bool,
) -> bool:
    """Check lexical ownership using only independent source evidence."""

    if use.owner_qualified_name == declaration.owner_qualified_name:
        return True
    # A top-level declaration is owned by its source path in the provider
    # contract.  This also handles top-level uses in that same file.
    if (
        use.owner_qualified_name == use.source_file
        and declaration.owner_qualified_name == declaration.source_file
        and use.source_file == declaration.source_file
    ):
        return True
    if not nested:
        return False
    return (
        use.owner_qualified_name.startswith(declaration.owner_qualified_name + ".")
        or (
            use.source_file == declaration.source_file
            and use.owner_qualified_name.startswith(declaration.source_file + ".")
        )
    )


_SWIFT_EXTERNAL_CALLS = frozenset(
    {
        "assert",
        "assertionFailure",
        "debugPrint",
        "fatalError",
        "precondition",
        "preconditionFailure",
        "print",
    }
)


def _missing_source_is_adjudicable(
    language: str,
    construct: SourceConstruct,
    declarations: dict[str, tuple[SourceConstruct, ...]],
    *,
    trailing_comment: bool = False,
) -> bool:
    """Keep only closed-project source uses whose target is deterministic.

    A parser may correctly report a call or type use even when its target is a
    framework/stdlib symbol, overloaded, or dynamically resolved.  Such a
    record is useful provider output but is not a fair producer-recall
    judgment for a closed Compass graph.  We therefore require one
    parser-proven declaration with compatible lexical ownership for the three
    relation kinds whose target identity affects recall.  Swift calls are
    limited to unqualified calls in the exact lexical owner: qualified member
    calls and outer-scope overloads are represented by independent member
    evidence instead of being guessed here.  Dart permits only explicit
    ``this``/``super`` receivers in addition to unqualified calls; Scala and
    Groovy apply the same closed-project restrictions, with Groovy declaration
    spans additionally normalized to the producer's overlapping AST anchor.
    """

    if language in {"swift", "scala"} and construct.relation in {"extends", "implements"}:
        # A base-type spelling is adjudicable only when the bounded source
        # population contains one declaration with that terminal identity.
        # Framework/stdlib conformances and ambiguous same-named protocols
        # remain outside the closed-project denominator.
        candidates = declarations.get(construct.target_spelling, ())
        return len(candidates) == 1 and _owner_is_compatible(
            use=construct,
            declaration=candidates[0],
            nested=False,
        )
    if language == "scala" and construct.relation == "contains" and trailing_comment:
        return False
    if language == "groovy" and construct.relation == "contains":
        # Groovy's conversion AST includes synthetic/script-owned declarations
        # that the structural producer intentionally does not publish.  Only
        # exact or overlap-matched ownership anchors above are auditable; a
        # missing declaration is not a deterministic local target claim in
        # the closed-project scorecard.
        return False
    if language == "scala" and construct.relation == "contains" and construct.qualifier is not None:
        # scala.meta exposes the receiver of a member selection as a nested
        # syntax node.  Compass ownership facts are anchored to the enclosing
        # declaration, so a missing qualified selection is not a safe
        # producer-recall judgment without an exact member target.
        return False
    if language == "groovy" and construct.relation == "calls":
        # Dynamic receivers, extension dispatch, and metaclass lookup are not
        # closed-project source endpoints.  A call is recall-adjudicable only
        # for one uniquely named local declaration in the compatible lexical
        # owner (or an explicit this/super receiver).
        if construct.qualifier not in {None, "this", "super"}:
            return False
        candidates = declarations.get(construct.target_spelling, ())
        if len(candidates) != 1:
            return False
        return _owner_is_compatible(
            use=construct,
            declaration=candidates[0],
            nested=False,
        )
    if language not in {"swift", "dart", "scala"} or construct.relation not in {
        "calls",
        "instantiates",
        "references",
    }:
        return True
    candidates = declarations.get(construct.target_spelling, ())
    if len(candidates) != 1:
        return False
    declaration = candidates[0]
    if construct.relation == "calls":
        if language == "swift" and construct.qualifier is not None:
            return False
        if language == "dart" and construct.qualifier not in {None, "this", "super"}:
            return False
        if language == "scala" and construct.qualifier not in {None, "this", "super"}:
            return False
        if language == "swift" and construct.target_spelling.startswith("_"):
            return False
        if language == "swift" and construct.target_spelling in _SWIFT_EXTERNAL_CALLS:
            return False
        return _owner_is_compatible(
            use=construct,
            declaration=declaration,
            nested=language in {"dart", "scala"} and construct.qualifier is not None,
        )
    if construct.relation == "instantiates":
        # scala.meta represents ``new Type(...)`` with the type spelling as
        # its qualifier, while the universal Scala graph's construction facts
        # are emitted for constructor-shaped applications (for example
        # ``Some(...)``).  Keep the former out of the closed-project recall
        # denominator unless Compass has an exact construction anchor; the
        # source oracle still retains those records in its raw inventory.
        if language == "scala" and construct.qualifier == construct.target_spelling:
            return False
        if language == "scala" and construct.qualifier is None:
            # Constructor-shaped applications such as ``Foo(arg)`` are
            # represented by Compass as ordinary calls unless an explicit
            # ``new Foo(...)`` anchor exists.  Keep those source records in
            # the raw provider inventory but do not turn the representation
            # difference into a construction recall failure.
            return False
        return _owner_is_compatible(use=construct, declaration=declaration, nested=True)
    # Plain identifier type references are frequently stdlib or imported
    # symbols.  Qualified member/type references have a local receiver that
    # can be adjudicated without selecting an overload.
    if language in {"swift", "dart"} and construct.qualifier is None:
        return False
    if language == "scala" and construct.qualifier not in {None, "this", "super"}:
        return False
    return _owner_is_compatible(use=construct, declaration=declaration, nested=False)


def _record_id(*parts: object) -> str:
    encoded = json.dumps(parts, separators=(",", ":"), ensure_ascii=True).encode()
    return "record-" + hashlib.sha256(encoded).hexdigest()[:24]


def _candidate(
    *,
    pool: str,
    corpus: str,
    language: str,
    producer: str,
    construct: SourceConstruct,
    source_id: str,
    source_language: str,
    target_id: str,
    target_language: str,
    target_cluster: str,
    judgment: str,
    reason: str,
    confidence: str,
    snippet_sha256: str,
) -> dict[str, Any]:
    return {
        "id": _record_id(
            pool,
            corpus,
            construct.source_file,
            construct.relation,
            construct.start_byte,
            construct.end_byte,
            source_id,
            target_id,
        ),
        "corpus": corpus,
        "pool": pool,
        "producer": producer,
        "capability": construct.capability,
        "language": language,
        "relation": construct.relation,
        "confidence": confidence,
        "targetCluster": target_cluster,
        "source": {"nodeId": source_id, "language": source_language},
        "target": {"nodeId": target_id, "language": target_language},
        "occurrence": {
            "file": construct.source_file,
            "startByte": construct.start_byte,
            "endByte": construct.end_byte,
            "snippetSha256": snippet_sha256,
        },
        "judgment": judgment,
        "reason": reason,
    }


def _cap_cluster_sample(records: list[dict[str, Any]], limit: int) -> list[dict[str, Any]]:
    target = min(len(records), limit)
    if target == 0:
        return []
    by_cluster: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        by_cluster[record["targetCluster"]].append(record)
    per_cluster = max(1, target // 10)
    taken: dict[str, int] = defaultdict(int)
    keys = sorted(by_cluster)
    selected: list[dict[str, Any]] = []
    while keys and len(selected) < limit:
        for key in list(keys):
            if len(selected) >= limit:
                break
            values = by_cluster[key]
            if values and taken[key] < per_cluster:
                selected.append(values.pop(0))
                taken[key] += 1
            if len(selected) >= target or not values or taken[key] >= per_cluster:
                keys.remove(key)
    return selected


def _relation_sample(records: list[dict[str, Any]], limit: int) -> list[dict[str, Any]]:
    by_relation: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        by_relation[record["relation"]].append(record)
    relations = sorted(by_relation)
    if not relations:
        return []
    per_relation = max(1, limit // len(relations))
    selected: list[dict[str, Any]] = []
    for relation in relations:
        selected.extend(_cap_cluster_sample(by_relation[relation], per_relation))
    return selected[:limit]


def _enforce_cluster_diversity(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Apply the scorecard's 10% target-cluster rule after all filtering.

    Sampling happens before advertised capabilities and required relations are
    known.  A cluster can therefore be exactly one record over the limit after
    those filters (and after de-duplication).  Remove the lexicographically
    last record from the offending stratum until every scorecard dimension is
    within its published bound.  This is deterministic and does not alter the
    source-derived denominator.
    """

    dimensions = (
        "corpus",
        "producer",
        "frameworkPack",
        "language",
        "relation",
        "capability",
    )

    def value(record: dict[str, Any], dimension: str) -> str:
        if dimension == "frameworkPack":
            return str(record.get("frameworkPack") or "none")
        return str(record[dimension])

    remaining = list(records)
    while True:
        violation: tuple[str, str, str, int, int] | None = None
        for dimension in dimensions:
            groups: dict[str, list[dict[str, Any]]] = defaultdict(list)
            for record in remaining:
                if record.get("pool") != "accepted":
                    continue
                groups[value(record, dimension)].append(record)
            for key, group in sorted(groups.items()):
                clusters = Counter(record["targetCluster"] for record in group)
                for cluster, count in sorted(clusters.items()):
                    if count * 10 > len(group):
                        candidate = (dimension, key, cluster, count, len(group))
                        if violation is None or candidate < violation:
                            violation = candidate
        if violation is None:
            return remaining
        dimension, key, cluster, _, _ = violation
        candidates = [
            record
            for record in remaining
            if record.get("pool") == "accepted"
            and value(record, dimension) == key
            and record["targetCluster"] == cluster
        ]
        if not candidates:
            return remaining
        remove = max(candidates, key=lambda record: record["id"])
        remaining.remove(remove)


def _load_qualification_manifest(path: Path, language: str) -> dict[str, dict[str, Any]]:
    raw = tomllib.loads(path.read_text(encoding="utf-8"))
    if raw.get("language") != language:
        raise RuntimeError(f"qualification manifest language mismatch: {path}")
    repositories = raw.get("repository")
    if not isinstance(repositories, list):
        raise RuntimeError(f"qualification manifest has no repositories: {path}")
    result: dict[str, dict[str, Any]] = {}
    for repository in repositories:
        if not isinstance(repository, dict) or not isinstance(repository.get("name"), str):
            raise RuntimeError(f"invalid repository entry in {path}")
        result[repository["name"]] = repository
    return result


def _parse_corpus(value: str) -> tuple[str, Path, Path]:
    name, separator, rest = value.partition("=")
    if not separator:
        raise ValueError("--corpus must be NAME=ROOT=GRAPH")
    root_text, separator, graph_text = rest.partition("=")
    if not separator or not name or not root_text or not graph_text:
        raise ValueError("--corpus must be NAME=ROOT=GRAPH")
    return name, Path(root_text).resolve(), Path(graph_text).resolve()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--language", required=True, choices=("swift", "dart", "scala", "groovy"))
    parser.add_argument("--qualification-manifest", type=Path, required=True)
    parser.add_argument("--corpus", action="append", required=True, metavar="NAME=ROOT=GRAPH")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-accepted", type=int, default=100_000)
    parser.add_argument("--max-source", type=int, default=100_000)
    args = parser.parse_args()
    if args.max_accepted < 1 or args.max_source < 1:
        parser.error("sampling limits must be positive")

    specifications = _load_qualification_manifest(args.qualification_manifest, args.language)
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
        if not root.is_dir() or not graph.is_file():
            raise RuntimeError(f"missing corpus root or graph for {name}")
        repository = specifications.get(name)
        if repository is None:
            raise RuntimeError(f"{name!r} is not present in the qualification manifest")
        if _commit(root) != repository["commit"]:
            raise RuntimeError(f"{name} checkout is not at the manifest commit")
        source_link = staging / "sources" / name
        graph_link = staging / "graphs" / f"{name}.json"
        source_link.symlink_to(root, target_is_directory=True)
        graph_link.symlink_to(graph)
        include_globs = tuple(repository.get("sourceGlobs", ()))
        exclude_globs = tuple(repository.get("excludeGlobs", ()))
        inventory = independent_source_inventory(
            root,
            args.language,
            include_globs=include_globs,
            exclude_globs=exclude_globs,
        )
        declarations = _declaration_index(inventory)
        declared_names = _declared_names(
            root,
            args.language,
            include_globs,
            exclude_globs,
        )
        provider = independent_source_provider_identity(args.language)
        nodes = _nodes(graph, args.language)
        edges = _edges(graph, nodes, args.language)
        by_anchor: dict[tuple[str, str, int, int], list[dict[str, Any]]] = defaultdict(list)
        by_file_relation: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
        by_file_relation_terminal: dict[
            tuple[str, str, str], list[dict[str, Any]]
        ] = defaultdict(list)
        for edge in edges:
            file, start, end = edge["anchor"]
            by_anchor[(edge["relation"], file, start, end)].append(edge)
            by_file_relation[(file, edge["relation"])].append(edge)
            terminal = (
                edge["targetNode"].get("qualifiedName") or edge["target"]
            ).rsplit(".", 1)[-1]
            by_file_relation_terminal[
                (file, edge["relation"], terminal)
            ].append(edge)
        file_nodes: dict[str, str] = {}
        for node in nodes.values():
            source = node.get("sourceFile")
            if isinstance(source, str) and source and node.get("kind") == "file":
                file_nodes.setdefault(source, node["id"])
        accepted_part: list[dict[str, Any]] = []
        source_part: list[dict[str, Any]] = []
        for edge in edges:
            relation = edge["relation"]
            capability = RELATION_CAPABILITY.get(relation)
            if capability is None:
                continue
            file, start, end = edge["anchor"]
            construct = SourceConstruct(
                file,
                relation,
                capability,
                "graph-source",
                edge["targetNode"].get("qualifiedName") or edge["target"],
                None,
                start,
                end,
                1,
            )
            snippet = _snippet(root, construct)
            if snippet is None:
                continue
            target_node = edge["targetNode"]
            target_cluster = _target_cluster(
                target_node.get("qualifiedName", "") or edge["target"],
                edge["target"],
            )
            accepted_part.append(
                _candidate(
                    pool="accepted",
                    corpus=name,
                    language=args.language,
                    producer=args.language,
                    construct=construct,
                    source_id=edge["source"],
                    source_language=args.language,
                    target_id=edge["target"],
                    target_language=target_node.get("language") or args.language,
                    target_cluster=target_cluster,
                    judgment="correct",
                    reason="exact Compass source anchor retained for independent review",
                    confidence=edge["confidence"],
                    snippet_sha256=snippet,
                )
            )
        for construct in inventory.constructs:
            capability = RELATION_CAPABILITY.get(construct.relation)
            if capability is None:
                continue
            snippet = _snippet(root, construct)
            if snippet is None:
                continue
            aliases = RELATION_ALIASES.get(construct.relation, frozenset((construct.relation,)))
            matches = [
                edge
                for relation in aliases
                for edge in by_anchor.get((relation, construct.source_file, construct.start_byte, construct.end_byte), ())
            ]
            overlap_match = False
            if not matches:
                matches = _declaration_overlap_matches(
                    args.language,
                    construct,
                    aliases,
                    by_file_relation,
                    by_file_relation_terminal,
                )
                overlap_match = bool(matches)
            if matches:
                edge = sorted(matches, key=lambda item: (item["source"], item["target"]))[0]
                target_node = edge["targetNode"]
                matched_construct = (
                    construct
                    if construct.relation == edge["relation"]
                    else replace(construct, relation=edge["relation"])
                )
                if (
                    args.language in {"dart", "groovy", "scala"}
                    and overlap_match
                    and (
                        construct.start_byte != edge["anchor"][1]
                        or construct.end_byte != edge["anchor"][2]
                    )
                ):
                    # The independent source oracle may anchor ownership to
                    # the whole declaration, while the tree-sitter producer
                    # anchors the same fact to the normalized name/type span.
                    # The overlap match is still source-grounded; publish the
                    # graph's exact occurrence and recompute the snippet
                    # digest so the audit manifest remains self-validating.
                    matched_construct = replace(
                        matched_construct,
                        start_byte=edge["anchor"][1],
                        end_byte=edge["anchor"][2],
                    )
                    snippet = _snippet(root, matched_construct)
                    if snippet is None:
                        continue
                source_part.append(
                    _candidate(
                        pool="source_oracle",
                        corpus=name,
                        language=args.language,
                        producer=args.language,
                        construct=matched_construct,
                        source_id=edge["source"],
                        source_language=args.language,
                        target_id=edge["target"],
                        target_language=target_node.get("language") or args.language,
                        target_cluster=_target_cluster(
                            target_node.get("qualifiedName", "") or edge["target"],
                            edge["target"],
                        ),
                        judgment="correct",
                        reason="independent source construct has an exact Compass relation anchor",
                        confidence="source_oracle",
                        snippet_sha256=snippet,
                    )
                )
            else:
                if construct.relation in {"imports", "reexports"}:
                    continue
                if not _missing_source_is_adjudicable(
                    args.language,
                    construct,
                    declarations,
                    trailing_comment=(
                        args.language == "scala"
                        and construct.relation == "contains"
                        and _has_trailing_comment(root, construct)
                    ),
                ):
                    continue
                terminal = construct.target_spelling.rsplit(".", 1)[-1].rsplit("::", 1)[-1]
                if terminal not in declared_names:
                    # The independent oracle proves a source use, but no
                    # declaration in the bounded project can satisfy it.  It
                    # is an external/unresolved use and is not a local recall
                    # denominator for this closed-project audit.
                    continue
                source_id = file_nodes.get(construct.source_file, "oracle-source-" + hashlib.sha256(construct.source_file.encode()).hexdigest()[:16])
                target_id = "oracle-target-" + hashlib.sha256(
                    json.dumps((name, construct.source_file, construct.start_byte, construct.end_byte), separators=(",", ":")).encode()
                ).hexdigest()[:16]
                source_part.append(
                    _candidate(
                        pool="source_oracle",
                        corpus=name,
                        language=args.language,
                        producer=args.language,
                        construct=construct,
                        source_id=source_id,
                        source_language=args.language,
                        target_id=target_id,
                        target_language=args.language,
                        target_cluster=_target_cluster(construct.target_spelling, target_id),
                        judgment="missing",
                        reason="independent source construct has no exact Compass relation anchor",
                        confidence="source_oracle",
                        snippet_sha256=snippet,
                    )
                )
        accepted_part = _relation_sample(accepted_part, args.max_accepted)
        source_part = _relation_sample(source_part, args.max_source)
        accepted.extend(accepted_part)
        source_records.extend(source_part)
        commit = _commit(root)
        corpora.append(
            {
                "name": name,
                "commit": commit,
                "path": f"sources/{name}",
                "graph": f"graphs/{name}.json",
                "graphSha256": _sha256(graph),
                "sourceGlobs": list(include_globs),
                "excludeGlobs": list(exclude_globs),
            }
        )
        source_oracles.append(
            {
                "corpus": name,
                "producer": args.language,
                "provider": provider,
                "scannedFiles": inventory.scanned_files,
                "parsedFiles": inventory.parsed_files,
                "inventorySha256": source_construct_inventory_sha256(args.language, inventory),
            }
        )
        coverage.append(
            {
                "corpus": name,
                "scannedFiles": inventory.scanned_files,
                "parsedFiles": inventory.parsed_files,
                "accepted": len(accepted_part),
                "sourceOracle": len(source_part),
                "graphEdges": len(edges),
            }
        )

    capability_counts = Counter(record["capability"] for record in accepted)
    relation_counts = Counter(record["relation"] for record in accepted)
    advertised = [
        {"producer": args.language, "capability": capability}
        for capability, count in sorted(capability_counts.items())
        if count >= 100
    ]
    allowed_capabilities = {(item["producer"], item["capability"]) for item in advertised}
    accepted = [record for record in accepted if (record["producer"], record["capability"]) in allowed_capabilities]
    source_records = [record for record in source_records if (record["producer"], record["capability"]) in allowed_capabilities]
    required_relations = sorted({record["relation"] for record in accepted if relation_counts[record["relation"]] >= 100})
    accepted = [record for record in accepted if record["relation"] in required_relations]
    source_records = [record for record in source_records if record["relation"] in required_relations]
    accepted = list({record["id"]: record for record in accepted}.values())
    source_records = list({record["id"]: record for record in source_records}.values())
    accepted = _enforce_cluster_diversity(accepted)
    records = sorted(accepted + source_records, key=lambda record: record["id"])
    manifest = {
        "schema": "compass.quality-audit/2",
        "mode": "qualification",
        "corpora": sorted(corpora, key=lambda item: item["name"]),
        "sourceOracles": sorted(source_oracles, key=lambda item: (item["corpus"], item["producer"])),
        "advertisedCapabilities": advertised,
        "requiredRelations": required_relations,
        "records": records,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(manifest, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "schema": "compass.universal-quality-audit-build/1",
                "language": args.language,
                "manifest": str(args.output),
                "auditRoot": str(staging),
                "corpora": coverage,
                "accepted": len(accepted),
                "sourceOracle": len(source_records),
                "advertisedCapabilities": advertised,
                "requiredRelations": required_relations,
            },
            sort_keys=True,
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
