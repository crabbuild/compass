#!/usr/bin/env python3
"""Build a deterministic Ruby universal-candidate quality-audit manifest.

The manifest is qualification data, never product input.  It joins exact
Tree-sitter graph anchors with the independently pinned Ripper inventory and
keeps only conservative identity matches.  Unmatched source facts remain
``missing`` recall records instead of being turned into invented graph facts.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any, Iterable

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


ADAPTER = "ruby"
PROVIDER = "ruby_ripper_4_0_6"
CAPABILITY_BY_RELATION = {
    "aliases": "aliases",
    "calls": "calls",
    "contains": "ownership",
    "extends": "base_types",
    "implements": "traits",
    "imports": "imports",
    "instantiates": "construction",
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
    if completed.returncode != 0:
        raise RuntimeError(f"could not identify corpus revision at {root}: {completed.stderr.strip()}")
    value = completed.stdout.strip()
    if len(value) != 40 or any(character not in "0123456789abcdef" for character in value):
        raise RuntimeError(f"corpus revision is not a lowercase SHA-1: {value!r}")
    return value


def _node_source_file(node: dict[str, Any]) -> str | None:
    source = node.get("source")
    if not isinstance(source, dict):
        return None
    value = source.get("file")
    return value if isinstance(value, str) and value else None


def _node_range(node: dict[str, Any]) -> tuple[int, int] | None:
    source = node.get("source")
    if not isinstance(source, dict):
        return None
    start = source.get("startByte")
    end = source.get("endByte")
    if not isinstance(start, int) or isinstance(start, bool) or not isinstance(end, int) or isinstance(end, bool):
        return None
    return start, end


def _graph_nodes(path: Path) -> tuple[dict[str, dict[str, Any]], int]:
    nodes: dict[str, dict[str, Any]] = {}
    count = 0
    for raw in iter_top_level_array(path, "nodes"):
        node_id = raw.get("id")
        if not isinstance(node_id, str) or not node_id:
            continue
        nodes[node_id] = {
            "id": node_id,
            "language": raw.get("language") if isinstance(raw.get("language"), str) else "",
            "qualifiedName": raw.get("qualifiedName") if isinstance(raw.get("qualifiedName"), str) else "",
            "kind": raw.get("kind") if isinstance(raw.get("kind"), str) else "",
            "sourceFile": _node_source_file(raw),
            "sourceRange": _node_range(raw),
        }
        count += 1
    return nodes, count


def _graph_edges(path: Path, nodes: dict[str, dict[str, Any]]) -> tuple[list[dict[str, Any]], int]:
    edges: list[dict[str, Any]] = []
    count = 0
    for raw in iter_top_level_array(path, "links"):
        count += 1
        source = raw.get("source")
        target = raw.get("target")
        relation = raw.get("kind", raw.get("relation"))
        if not isinstance(source, str) or not isinstance(target, str) or not isinstance(relation, str):
            continue
        source_node = nodes.get(source)
        target_node = nodes.get(target)
        if source_node is None or target_node is None:
            continue
        if source_node["language"] != ADAPTER or target_node["language"] != ADAPTER:
            continue
        anchor = _edge_anchor(raw)
        if anchor is None:
            continue
        edges.append(
            {
                "source": source,
                "target": target,
                "relation": relation.casefold(),
                "anchor": anchor,
                "confidence": raw.get("confidence", "exact"),
            }
        )
    return edges, count


def _method_owner_matches(owner: str, node: dict[str, Any], source_file: str) -> bool:
    qualified = node["qualifiedName"]
    if owner == qualified:
        return True
    if owner == source_file and node["sourceFile"] == source_file:
        return True
    return qualified.startswith(owner + "#") or qualified.startswith(owner + ".") or qualified.startswith(owner + "::")


def _target_file_matches(target: str, node: dict[str, Any]) -> bool:
    source_file = node["sourceFile"]
    if not source_file:
        return False
    normalized = target.lstrip("./")
    candidates = {normalized}
    if not normalized.endswith(".rb"):
        candidates.add(normalized + ".rb")
    candidates.add(normalized.rstrip("/") + "/index.rb")
    return source_file in candidates


def _target_matches(construct: SourceConstruct, node: dict[str, Any]) -> bool:
    target = construct.target_spelling
    qualified = node["qualifiedName"]
    relation = construct.relation
    if relation == "imports":
        return _target_file_matches(target, node)
    if relation in {"contains", "extends", "implements"}:
        return qualified == target or qualified.lstrip("::") == target.lstrip("::")
    if relation == "instantiates":
        class_name = target.rsplit("#", 1)[0] if "#" in target else target
        return qualified == class_name or qualified.endswith("::" + class_name) or qualified == class_name.rsplit("::", 1)[-1]
    if relation == "calls":
        method_name = target.rsplit("#", 1)[-1] if "#" in target else target.rsplit(".", 1)[-1]
        if not (qualified.endswith("#" + method_name) or qualified.endswith("." + method_name)):
            return False
        if "#" not in target and "." not in target:
            return True
        if "#" in target:
            receiver = target.rsplit("#", 1)[0]
            return qualified == target or qualified.startswith(receiver + "#")
        receiver = target.rsplit(".", 1)[0]
        return qualified == target or qualified.startswith(receiver + ".")
    return False


def _has_local_target(
    construct: SourceConstruct,
    qualified: set[str],
    type_names: set[str],
    source_declaration_kinds: dict[str, list[str]],
) -> bool:
    """Return whether an oracle target names a declaration in this project.

    Ruby source routinely calls stdlib/gem methods that Compass must not
    invent as project relationships.  Those facts are still useful evidence,
    but they are classified as external instead of lowering recall for the
    closed project graph.  This check is deliberately identity-only: it does
    not use a terminal-name fallback for qualified calls.
    """

    target = construct.target_spelling
    relation = construct.relation
    if relation == "imports":
        return False
    if relation in {"contains", "extends", "implements"}:
        normalized = target.lstrip("::")
        return any(
            value == target or value.lstrip("::") == normalized
            for value in qualified
        ) and _source_target_is_unambiguous(
            relation, target, source_declaration_kinds
        )
    if relation == "instantiates":
        class_name = target.rsplit("#", 1)[0] if "#" in target else target
        candidates = _ruby_lexical_names(construct.owner_qualified_name, class_name)
        matches = {
            candidate
            for candidate in candidates
            if candidate in type_names
            and _source_target_is_unambiguous(
                "instantiates", candidate, source_declaration_kinds
            )
        }
        return len(matches) == 1
    if relation == "calls":
        if "#" in target:
            receiver, method = target.rsplit("#", 1)
            return (target in qualified or (receiver + "#" + method) in qualified) and _source_target_is_unambiguous(
                relation, target, source_declaration_kinds
            )
        if "." in target:
            receiver, method = target.rsplit(".", 1)
            return (target in qualified or (receiver + "." + method) in qualified) and _source_target_is_unambiguous(
                relation, target, source_declaration_kinds
            )
        owner = _ruby_owner_type(construct.owner_qualified_name)
        if owner is None:
            return False
        return any(
            candidate in qualified
            and _source_target_is_unambiguous(relation, candidate, source_declaration_kinds)
            for candidate in (f"{owner}#{target}", f"{owner}.{target}")
        )
    return False


def _source_target_is_unambiguous(
    relation: str,
    target: str,
    source_declaration_kinds: dict[str, list[str]],
) -> bool:
    """Apply the independent oracle's fail-closed declaration ambiguity rule."""

    kinds = source_declaration_kinds.get(target)
    if not kinds:
        return True
    if relation == "calls":
        # A repeated method declaration is a genuine Ruby dispatch
        # ambiguity, even when every declaration has the same kind.
        return len(kinds) == 1 and kinds[0] == "method"
    if relation in {"extends", "instantiates"}:
        return all(kind in {"class", "module"} for kind in kinds)
    if relation == "implements":
        return all(kind == "module" for kind in kinds)
    return True


