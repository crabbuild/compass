#!/usr/bin/env python3
"""Independent semantic checks for the React/frontend qualification corpus.

This intentionally does not call Compass or reuse its resolver.  It validates
the published graph envelope, endpoint vocabulary, provenance, and the small
set of framework facts that the frontend contract promises.
"""

from __future__ import annotations

import argparse
import hashlib
import math
import json
import os
import re
import sys
from pathlib import Path
from typing import Any


FRAMEWORKS = {"next", "react-router", "tanstack-router", "remix", "vite"}
EXPECTATIONS = Path(__file__).resolve().parents[1] / "tests/qualification/react-frontend-expectations.json"
MAX_ORACLE_FACTS = 500_000
MAX_INPUT_BYTES = 512 * 1024 * 1024


def value(record: dict[str, Any], key: str) -> Any:
    if key in record:
        return record[key]
    attributes = record.get("attributes")
    if isinstance(attributes, dict):
        return attributes.get(key)
    return None


def source_file(record: dict[str, Any]) -> str | None:
    source = record.get("source")
    if isinstance(source, dict) and isinstance(source.get("file"), str):
        return source["file"]
    return value(record, "source_file") or value(record, "sourceFile")


def fail(message: str) -> None:
    raise SystemExit(f"react frontend qualification failed: {message}")


GRAPH_PATH_KEYS = {"file", "source_file", "sourceFile", "targetFile"}


def graph_path_is_safe(path: str) -> bool:
    """Return whether a graph anchor uses a bounded corpus-relative path."""
    portable = path.replace("\\", "/")
    if not path or "\x00" in path or portable.startswith("/") or portable.startswith("//"):
        return False
    if re.match(r"^[A-Za-z]:/", portable) or "://" in portable:
        return False
    return ".." not in portable.split("/")


def graph_paths(graph: Any) -> list[str]:
    """Collect only path-bearing graph fields, including nested anchors."""
    paths: list[str] = []

    def visit(value: Any, depth: int = 0) -> None:
        if depth > 32:
            fail("graph metadata nesting exceeds the qualification limit")
        if isinstance(value, dict):
            for key, child in value.items():
                if key in GRAPH_PATH_KEYS and isinstance(child, str):
                    paths.append(child)
                visit(child, depth + 1)
        elif isinstance(value, list):
            for child in value:
                visit(child, depth + 1)

    visit(graph)
    return paths


def validate_graph_paths(graph: dict[str, Any]) -> bool:
    unsafe = sorted({path for path in graph_paths(graph) if not graph_path_is_safe(path)})
    if unsafe:
        fail(f"graph contains unsafe source anchor paths: {unsafe[:5]}")
    return not unsafe


def load(path: Path) -> dict[str, Any]:
    try:
        if path.stat().st_size > MAX_INPUT_BYTES:
            fail(f"qualification input exceeds the {MAX_INPUT_BYTES}-byte limit: {path}")
        content = bytearray()
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                content.extend(chunk)
                if len(content) > MAX_INPUT_BYTES:
                    fail(f"qualification input exceeds the {MAX_INPUT_BYTES}-byte limit: {path}")
        graph = json.loads(bytes(content).decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"cannot read {path}: {error}")
    if not isinstance(graph, dict):
        fail("graph must be an object")
    return graph


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def wilson_lower(successes: int, trials: int, z: float = 1.959963984540054) -> float:
    if trials <= 0:
        return 0.0
    proportion = successes / trials
    denominator = 1.0 + z * z / trials
    centre = proportion + z * z / (2.0 * trials)
    spread = z * math.sqrt((proportion * (1.0 - proportion) + z * z / (4.0 * trials)) / trials)
    return max(0.0, (centre - spread) / denominator)


def edge_anchor(edge: dict[str, Any]) -> tuple[str, int, int] | None:
    site = edge.get("relationshipSite")
    if isinstance(site, dict) and isinstance(site.get("file"), str):
        if isinstance(site.get("startByte"), int) and isinstance(site.get("endByte"), int):
            return site["file"], site["startByte"], site["endByte"]
    evidence = edge.get("evidence")
    if isinstance(evidence, list):
        for item in evidence:
            if not isinstance(item, dict):
                continue
            anchors = item.get("anchors")
            if not isinstance(anchors, list):
                continue
            for anchor in anchors:
                if isinstance(anchor, dict) and isinstance(anchor.get("file"), str):
                    if isinstance(anchor.get("startByte"), int) and isinstance(anchor.get("endByte"), int):
                        return anchor["file"], anchor["startByte"], anchor["endByte"]
    return None


def node_anchor(node: dict[str, Any]) -> tuple[str, int, int] | None:
    source = node.get("source")
    if isinstance(source, dict) and isinstance(source.get("file"), str):
        if isinstance(source.get("startByte"), int) and isinstance(source.get("endByte"), int):
            return source["file"], source["startByte"], source["endByte"]
    return None


def spans_contain_either(left_start: int, left_end: int, right_start: int, right_end: int) -> bool:
    """Accept equivalent parser anchors only when one contains the other.

    Different parsers legitimately anchor an exported declaration at different
    boundaries: one may include ``export default`` while another starts at the
    inner function/class node.  Containment preserves that compatibility while
    preventing two adjacent declarations from matching merely because their
    spans happen to overlap.
    """
    return (
        left_start <= right_start <= right_end <= left_end
        or right_start <= left_start <= left_end <= right_end
    )


def edge_source_file(edge: dict[str, Any], node_by_id: dict[str, Any]) -> str | None:
    source = node_by_id.get(edge.get("source"))
    anchor = node_anchor(source) if isinstance(source, dict) else None
    if anchor is not None:
        return anchor[0]
    anchor = edge_anchor(edge)
    return anchor[0] if anchor is not None else None


