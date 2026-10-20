"""Exact finite-population arithmetic on synthetic counts, never source judgments."""
from fractions import Fraction
from math import comb
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_graph_bounds as bounds


def frame(tool="compass", *, repository="one", population=10, sample=2,
          correct=2, incorrect=0, successful=True):
    return dict(repository=repository, tool=tool, population=population,
                sampleSize=sample, correct=correct, incorrect=incorrect,
                buildSuccessful=successful)


class CountBoundsTests(unittest.TestCase):
    def test_integer_tail_recurrence_matches_direct_enumeration(self):
        for population in range(11):
            for sample in range(population + 1):
                denominator = comb(population, sample)
                for actual in range(population + 1):
                    for observed in range(-1, sample + 2):
                        for upper in [False, True]:
                            expected = sum(comb(actual, k) * comb(population - actual, sample - k)
                                for k in range(max(0, sample - population + actual), min(sample, actual) + 1)
                                if (k >= observed if upper else k <= observed))
                            self.assertEqual(bounds._tail(population, actual, sample, observed,
                                denominator, upper, bounds._Work()), expected)

    def test_small_hand_calculated_endpoints_and_census(self):
        self.assertEqual(bounds.count_bounds(10, 2, 2, 0), (3, 10))
        self.assertEqual(bounds.count_bounds(10, 2, 0, 2), (0, 7))
        self.assertEqual(bounds.count_bounds(10, 2, 1, 1), (1, 9))
        self.assertEqual(bounds.count_bounds(10, 10, 7, 2), (7, 8))
        self.assertEqual(bounds.count_bounds(10, 0, 0, 0), (0, 10))
        self.assertIsNone(bounds.count_bounds(0, 0, 0, 0))

    def test_exact_exhaustive_hypergeometric_coverage(self):
        # Enumerate every possible sample count and its exact probability;
        # this checks the confidence procedure rather than a simulation seed.
        for family in (1, 2, 4):
            for population in range(1, 11):
                for sample in range(1, population + 1):
                    for actual in range(population + 1):
                        failure = Fraction(0)
                        for observed in range(max(0, sample - population + actual),
                                              min(sample, actual) + 1):
                            low, high = bounds.count_bounds(population, sample,
                                observed, sample - observed, family_size=family)
                            if not low <= actual <= high:
                                failure += Fraction(comb(actual, observed) *
                                    comb(population - actual, sample - observed),
                                    comb(population, sample))
                        self.assertLessEqual(failure, Fraction(1, 20 * family),
                            (population, sample, actual, family, failure))

    def test_unknown_labels_enclose_every_completion(self):
        for correct in range(5):
            for incorrect in range(5 - correct):
                low, high = bounds.count_bounds(20, 4, correct, incorrect)
                for actual_sample in range(correct, 4 - incorrect + 1):
                    complete = bounds.count_bounds(20, 4, actual_sample, 4 - actual_sample)
                    self.assertLessEqual(low, complete[0])
                    self.assertGreaterEqual(high, complete[1])

    def test_simultaneous_family_widens_without_changing_sample(self):
        ordinary = bounds.count_bounds(10, 2, 2, 0)
        simultaneous = bounds.count_bounds(10, 2, 2, 0, family_size=2)
        self.assertLessEqual(simultaneous[0], ordinary[0])
        self.assertGreaterEqual(simultaneous[1], ordinary[1])

    def test_invalid_counts_and_work_limit_fail_explicitly(self):
        for args in [(True, 1, 1, 0), (10, 11, 1, 0), (10, 2, 2, 1),
                     (4_000_001, 2, 1, 1), (5000, 4097, 1, 1)]:
            with self.subTest(args=args), self.assertRaises(ValueError):
                bounds.count_bounds(*args)
        with patch.object(bounds, "MAX_ARITHMETIC_WORK", 1), self.assertRaisesRegex(ValueError, "work limit"):
            bounds.count_bounds(100, 10, 9, 1)


class StratifiedBoundsTests(unittest.TestCase):
    def test_census_weighting_uses_populations_not_average_graphs(self):
        frames = [frame(population=1, sample=1, correct=0, incorrect=1),
                  frame(repository="two", population=9, sample=9, correct=9),
                  frame("graphify", population=1, sample=1, correct=1),
                  frame("graphify", repository="two", population=9, sample=9, correct=0, incorrect=9)]
        result = bounds.population_bounds(frames)
        self.assertEqual(result["tools"]["compass"], {"lower": {"numerator": 9, "denominator": 10},
                                                      "upper": {"numerator": 9, "denominator": 10}})
        self.assertEqual(result["difference"], {"lower": {"numerator": 4, "denominator": 5},
                                                "upper": {"numerator": 4, "denominator": 5}})
        self.assertEqual(result, bounds.population_bounds(list(reversed(frames))))
        self.assertFalse(result["qualificationEstablished"])
        self.assertFalse(result["registeredDesignAccepted"])

    def test_empty_and_failed_builds_never_imply_perfect_precision(self):
        empty = [frame(tool, population=0, sample=0, correct=0) for tool in bounds.TOOLS]
        result = bounds.population_bounds(empty)
        self.assertIsNone(result["tools"]["compass"])
        self.assertIsNone(result["difference"])
        failed = bounds.population_bounds([frame(successful=False), frame("graphify")])
        self.assertIsNone(failed["tools"]["compass"])
        self.assertIsNone(failed["difference"])

    def test_missing_or_duplicate_tool_frames_reject(self):
        for frames in [[frame()], [frame(), frame(), frame("graphify")]]:
            with self.assertRaises(ValueError):
                bounds.population_bounds(frames)

    def test_no_review_remains_full_uncertainty(self):
        result = bounds.population_bounds([frame(correct=0), frame("graphify", correct=0)])
        self.assertEqual(result["tools"]["compass"]["lower"]["numerator"], 0)
        self.assertEqual(result["tools"]["compass"]["upper"], {"numerator": 1, "denominator": 1})
        self.assertEqual(result["difference"]["lower"], {"numerator": -1, "denominator": 1})
        self.assertEqual(result["difference"]["upper"], {"numerator": 1, "denominator": 1})

    def test_aggregate_work_failure_has_no_successful_prefix(self):
        with patch.object(bounds, "MAX_ARITHMETIC_WORK", 1), self.assertRaisesRegex(ValueError, "work limit"):
            bounds.population_bounds([frame(), frame("graphify")])

    def test_population_sample_and_frame_limits_are_checked_before_arithmetic(self):
        oversized = [frame(repository=str(i)) for i in range(65)]
        for frames in [oversized, [frame(population=True), frame("graphify")],
                       [dict(frame(), extra=True), frame("graphify")]]:
            with self.assertRaises(ValueError):
                bounds.population_bounds(frames)
        many = [frame(tool, repository=str(i), population=4096, sample=4096, correct=4096)
                for i in range(9) for tool in bounds.TOOLS]
        with self.assertRaisesRegex(ValueError, "aggregate"):
            bounds.population_bounds(many)


if __name__ == "__main__":
    unittest.main()
