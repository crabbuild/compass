from __future__ import annotations

from pathlib import Path
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest

from benchmarks.performance.compass.adapters import GraphifyAdapter, ToolAdapter
from benchmarks.performance.compass.model import (
    ProcessMetrics,
    QueryEdgeOracle,
    QueryNodeOracle,
    QueryOracle,
    QuerySourceAnchorOracle,
    RepositorySpec,
    Sample,
    ToolRevision,
)
from benchmarks.performance.compass.workloads import (
    _result,
    _mcp_records,
    _append_query_sample,
    graph_neutral_mutation,
    run_build_matrix,
    run_compassql_matrix,
    run_query_matrix,
    select_mutation_file,
    validate_query_output,
)
from benchmarks.performance.compass.workspace import QualificationWorkspace


FIXTURE = Path(__file__).parent / "fixtures" / "compass_graph.json"
FAKE_TOOL = Path(__file__).parent / "helpers" / "fake_tool.py"


def node_oracle(qualified_name: str, source: str) -> QueryNodeOracle:
    return QueryNodeOracle(qualified_name, QuerySourceAnchorOracle(source))


def discovery_json(payload: dict[str, object]) -> str:
    payload = dict(payload)
    seeds = payload.get("seeds", [])
    nodes = payload.get("nodes", [])
    edges = payload.get("edges", [])
    stats = dict(payload.get("stats", {}))
    stats.setdefault("candidateProbes", 1)
    stats.setdefault("candidateNodes", len(nodes))
    stats.setdefault("candidatesAdmitted", len(seeds))
    stats.setdefault("visitedNodes", len(nodes))
    stats.setdefault("expandedRelationships", len(edges))
    stats.setdefault("returnedNodes", len(nodes))
    stats.setdefault("returnedEdges", len(edges))
    payload["stats"] = stats
    canonical = json.dumps(
        payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")
    return json.dumps(
        {
            "schema": "compass.query.discovery-result/1",
            "result": payload,
            "semanticResultDigest": "sha256:" + hashlib.sha256(canonical).hexdigest(),
        },
        sort_keys=True,
    )


class FakeAdapter(ToolAdapter):
    def build_command(self, checkout: Path, output: Path, *, force: bool = False):
        return (
            sys.executable,
            str(FAKE_TOOL),
            "build",
            "--output",
            str(output),
            "--graph",
            str(FIXTURE),
        )

    def query_command(self, graph: Path, question: str):
        qualified_name = "URLResolver" if question == "where" else "safe"
        payload = {
            "schema": "compass.query.discovery/1",
            "selectedDirection": "both",
            "seeds": [
                {
                    "nodeId": "n:seed",
                    "source": {"file": "src/main.py"},
                    "ambiguous": False,
                }
            ],
            "nodes": [
                {"id": "n:seed", "qualifiedName": qualified_name, "source": {"file": "src/main.py"}}
            ],
            "edges": [],
            "diagnostics": [],
            "stats": {"candidateNodes": 1, "expandedRelationships": 0},
            "truncated": False,
        }
        return (
            sys.executable,
            str(FAKE_TOOL),
            "query",
            "--text",
            discovery_json(payload),
        )

    def compassql_command(self, graph: Path, query: str):
        return (
            sys.executable,
            "-c",
            'print(\'{"columns":["id"],"rows":[{"id":"fixture:function"}]}\')',
        )

    def graph_path(self, output: Path) -> Path:
        return output / "graph.json"


class CheckoutWritingAdapter(FakeAdapter):
    def build_command(self, checkout: Path, output: Path, *, force: bool = False):
        script = (
            "from pathlib import Path; import shutil; "
            f"checkout = Path({str(checkout)!r}); output = Path({str(output)!r}); "
            f"fixture = Path({str(FIXTURE)!r}); "
            "output.mkdir(parents=True, exist_ok=True); "
            "shutil.copyfile(fixture, output / 'graph.json'); "
            "cache = checkout / 'graphify-out' / 'cache'; "
            "cache.mkdir(parents=True, exist_ok=True); "
            "(cache / 'stat-index.json').write_text('{}')"
        )
        return (sys.executable, "-c", script)

    def cleanup_checkout(self, checkout: Path) -> None:
        GraphifyAdapter.cleanup_checkout(self, checkout)


def git(cwd: Path, *arguments: str) -> str:
    return subprocess.check_output(["git", *arguments], cwd=cwd, text=True).strip()


class WorkloadTests(unittest.TestCase):
    def test_query_sample_rejects_valid_output_with_nonzero_status_or_timeout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "query.json"
            payload = {
                "schema": "compass.query.discovery/1",
                "selectedDirection": "both",
                "seeds": [],
                "nodes": [],
                "edges": [],
                "diagnostics": [{"code": "no_match"}],
                "stats": {},
                "truncated": False,
            }
            output.write_text(discovery_json(payload), encoding="utf-8")
            oracle = QueryOracle("absent", allow_no_match=True)
            spec = RepositorySpec(
                "repo", "https://example.invalid/repo.git", ".rs", (oracle,)
            )
            for return_code, timed_out, expected in [
                (7, False, "return code 7"),
                (0, True, "timed out"),
            ]:
                metrics = ProcessMetrics(
                    1.0,
                    0.0,
                    0.0,
                    1,
                    return_code,
                    None,
                    timed_out,
                    ("query",),
                    str(root),
                    str(output),
                    str(root / "query.err"),
                    "a",
                    "b",
                )
                samples: list[Sample] = []
                failures: list[str] = []
                _append_query_sample(
                    samples,
                    failures,
                    self.adapter(),
                    spec,
                    "query-1-fresh",
                    1,
                    metrics,
                    oracle,
                    {},
                )
                self.assertFalse(samples[0].eligible)
                self.assertIn(expected, samples[0].error or "")

    def test_mcp_record_validation_rejects_path_escape_and_missing_iterations(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output_root = root / "responses"
            output_root.mkdir()
            escaped = root / "escaped.json"
            escaped.write_text("{}", encoding="utf-8")
            worker = root / "worker.json"
            record = {
                "schema": "compass.performance.mcp-query-session-record/1",
                "query_index": 1,
                "iteration": 0,
                "wall_seconds": 0.1,
                "peak_rss_kib": 1,
                "output": str(escaped),
            }
            worker.write_text(
                json.dumps(
                    {
                        "schema": "compass.performance.mcp-query-session/1",
                        "records": [record],
                    }
                ),
                encoding="utf-8",
            )
            metrics = ProcessMetrics(
                1.0,
                0.0,
                0.0,
                1,
                0,
                None,
                False,
                ("worker",),
                str(root),
                str(worker),
                str(root / "worker.err"),
                "a",
                "b",
            )
            with self.assertRaisesRegex(RuntimeError, "escaped"):
                _mcp_records(metrics, output_root, query_count=1, batches=0)
            response = output_root / "query-1-0.json"
            response.write_text("{}", encoding="utf-8")
            record["output"] = str(response)
            worker.write_text(
                json.dumps(
                    {
                        "schema": "compass.performance.mcp-query-session/1",
                        "records": [record],
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(RuntimeError, "incomplete"):
                _mcp_records(metrics, output_root, query_count=1, batches=1)

    def test_legacy_quality_failures_preserve_performance_aggregate_and_failures(self) -> None:
        samples = []
        for iteration in range(1, 4):
            metrics = ProcessMetrics(
                float(iteration),
                0.0,
                0.0,
                100,
                0,
                None,
                False,
                ("legacy",),
                "/tmp",
                "/tmp/out",
                "/tmp/err",
                "a",
                "b",
            )
            samples.append(
                Sample(
                    f"compass:repo:query-1-fresh:{iteration}",
                    "compass",
                    "repo",
                    "query-1-fresh",
                    iteration,
                    True,
                    metrics,
                    "digest",
                    evidence={"legacy_semantic_digest": True},
                )
            )
        result = _result(
            "compass",
            "repo",
            "query-1-fresh",
            samples,
            ["query-1-fresh[1]: strict rank miss"],
        )
        self.assertIsNotNone(result.aggregate)
        self.assertFalse(result.correctness.passed)
        self.assertEqual(
            result.correctness.failures,
            ("query-1-fresh[1]: strict rank miss",),
        )

    def make_checkout(self, root: Path) -> Path:
        checkout = root / "checkout"
        checkout.mkdir()
        git(checkout, "init", "-q")
        git(checkout, "config", "user.name", "Compass")
        git(checkout, "config", "user.email", "compass@example.invalid")
        source = checkout / "src" / "main.py"
        source.parent.mkdir()
        source.write_text("def run():\n    return 1\n" + "# filler\n" * 200, encoding="utf-8")
        git(checkout, "add", "src/main.py")
        git(checkout, "commit", "-q", "-m", "fixture")
        return checkout

    def adapter(self) -> FakeAdapter:
        revision = ToolRevision(
            "compass",
            "https://example.invalid/compass.git",
            "a" * 40,
            "b" * 40,
            False,
            "c" * 64,
        )
        return FakeAdapter(Path(sys.executable), revision)

    def graphify_adapter(self) -> FakeAdapter:
        revision = ToolRevision(
            "graphify",
            "https://example.invalid/graphify.git",
            "d" * 40,
            "e" * 40,
            False,
            "f" * 64,
        )
        return FakeAdapter(Path(sys.executable), revision)

    def test_mutation_selection_and_restoration_are_clean(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            checkout = self.make_checkout(Path(directory))
            source = select_mutation_file(checkout, ".py")
            original = source.read_bytes()
            with graph_neutral_mutation(checkout, source):
                self.assertNotEqual(source.read_bytes(), original)
            self.assertEqual(source.read_bytes(), original)
            self.assertEqual(git(checkout, "status", "--porcelain"), "")

    def test_mutation_allows_graphify_cache_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            checkout = self.make_checkout(Path(directory))
            cache = checkout / "graphify-out" / "cache"
            cache.mkdir(parents=True)
            (cache / "stat-index.json").write_text("{}", encoding="utf-8")
            source = select_mutation_file(checkout, ".py")
            original = source.read_bytes()
            with graph_neutral_mutation(checkout, source):
                self.assertNotEqual(source.read_bytes(), original)
            self.assertEqual(source.read_bytes(), original)
            self.assertEqual(git(checkout, "status", "--porcelain"), "")
            self.assertFalse(cache.exists())

    def test_build_matrix_produces_three_correct_workloads(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = QualificationWorkspace.create(root / "workspace")
            checkout = self.make_checkout(root)
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (
                    QueryOracle(
                        "where",
                        expected_seeds=(node_oracle("URLResolver", "src/main.py"),),
                        relevant_nodes=(node_oracle("URLResolver", "src/main.py"),),
                    ),
                    QueryOracle(
                        "how",
                        expected_seeds=(node_oracle("safe", "src/main.py"),),
                        relevant_nodes=(node_oracle("safe", "src/main.py"),),
                    ),
                ),
            )
            results = run_build_matrix(
                self.adapter(),
                checkout,
                workspace.root / "artifacts",
                spec,
                timeout_seconds=5,
            )
            self.assertEqual([result.workload for result in results], ["cold", "warm", "incremental"])
            self.assertTrue(all(result.correctness.passed for result in results))
            self.assertTrue(all(result.aggregate is not None for result in results))

    def test_graphify_build_matrix_validates_without_a_compass_index(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = QualificationWorkspace.create(root / "workspace")
            checkout = self.make_checkout(root)
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (),
            )
            results = run_build_matrix(
                self.graphify_adapter(),
                checkout,
                workspace.root / "artifacts",
                spec,
                timeout_seconds=5,
            )
            self.assertTrue(all(result.correctness.passed for result in results))
            self.assertTrue(all(result.aggregate is not None for result in results))

    def test_build_matrix_cleans_tool_owned_checkout_artifacts_after_every_run(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = QualificationWorkspace.create(root / "workspace")
            checkout = self.make_checkout(workspace.root)
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (),
            )
            adapter = CheckoutWritingAdapter(Path(sys.executable), self.graphify_adapter().revision)

            results = run_build_matrix(
                adapter,
                checkout,
                workspace.root / "artifacts",
                spec,
                timeout_seconds=5,
            )

            self.assertTrue(all(result.correctness.passed for result in results))
            self.assertFalse((checkout / "graphify-out").exists())
            self.assertEqual(git(checkout, "status", "--porcelain"), "")

    def test_query_matrix_requires_oracle_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            graph = root / "graph.json"
            graph.write_text("{}", encoding="utf-8")
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (
                    QueryOracle(
                        "where",
                        forbidden=("forbidden",),
                        expected_seeds=(node_oracle("URLResolver", "src/main.py"),),
                        relevant_nodes=(node_oracle("URLResolver", "src/main.py"),),
                    ),
                    QueryOracle(
                        "how",
                        expected_seeds=(node_oracle("safe", "src/main.py"),),
                        relevant_nodes=(node_oracle("safe", "src/main.py"),),
                    ),
                ),
            )
            results = run_query_matrix(
                self.adapter(),
                graph,
                root / "artifacts",
                spec,
                batches=10,
                timeout_seconds=5,
            )
            self.assertEqual(len(results), 2)
            self.assertTrue(all(result.correctness.passed for result in results))

    def test_query_validation_rejects_forbidden_evidence(self) -> None:
        result = validate_query_output(
            "URLResolver also emitted WrongRoute",
            QueryOracle("where", ("URLResolver",), ("WrongRoute",)),
        )
        self.assertFalse(result.passed)

    def test_compass_query_validation_uses_typed_discovery_evidence(self) -> None:
        oracle = QueryOracle(
            question="where is URL resolution implemented",
            expected_seeds=(
                node_oracle("django.urls.resolvers.URLResolver", "django/urls/resolvers.py"),
            ),
            relevant_nodes=(
                node_oracle("django.urls.resolvers.URLResolver", "django/urls/resolvers.py"),
            ),
            expected_direction="both",
        )
        result = validate_query_output(
            discovery_json(json.loads("""{
              "schema":"compass.query.discovery/1",
              "selectedDirection":"both",
              "seeds":[{"nodeId":"n:url","source":{"file":"django/urls/resolvers.py"},"ambiguous":false}],
              "nodes":[{"id":"n:url","qualifiedName":"django.urls.resolvers.URLResolver","source":{"file":"django/urls/resolvers.py"}}],
              "edges":[],"diagnostics":[],"stats":{"candidateNodes":4,"expandedRelationships":2},"truncated":false
            }""")),
            oracle,
            tool="compass",
        )
        self.assertTrue(result.passed, result.failures)
        self.assertTrue(result.metrics["top1"])
        self.assertEqual(result.metrics["candidate_nodes"], 4)

    def test_compass_query_validation_rejects_wrong_seed_direction_and_missing_anchor(self) -> None:
        oracle = QueryOracle(
            question="what calls target",
            expected_seeds=(node_oracle("pkg.target", "src/target.rs"),),
            expected_direction="incoming",
        )
        result = validate_query_output(
            '{"schema":"compass.query.discovery/1","selectedDirection":"outgoing",'
            '"seeds":[{"nodeId":"n:other","source":null,"ambiguous":false}],'
            '"nodes":[{"id":"n:other","qualifiedName":"pkg.other","source":null}],"edges":[],"diagnostics":[],"stats":{},"truncated":false}',
            oracle,
            tool="compass",
        )
        self.assertFalse(result.passed)
        self.assertTrue(any("missing expected seeds" in failure for failure in result.failures))
        self.assertTrue(any("direction mismatch" in failure for failure in result.failures))

    def test_compass_seed_identity_is_resolved_through_returned_nodes(self) -> None:
        oracle = QueryOracle(
            "find target",
            expected_seeds=(node_oracle("pkg.Target", "src/target.rs"),),
        )
        result = validate_query_output(
            discovery_json(json.loads('{"schema":"compass.query.discovery/1","selectedDirection":"both",'
            '"seeds":[{"nodeId":"n:target","qualifiedName":"spoofed","ambiguous":false}],'
            '"nodes":[{"id":"n:target","qualifiedName":"pkg.Target","source":{"file":"src/target.rs"}}],'
            '"edges":[],"diagnostics":[],"stats":{},"truncated":false}')),
            oracle,
            tool="compass",
        )
        self.assertTrue(result.passed, result.failures)

    def test_compass_top_one_accepts_declared_alternative_but_rejects_other_seed(self) -> None:
        oracle = QueryOracle(
            "find target",
            expected_seeds=(node_oracle("pkg.Target", "src/target.rs"),),
            acceptable_seeds=(node_oracle("pkg.TargetAlias", "src/alias.rs"),),
        )
        base = {
            "schema": "compass.query.discovery/1",
            "selectedDirection": "both",
            "nodes": [
                {"id": "target", "qualifiedName": "pkg.Target", "source": {"file": "src/target.rs"}},
                {"id": "alias", "qualifiedName": "pkg.TargetAlias", "source": {"file": "src/alias.rs"}},
                {"id": "other", "qualifiedName": "pkg.Other", "source": {"file": "src/other.rs"}},
            ],
            "edges": [], "diagnostics": [], "stats": {}, "truncated": False,
        }
        accepted = dict(base, seeds=[{"nodeId": "alias", "ambiguous": False}, {"nodeId": "target", "ambiguous": False}])
        rejected = dict(base, seeds=[{"nodeId": "other", "ambiguous": False}, {"nodeId": "target", "ambiguous": False}])
        self.assertTrue(validate_query_output(discovery_json(accepted), oracle, tool="compass").passed)
        failure = validate_query_output(json.dumps(rejected), oracle, tool="compass")
        self.assertFalse(failure.passed)
        self.assertTrue(any("top-ranked" in item for item in failure.failures))

    def test_compass_relevant_node_requires_matching_source_anchor(self) -> None:
        oracle = QueryOracle(
            "find target",
            expected_seeds=(node_oracle("pkg.Target", "src/target.rs"),),
            relevant_nodes=(node_oracle("pkg.Helper", "src/helper.rs"),),
        )
        payload = {
            "schema": "compass.query.discovery/1", "selectedDirection": "both",
            "seeds": [{"nodeId": "target", "ambiguous": False}],
            "nodes": [
                {"id": "target", "qualifiedName": "pkg.Target", "source": {"file": "src/target.rs"}},
                {"id": "helper", "qualifiedName": "pkg.Helper", "source": {"file": "tests/helper.rs"}},
            ],
            "edges": [], "diagnostics": [], "stats": {}, "truncated": False,
        }
        result = validate_query_output(json.dumps(payload), oracle, tool="compass")
        self.assertFalse(result.passed)
        self.assertTrue(any("missing relevant nodes" in item for item in result.failures))

    def test_compass_expected_edge_direction_is_enforced_relative_to_seed(self) -> None:
        oracle = QueryOracle(
            "what calls target",
            expected_seeds=(node_oracle("pkg.Target", "src/target.rs"),),
            expected_direction="incoming",
            expected_edges=(QueryEdgeOracle("pkg.Target", "calls", "pkg.Caller", "incoming"),),
        )
        payload = {
            "schema": "compass.query.discovery/1", "selectedDirection": "incoming",
            "seeds": [{"nodeId": "target", "ambiguous": False}],
            "nodes": [
                {"id": "target", "qualifiedName": "pkg.Target", "source": {"file": "src/target.rs"}},
                {"id": "caller", "qualifiedName": "pkg.Caller", "source": {"file": "src/caller.rs"}},
            ],
            "edges": [{"source": "target", "target": "caller", "kind": "calls"}],
            "diagnostics": [], "stats": {}, "truncated": False,
        }
        result = validate_query_output(json.dumps(payload), oracle, tool="compass")
        self.assertFalse(result.passed)
        self.assertTrue(any("edge direction mismatch" in item for item in result.failures))

    def test_compass_no_match_false_positive_is_explicit(self) -> None:
        oracle = QueryOracle(
            "find target",
            expected_seeds=(node_oracle("pkg.Target", "src/target.rs"),),
        )
        payload = {
            "schema": "compass.query.discovery/1", "selectedDirection": "both",
            "seeds": [], "nodes": [], "edges": [],
            "diagnostics": [{"code": "no_match"}], "stats": {}, "truncated": False,
        }
        result = validate_query_output(json.dumps(payload), oracle, tool="compass")
        self.assertFalse(result.passed)
        self.assertTrue(result.metrics["no_match_false_positive"])

    def test_compass_expected_no_match_requires_diagnostic_and_zero_seeds(self) -> None:
        oracle = QueryOracle("find absent target", allow_no_match=True)
        valid = {
            "schema": "compass.query.discovery/1",
            "selectedDirection": "both",
            "seeds": [],
            "nodes": [],
            "edges": [],
            "diagnostics": [{"code": "no_match"}],
            "stats": {},
            "truncated": False,
        }
        self.assertTrue(
            validate_query_output(discovery_json(valid), oracle, tool="compass").passed
        )

        missing_diagnostic = dict(valid, diagnostics=[])
        result = validate_query_output(
            json.dumps(missing_diagnostic), oracle, tool="compass"
        )
        self.assertFalse(result.passed)
        self.assertTrue(any("expected no_match" in item for item in result.failures))

        returned_seed = dict(
            valid,
            seeds=[{"nodeId": "target", "ambiguous": False}],
            nodes=[
                {
                    "id": "target",
                    "qualifiedName": "pkg.Target",
                    "source": {"file": "src/target.rs"},
                }
            ],
        )
        result = validate_query_output(json.dumps(returned_seed), oracle, tool="compass")
        self.assertFalse(result.passed)
        self.assertTrue(any("returned seeds" in item for item in result.failures))

    def test_compass_relevance_metrics_are_normalized_and_bounded_to_top_ten(self) -> None:
        target_one = node_oracle("pkg.TargetOne", "src/one.rs")
        target_two = node_oracle("pkg.TargetTwo", "src/two.rs")
        oracle = QueryOracle(
            "find targets",
            expected_seeds=(target_one,),
            relevant_nodes=(target_one, target_two),
        )
        nodes = [
            {
                "id": "one",
                "qualifiedName": "pkg.TargetOne",
                "source": {"file": "src/one.rs"},
            },
            {
                "id": "two",
                "qualifiedName": "pkg.TargetTwo",
                "source": {"file": "src/two.rs"},
            },
        ]
        nodes.extend(
            {
                "id": f"other-{index}",
                "qualifiedName": f"pkg.Other{index}",
                "source": {"file": f"src/other-{index}.rs"},
            }
            for index in range(9)
        )
        seeds = [{"nodeId": "one", "ambiguous": False}]
        seeds.extend(
            {"nodeId": f"other-{index}", "ambiguous": False}
            for index in range(9)
        )
        seeds.append({"nodeId": "two", "ambiguous": False})
        payload = {
            "schema": "compass.query.discovery/1",
            "selectedDirection": "both",
            "seeds": seeds,
            "nodes": nodes,
            "edges": [],
            "diagnostics": [],
            "stats": {},
            "truncated": False,
        }
        result = validate_query_output(discovery_json(payload), oracle, tool="compass")
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["mrr_at_10"], 1.0)
        self.assertEqual(result.metrics["recall_at_10"], 0.5)
        self.assertNotIn("mrr_millionths", result.metrics)

        cutoff_oracle = QueryOracle(
            "find second target",
            expected_seeds=(target_one,),
            relevant_nodes=(target_two,),
        )
        cutoff = validate_query_output(
            discovery_json(payload), cutoff_oracle, tool="compass"
        )
        self.assertTrue(cutoff.passed, cutoff.failures)
        self.assertEqual(cutoff.metrics["mrr_at_10"], 0.0)
        self.assertEqual(cutoff.metrics["recall_at_10"], 0.0)

    def test_compass_source_less_forbidden_seed_rejects_unresolved_distractor(self) -> None:
        oracle = QueryOracle(
            "find target",
            expected_seeds=(node_oracle("pkg.Target", "src/target.rs"),),
            forbidden_seeds=(QueryNodeOracle("Target", None),),
        )
        payload = {
            "schema": "compass.query.discovery/1",
            "selectedDirection": "both",
            "seeds": [
                {"nodeId": "target", "ambiguous": False},
                {"nodeId": "placeholder", "ambiguous": False},
            ],
            "nodes": [
                {
                    "id": "target",
                    "qualifiedName": "pkg.Target",
                    "source": {"file": "src/target.rs"},
                },
                {"id": "placeholder", "qualifiedName": "Target", "source": None},
            ],
            "edges": [],
            "diagnostics": [],
            "stats": {},
            "truncated": False,
        }
        result = validate_query_output(json.dumps(payload), oracle, tool="compass")
        self.assertFalse(result.passed)
        self.assertTrue(any("forbidden seed" in item for item in result.failures))

    def test_compassql_matrix_canonicalizes_results(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            graph = root / "graph.json"
            graph.write_text("{}", encoding="utf-8")
            spec = RepositorySpec(
                "fixture",
                "https://example.invalid/fixture.git",
                ".py",
                (QueryOracle("where", ("URLResolver",)), QueryOracle("how", ("safe",))),
            )
            results = run_compassql_matrix(
                self.adapter(),
                graph,
                root / "artifacts",
                spec,
                batches=10,
                timeout_seconds=5,
            )
            self.assertEqual(7, len(results))
            self.assertTrue(all(result.correctness.passed for result in results))
            self.assertTrue(all(len(result.samples) == 10 for result in results))


if __name__ == "__main__":
    unittest.main()