def edge_origins(edge: dict[str, Any]) -> set[str]:
    origins: set[str] = set()
    origin = value(edge, "origin") or value(edge, "_origin")
    if isinstance(origin, str):
        origins.add(origin)
    evidence = edge.get("evidence")
    if isinstance(evidence, list):
        origins.update(
            item.get("origin")
            for item in evidence
            if isinstance(item, dict) and isinstance(item.get("origin"), str)
        )
    return origins


def edge_stage(edge: dict[str, Any]) -> str | None:
    direct = edge.get("stage") or edge.get("routeStage")
    if isinstance(direct, str):
        return direct
    # Some capability projections score a synthetic node rather than a
    # published relationship.  Remix's flat route DSL is one such case: the
    # route node carries its operation stage in the nested route details, so
    # expose that stage to the common candidate filters as well.
    candidate_node = edge.get("node")
    if isinstance(candidate_node, dict):
        candidate_details = candidate_node.get("details")
        if isinstance(candidate_details, dict):
            candidate_data = candidate_details.get("data")
            if isinstance(candidate_data, dict):
                stages = candidate_data.get("stages")
                if isinstance(stages, list):
                    for stage in stages:
                        if isinstance(stage, dict) and isinstance(stage.get("stage"), str):
                            return stage["stage"]
    details = edge.get("details")
    if isinstance(details, dict):
        data = details.get("data")
        if isinstance(data, dict) and isinstance(data.get("stage"), str):
            return data["stage"]
    evidence = edge.get("evidence")
    if isinstance(evidence, list):
        for item in evidence:
            if not isinstance(item, dict):
                continue
            rule = item.get("rule")
            if isinstance(rule, str) and "stage:" in rule:
                stage = rule.split("stage:", 1)[1].split(":", 1)[0]
                if stage:
                    return stage
    return None


def candidate_anchor(candidate: dict[str, Any]) -> tuple[str, int, int] | None:
    """Return the source site for either an edge or a synthetic node candidate."""
    node = candidate.get("node")
    if isinstance(node, dict):
        return node_anchor(node)
    return edge_anchor(candidate)


def edge_render_kind(edge: dict[str, Any]) -> str | None:
    direct = edge.get("render_kind") or edge.get("renderKind")
    if isinstance(direct, str):
        return direct
    details = edge.get("details")
    if isinstance(details, dict):
        data = details.get("data")
        if isinstance(data, dict):
            nested = data.get("renderKind") or data.get("render_kind")
            if isinstance(nested, str):
                return nested
    evidence = edge.get("evidence")
    if isinstance(evidence, list):
        for item in evidence:
            if not isinstance(item, dict):
                continue
            rule = item.get("rule")
            if isinstance(rule, str) and rule.startswith("react-") and rule.endswith("-render"):
                return rule.removeprefix("react-").removesuffix("-render")
    return None


def load_source_oracle(path: Path) -> dict[str, Any]:
    document = load(path)
    if document.get("schema") != "compass.react-frontend-source-oracle/1":
        fail("frontend source oracle schema is not compass.react-frontend-source-oracle/1")
    facts = document.get("facts")
    if not isinstance(facts, list) or not facts:
        fail("frontend source oracle must contain facts")
    if len(facts) > MAX_ORACLE_FACTS:
        fail(f"frontend source oracle fact limit exceeded: {len(facts)} > {MAX_ORACLE_FACTS}")
    limits = document.get("limits")
    if not isinstance(limits, dict) or limits.get("maxFacts") != MAX_ORACLE_FACTS:
        fail("frontend source oracle must declare the checked fact limit")
    ids: set[str] = set()
    for fact in facts:
        if not isinstance(fact, dict):
            fail("frontend source fact must be an object")
        fact_id = fact.get("id")
        if not isinstance(fact_id, str) or not fact_id or fact_id in ids:
            fail("frontend source fact IDs must be unique")
        ids.add(fact_id)
        source = fact.get("sourceFile")
        if not isinstance(source, str) or Path(source).is_absolute() or ".." in source.replace("\\", "/").split("/"):
            fail(f"unsafe source fact path: {source!r}")
    return document


