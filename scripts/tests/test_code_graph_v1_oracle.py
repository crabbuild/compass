#!/usr/bin/env python3
"""Post-implementation regression tests for the code-graph v1 oracle."""

from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from code_graph_v1_oracle import (  # noqa: E402
    QualificationError,
    assert_coverage,
    assert_flows,
    assert_negatives,
    assert_route_containment_negatives,
    assert_topology,
    canonical_bytes,
    endpoint_allowed,
    load_json,
    load_manifest,
    load_topology_policy,
    qualification_summary,
    topology_metrics,
    topology_report,
    validate_graph,
)


def anchor(file: str = "sample.py", start: int = 0, end: int = 1) -> dict[str, object]:
    return {
        "file": file,
        "startByte": start,
        "endByte": end,
        "startLine": 1,
        "startColumn": start,
        "endLine": 1,
        "endColumn": end,
    }


def evidence(file: str = "sample.py") -> list[dict[str, object]]:
    return [{
        "extractor": "compass.languages.python",
        "origin": "ast",
        "confidence": "exact",
        "anchors": [anchor(file)],
    }]


def node(identity: str, kind: str, *, qualified: str | None = None) -> dict[str, object]:
    return {
        "id": identity,
        "kind": kind,
        "name": identity,
        "qualifiedName": qualified or identity,
        "language": "python",
        "roles": [],
        "source": anchor(),
        "evidence": evidence(),
    }