def _ruby_owner_type(owner: str) -> str | None:
    """Return the receiver type for a Ruby instance/singleton declaration."""

    namespace_end = owner.rfind("::")
    candidates = [owner.rfind("#"), owner.rfind(".")]
    separator = max(candidates)
    if separator <= namespace_end:
        return None
    return owner[:separator]


def _ruby_lexical_names(owner: str, raw: str) -> tuple[str, ...]:
    """Enumerate Ruby constant lookup candidates from the innermost owner out."""

    normalized = raw.lstrip("::")
    if not normalized:
        return ()
    if raw.startswith("::"):
        return (normalized,)
    owner_type = _ruby_owner_type(owner) or ""
    parts = owner_type.split("::") if owner_type else []
    candidates = [
        ("::".join(parts[:index]) + "::" if index else "") + normalized
        for index in range(len(parts), -1, -1)
    ]
    return tuple(dict.fromkeys(candidates))


def _snippet_hash(root: Path, source_file: str, start: int, end: int) -> str | None:
    path = (root / source_file).resolve()
    try:
        path.relative_to(root.resolve())
        contents = path.read_bytes()
    except (OSError, ValueError):
        return None
    if start < 0 or end <= start or end > len(contents):
        return None
    return hashlib.sha256(contents[start:end].replace(b"\r\n", b"\n")).hexdigest()