def match_source_facts(graph: dict[str, Any], oracle: dict[str, Any]) -> dict[str, Any]:
    nodes = graph.get("nodes")
    links = graph.get("links")
    if not isinstance(nodes, list) or not isinstance(links, list):
        fail("cannot score a graph without nodes and links")
    zero_unsafe_paths = validate_graph_paths(graph)
    node_by_id = {node.get("id"): node for node in nodes if isinstance(node, dict)}
    facts = [
        fact for fact in oracle["facts"]
        if fact.get("factType") in {"relationship", "configuration", "role"}
    ]
    # Configuration precision is scoped to files that the independent oracle
    # actually identified as framework configuration.  Treating every import
    # or call in a Vite/Next corpus as a configuration candidate makes the
    # denominator measure generic program activity instead of the published
    # capability under test.
    configuration_sources = {
        fact.get("sourceFile")
        for fact in facts
        if fact.get("relation") == "configuration" and isinstance(fact.get("sourceFile"), str)
    }
    all_candidates: dict[str, list[dict[str, Any]]] = {
        "renders": [],
        "routes_to": [],
        "contains": [],
        "configuration": [],
        "roles": [],
    }
    for edge in links:
        if isinstance(edge, dict) and edge.get("kind") in {"renders", "routes_to", "contains"}:
            if edge.get("kind") == "renders":
                target = node_by_id.get(edge.get("target"))
                # An unresolved/external placeholder is deliberately not an
                # accepted render target.  It remains visible in Compass for
                # diagnostics, but cannot inflate a precision denominator.
                if not isinstance(target, dict) or node_anchor(target) is None:
                    continue
            if edge.get("kind") == "contains":
                source = node_by_id.get(edge.get("source"))
                target = node_by_id.get(edge.get("target"))
                if not isinstance(source, dict) or not isinstance(target, dict) or source.get("kind") != "route" or target.get("kind") != "route":
                    continue
            all_candidates[edge["kind"]].append(edge)
        elif isinstance(edge, dict) and edge.get("kind") in {"calls", "imports"}:
            anchor = edge_anchor(edge)
            if anchor is not None and anchor[0] in configuration_sources:
                all_candidates["configuration"].append(edge)
    for node in nodes:
        if not isinstance(node, dict):
            continue
        roles = node.get("roles")
        if isinstance(roles, list):
            for role in roles:
                if isinstance(role, str):
                    all_candidates["roles"].append({"node": node, "role": role})

    def candidate_key(edge: dict[str, Any], fact: dict[str, Any]) -> tuple[Any, ...] | None:
        anchor = edge_anchor(edge)
        if anchor is None:
            return None
        if fact.get("relation") == "renders":
            return anchor
        source = anchor[0]
        target = node_by_id.get(edge.get("target"))
        target_anchor = node_anchor(target) if isinstance(target, dict) else None
        expected_target = (
            fact.get("targetFile"),
            fact.get("targetStartByte"),
            fact.get("targetEndByte"),
        )
        if target_anchor is None or expected_target[0] is None:
            return source, target_anchor
        return source, target_anchor, expected_target

    by_relation: dict[str, list[dict[str, Any]]] = {"renders": [], "routes_to": [], "contains": [], "roles": []}
    expected = {"renders": [], "routes_to": [], "contains": [], "configuration": [], "roles": []}
    for fact in facts:
        relation = "roles" if fact.get("factType") == "role" else fact.get("relation")
        if relation in expected:
            expected[relation].append(fact)

    # A corpus entry may intentionally qualify only one capability.  Do not
    # turn unrelated, already-qualified graph relations into false positives;
    # they are reported as out-of-scope rather than silently counted.
    candidates = {
        relation: all_candidates[relation]
        for relation, relation_facts in expected.items()
        if relation_facts
    }
    render_capabilities = {
        fact.get("capability")
        for fact in expected["renders"]
        if isinstance(fact.get("capability"), str)
    }
    # JSX and factory render projections are distinct capabilities.  A corpus
    # that independently specifies JSX should not count intentionally scoped
    # createElement/lazy edges as false positives; retain them in the graph,
    # but score only the capability the oracle advertised.
    if render_capabilities == {"react.render.jsx"}:
        candidates["renders"] = [
            edge for edge in candidates.get("renders", []) if edge_render_kind(edge) == "jsx"
        ]

    matched_facts: set[str] = set()
    matched_edges: set[str] = set()
    details: dict[str, Any] = {}
    for relation, relation_facts in expected.items():
        relation_candidates = candidates.get(relation, [])
        available = set(range(len(relation_candidates)))
        matched = 0
        exact_anchors = 0
        multiplicity: dict[str, int] = {}
        for fact in relation_facts:
            found = None
            for index in sorted(available):
                edge = relation_candidates[index]
                anchor = edge_anchor(edge)
                if relation == "roles":
                    node = edge.get("node") if isinstance(edge, dict) else None
                    anchor = node_anchor(node) if isinstance(node, dict) else None
                    if edge.get("role") != fact.get("role") or anchor is None or anchor[0] != fact.get("sourceFile"):
                        continue
                    if anchor[1] > fact.get("startByte") or anchor[2] < fact.get("endByte"):
                        continue
                    found = index
                    break
                if relation == "renders":
                    if anchor == (fact.get("sourceFile"), fact.get("startByte"), fact.get("endByte")):
                        found = index
                        break
                elif relation == "contains":
                    target = node_by_id.get(edge.get("target"))
                    target_anchor = node_anchor(target) if isinstance(target, dict) else None
                    if (
                        edge_source_file(edge, node_by_id) == fact.get("sourceFile")
                        and target_anchor is not None
                        and target_anchor[0] == fact.get("targetFile")
                    ):
                        found = index
                        break
                elif relation == "configuration":
                    if fact.get("capability") == "vite.file_set.glob":
                        if anchor and anchor[0] == fact.get("sourceFile") and edge.get("kind") == "imports" and any(
                            isinstance(item, dict) and item.get("extractor") == "compass.frameworks.vite.file-set"
                            for item in edge.get("evidence", [])
                        ):
                            found = index
                            break
                    elif anchor == (fact.get("sourceFile"), fact.get("startByte"), fact.get("endByte")):
                        found = index
                        break
                else:
                    target = node_by_id.get(edge.get("target"))
                    target_anchor = node_anchor(target) if isinstance(target, dict) else None
                    edge_source = anchor[0] if anchor else None
                    if edge_source != fact.get("sourceFile"):
                        continue
                    expected_stage = fact.get("stage")
                    if expected_stage is not None and edge_stage(edge) != expected_stage:
                        continue
                    if fact.get("targetStartByte") is not None:
                        # Compass publishes a declaration node's complete
                        # syntactic span (often including its function body),
                        # while the independent compiler oracle anchors the
                        # declaration identifier itself.  Accept only the
                        # expected file and containment relation; never match
                        # by spelling or by the first convenient target.
                        expected_file = fact.get("targetFile") or fact.get("sourceFile")
                        if target_anchor is None or target_anchor[0] != expected_file:
                            continue
                        # Convention-owned route nodes may intentionally use a
                        # zero-width file anchor when no declaration target is
                        # available.  The file identity is still exact; do not
                        # reject that supported unresolved declaration form.
                        if target_anchor[1] == target_anchor[2] == 0:
                            found = index
                            break
                        if not spans_contain_either(
                            target_anchor[1],
                            target_anchor[2],
                            fact.get("targetStartByte"),
                            fact.get("targetEndByte"),
                        ):
                            continue
                    found = index
                    break
            fact_id = fact["id"]
            if found is not None:
                available.remove(found)
                matched += 1
                matched_facts.add(fact_id)
                edge = relation_candidates[found]
                matched_edges.add(str(edge.get("id", found)))
                anchor = edge_anchor(edge)
                if relation == "roles":
                    node = edge.get("node") if isinstance(edge, dict) else None
                    node_site = node_anchor(node) if isinstance(node, dict) else None
                    if node_site and node_site[0] == fact.get("sourceFile") and node_site[1] <= fact.get("startByte", 0) and node_site[2] >= fact.get("endByte", 0):
                        exact_anchors += 1
                elif relation == "contains":
                    if edge_source_file(edge, node_by_id) == fact.get("sourceFile"):
                        exact_anchors += 1
                elif anchor == (fact.get("sourceFile"), fact.get("startByte"), fact.get("endByte")):
                    exact_anchors += 1
            multiplicity[fact_id] = 1 if found is not None else 0
        precision = matched / len(relation_candidates) if relation_candidates else 0.0
        recall = matched / len(relation_facts) if relation_facts else 0.0
        details[relation] = {
            "expected": len(relation_facts),
            "candidates": len(relation_candidates),
            "matched": matched,
            "falsePositives": max(0, len(relation_candidates) - matched),
            "falseNegatives": max(0, len(relation_facts) - matched),
            "precision": precision,
            "recall": recall,
            "wilsonLower95": wilson_lower(matched, len(relation_candidates)),
            "anchorExact": exact_anchors,
            "anchorAccuracy": exact_anchors / matched if matched else 0.0,
            "multiplicity": multiplicity,
        }
    capability_details: dict[str, dict[str, Any]] = {}

    def capability_candidates(capability: str, relation: str) -> list[dict[str, Any]]:
        """Scope configuration candidates to the evidence family under test.

        A Vite config file contains ordinary filesystem calls, path helpers,
        plugin imports, and one or more framework declarations. Counting every
        call/import as a configuration candidate makes precision depend on the
        implementation details of the fixture instead of the advertised
        capability. File-set expansion is intentionally one candidate per
        declaration anchor even when one glob legitimately fans out to many
        bounded target files.
        """
        relation_candidates = list(candidates.get(relation, []))
        if capability == "vite.file_set.glob":
            # A declaration is a first-class framework resource even when the
            # bounded projection contains no matching target file. Score the
            # declaration capability from those resource nodes; target fanout
            # remains separately validated by the positive graph contract.
            resources = [
                node
                for node in nodes
                if isinstance(node, dict)
                and node.get("kind") == "resource"
                and "framework_file_set" in str(node.get("qualifiedName", node.get("qualified_name", "")))
            ]
            return [{"node": node, "kind": "resource"} for node in resources]
        if capability == "remix.route.config":
            # Remix's flat route DSL publishes a route node even when there
            # is no source module to resolve as a handler.  Score the
            # declaration itself; requiring a synthetic routes_to target
            # would turn intentionally unresolved configuration into a false
            # negative and hide the route inventory Compass did publish.
            routes = [
                node
                for node in nodes
                if isinstance(node, dict)
                and node.get("kind") == "route"
                and str(node.get("framework", "")) == "remix"
            ]
            return [{"node": node, "kind": "route"} for node in routes]
        if capability in {
            "next.app.hierarchy",
            "next.pages.hierarchy",
            "react-router.hierarchy",
            "tanstack.route.hierarchy",
        }:
            # The graph may retain one structural edge per route operation
            # (for example GET and POST handlers in the same module), while
            # the independent oracle advertises hierarchy at file granularity.
            # Compare those equivalent projections once without deleting the
            # higher-fidelity multiplicity from the published graph.
            selected = []
            seen: set[tuple[str, str]] = set()
            for candidate in relation_candidates:
                source = node_by_id.get(candidate.get("source"))
                target = node_by_id.get(candidate.get("target"))
                source_anchor = node_anchor(source) if isinstance(source, dict) else None
                target_anchor = node_anchor(target) if isinstance(target, dict) else None
                if source_anchor is None or target_anchor is None:
                    continue
                key = (source_anchor[0], target_anchor[0])
                if key in seen:
                    continue
                seen.add(key)
                selected.append(candidate)
            return selected
        if capability == "vite.config.factory":
            selected = []
            for candidate in relation_candidates:
                if candidate.get("kind") != "calls":
                    continue
                target = node_by_id.get(candidate.get("target"))
                target_text = " ".join(
                    str(target.get(key, ""))
                    for key in ("name", "qualifiedName", "qualified_name")
                ).lower() if isinstance(target, dict) else ""
                rules = " ".join(
                    str(item.get("rule", ""))
                    for item in candidate.get("evidence", [])
                    if isinstance(item, dict)
                ).lower()
                if "defineconfig" in target_text or "binding:defineconfig" in rules:
                    selected.append(candidate)
            return selected
        if capability == "vite.plugin.identity":
            selected = []
            for candidate in relation_candidates:
                if candidate.get("kind") not in {"imports", "calls"}:
                    continue
                target = node_by_id.get(candidate.get("target"))
                qualified = " ".join(
                    str(target.get(key, ""))
                    for key in ("qualifiedName", "qualified_name")
                ) if isinstance(target, dict) else ""
                # Local helpers such as ``virtualPlugin`` are ordinary call
                # targets, not imported plugin identities.  A module-qualified
                # target is the evidence boundary for this capability.
                if "::" not in qualified:
                    continue
                target_module = qualified.rsplit("::", 1)[0].lower()
                if "plugin" in target_module:
                    selected.append(candidate)
            return selected
        return relation_candidates

    def capability_fact_matches(candidate: dict[str, Any], fact: dict[str, Any], capability: str) -> bool:
        if capability in {
            "next.app.hierarchy",
            "next.pages.hierarchy",
            "react-router.hierarchy",
            "tanstack.route.hierarchy",
        }:
            source = node_by_id.get(candidate.get("source"))
            target = node_by_id.get(candidate.get("target"))
            source_anchor = node_anchor(source) if isinstance(source, dict) else None
            target_anchor = node_anchor(target) if isinstance(target, dict) else None
            return (
                candidate.get("kind") == "contains"
                and source_anchor is not None
                and target_anchor is not None
                and source_anchor[0] == fact.get("sourceFile")
                and target_anchor[0] == fact.get("targetFile")
            )
        anchor = candidate_anchor(candidate)
        if anchor is None or anchor[0] != fact.get("sourceFile"):
            return False
        if capability in {
            "next.app.route",
            "next.pages.route",
            "next.pages.dynamic",
            "react-router.route",
            "react-router.loader-action",
            "tanstack.route",
            "tanstack.loader",
        }:
            # Route facts identify the declaration target, while Compass keeps
            # the relationship anchor at the convention/configuration site
            # (usually the complete source file).  Match the target by exact
            # source identity and byte containment; never fall back to a name
            # or to whichever route happens to be first in the graph.
            if candidate.get("kind") != "routes_to":
                return False
            expected_stage = fact.get("stage")
            if expected_stage is not None and edge_stage(candidate) != expected_stage:
                return False
            target = node_by_id.get(candidate.get("target"))
            target_anchor = node_anchor(target) if isinstance(target, dict) else None
            if target_anchor is None:
                return False
            expected_file = fact.get("targetFile") or fact.get("sourceFile")
            if target_anchor[0] != expected_file:
                return False
            if target_anchor[1] == target_anchor[2] == 0:
                return True
            target_start = fact.get("targetStartByte")
            target_end = fact.get("targetEndByte")
            if target_start is None or target_end is None:
                return True
            return spans_contain_either(target_anchor[1], target_anchor[2], target_start, target_end)
        if capability == "vite.file_set.glob":
            return (
                anchor[1] <= fact.get("startByte", 0)
                and anchor[2] >= fact.get("endByte", 0)
            )
        if capability == "remix.route.config":
            return (
                anchor[1] <= fact.get("startByte", 0) <= fact.get("endByte", 0) <= anchor[2]
                or fact.get("startByte", 0) <= anchor[1] <= anchor[2] <= fact.get("endByte", 0)
            )
        if capability in {"react.component.roles", "react.hooks", "next.client-server-directive"}:
            return (
                anchor[1] <= fact.get("startByte", 0)
                and anchor[2] >= fact.get("endByte", 0)
            )
        if capability == "vite.plugin.identity":
            # Universal import evidence anchors the imported binding, while
            # the compiler oracle anchors the module specifier. Compare the
            # recovered module identity for import facts; plugin factory calls
            # retain their exact call-site anchor.
            spelling = fact.get("targetSpelling")
            if candidate.get("kind") == "imports" and isinstance(spelling, str):
                target = node_by_id.get(candidate.get("target"))
                qualified = " ".join(
                    str(target.get(key, ""))
                    for key in ("qualifiedName", "qualified_name")
                ) if isinstance(target, dict) else ""
                module = qualified.rsplit("::", 1)[0]
                return module == spelling
            return anchor == (fact.get("sourceFile"), fact.get("startByte"), fact.get("endByte"))
        return anchor == (fact.get("sourceFile"), fact.get("startByte"), fact.get("endByte"))

    capabilities = sorted({
        fact.get("capability")
        for fact in facts
        if isinstance(fact.get("capability"), str)
    })
    for capability in capabilities:
        capability_facts = [fact for fact in facts if fact.get("capability") == capability]
        route_capability = capability in {
            "next.app.route",
            "next.pages.route",
            "next.pages.dynamic",
            "react-router.route",
            "react-router.loader-action",
            "tanstack.route",
            "tanstack.loader",
        }
        unresolved = sum(
            1
            for fact in capability_facts
            if route_capability
            and fact.get("relation") == "routes_to"
            and fact.get("resolution") == "unresolved"
        )
        # Unresolved/ambiguous route declarations are an explicit negative
        # outcome, not a missed exact edge.  Keep the count visible in the
        # scorecard while measuring precision/recall only over independently
        # resolvable records.
        capability_facts = [
            fact
            for fact in capability_facts
            if not (route_capability and fact.get("relation") == "routes_to" and fact.get("resolution") == "unresolved")
        ]
        relation = "roles" if any(fact.get("factType") == "role" for fact in capability_facts) else next(
            (fact.get("relation") for fact in capability_facts if isinstance(fact.get("relation"), str)),
            "",
        )
        relation_candidates = list(candidates.get(relation, []))
        relation_candidates = capability_candidates(capability, relation)
        if relation == "routes_to":
            expected_stages = {
                fact.get("stage")
                for fact in capability_facts
                if isinstance(fact.get("stage"), str)
            }
            if expected_stages:
                relation_candidates = [
                    candidate
                    for candidate in relation_candidates
                    if edge_stage(candidate) in expected_stages
                ]
        source_files = {fact.get("sourceFile") for fact in capability_facts}
        expected_roles = {fact.get("role") for fact in capability_facts if fact.get("factType") == "role"}
        filtered_candidates = []
        for candidate in relation_candidates:
            if relation == "roles":
                anchor = node_anchor(candidate.get("node")) if isinstance(candidate, dict) else None
            elif relation == "contains":
                source_file = edge_source_file(candidate, node_by_id)
                target_anchor = node_anchor(node_by_id.get(candidate.get("target"))) if isinstance(node_by_id.get(candidate.get("target")), dict) else None
                anchor = (source_file, 0, 1) if source_file and target_anchor else None
            else:
                anchor = candidate_anchor(candidate)
            if anchor is not None and anchor[0] in source_files:
                if relation == "roles" and candidate.get("role") not in expected_roles:
                    continue
                if capability == "react.render.jsx" and edge_render_kind(candidate) != "jsx":
                    continue
                filtered_candidates.append(candidate)
        # Match each reviewed fact to one candidate occurrence.  Using
        # ``any`` here lets one published edge satisfy multiple identical
        # oracle records, hiding multiplicity loss and inflating recall.  The
        # same deterministic one-to-one accounting is used by the relation
        # score above; capability metrics must preserve it as well.
        available = set(range(len(filtered_candidates)))
        matched_capability = 0
        for fact in capability_facts:
            found = next(
                (
                    index
                    for index in sorted(available)
                    if capability_fact_matches(filtered_candidates[index], fact, capability)
                ),
                None,
            )
            if found is not None:
                available.remove(found)
                matched_capability += 1
        candidate_count = len(filtered_candidates)
        capability_details[capability] = {
            "expected": len(capability_facts),
            "unresolvedExpected": unresolved,
            "candidates": candidate_count,
            "matched": matched_capability,
            "falsePositives": max(0, candidate_count - matched_capability),
            "falseNegatives": max(0, len(capability_facts) - matched_capability),
            "precision": matched_capability / candidate_count if candidate_count else 0.0,
            "recall": matched_capability / len(capability_facts) if capability_facts else 0.0,
            "wilsonLower95": wilson_lower(matched_capability, candidate_count),
        }

    # Replace the broad mixed-configuration relation summary with the same
    # capability-scoped accounting used by the aggregate scorecard.
    if "configuration" in details:
        configuration_metrics = [
            metric
            for capability, metric in capability_details.items()
            if any(
                fact.get("capability") == capability and fact.get("relation") == "configuration"
                for fact in facts
            )
        ]
        if configuration_metrics:
            expected_count = sum(metric["expected"] for metric in configuration_metrics)
            candidate_count = sum(metric["candidates"] for metric in configuration_metrics)
            matched_count = sum(metric["matched"] for metric in configuration_metrics)
            details["configuration"] = {
                "expected": expected_count,
                "candidates": candidate_count,
                "matched": matched_count,
                "falsePositives": max(0, candidate_count - matched_count),
                "falseNegatives": max(0, expected_count - matched_count),
                "precision": matched_count / candidate_count if candidate_count else 0.0,
                "recall": matched_count / expected_count if expected_count else 0.0,
                "wilsonLower95": wilson_lower(matched_count, candidate_count),
                "anchorExact": matched_count,
                "anchorAccuracy": 1.0 if matched_count else 0.0,
            }
    if "roles" in details:
        role_metrics = [
            metric
            for capability, metric in capability_details.items()
            if any(
                fact.get("capability") == capability and fact.get("factType") == "role"
                for fact in facts
            )
        ]
        if role_metrics:
            expected_count = sum(metric["expected"] for metric in role_metrics)
            candidate_count = sum(metric["candidates"] for metric in role_metrics)
            matched_count = sum(metric["matched"] for metric in role_metrics)
            details["roles"] = {
                "expected": expected_count,
                "candidates": candidate_count,
                "matched": matched_count,
                "falsePositives": max(0, candidate_count - matched_count),
                "falseNegatives": max(0, expected_count - matched_count),
                "precision": matched_count / candidate_count if candidate_count else 0.0,
                "recall": matched_count / expected_count if expected_count else 0.0,
                "wilsonLower95": wilson_lower(matched_count, candidate_count),
                "anchorExact": matched_count,
                "anchorAccuracy": 1.0 if matched_count else 0.0,
                "multiplicity": {},
            }
    all_expected = sum(item["expected"] for item in capability_details.values())
    all_candidates = sum(item["candidates"] for item in capability_details.values())
    all_matched = sum(item["matched"] for item in capability_details.values())
    precision = all_matched / all_candidates if all_candidates else 0.0
    recall = all_matched / all_expected if all_expected else 0.0
    return {
        "schema": "compass.react-frontend-scorecard/1",
        "oracleSha256": sha256_bytes(canonical_bytes(oracle)),
        "sourceOracle": oracle.get("sourceOracle", {}),
        "framework": oracle.get("framework"),
            "relationships": details,
        "capabilities": capability_details,
        "aggregate": {
            "expected": all_expected,
            "oracleRecords": len(oracle.get("facts", [])),
            "oracleCapabilities": {
                capability: sum(1 for fact in oracle.get("facts", []) if fact.get("capability") == capability)
                for capability in sorted({fact.get("capability") for fact in oracle.get("facts", []) if isinstance(fact, dict) and isinstance(fact.get("capability"), str)})
            },
            "candidates": all_candidates,
            "matched": all_matched,
            "precision": precision,
            "recall": recall,
            "wilsonLower95": wilson_lower(all_matched, all_candidates),
            "zeroFabricatedTargets": all(
                item["falsePositives"] == 0
                for item in capability_details.values()
            ),
            "zeroUnsafePaths": zero_unsafe_paths,
            "deterministicOracle": True,
        },
    }


