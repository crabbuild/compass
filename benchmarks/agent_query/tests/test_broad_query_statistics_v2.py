"""Synthetic fixed-denominator arithmetic; no independent source judgments."""
import copy
import unittest

from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_protocol import precision_view
from benchmarks.agent_query.broad_query_score_v2 import score_task_v2
from benchmarks.agent_query.broad_query_statistics_v2 import paired_interval_v2, summary_v2
from benchmarks.agent_query.tests.test_broad_query_score_v2 import scoring_fixture


def row(supported=7, unsupported=3, *, complete=True, empty=False):
    task, raw, budget, normalized, judgment = scoring_fixture(0 if empty else 1)
    result = score_task_v2(task, normalized, judgment, None, raw, budget)
    if empty: supported, unsupported = 0, 0
    view = precision_view(supported, unsupported, 0, semantic_complete=complete,
        judgment_complete=complete, empty_confirmed=empty and complete)
    result.update(supportedClaims=supported, unsupportedClaims=unsupported, unjudgedClaims=0,
        semanticCoverageComplete=complete, judgmentComplete=complete, normalizationComplete=complete,
        precisionView=view, claimPrecision=view["fullMeasuredPrecision"],
        precisionWithEmptyZero=view["formalPrecisionEmptyZero"],
        falseEmpty=False if complete else None, success=complete and not unsupported)
    return result


class StatisticsTests(unittest.TestCase):
    def test_complete_inputs_preserve_the_existing_cluster_estimand_and_interval_exactly(self):
        rows = [("a", "one", .7, .4), ("a", "one", .8, .5), ("a", "two", 0., 1.),
                ("b", "three", 1., 0.), ("b", "four", .4, .4)]
        self.assertEqual(paired_interval_v2(rows, samples=1000), old.paired_interval(rows, samples=1000))
        self.assertEqual(paired_interval_v2(list(reversed(rows)), samples=1000),
            paired_interval_v2(rows, samples=1000))

    def test_unknown_precision_keeps_the_complete_cohort_and_has_no_numeric_interval(self):
        for candidate, comparator in ((.7, None), (None, .7), (None, None)):
            rows = [("a", "one", .7, .4), ("a", "two", candidate, comparator),
                    ("b", "three", .4, .7), ("b", "four", 1., 1.)]
            result = paired_interval_v2(rows)
            self.assertEqual(result["rowsRetained"], 4)
            self.assertEqual(result["clusters"], 4)
            self.assertEqual(result["repositories"], 2)
            self.assertEqual(result["missingPairRows"], 1)
            for key in ("estimate", "lower", "upper"): self.assertIsNone(result[key])
            self.assertNotIn("bootstrapLower", result)
            self.assertFalse(result["formalPrecisionComparisonAvailable"])

    def test_unknown_required_matrix_row_outside_this_cohort_still_blocks_precision(self):
        result = paired_interval_v2([("a", "one", .7, .4)], unavailable=True)
        self.assertIsNone(result["estimate"])
        self.assertEqual(result["rowsRetained"], 1)
        self.assertEqual(result["missingPairRows"], 0)

    def test_missing_first_pair_does_not_hide_late_invalid_values(self):
        for value in (True, float("nan"), float("inf"), -1, "0.7"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                paired_interval_v2([("a", "one", .7, None), ("a", "two", value, .7)])

    def test_unknown_summary_precision_and_false_empty_are_not_skip_none_averages(self):
        result = summary_v2([row(), row(0, 0, complete=False)])
        self.assertEqual(result["tasks"], 2)
        self.assertIsNone(result["claimPrecision"])
        self.assertIsNone(result["unsupportedClaimRate"])
        self.assertIsNone(result["falseEmptyRate"])
        self.assertEqual(result["diagnosticKnownClaimPrecision"], .7)
        self.assertEqual(result["precision"]["unknownRows"], 1)
        self.assertIsNone(result["precision"]["completeRowMacroEmptyZero"])

    def test_complete_empty_is_known_zero_denominator_not_unknown(self):
        result = summary_v2([row(), row(empty=True)])
        self.assertEqual(result["claimPrecision"], .7)
        self.assertEqual(result["precision"]["completeRowMacroEmptyZero"], .35)
        self.assertEqual(result["precision"]["unknownRows"], 0)
        self.assertIsNone(summary_v2([row(empty=True)])["claimPrecision"])
        self.assertIsNone(summary_v2([])["precision"])

    def test_late_forged_precision_or_derived_row_values_are_rejected(self):
        for field in ("precisionView", "claimPrecision", "supportedClaims", "judgmentComplete"):
            forged = copy.deepcopy(row())
            if field == "precisionView": forged[field]["formalPrecisionAvailable"] = False
            elif field == "claimPrecision": forged[field] = 1.
            elif field == "supportedClaims": forged[field] = 8
            else: forged[field] = False
            with self.subTest(field=field), self.assertRaises(ValueError):
                summary_v2([row(0, 0, complete=False), forged])


if __name__ == "__main__": unittest.main()