def _record_id(kind: str, corpus: str, construct: SourceConstruct, target: str) -> str:
    identity = [
        kind,
        corpus,
        construct.source_file,
        construct.relation,
        construct.capability,
        construct.owner_qualified_name,
        construct.target_spelling,
        construct.start_byte,
        construct.end_byte,
        target,
    ]
    return "ruby-" + kind + "-" + hashlib.sha256(
        json.dumps(identity, separators=(",", ":"), ensure_ascii=True).encode("utf-8")
    ).hexdigest()[:24]


def _synthetic_target(corpus: str, construct: SourceConstruct) -> str:
    return "oracle:ruby:" + hashlib.sha256(
        json.dumps(
            [corpus, construct.source_file, construct.relation, construct.target_spelling, construct.start_byte, construct.end_byte],
            separators=(",", ":"),
        ).encode("utf-8")
    ).hexdigest()[:32]


def _candidate(
    *,
    kind: str,
    corpus: str,
    construct: SourceConstruct,
    source_id: str,
    target_id: str,
    source_node: dict[str, Any],
    target_node: dict[str, Any] | None,
    snippet: str,
    judgment: str,
    reason: str,
    confidence: str,
) -> dict[str, Any]:
    capability = CAPABILITY_BY_RELATION[construct.relation]
    target_label = (target_node or {}).get("qualifiedName") or construct.target_spelling
    return {
        "id": _record_id(kind, corpus, construct, target_id),
        "corpus": corpus,
        "pool": "accepted" if kind == "accepted" else "source_oracle",
        "adapter": ADAPTER,
        "capability": capability,
        "language": ADAPTER,
        "relation": construct.relation,
        "confidence": confidence,
        "targetCluster": _target_cluster(target_label, target_id),
        "source": {"nodeId": source_id, "language": ADAPTER},
        "target": {"nodeId": target_id, "language": ADAPTER},
        "occurrence": {
            "file": construct.source_file,
            "startByte": construct.start_byte,
            "endByte": construct.end_byte,
            "snippetSha256": snippet,
        },
        "judgment": judgment,
        "reason": reason,
    }


def _round_robin(records: Iterable[dict[str, Any]], limit: int) -> list[dict[str, Any]]:
    groups: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in sorted(records, key=lambda item: item["id"]):
        groups[record["targetCluster"]].append(record)
    selected: list[dict[str, Any]] = []
    keys = sorted(groups)
    cursor = 0
    while keys and len(selected) < limit:
        key = keys[cursor % len(keys)]
        values = groups[key]
        selected.append(values.pop(0))
        if not values:
            keys.remove(key)
            cursor = 0
        else:
            cursor += 1
    return selected


def _stratified_source_sample(
    records: Iterable[dict[str, Any]],
    limit: int,
) -> list[dict[str, Any]]:
    """Keep every small relation family and sample large families evenly."""

    by_relation: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        by_relation[record["relation"]].append(record)
    relations = sorted(by_relation)
    if not relations:
        return []
    quotient, remainder = divmod(limit, len(relations))
    selected: list[dict[str, Any]] = []
    for index, relation in enumerate(relations):
        quota = quotient + int(index < remainder)
        selected.extend(_round_robin(by_relation[relation], quota))
    return selected