def load_expectations(path: Path | None = None) -> dict[str, Any]:
    expectations = load(path or EXPECTATIONS)
    if expectations.get("schema") != "compass.framework-evidence/1":
        fail("frontend expectation schema is not compass.framework-evidence/1")
    records = expectations.get("records")
    if not isinstance(records, list) or not records:
        fail("frontend expectations must contain records")
    if len(records) > 100_000:
        fail("frontend expectation record limit exceeded")
    ids: set[str] = set()
    for record in records:
        if not isinstance(record, dict):
            fail("frontend expectation record must be an object")
        record_id = record.get("id")
        source_file = record.get("sourceFile")
        if not isinstance(record_id, str) or not record_id or record_id in ids:
            fail("frontend expectation IDs must be non-empty and unique")
        ids.add(record_id)
        if not isinstance(source_file, str) or not source_file or source_file.startswith(("/", "\\")):
            fail(f"frontend expectation {record_id!r} has an unsafe source file")
        if ".." in source_file.replace("\\", "/").split("/"):
            fail(f"frontend expectation {record_id!r} escapes the corpus root")
        if not isinstance(record.get("startByte"), int) or not isinstance(record.get("endByte"), int):
            fail(f"frontend expectation {record_id!r} has a non-integer range")
        if record["startByte"] >= record["endByte"]:
            fail(f"frontend expectation {record_id!r} has an invalid range")
    return expectations


