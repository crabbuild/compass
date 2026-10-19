"""Synthetic arithmetic/limit controls; no real graphs or source judgments."""
import copy
import hashlib
from unittest import TestCase
from unittest.mock import patch

from benchmarks.agent_query import broad_graph_statistics as graph
from benchmarks.agent_query import broad_query_review as query


def row(identifier="a", *, repository="one", cluster=None, candidate=True, comparator=False):
    return dict(id=identifier, repository=repository, cluster=cluster or identifier,
        language="python", relation="calls",
        sourceFactSha256=hashlib.sha256(identifier.encode()).hexdigest(),
        compass=candidate, graphify=comparator)


class PairedGraphCoverageTests(TestCase):
    def test_complete_pairs_reuse_existing_estimator_without_changing_its_scope(self):
        rows = [row("a"), row("b", candidate=False, comparator=True),
                row("c", repository="two"), row("d", repository="two", comparator=True)]
        expected = query.paired_interval([
            (r["repository"], r["cluster"], float(r["compass"]), float(r["graphify"]))
            for r in rows], samples=1000)
        result = graph.paired_graph_coverage(rows, resamples=1000)
        self.assertEqual(result["pairedClusterWeightedRecallDifference"], expected)
        self.assertEqual(result, graph.paired_graph_coverage(list(reversed(rows)), resamples=1000))
        self.assertFalse(result["qualificationEstablished"])

    def test_unknown_pairs_keep_both_denominators_and_withhold_paired_interval(self):
        for candidate, comparator in [(None, True), (True, None), (None, None)]:
            result = graph.paired_graph_coverage([row(), row("b", candidate=candidate, comparator=comparator)])
            self.assertEqual(result["rowsRetained"], 2)
            self.assertEqual(result["missingPairRows"], 1)
            self.assertFalse(result["completePairs"])
            for field in ["estimate", "lower", "upper"]:
                self.assertIsNone(result["pairedClusterWeightedRecallDifference"][field])
            for tool, value in [("compass", candidate), ("graphify", comparator)]:
                counts = result["coverage"][tool]
                self.assertEqual(counts["sourceFacts"], 2)
                if value is None:
                    self.assertEqual(counts["unknown"], 1)
                    self.assertIsNone(counts["microRecall"])

    def test_empty_graph_observation_is_missing_not_unknown_or_omitted(self):
        result = graph.paired_graph_coverage([row(candidate=False, comparator=False)], resamples=1000)
        self.assertTrue(result["completePairs"])
        for counts in result["coverage"].values():
            self.assertEqual(counts["microRecall"], 0)
            self.assertEqual(counts["missing"], 1)
        self.assertFalse(result["qualificationEstablished"])

    def test_empty_panel_never_means_perfect_recall_or_complete_evidence(self):
        result = graph.paired_graph_coverage([])
        self.assertFalse(result["completePairs"])
        self.assertIsNone(result["pairedClusterWeightedRecallDifference"]["estimate"])
        for counts in result["coverage"].values():
            self.assertIsNone(counts["microRecall"])

    def test_micro_and_cluster_weighted_estimands_are_explicitly_distinct(self):
        rows = [row("a", cluster="shared"), row("b", cluster="shared"),
                row("c", candidate=False, comparator=True)]
        result = graph.paired_graph_coverage(rows, resamples=1000)
        self.assertEqual(result["declaredClusters"], 2)
        self.assertEqual(result["coverage"]["compass"]["microRecall"], 2 / 3)
        self.assertEqual(result["pairedClusterWeightedRecallDifference"]["estimate"], 0)

    def test_declared_cluster_does_not_hide_a_loss_within_it(self):
        rows = [row("a", cluster="shared"), row("b", cluster="shared", candidate=False, comparator=True)]
        interval = graph.paired_graph_coverage(rows, resamples=1000)["pairedClusterWeightedRecallDifference"]
        self.assertEqual(interval["clusters"], 1)
        self.assertLess(interval["finiteTaskRiskLower"], 0)

    def test_duplicate_ids_or_canonical_source_facts_cannot_inflate_sample(self):
        original = row()
        for field in ["id", "sourceFactSha256"]:
            duplicate = row("b")
            duplicate[field] = original[field]
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "duplicate"):
                graph.paired_graph_coverage([original, duplicate])

    def test_late_invalid_rows_are_rejected_even_after_unknown_observations(self):
        for value in [1, 0, 0.5, "present", [], float("nan")]:
            invalid = row("b", candidate=value)
            with self.subTest(value=value), self.assertRaises(ValueError):
                graph.paired_graph_coverage([row(candidate=None), invalid])

    def test_strata_preserve_missing_observations_and_deterministic_keys(self):
        rows = [row(), row("b", candidate=None)]
        rows[1].update(language="rust", relation="references")
        result = graph.paired_graph_coverage(rows)
        self.assertEqual(list(result["strata"]["language"]), ["python", "rust"])
        self.assertIsNone(result["strata"]["relation"]["references"]["compass"]["microRecall"])

    def test_unknown_fields_missing_digest_and_control_characters_are_rejected(self):
        for change in [dict(extra=True), dict(sourceFactSha256="0" * 63), dict(cluster="bad\ncluster")]:
            invalid = row()
            invalid.update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                graph.paired_graph_coverage([invalid])

    def test_resampling_work_is_bounded_before_bootstrap(self):
        with patch.object(graph, "MAX_RESAMPLING_WORK", 1999), patch.object(query, "paired_interval") as bootstrap:
            with self.assertRaisesRegex(ValueError, "work limit"):
                graph.paired_graph_coverage([row(), row("b")], resamples=1000)
            bootstrap.assert_not_called()

    def test_source_count_text_cluster_and_repository_limits_are_explicit(self):
        for field, limit, message in [("MAX_SOURCE_FACTS", 1, "contain"),
                ("MAX_IDENTIFIER_BYTES", 1, "byte limit"),
                ("MAX_DECLARED_CLUSTERS", 1, "cluster or repository"),
                ("MAX_REPOSITORIES", 1, "cluster or repository")]:
            with self.subTest(field=field), patch.object(graph, field, limit), self.assertRaisesRegex(ValueError, message):
                graph.paired_graph_coverage([row(), row("b", repository="two")])

    def test_unbalanced_repository_resampling_uses_worst_case_draw_bound(self):
        rows = [row("a"), row("b"), row("c"), row("d", repository="two")]
        with patch.object(graph, "MAX_RESAMPLING_WORK", 6000), patch.object(query, "paired_interval") as bootstrap:
            with self.assertRaisesRegex(ValueError, "work limit"):
                graph.paired_graph_coverage(rows, resamples=1000)
            bootstrap.assert_not_called()
        rows[2]["repository"] = "two"
        with patch.object(graph, "MAX_RESAMPLING_WORK", 6000):
            self.assertTrue(graph.paired_graph_coverage(rows, resamples=1000)["completePairs"])

    def test_stratum_limit_prevents_unbounded_output_fanout(self):
        for field in ["language", "relation"]:
            rows = [row(), row("b")]
            rows[1][field] = "other"
            with self.subTest(field=field), patch.object(graph, "MAX_STRATUM_VALUES", 1), self.assertRaisesRegex(ValueError, "stratum limit"):
                graph.paired_graph_coverage(rows)

    def test_inputs_are_not_mutated_and_positive_arithmetic_never_qualifies(self):
        rows = [row(str(i)) for i in range(24)]
        before = copy.deepcopy(rows)
        result = graph.paired_graph_coverage(rows, resamples=1000)
        self.assertEqual(rows, before)
        self.assertGreater(result["pairedClusterWeightedRecallDifference"]["lower"], 0)
        self.assertFalse(result["qualificationEstablished"])
        self.assertIn("precision", " ".join(result["missingQualificationEvidence"]))
