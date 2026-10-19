"""Synthetic sampling/denominator controls; no independent graph judgments."""
import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_graph_precision as precision
from benchmarks.agent_query import broad_graph_statistics as statistics
from benchmarks.agent_query.tests.test_broad_graph_evidence import Fixture


class PrecisionFixture:
    def __init__(self, root):
        self.coverage = Fixture(root)
        self.design = dict(schema=precision.DESIGN_SCHEMA, frozenAt="2026-01-01T12:00:00Z",
            coverageRegistrationSha256="", supportSha256=precision.support_hashes(),
            sampler=precision.SAMPLER, seed=29, quotaPerGraph=1)
        self.reviews = dict(schema=precision.REVIEW_SCHEMA, coverageCaptureSha256="", designSha256="",
            reviewedAt="2026-01-03T00:00:00Z", observations=[])

    def publish_input(self):
        coverage_input = self.coverage.publish_input()
        self.design["coverageRegistrationSha256"] = coverage_input["registration"]["sha256"]
        design_ref = self.coverage.put("precision-design.json", self.design)
        self.reviews.update(coverageCaptureSha256=coverage_input["capture"]["sha256"], designSha256=design_ref["sha256"])
        manifest = dict(schema=precision.INPUT_SCHEMA,
            coverageInput=self.coverage.put("coverage-input.json", coverage_input), design=design_ref,
            reviews=self.coverage.put("precision-reviews.json", self.reviews))
        return manifest

    def assess(self):
        return precision.assess_graph_precision(self.publish_input(), self.coverage.root)

    def label(self, plan, *, verdict="correct", issues=None):
        span = {key: value for key, value in self.coverage.fact["fact"]["occurrence"].items()}
        return dict(repository=plan["repository"], tool=plan["tool"], index=plan["index"],
            recordSha256=plan["recordSha256"], verdict=verdict, issues=[] if issues is None else issues,
            reason="Synthetic label, not source qualification.", witnesses=[] if verdict == "unreviewed" else [span])

    def label_all(self):
        self.reviews["observations"] = [self.label(row) for row in self.assess()["samplePlan"]]

    def enlarge(self, tool, count):
        graph = self.coverage.graphs[tool]
        graph["edges"] = [dict(graph["edges"][0], syntheticOccurrence=i) for i in range(count)]
        self.coverage.publish_graph(tool)


class SamplerTests(unittest.TestCase):
    def test_reproducible_unique_nonprefix_sample_and_exact_census(self):
        sample = precision.sample_indices(100, 12, dict(seed=29, repository="one", tool="compass"))
        self.assertEqual(sample, precision.sample_indices(100, 12, dict(tool="compass", repository="one", seed=29)))
        self.assertEqual(len(sample), 12)
        self.assertEqual(sample, sorted(set(sample)))
        self.assertTrue(all(0 <= value < 100 for value in sample))
        self.assertGreater(max(sample), 11)
        self.assertNotEqual(sample, precision.sample_indices(100, 12, dict(seed=30, repository="one", tool="compass")))
        self.assertEqual(precision.sample_indices(3, 3, {}), [0, 1, 2])
        self.assertEqual(precision.sample_indices(0, 0, {}), [])

    def test_sampling_limits_reject_without_partial_sample(self):
        for population, count in [(True, 1), (4_000_001, 1), (10, 11), (5000, 4097)]:
            with self.subTest(population=population, count=count), self.assertRaises(ValueError):
                precision.sample_indices(population, count, {})
        with patch.object(precision, "MAX_RANDOM_DRAWS", 0), self.assertRaisesRegex(ValueError, "draw limit"):
            precision.sample_indices(20, 1, {})

    def test_rejection_sampling_is_bounded(self):
        class Digest:
            def digest(self): return b"\xff" * 32
        with patch.object(precision.hashlib, "sha256", return_value=Digest()), self.assertRaisesRegex(ValueError, "rejection limit"):
            precision.sample_indices(3, 1, {})


class GraphPrecisionTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.fixture = PrecisionFixture(Path(directory.name))

    def test_missing_reviews_keep_full_sample_denominator(self):
        result = self.fixture.assess()
        self.assertEqual(len(result["samplePlan"]), 2)
        for tool in statistics.TOOLS:
            view = result["precision"][tool]["overall"]
            self.assertEqual(view["sampledEdges"], 1)
            self.assertEqual(view["unreviewed"], 1)
            self.assertIsNone(view["samplePrecision"])
            self.assertEqual(view["identifiedPopulationPrecisionBounds"], [0, 1])
        self.assertFalse(result["qualificationEstablished"])

    def test_complete_census_is_scoped_and_never_independent_qualification(self):
        self.fixture.label_all()
        result = self.fixture.assess()
        for tool in statistics.TOOLS:
            view = result["precision"][tool]["overall"]
            self.assertEqual(view["samplePrecision"], 1)
            self.assertEqual(view["populationPrecision"], 1)
            self.assertEqual(view["identifiedPopulationPrecisionBounds"], [1, 1])
            self.assertIsNone(view["populationConfidenceInterval"])
            self.assertFalse(view["independenceEstablished"])
        self.assertIsNone(result["precisionComparisonInterval"])
        self.assertFalse(result["qualificationEstablished"])

    def test_incorrect_and_unreviewed_records_are_not_dropped(self):
        self.fixture.enlarge("compass", 3)
        self.fixture.design["quotaPerGraph"] = 3
        self.fixture.label_all()
        observations = self.fixture.reviews["observations"]
        selected = [o for o in observations if o["tool"] == "compass"]
        selected[0].update(verdict="incorrect", issues=["target_identity"])
        selected[1].update(verdict="unreviewed", issues=[], witnesses=[])
        result = self.fixture.assess()
        view = result["precision"]["compass"]["overall"]
        self.assertEqual((view["correct"], view["incorrect"], view["unreviewed"]), (1, 1, 1))
        self.assertEqual(view["sampledEdges"], 3)
        self.assertIsNone(view["samplePrecision"])
        self.assertEqual(view["identifiedPopulationPrecisionBounds"], [1 / 3, 1 - 1 / 3])

    def test_subsample_is_not_a_whole_population_census(self):
        self.fixture.enlarge("compass", 10)
        self.fixture.design["quotaPerGraph"] = 2
        self.fixture.label_all()
        view = self.fixture.assess()["precision"]["compass"]["overall"]
        self.assertEqual(view["samplePrecision"], 1)
        self.assertEqual(view["populationWeightedPointEstimate"], 1)
        self.assertIsNone(view["populationPrecision"])
        self.assertEqual(view["identifiedPopulationPrecisionBounds"], [.2, 1])
        self.assertEqual(view["unsampledEdges"], 8)

    def test_population_size_weights_are_not_pooled_sample_precision(self):
        frames = [dict(population=100, sampleKeys=["a"], buildSuccessful=True),
                  dict(population=10, sampleKeys=["b"], buildSuccessful=True)]
        view = precision._view(frames, ["a", "b"], dict(a="correct", b="incorrect"))
        self.assertEqual(view["samplePrecision"], .5)
        self.assertAlmostEqual(view["populationWeightedPointEstimate"], 100 / 110)
        self.assertIsNone(view["populationPrecision"])

    def test_nonselected_relation_remains_visible(self):
        self.fixture.enlarge("compass", 10)
        for index, record in enumerate(self.fixture.coverage.graphs["compass"]["edges"]):
            record["kind"] = "calls" if index == 0 else f"rare-{index}"
        self.fixture.coverage.publish_graph("compass")
        result = self.fixture.assess()
        unselected = [view for kind, view in result["precision"]["compass"]["byRecordedRelation"].items()
            if kind.startswith("rare-") and view["sampledEdges"] == 0]
        self.assertGreaterEqual(len(unselected), 8)
        for view in unselected:
            self.assertEqual(view["capturedEdges"], 1)
            self.assertEqual(view["identifiedPopulationPrecisionBounds"], [0, 1])
            self.assertIsNone(view["samplePrecision"])

    def test_ledger_cannot_select_other_records_or_duplicate_a_label(self):
        self.fixture.enlarge("compass", 10)
        result = self.fixture.assess()
        selected = next(row for row in result["samplePlan"] if row["tool"] == "compass")
        label = self.fixture.label(selected)
        label["index"] = (selected["index"] + 1) % 10
        self.fixture.reviews["observations"] = [label]
        with self.assertRaisesRegex(ValueError, "non-sampled"):
            self.fixture.assess()
        label = self.fixture.label(selected)
        self.fixture.reviews["observations"] = [label, copy.deepcopy(label)]
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.fixture.assess()

    def test_labels_require_exact_records_and_pinned_source_witnesses(self):
        self.fixture.label_all()
        label = self.fixture.reviews["observations"][0]
        original = copy.deepcopy(label)
        for change in [dict(recordSha256="0" * 64), dict(witnesses=[]),
                dict(witnesses=[dict(original["witnesses"][0], sha256="0" * 64)])]:
            label.clear(); label.update(copy.deepcopy(original)); label.update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.fixture.assess()

    def test_unknown_label_does_not_hide_late_forged_witness(self):
        self.fixture.label_all()
        first, second = self.fixture.reviews["observations"]
        first.update(verdict="unreviewed", witnesses=[])
        second["witnesses"][0]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "source anchor digest"):
            self.fixture.assess()

    def test_undirected_records_are_not_penalized_for_storage_orientation(self):
        coverage = self.fixture.coverage
        coverage.fact["fact"]["requiresDirection"] = False
        for item in coverage.reviews["observations"]:
            item["sourceFactSha256"] = coverage.fact_digest()
        coverage.graphs["graphify"]["directed"] = False
        coverage.publish_graph("graphify")
        self.fixture.label_all()
        label = next(o for o in self.fixture.reviews["observations"] if o["tool"] == "graphify")
        label.update(verdict="incorrect", issues=["direction"])
        with self.assertRaisesRegex(ValueError, "invented direction error"):
            self.fixture.assess()
        label.update(issues=["target_identity"])
        self.assertEqual(self.fixture.assess()["precision"]["graphify"]["overall"]["incorrect"], 1)

    def test_failed_builds_do_not_disappear_from_precision(self):
        coverage = self.fixture.coverage
        coverage.builds["compass"]["timed_out"] = True
        coverage.entry("compass")["buildCapture"] = coverage.put("compass-build.json", coverage.builds["compass"])
        coverage.observation("compass").update(status="unreviewed", edges=[])
        self.fixture.label_all()
        view = self.fixture.assess()["precision"]["compass"]["overall"]
        self.assertEqual(view["capturedEdges"], 1)
        self.assertIsNone(view["populationEdges"])
        self.assertIsNone(view["samplePrecision"])
        self.assertIsNone(view["populationWeightedPointEstimate"])

    def test_empty_graphs_have_undefined_precision(self):
        for tool in statistics.TOOLS:
            coverage = self.fixture.coverage
            coverage.graphs[tool].update(nodes=[], edges=[])
            ref = coverage.put(tool + "-graph.json", coverage.graphs[tool])
            coverage.entry(tool)["artifact"] = ref
            coverage.observation(tool).update(graphSha256=ref["sha256"], status="missing", edges=[])
        result = self.fixture.assess()
        self.assertEqual(result["samplePlan"], [])
        for tool in statistics.TOOLS:
            self.assertIsNone(result["precision"][tool]["overall"]["samplePrecision"])
            self.assertIsNone(result["precision"][tool]["overall"]["populationPrecision"])

    def test_design_sampler_versions_and_support_are_checked(self):
        original = copy.deepcopy(self.fixture.design)
        for change in [dict(schema="other/99"), dict(sampler="other/99"),
                dict(supportSha256={}), dict(quotaPerGraph=0), dict(seed=True),
                dict(frozenAt="2025-12-31T00:00:00Z"),
                dict(frozenAt="2026-01-02T00:00:00Z")]:
            self.fixture.design.update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.fixture.assess()
            self.fixture.design = copy.deepcopy(original)

    def test_review_capture_and_design_bindings_are_recomputed(self):
        for field in ["coverageCaptureSha256", "designSha256"]:
            manifest = self.fixture.publish_input()
            forged = copy.deepcopy(self.fixture.reviews)
            forged[field] = "0" * 64
            manifest["reviews"] = self.fixture.coverage.put("forged-precision-reviews.json", forged)
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "bound to its capture"):
                precision.assess_graph_precision(manifest, self.fixture.coverage.root)

    def test_design_cannot_rebind_source_registration(self):
        manifest = self.fixture.publish_input()
        forged = dict(self.fixture.design, coverageRegistrationSha256="0" * 64)
        manifest["design"] = self.fixture.coverage.put("forged-precision-design.json", forged)
        with self.assertRaisesRegex(ValueError, "source registration"):
            precision.assess_graph_precision(manifest, self.fixture.coverage.root)

    def test_support_mutation_after_scoring_is_rejected(self):
        before = precision.support_hashes()
        changed = dict(before, precision="0" * 64)
        with patch.object(precision, "support_hashes", side_effect=[before, changed]), self.assertRaisesRegex(ValueError, "support changed"):
            self.fixture.assess()

    def test_source_and_graph_mutations_after_coverage_check_are_rejected(self):
        for target in ["store.py", "compass-graph.json"]:
            with self.subTest(target=target):
                directory = tempfile.TemporaryDirectory()
                self.addCleanup(directory.cleanup)
                fixture = PrecisionFixture(Path(directory.name))
                original = precision._view
                def mutate(*args, **kwargs):
                    result = original(*args, **kwargs)
                    file = fixture.coverage.source / target if target == "store.py" else fixture.coverage.root / target
                    file.write_bytes(b"changed")
                    return result
                with patch.object(precision, "_view", side_effect=mutate), self.assertRaises(ValueError):
                    fixture.assess()

    def test_observation_issues_and_source_budgets_are_bounded(self):
        self.fixture.label_all()
        label = self.fixture.reviews["observations"][0]
        label["issues"] = ["direction"]
        with self.assertRaisesRegex(ValueError, "incorrect edge"):
            self.fixture.assess()
        label["issues"] = []
        for field in ["MAX_WITNESS_BYTES", "MAX_SAMPLED_EDGES", "MAX_POPULATION_EDGES"]:
            with self.subTest(field=field), patch.object(precision, field, 1), self.assertRaisesRegex(ValueError, "limit exceeded"):
                self.fixture.assess()