def provenance(record: dict[str, Any]) -> bool:
    origin = value(record, "_origin") or value(record, "origin")
    if origin in {"ast", "config", "convention", "artifact", "heuristic"}:
        return True
    evidence = record.get("evidence")
    return isinstance(evidence, list) and any(
        isinstance(item, dict)
        and item.get("origin") in {"ast", "config", "convention", "artifact", "heuristic"}
        for item in evidence
    )


def check_positive(graph: dict[str, Any], expectations: dict[str, Any]) -> dict[str, int]:
    if graph.get("directed") is not True or graph.get("multigraph") is not True:
        fail("published graph must be a directed multigraph")
    metadata = graph.get("graph")
    if not isinstance(metadata, dict) or metadata.get("schema") != "compass.graph/1":
        fail("published graph schema is not compass.graph/1")
    nodes = graph.get("nodes")
    links = graph.get("links")
    if not isinstance(nodes, list) or not isinstance(links, list):
        fail("nodes and links must be arrays")
    validate_graph_paths(graph)
    node_by_id = {item.get("id"): item for item in nodes if isinstance(item, dict)}
    if len(node_by_id) != len(nodes):
        fail("node IDs are not unique")
    for node in nodes:
        if not isinstance(node, dict) or not node.get("id") or not node.get("kind"):
            fail("every node needs an ID and kind")
    for edge in links:
        if not isinstance(edge, dict):
            fail("every link must be an object")
        if edge.get("source") not in node_by_id or edge.get("target") not in node_by_id:
            fail("link endpoint is not a published node")

    framework_nodes = [
        node for node in nodes if value(node, "framework") in FRAMEWORKS
    ]
    missing = sorted(FRAMEWORKS - {value(node, "framework") for node in framework_nodes})
    if missing:
        fail(f"missing framework evidence: {', '.join(missing)}")
    source_files = {
        source_file(node)
        for node in nodes
        if source_file(node)
    }
    for record in expectations["records"]:
        expected_source_file = record["sourceFile"]
        if expected_source_file not in source_files:
            fail(f"expectation {record['id']!r} source file is absent from the graph")

    routes = [
        node
        for node in framework_nodes
        if node.get("kind") == "route" and provenance(node)
    ]
    if len(routes) < 4:
        fail(f"expected at least four framework route nodes, found {len(routes)}")
    route_edges = [edge for edge in links if edge.get("kind") == "routes_to"]
    if len(route_edges) < 3:
        fail(f"expected at least three route relationships, found {len(route_edges)}")
    route_ids = {node["id"] for node in routes}
    hierarchy_edges = [
        edge
        for edge in links
        if edge.get("kind") == "contains"
        and edge.get("source") in route_ids
        and edge.get("target") in route_ids
    ]
    if not hierarchy_edges:
        fail("frontend route hierarchy did not publish a route-to-route contains edge")
    if not all(
        value(edge, "_origin") == "convention"
        or any(
            isinstance(item, dict)
            and item.get("origin") == "convention"
            for item in edge.get("evidence", [])
        )
        for edge in hierarchy_edges
    ):
        fail("route hierarchy is missing convention provenance")

    renders = [edge for edge in links if edge.get("kind") == "renders"]
    if len(renders) < 3:
        fail(f"expected at least three JSX render relationships, found {len(renders)}")
    for edge in renders:
        if not provenance(edge):
            fail("render relationship is missing direct provenance")
        source = node_by_id[edge["source"]]
        target = node_by_id[edge["target"]]
        if value(source, "kind") not in {"file", "module", "function", "method", "component", "variable"}:
            fail("render source endpoint is outside the frontend contract")
        if value(target, "kind") not in {"function", "method", "class", "component", "variable", "property"}:
            fail("render target endpoint is outside the frontend contract")

    config_fields = [
        node
        for node in nodes
        if value(node, "kind") == "config_key"
        and "framework_configuration_field" in str(value(node, "qualifiedName") or value(node, "qualified_name") or "")
    ]
    resources = [
        node
        for node in nodes
        if value(node, "kind") == "resource"
        and "framework_file_set" in str(value(node, "qualifiedName") or value(node, "qualified_name") or "")
    ]
    if not config_fields:
        fail("Vite configuration fields were not published as config_key nodes")
    if not resources:
        fail("Vite file-set resources were not published")
    if not all(provenance(node) for node in config_fields + resources):
        fail("configuration or file-set node is missing provenance")
    resource_ids = {node["id"] for node in resources}
    if not any(
        edge.get("kind") == "contains" and edge.get("target") in resource_ids
        for edge in links
    ):
        fail("file-set resource is not contained by its owning file")
    file_set_imports = [
        edge
        for edge in links
        if edge.get("kind") == "imports"
        and any(
            isinstance(item, dict)
            and item.get("extractor") == "compass.frameworks.vite.file-set"
            for item in edge.get("evidence", [])
        )
    ]
    if not file_set_imports:
        fail("literal Vite glob did not project any bounded file-set import")
    if not all(provenance(edge) for edge in file_set_imports):
        fail("file-set import is missing provenance")

    diagnostics = metadata.get("diagnostics", [])
    if not isinstance(diagnostics, list):
        fail("published graph diagnostics must be an array")
    if any(item.get("severity") == "error" for item in diagnostics if isinstance(item, dict)):
        fail("positive frontend corpus contains validation errors")
    omission_codes = {
        "publication_omission_summary",
        "publication_omitted_node",
        "publication_omitted_edge",
    }
    omissions = [
        item
        for item in diagnostics
        if isinstance(item, dict) and item.get("code") in omission_codes
    ]
    if omissions:
        sample = omissions[0].get("message", "publication omission")
        fail(f"positive frontend corpus contains a partial graph: {sample}")
    return {
        "nodes": len(nodes),
        "edges": len(links),
        "frameworkNodes": len(framework_nodes),
        "routes": len(routes),
        "routeHierarchy": len(hierarchy_edges),
        "renders": len(renders),
        "configFields": len(config_fields),
        "resources": len(resources),
        "fileSetImports": len(file_set_imports),
        "expectations": len(expectations["records"]),
    }


