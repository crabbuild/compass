"""Synthetic metric-contract feedback only; no actual quality evidence."""
import unittest
from benchmarks.agent_query.broad_query_protocol import precision_view, aggregate_precision, paired_precision_status


def complete(s=7, u=3):
    return precision_view(s,u,0,semantic_complete=True,judgment_complete=True)


def unknown():
    return precision_view(0,0,1,semantic_complete=False,judgment_complete=False)


class PrecisionCompletenessTests(unittest.TestCase):
    def assert_rejected_everywhere(self, row):
        for operation in (
            lambda: aggregate_precision([row]),
            lambda: paired_precision_status({"one":row}, {"one":complete()}),
            lambda: paired_precision_status({"one":complete()}, {"one":row}),
        ):
            with self.assertRaises(ValueError): operation()

    def test_forged_availability_cannot_promote_unknown_precision(self):
        row = precision_view(7,3,0,semantic_complete=False,judgment_complete=False)
        row["formalPrecisionAvailable"] = True
        row["formalPrecisionEmptyZero"] = .7
        self.assert_rejected_everywhere(row)

    def test_aggregate_validates_all_primitive_counts_and_flags(self):
        for key in ("knownSupported", "knownUnsupported", "knownUnjudged"):
            for value in (-1, True, 2049, 1.0, "1", None):
                with self.subTest(key=key, value=value):
                    row = complete(); row[key] = value
                    self.assert_rejected_everywhere(row)
        for key in ("semanticCoverageComplete", "judgmentComplete", "emptyConfirmed"):
            for value in (0, 1, None, "true"):
                with self.subTest(key=key, value=value):
                    row = complete(); row[key] = value
                    self.assert_rejected_everywhere(row)
        row = complete(); row["knownSupported"] = 2048
        self.assert_rejected_everywhere(row)
        row = complete(); row["emptyConfirmed"] = True
        self.assert_rejected_everywhere(row)
        row = complete(); row["knownUnjudged"] = 1
        self.assert_rejected_everywhere(row)

    def test_aggregate_rejects_derived_field_shape_type_and_nan_drift(self):
        for key in complete():
            row = complete(); del row[key]
            with self.subTest(missing=key): self.assert_rejected_everywhere(row)
        for key, value in (("knownClaimCount",11), ("knownClaimCount",10.0),
                           ("fullMeasuredPrecision",True), ("formalPrecisionAvailable",1),
                           ("note","untrusted alternate meaning"), ("unexpected",True)):
            with self.subTest(key=key, value=value):
                row = complete(); row[key] = value
                self.assert_rejected_everywhere(row)
        for key in ("fullMeasuredPrecision", "formalPrecisionEmptyZero", "judgedOnlyPrecision",
                    "knownSupportFraction", "completionPenalizedPrecision"):
            for value in (float("nan"), float("inf"), -.1, True):
                with self.subTest(key=key, value=value):
                    row = complete(); row[key] = value
                    self.assert_rejected_everywhere(row)
        for value in (None, [], "precision"):
            with self.subTest(row=value): self.assert_rejected_everywhere(value)

    def test_canonical_rows_retain_unknown_and_empty_without_mutation(self):
        empty = precision_view(0,0,0,semantic_complete=True,judgment_complete=True,empty_confirmed=True)
        rows = [complete(), empty, unknown()]
        before = [dict(row) for row in rows]
        result = aggregate_precision(rows)
        self.assertEqual(rows,before)
        self.assertEqual(result["rowsRetained"],3)
        self.assertEqual(result["unknownRows"],1)
        self.assertIsNone(result["fullMeasuredPooledPrecision"])
        self.assertIsNone(result["completeRowMacroEmptyZero"])
        self.assertAlmostEqual(result["diagnosticCompletionPenalizedMacro"],.7/3)
        measured = aggregate_precision(rows[:2])
        self.assertAlmostEqual(measured["fullMeasuredPooledPrecision"],.7)
        self.assertAlmostEqual(measured["completeRowMacroEmptyZero"],.35)
        pair = paired_precision_status(dict(enumerate(rows)),dict(enumerate(rows)))
        self.assertEqual(pair["rowsRetained"],3)
        self.assertFalse(pair["precisionInputsComplete"])
        self.assertNotIn("formalDifference",pair)
        self.assertNotIn("nonInferiority",pair)

    def test_zero_fill_cannot_manufacture_paired_precision_ni(self):
        row = paired_precision_status({"one":complete()}, {"one":unknown()})
        self.assertAlmostEqual(row["diagnosticCompletionPenalizedDifference"], .7)
        self.assertNotIn("formalDifference",row)
        self.assertFalse(row["formalPrecisionComparisonAvailable"])
        self.assertFalse(row["precisionInputsComplete"])
        self.assertEqual(row["rowsRetained"],1)

    def test_both_incomplete_does_not_become_equal_precision(self):
        row = paired_precision_status({"one":unknown()}, {"one":unknown()})
        self.assertEqual(row["diagnosticCompletionPenalizedDifference"],0)
        self.assertNotIn("formalDifference",row)
        self.assertFalse(row["precisionInputsComplete"])

    def test_any_unknown_prevents_skip_none_and_known_count_pooling(self):
        row = aggregate_precision([complete(),unknown()])
        self.assertEqual(row["rowsRetained"],2)
        self.assertEqual(row["unknownRows"],1)
        self.assertIsNone(row["completeRowMacroEmptyZero"])
        self.assertIsNone(row["fullMeasuredPooledPrecision"])
        self.assertAlmostEqual(row["diagnosticCompletionPenalizedMacro"],.35)

    def test_unjudged_full_coverage_and_unknown_coverage_are_distinct(self):
        labeled = precision_view(7,2,1,semantic_complete=True,judgment_complete=False)
        hidden = precision_view(7,2,1,semantic_complete=False,judgment_complete=False)
        self.assertAlmostEqual(labeled["knownSupportFraction"],.7)
        self.assertIsNone(hidden["knownSupportFraction"])
        for row in (labeled,hidden):
            self.assertAlmostEqual(row["judgedOnlyPrecision"],7/9)
            self.assertIsNone(row["fullMeasuredPrecision"])
            self.assertIsNone(row["formalPrecisionEmptyZero"])

    def test_confirmed_empty_is_not_a_rejection_with_zero_records(self):
        empty = precision_view(0,0,0,semantic_complete=True,judgment_complete=True,empty_confirmed=True)
        rejected = precision_view(0,0,0,semantic_complete=False,judgment_complete=False)
        self.assertIsNone(empty["fullMeasuredPrecision"])
        self.assertEqual(empty["formalPrecisionEmptyZero"],0)
        self.assertTrue(empty["formalPrecisionAvailable"])
        self.assertIsNone(rejected["formalPrecisionEmptyZero"])
        self.assertFalse(rejected["formalPrecisionAvailable"])

    def test_complete_unchanged_empty_zero_arithmetic_and_matrix_identity(self):
        row = paired_precision_status({"a":complete(),"b":complete(1,0)},
                                      {"a":complete(6,4),"b":complete(1,0)})
        self.assertNotIn("formalDifference",row)
        self.assertAlmostEqual(row["diagnosticCompletionPenalizedDifference"],.05)
        self.assertTrue(row["formalPrecisionComparisonAvailable"])
        with self.assertRaises(ValueError):
            paired_precision_status({"a":complete()}, {"other":complete()})
        for args, kwargs in [((0,0,0),dict(semantic_complete=True,judgment_complete=True)),
                             ((1,0,1),dict(semantic_complete=True,judgment_complete=True)),
                             ((1,0,0),dict(semantic_complete=True,judgment_complete=True,empty_confirmed=True))]:
            with self.assertRaises(ValueError): precision_view(*args,**kwargs)


if __name__ == "__main__": unittest.main()