def _cluster_capped_sample(
    records: Iterable[dict[str, Any]],
    limit: int,
) -> list[dict[str, Any]]:
    """Select the largest deterministic sample whose clusters are <=10%."""

    groups: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in sorted(records, key=lambda item: item["id"]):
        groups[record["targetCluster"]].append(record)
    if not groups:
        return []
    available = sum(len(values) for values in groups.values())
    target = min(limit, available)
    selected_target = 0
    cap = 0
    while target >= 10:
        cap = target // 10
        if sum(min(len(values), cap) for values in groups.values()) >= target:
            selected_target = target
            break
        target -= 1
    if selected_target == 0:
        return []
    pools = {
        key: values[:cap]
        for key, values in sorted(groups.items())
    }
    selected: list[dict[str, Any]] = []
    keys = sorted(pools)
    cursor = 0
    while keys and len(selected) < selected_target:
        key = keys[cursor % len(keys)]
        values = pools[key]
        selected.append(values.pop(0))
        if not values:
            keys.remove(key)
            cursor = 0
        else:
            cursor += 1
    return selected


def _cluster_capped_by_relation(
    records: Iterable[dict[str, Any]],
    limit: int,
) -> list[dict[str, Any]]:
    by_relation: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        by_relation[record["relation"]].append(record)
    relations = sorted(by_relation)
    if not relations:
        return []
    quotient, remainder = divmod(limit, len(relations))
    selected: list[dict[str, Any]] = []
    for index, relation in enumerate(relations):
        selected.extend(
            _cluster_capped_sample(
                by_relation[relation],
                quotient + int(index < remainder),
            )
        )
    return selected