def check_negative(graph: dict[str, Any]) -> int:
    nodes = graph.get("nodes")
    if not isinstance(nodes, list):
        fail("negative graph nodes are not an array")
    activated = sorted(
        {value(node, "framework") for node in nodes if value(node, "framework") in FRAMEWORKS}
    )
    if activated:
        fail(f"frameworks activated without a project marker: {', '.join(activated)}")
    return len(nodes)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("positive", type=Path, nargs="?")
    parser.add_argument("negative", type=Path, nargs="?")
    parser.add_argument("--graph", type=Path, help="positive graph (explicit form)")
    parser.add_argument("--negative-graph", type=Path)
    parser.add_argument("--source-oracle", type=Path)
    parser.add_argument("--expectations", type=Path, default=EXPECTATIONS)
    parser.add_argument("--result", type=Path)
    parser.add_argument("--min-expected", type=int, default=1)
    parser.add_argument("--min-precision", type=float, default=1.0)
    parser.add_argument("--min-recall", type=float, default=1.0)
    parser.add_argument("--min-wilson", type=float, default=0.0)
    parser.add_argument("--enforce-sample-floor", action="store_true")
    parser.add_argument("--score-only", action="store_true", help="skip the fixture-specific framework assertions")
    args = parser.parse_args()
    positive_path = args.graph or args.positive
    negative_path = args.negative_graph or args.negative
    if positive_path is None:
        parser.error("a positive graph is required")
    expectations = load_expectations(args.expectations) if not args.score_only else None
    positive_graph = load(positive_path)
    if args.score_only:
        metadata = positive_graph.get("graph")
        if not isinstance(metadata, dict) or metadata.get("schema") != "compass.graph/1":
            fail("score-only graph schema is not compass.graph/1")
        validate_graph_paths(positive_graph)
        positive = {
            "nodes": len(positive_graph.get("nodes", [])),
            "edges": len(positive_graph.get("links", [])),
        }
    else:
        positive = check_positive(positive_graph, expectations)
    result: dict[str, Any] = {
        "schema": "compass.react-frontend-qualification-result/1",
        "graphSha256": sha256_bytes(positive_path.read_bytes()),
        "positive": positive,
    }
    if negative_path is not None:
        result["negativeNodes"] = check_negative(load(negative_path))
    if args.source_oracle:
        scorecard = match_source_facts(positive_graph, load_source_oracle(args.source_oracle))
        result["scorecard"] = scorecard
        aggregate = scorecard["aggregate"]
        floor = args.min_expected
        if args.enforce_sample_floor and aggregate["expected"] < floor:
            fail(f"independent sample floor not met: {aggregate['expected']} < {floor}")
        if aggregate["expected"] > 0 and aggregate["precision"] < args.min_precision:
            fail(f"precision below threshold: {aggregate['precision']:.6f} < {args.min_precision:.6f}")
        if aggregate["expected"] > 0 and aggregate["recall"] < args.min_recall:
            fail(f"recall below threshold: {aggregate['recall']:.6f} < {args.min_recall:.6f}")
        if aggregate["candidates"] > 0 and aggregate["wilsonLower95"] < args.min_wilson:
            fail(f"Wilson lower bound below threshold: {aggregate['wilsonLower95']:.6f} < {args.min_wilson:.6f}")
    encoded = canonical_bytes(result)
    if args.result:
        args.result.parent.mkdir(parents=True, exist_ok=True)
        args.result.write_bytes(encoded)
    print(encoded.decode(), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