class OracleTests(unittest.TestCase):
    maxDiff = None

    def setUp(self) -> None:
        self.manifest = load_json(
            ROOT / "tests/qualification/code-graph-v1-semantic.json"
        )

    def graph(self, nodes: list[dict], links: list[dict] | None = None) -> dict:
        return {
            "directed": True,
            "multigraph": True,
            "graph": {
                "schema": "compass.graph/1",
                "files": [{
                    "id": "file:sample",
                    "path": "sample.py",
                    "byteSize": 8,
                    "extractionStatus": "extracted",
                }],
                "coverage": [],
                "diagnostics": [],
            },
            "nodes": nodes,
            "links": links or [],
        }

    def test_canonical_bytes_are_order_independent(self) -> None:
        self.assertEqual(canonical_bytes({"b": 2, "a": 1}), canonical_bytes({"a": 1, "b": 2}))

    def test_endpoint_matrix_rejects_inheritance_to_variable(self) -> None:
        self.assertFalse(endpoint_allowed({"kind": "class"}, {"kind": "extends"}, {"kind": "variable"}))

    def test_endpoint_matrix_accepts_explicit_route_stage_variables(self) -> None:
        for role in ("route_handler", "service", "middleware"):
            self.assertTrue(endpoint_allowed(
                {"kind": "route"},
                {"kind": "routes_to"},
                {"kind": "variable", "roles": [role]},
            ))
        self.assertFalse(endpoint_allowed(
            {"kind": "route"},
            {"kind": "routes_to"},
            {"kind": "variable", "roles": []},
        ))

    def test_validate_graph_accepts_unresolved_route_stage_placeholder(self) -> None:
        route = node("route:1", "route")
        placeholder = node("symbol:1", "variable")
        placeholder.update({
            "language": "typescript",
            "qualifiedName": "./handlers::authenticate",
            "details": {"type": "symbol", "data": {}},
            "evidence": [{
                "extractor": "compass.graph.external-placeholder",
                "origin": "heuristic",
                "confidence": "inferred",
                "rule": "external-symbol-placeholder",
                "wiringSite": anchor("sample.py", 0, 1),
            }],
        })
        route["details"] = {"type": "route", "data": {"stages": [{
            "sourceAnchor": anchor("sample.py", 0, 1),
            "candidates": [{"nodeId": placeholder["id"]}],
        }]}}
        validate_graph(self.graph([route, placeholder]), self.manifest)

    def test_endpoint_matrix_accepts_dart_implicit_interface_classes(self) -> None:
        self.assertTrue(endpoint_allowed(
            {"kind": "class", "language": "dart"},
            {"kind": "implements"},
            {"kind": "class", "language": "dart"},
        ))
        self.assertFalse(endpoint_allowed(
            {"kind": "class", "language": "csharp"},
            {"kind": "implements"},
            {"kind": "class", "language": "csharp"},
        ))

    def test_endpoint_matrix_accepts_top_level_instantiations(self) -> None:
        for source_kind in ("file", "module"):
            with self.subTest(source_kind=source_kind):
                self.assertTrue(endpoint_allowed(
                    {"kind": source_kind},
                    {"kind": "instantiates"},
                    {"kind": "class", "language": "python"},
                ))

    def test_endpoint_matrix_accepts_nested_config_containment(self) -> None:
        self.assertTrue(endpoint_allowed(
            {"kind": "config_key"},
            {"kind": "contains"},
            {"kind": "config_key"},
        ))

    def test_endpoint_matrix_accepts_object_variable_properties(self) -> None:
        self.assertTrue(endpoint_allowed(
            {"kind": "variable"},
            {"kind": "contains"},
            {"kind": "property"},
        ))

    def test_endpoint_matrix_accepts_only_rust_enum_member_instantiations(self) -> None:
        self.assertTrue(endpoint_allowed(
            {"kind": "function"},
            {"kind": "instantiates"},
            {"kind": "enum_member", "language": "rust"},
        ))
        self.assertFalse(endpoint_allowed(
            {"kind": "function"},
            {"kind": "instantiates"},
            {"kind": "enum_member", "language": "python"},
        ))

    def test_endpoint_matrix_accepts_scoped_generic_parameter_relationships(self) -> None:
        for source_kind, relation, target_kind in (
            ("parameter", "references", "parameter"),
            ("parameter", "references", "trait"),
            ("field", "type_of", "parameter"),
            ("function", "returns", "parameter"),
        ):
            with self.subTest(
                source_kind=source_kind,
                relation=relation,
                target_kind=target_kind,
            ):
                self.assertTrue(endpoint_allowed(
                    {"kind": source_kind},
                    {"kind": relation},
                    {"kind": target_kind},
                ))

    def test_validate_graph_rejects_unknown_producer(self) -> None:
        item = node("function:a", "function")
        item["evidence"][0]["extractor"] = "compass.languages.unknown"
        with self.assertRaisesRegex(QualificationError, "unknown_producer"):
            validate_graph(self.graph([item]), self.manifest)

    def test_validate_graph_rejects_out_of_bounds_anchor(self) -> None:
        item = node("function:a", "function")
        item["source"]["endByte"] = 9
        with self.assertRaisesRegex(QualificationError, "invalid_anchor"):
            validate_graph(self.graph([item]), self.manifest)

    def test_validate_graph_rejects_non_recursive_self_loop(self) -> None:
        item = node("function:a", "function")
        edge = {
            "id": "edge:1",
            "key": "edge:1",
            "kind": "references",
            "source": item["id"],
            "target": item["id"],
            "relationshipSite": anchor(),
            "evidence": evidence(),
        }
        with self.assertRaisesRegex(QualificationError, "non_recursive_self_loop"):
            validate_graph(self.graph([item], [edge]), self.manifest)

    def test_negative_rejects_exact_route(self) -> None:
        route = node("route:1", "route")
        route["framework"] = "near-match"
        route["source"]["file"] = "negative.py"
        route["details"] = {"type": "route", "data": {"resolution": "exact"}}
        manifest = {"negatives": [{
            "id": "negative",
            "routeFramework": "near-match",
            "source": "negative.py",
        }]}
        with self.assertRaisesRegex(QualificationError, "framework_negative"):
            assert_negatives(self.graph([route]), manifest)

    def test_coverage_rejects_false_complete(self) -> None:
        graph = self.graph([])
        graph["graph"]["files"][0]["extractionStatus"] = "partial"
        graph["graph"]["coverage"] = [{"fileId": "file:sample", "status": "complete"}]
        manifest = {"coverage": [{
            "id": "partial",
            "source": "sample.py",
            "forbidCompleteWhen": ["partial"],
        }]}
        with self.assertRaisesRegex(QualificationError, "false_coverage"):
            assert_coverage(graph, manifest)

    def test_route_containment_negatives_require_present_endpoints_and_direction(self) -> None:
        source, target = node("route:a", "route"), node("route:b", "route")
        source["source"]["file"] = "a.py"
        target["source"]["file"] = "b.py"
        manifest = {"routeContainmentNegatives": [{
            "id": "independent", "sourceFile": "a.py", "targetFile": "b.py",
            "reason": "Independent apps; neither imports or mounts the other.",
        }]}
        graph = self.graph([source, target])
        self.assertEqual(assert_route_containment_negatives(graph, manifest), {
            "route_containment_negatives": 1,
        })
        for endpoints in ([source], [target], []):
            with self.subTest(endpoints=endpoints), self.assertRaisesRegex(
                QualificationError, "route_containment_missing_endpoint"
            ):
                assert_route_containment_negatives(self.graph(endpoints), manifest)
        edge = {"id": "false-parent", "kind": "contains", "source": source["id"], "target": target["id"]}
        with self.assertRaisesRegex(QualificationError, "route_containment_negative"):
            assert_route_containment_negatives(self.graph([source, target], [edge]), manifest)
        edge["source"], edge["target"] = edge["target"], edge["source"]
        assert_route_containment_negatives(self.graph([source, target], [edge]), manifest)
        edge["source"], edge["target"] = edge["target"], edge["source"]
        edge["kind"] = "references"
        assert_route_containment_negatives(self.graph([source, target], [edge]), manifest)

    def test_route_containment_manifest_rejects_duplicate_or_unexplained_pairs(self) -> None:
        declared_sources = {
            item["path"] for item in load_json(ROOT / "tests/qualification/code-graph-v1-corpus.json")["files"]
        }
        load_manifest(ROOT / "tests/qualification/code-graph-v1-semantic.json", ROOT, declared_sources)
        for mutation in ("duplicate", "empty_reason", "unknown_field", "same_file", "non_list", "non_object"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                manifest = copy.deepcopy(self.manifest)
                item = manifest["routeContainmentNegatives"][0]
                if mutation == "duplicate":
                    duplicate = dict(item, id="different-id-same-pair")
                    manifest["routeContainmentNegatives"].append(duplicate)
                elif mutation == "empty_reason":
                    item["reason"] = ""
                elif mutation == "unknown_field":
                    item["typo"] = ""
                elif mutation == "non_list":
                    manifest["routeContainmentNegatives"] = None
                elif mutation == "non_object":
                    manifest["routeContainmentNegatives"] = [None]
                else:
                    item["targetFile"] = item["sourceFile"]
                path = Path(directory) / "manifest.json"
                path.write_text(json.dumps(manifest), encoding="utf-8")
                expected_error = "manifest_unknown_field" if mutation == "unknown_field" else "manifest_route_containment"
                with self.assertRaisesRegex(QualificationError, expected_error):
                    load_manifest(path, ROOT, declared_sources)

    def test_topology_separates_occurrences_from_unique_typed_connections(self) -> None:
        first = node("function:first", "function")
        second = node("function:second", "function")
        third = node("function:third", "function")
        fourth = node("function:isolated", "function")
        first["community"] = {"id": 1, "label": "first"}
        second["community"] = {"id": 1, "label": "renamed without identity change"}
        third["community"] = {"id": 2, "label": "second"}
        third["source"] = anchor("other.py")
        third["evidence"] = evidence("other.py")
        graph = self.graph([first, second, third, fourth])
        graph["graph"]["files"].append({
            "id": "file:other",
            "path": "other.py",
            "byteSize": 8,
            "extractionStatus": "extracted",
        })
        graph["links"] = [
            {
                "id": "edge:1",
                "key": "edge:1",
                "kind": "calls",
                "source": first["id"],
                "target": second["id"],
                "relationshipSite": anchor(),
                "evidence": evidence(),
            },
            {
                "id": "edge:2",
                "key": "edge:2",
                "kind": "calls",
                "source": first["id"],
                "target": second["id"],
                "relationshipSite": anchor(start=2, end=3),
                "evidence": evidence(),
            },
            {
                "id": "edge:3",
                "key": "edge:3",
                "kind": "references",
                "source": second["id"],
                "target": third["id"],
                "relationshipSite": anchor(),
                "evidence": evidence(),
            },
        ]

        self.assertEqual(topology_metrics(graph), {
            "communities": 2,
            "connectedComponents": 2,
            "crossCommunityEdges": 1,
            "crossFileEdges": 1,
            "crossFileEdgesPerThousandNodes": 250,
            "edgeBearingNodes": 3,
            "edgeBearingNodePermille": 750,
            "edges": 3,
            "exactConnectedComponents": 2,
            "exactCrossCommunityEdges": 1,
            "exactCrossFileEdges": 1,
            "exactCrossFileEdgesPerThousandNodes": 250,
            "exactEdgeBearingNodes": 3,
            "exactEdgeBearingNodePermille": 750,
            "exactEdges": 3,
            "exactIsolatedNodes": 1,
            "exactLargestComponentNodes": 3,
            "exactSelfLoops": 0,
            "exactUniqueTypedEndpointPairs": 2,
            "exactUniqueTypedEndpointPairsPerThousandNodes": 500,
            "isolatedNodes": 1,
            "largestComponentNodes": 3,
            "nodes": 4,
            "selfLoops": 0,
            "singletonCommunities": 1,
            "uniqueTypedEndpointPairs": 2,
            "uniqueTypedEndpointPairsPerThousandNodes": 500,
            "byRelation": {
                "calls": {
                    "crossFileEdges": 0,
                    "edgeBearingNodes": 2,
                    "edges": 2,
                    "exactCrossFileEdges": 0,
                    "exactEdgeBearingNodes": 2,
                    "exactEdges": 2,
                    "exactUniqueEndpointPairs": 1,
                    "uniqueEndpointPairs": 1,
                },
                "references": {
                    "crossFileEdges": 1,
                    "edgeBearingNodes": 2,
                    "edges": 1,
                    "exactCrossFileEdges": 1,
                    "exactEdgeBearingNodes": 2,
                    "exactEdges": 1,
                    "exactUniqueEndpointPairs": 1,
                    "uniqueEndpointPairs": 1,
                },
            },
        })

    def test_topology_policy_rejects_global_and_relationship_regressions(self) -> None:
        metrics = {
            "connectedComponents": 3,
            "crossFileEdges": 4,
            "edgeBearingNodes": 8,
            "isolatedNodes": 2,
            "uniqueTypedEndpointPairs": 7,
            "byRelation": {
                "calls": {
                    "crossFileEdges": 1,
                    "edgeBearingNodes": 3,
                    "edges": 4,
                    "uniqueEndpointPairs": 2,
                },
            },
        }
        policy = {
            "minimums": {
                "crossFileEdges": 4,
                "edgeBearingNodes": 8,
                "uniqueTypedEndpointPairs": 7,
            },
            "maximums": {"connectedComponents": 3, "isolatedNodes": 2},
            "relationshipMinimums": {
                "calls": {"crossFileEdges": 1, "uniqueEndpointPairs": 2},
            },
        }
        assert_topology(metrics, policy)

        disconnected = copy.deepcopy(metrics)
        disconnected["connectedComponents"] = 4
        with self.assertRaisesRegex(QualificationError, "topology_maximum"):
            assert_topology(disconnected, policy)

        missing_call = copy.deepcopy(metrics)
        missing_call["byRelation"]["calls"]["crossFileEdges"] = 0
        with self.assertRaisesRegex(QualificationError, "topology_relationship_minimum"):
            assert_topology(missing_call, policy)

    def test_topology_policy_and_report_are_strict_v1_contracts(self) -> None:
        policy = {
            "schema": "compass.code-graph-topology-policy/1",
            "topology": {
                "minimums": {"exactEdgeBearingNodePermille": 500},
                "maximums": {"exactIsolatedNodes": 1},
                "relationshipMinimums": {
                    "calls": {"exactUniqueEndpointPairs": 1},
                },
            },
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "topology.json"
            path.write_text(json.dumps(policy))
            self.assertEqual(load_topology_policy(path), policy)
            policy["extra"] = True
            path.write_text(json.dumps(policy))
            with self.assertRaisesRegex(QualificationError, "topology_policy_shape"):
                load_topology_policy(path)

        graph = self.graph([node("function:only", "function")])
        permissive = {
            "schema": "compass.code-graph-topology-policy/1",
            "topology": {
                "minimums": {"nodes": 1},
                "maximums": {"isolatedNodes": 1},
                "relationshipMinimums": {
                    "calls": {"exactUniqueEndpointPairs": 0},
                },
            },
        }
        report = topology_report(graph, permissive, graph_digest="sha256:test")
        self.assertEqual(report["schema"], "compass.code-graph-topology-report/1")
        self.assertEqual(report["graphDigest"], "sha256:test")
        self.assertEqual(report["metrics"]["exactIsolatedNodes"], 1)

    def test_flow_checks_exact_handler_identity_kind_and_language(self) -> None:
        route = node("route:1", "route")
        route.update({
            "framework": "demo",
            "details": {
                "type": "route",
                "data": {
                    "operation": "GET",
                    "path": "/ok",
                    "resolution": "exact",
                    "stages": [{"stage": "handler", "position": 0, "candidates": []}],
                },
            },
        })
        handler = node("function:handler", "function", qualified="handler()")
        edge = {
            "id": "edge:route",
            "kind": "routes_to",
            "source": route["id"],
            "target": handler["id"],
            "details": {"data": {"stage": "handler", "position": 0}},
            "evidence": [{
                "extractor": "compass.frameworks.demo",
                "origin": "ast",
                "confidence": "exact",
                "rule": "framework-route-stage:handler:0",
                "anchors": [anchor()],
            }],
        }
        manifest = {"flows": [{
            "id": "flow",
            "framework": "demo",
            "routeFramework": "demo",
            "operation": "GET",
            "path": "/ok",
            "routeSource": "sample.py",
            "handler": {"qualifiedName": "handler()"},
            "handlerSource": "sample.py",
            "relationship": "routes_to",
            "stage": "handler",
            "position": 0,
            "handlerKind": "function",
            "handlerLanguage": "python",
            "resolution": "exact",
            "origins": ["ast"],
            "producer": "compass.frameworks.demo",
            "rules": ["framework-route-stage:handler:0"],
            "allowHeuristic": False,
            "candidates": [],
        }]}
        graph = self.graph([route, handler], [edge])
        self.assertEqual(assert_flows(graph, manifest, ROOT), {
            "flows": 1,
            "frameworks": 1,
            "resolution_exact": 1,
        })
        handler["language"] = "ruby"
        with self.assertRaisesRegex(QualificationError, "flow_target_mismatch"):
            assert_flows(graph, manifest, ROOT)

    def test_manifest_rejects_unknown_top_level_field(self) -> None:
        invalid = copy.deepcopy(self.manifest)
        invalid["surprise"] = True
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            path.write_text(json.dumps(invalid), encoding="utf-8")
            with self.assertRaisesRegex(QualificationError, "manifest_unknown_field"):
                load_manifest(path, ROOT)

    def test_summary_is_deterministic_and_sorts_comparisons(self) -> None:
        graph = self.graph([])
        first = qualification_summary(
            compass_revision="abc",
            manifest_digest="sha256:manifest",
            graph_bytes=b"{}\n",
            graph=graph,
            assertions={"z": 2, "a": 1},
            comparisons={"warm": True, "clean": True},
        )
        second = qualification_summary(
            compass_revision="abc",
            manifest_digest="sha256:manifest",
            graph_bytes=b"{}\n",
            graph=graph,
            assertions={"a": 1, "z": 2},
            comparisons={"clean": True, "warm": True},
        )
        self.assertEqual(canonical_bytes(first), canonical_bytes(second))


if __name__ == "__main__":
    unittest.main()