def build_corpus(name: str, root: Path, graph: Path, *, max_accepted: int, max_source: int) -> tuple[dict[str, Any], list[dict[str, Any]], list[dict[str, Any]], dict[str, Any]]:
    root = root.resolve()
    graph = graph.resolve()
    nodes, node_count = _graph_nodes(graph)
    edges, edge_count = _graph_edges(graph, nodes)
    inventory = independent_source_inventory(root, ADAPTER)
    if independent_source_provider_identity(ADAPTER) != PROVIDER:
        raise RuntimeError("the pinned Ruby source provider identity changed")
    by_anchor: dict[tuple[str, str, int, int], list[dict[str, Any]]] = defaultdict(list)
    for edge in edges:
        file, start, end = edge["anchor"]
        by_anchor[(edge["relation"], file, start, end)].append(edge)
    file_nodes = {
        node["sourceFile"]: node
        for node in nodes.values()
        if node["language"] == ADAPTER and node["kind"] == "file" and node["sourceFile"]
    }
    qualified_nodes: dict[str, list[dict[str, Any]]] = defaultdict(list)
    source_nodes: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for node in nodes.values():
        if node["language"] != ADAPTER:
            continue
        qualified_nodes[node["qualifiedName"]].append(node)
        if node["sourceFile"]:
            source_nodes[node["sourceFile"]].append(node)
    for values in source_nodes.values():
        values.sort(
            key=lambda node: (
                (node["sourceRange"][1] - node["sourceRange"][0])
                if node["sourceRange"] is not None
                else 2**63,
                node["id"],
            )
        )
    accepted: list[dict[str, Any]] = []
    source_records: list[dict[str, Any]] = []
    accepted_keys: set[tuple[str, str, str, int, int]] = set()
    capability_counts: defaultdict[str, int] = defaultdict(int)
    external_counts: defaultdict[str, int] = defaultdict(int)
    source_contents: dict[str, bytes] = {}
    qualified_names = {
        node["qualifiedName"]
        for node in nodes.values()
        if isinstance(node.get("qualifiedName"), str) and node["qualifiedName"]
    }
    type_names = {
        node["qualifiedName"]
        for node in nodes.values()
        if node["kind"] in {"class", "trait", "module"}
        and node["qualifiedName"]
    }
    source_declaration_kinds: dict[str, list[str]] = defaultdict(list)
    for construct in inventory.constructs:
        if construct.relation == "contains" and construct.qualifier in {
            "class",
            "module",
            "method",
        }:
            source_declaration_kinds[construct.target_spelling].append(
                construct.qualifier
            )
    for construct in inventory.constructs:
        capability = CAPABILITY_BY_RELATION.get(construct.relation)
        if capability is None:
            continue
        if construct.source_file not in source_contents:
            source_path = (root / construct.source_file).resolve()
            try:
                source_path.relative_to(root)
                source_contents[construct.source_file] = source_path.read_bytes()
            except (OSError, ValueError):
                source_contents[construct.source_file] = b""
        contents = source_contents[construct.source_file]
        if construct.start_byte < 0 or construct.end_byte <= construct.start_byte or construct.end_byte > len(contents):
            snippet = None
        else:
            snippet = hashlib.sha256(
                contents[construct.start_byte : construct.end_byte].replace(b"\r\n", b"\n")
            ).hexdigest()
        if snippet is None:
            continue
        anchor_edges = by_anchor.get((construct.relation, construct.source_file, construct.start_byte, construct.end_byte), ())
        matched: list[dict[str, Any]] = []
        for edge in anchor_edges:
            source_node = nodes[edge["source"]]
            target_node = nodes[edge["target"]]
            if _method_owner_matches(construct.owner_qualified_name, source_node, construct.source_file) and _target_matches(construct, target_node):
                matched.append(edge)
        source_node: dict[str, Any] | None = None
        if matched:
            source_node = nodes[matched[0]["source"]]
        else:
            for node in qualified_nodes.get(construct.owner_qualified_name, ()):
                source_node = node
                break
            if source_node is None:
                for node in source_nodes.get(construct.source_file, ()):
                    source_range = node["sourceRange"]
                    if (
                        source_range is not None
                        and source_range[0] <= construct.start_byte
                        and construct.end_byte <= source_range[1]
                        and _method_owner_matches(
                            construct.owner_qualified_name,
                            node,
                            construct.source_file,
                        )
                    ):
                        source_node = node
                        break
        if source_node is None:
            source_node = file_nodes.get(construct.source_file)
        if source_node is None:
            continue
        if matched:
            for edge in matched:
                key = (edge["source"], edge["target"], construct.relation, construct.start_byte, construct.end_byte)
                if key in accepted_keys:
                    continue
                accepted_keys.add(key)
                target_node = nodes[edge["target"]]
                accepted.append(
                    _candidate(
                        kind="accepted",
                        corpus=name,
                        construct=construct,
                        source_id=edge["source"],
                        target_id=edge["target"],
                        source_node=source_node,
                        target_node=target_node,
                        snippet=snippet,
                        judgment="correct",
                        reason="independent Ripper token anchor and conservative Ruby identity match",
                        confidence="exact",
                    )
                )
                capability_counts[capability] += 1
            source_target = matched[0]["target"]
            source_judgment = "correct"
            source_reason = "independent Ripper fact is represented at the exact Compass anchor"
            source_target_node = nodes[source_target]
        else:
            source_target = _synthetic_target(name, construct)
            if _has_local_target(
                construct,
                qualified_names,
                type_names,
                source_declaration_kinds,
            ):
                source_judgment = "missing"
                source_reason = "independent Ripper fact names a project declaration but has no exact Compass match"
            else:
                external_counts[capability] += 1
                continue
            source_target_node = None
        source_records.append(
            _candidate(
                kind="source_oracle",
                corpus=name,
                construct=construct,
                source_id=source_node["id"],
                target_id=source_target,
                source_node=source_node,
                target_node=source_target_node,
                snippet=snippet,
                judgment=source_judgment,
                reason=source_reason,
                confidence="source_oracle",
            )
        )
    accepted = _cluster_capped_by_relation(accepted, max_accepted)
    source_records = _stratified_source_sample(source_records, max_source)
    corpus = {
        "name": name,
        "commit": _commit(root),
        "path": str(root),
        "graph": str(graph),
        "graphSha256": _sha256(graph),
    }
    coverage = {
        "corpus": name,
        "adapter": ADAPTER,
        "provider": PROVIDER,
        "scannedFiles": inventory.scanned_files,
        "parsedFiles": inventory.parsed_files,
        "inventorySha256": source_construct_inventory_sha256(ADAPTER, inventory),
        "graphNodes": node_count,
        "graphEdges": edge_count,
        "acceptedBeforeSampling": sum(capability_counts.values()),
        "acceptedAfterSampling": len(accepted),
        "sourceOracleAfterSampling": len(source_records),
        "acceptedCapabilities": dict(sorted(capability_counts.items())),
        "externalSourceFacts": dict(sorted(external_counts.items())),
    }
    return corpus, accepted, source_records, coverage


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", action="append", required=True, metavar="NAME=ROOT=GRAPH")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-accepted", type=int, default=50_000)
    parser.add_argument("--max-source", type=int, default=50_000)
    args = parser.parse_args()
    if args.max_accepted <= 0 or args.max_source <= 0:
        parser.error("sampling limits must be positive")
    parsed: list[tuple[str, Path, Path]] = []
    for value in args.corpus:
        parts = value.split("=", 2)
        if len(parts) != 3 or not all(parts):
            parser.error("--corpus must be NAME=ROOT=GRAPH")
        parsed.append((parts[0], Path(parts[1]), Path(parts[2])))
    parsed.sort(key=lambda item: item[0])
    if len({item[0] for item in parsed}) != len(parsed):
        parser.error("duplicate corpus name")
    base_root = Path(
        os.path.commonpath(
            [str(path.resolve()) for _, root, graph in parsed for path in (root, graph)]
        )
    ).resolve()
    corpora: list[dict[str, Any]] = []
    accepted: list[dict[str, Any]] = []
    source_records: list[dict[str, Any]] = []
    coverage: list[dict[str, Any]] = []
    for name, root, graph in parsed:
        corpus, accepted_part, source_part, coverage_part = build_corpus(
            name,
            root,
            graph,
            max_accepted=args.max_accepted,
            max_source=args.max_source,
        )
        try:
            corpus["path"] = root.resolve().relative_to(base_root).as_posix()
            corpus["graph"] = graph.resolve().relative_to(base_root).as_posix()
        except ValueError as error:
            raise RuntimeError(
                f"corpus {name!r} and graph must be beneath common audit root {base_root}"
            ) from error
        corpora.append(corpus)
        accepted.extend(accepted_part)
        source_records.extend(source_part)
        coverage.append(coverage_part)
    capability_counts = Counter(record["capability"] for record in accepted)
    accepted_capabilities = sorted(capability_counts)
    advertised = [
        {"adapter": ADAPTER, "capability": capability}
        for capability in accepted_capabilities
        if capability_counts[capability] >= 100
    ]
    advertised_keys = {(entry["adapter"], entry["capability"]) for entry in advertised}
    accepted = [record for record in accepted if (record["adapter"], record["capability"]) in advertised_keys]
    source_records = [record for record in source_records if (record["adapter"], record["capability"]) in advertised_keys]
    relation_counts = Counter(record["relation"] for record in accepted)
    relations = sorted(
        relation for relation, count in relation_counts.items() if count >= 100
    )
    allowed_relations = set(relations)
    accepted = [record for record in accepted if record["relation"] in allowed_relations]
    source_records = [record for record in source_records if record["relation"] in allowed_relations]
    records = sorted(accepted + source_records, key=lambda item: item["id"])
    manifest = {
        "schema": "compass.quality-audit",
        "mode": "qualification",
        "corpora": sorted(corpora, key=lambda item: item["name"]),
        "sourceOracles": sorted(
            [
                {
                    "corpus": item["corpus"],
                    "adapter": ADAPTER,
                    "provider": PROVIDER,
                    "scannedFiles": item["scannedFiles"],
                    "parsedFiles": item["parsedFiles"],
                    "inventorySha256": item["inventorySha256"],
                }
                for item in coverage
            ],
            key=lambda item: (item["corpus"], item["adapter"]),
        ),
        "advertisedCapabilities": sorted(advertised, key=lambda item: (item["adapter"], item["capability"])),
        "requiredRelations": relations,
        "records": records,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(manifest, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n", encoding="utf-8")
    report = {
        "schema": "compass.ruby-quality-audit-build/1",
        "manifest": str(args.output),
        "corpora": coverage,
        "advertisedCapabilities": advertised,
        "requiredRelations": relations,
        "accepted": len(accepted),
        "sourceOracle": len(source_records),
    }
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
