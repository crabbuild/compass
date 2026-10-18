"""Synthetic file-backed report publication/recompute; no real qualification."""
import copy
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_report_v2 as reports
from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.tests.test_broad_query_assess_v2 import Fixture


class ReportTests(unittest.TestCase):
    def setUp(self):
        self.fixture = Fixture(self)
        self.output = self.fixture.bundle.root / "reviewed-report.json"
        self.git = patch("benchmarks.agent_query.broad_query_assess_v2.SourceReader.git",
            side_effect=lambda root, *args: "a" * 40 + "\n" if "rev-parse" in args else "")
        self.git.start()
        self.addCleanup(self.git.stop)

    def test_score_and_check_recompute_exact_diagnostic_without_query_execution(self):
        report = reports.score_file_v2(self.fixture.input, self.output)
        assessment = reports.check_file_v2(self.output)
        self.assertEqual(report["assessment"], assessment)
        self.assertFalse(assessment["eligibleForSuperiorityClaim"])
        self.assertEqual(len(assessment["rows"]), 4)
        self.assertEqual(report["inputPath"], str(self.fixture.input))
        self.assertEqual(self.output.read_bytes(), reports.encode_report(report))
        with self.assertRaises(FileExistsError): reports.score_file_v2(self.fixture.input, self.output)

    def test_stored_eligibility_metrics_actors_or_numeric_types_are_not_trusted(self):
        report = reports.score_file_v2(self.fixture.input, self.output)
        for change in ("eligibility", "metric", "actor", "number_type"):
            with self.subTest(change=change):
                forged = copy.deepcopy(report)
                if change == "eligibility": forged["assessment"]["eligibleForSuperiorityClaim"] = True
                if change == "metric": forged["assessment"]["costs"][0]["workflowWallMs"] += 1
                if change == "actor": forged["actorSha256"]["protocol"] = "0" * 64
                if change == "number_type": forged["assessment"]["costs"][0]["workflowWallMs"] = 10.0
                with self.assertRaisesRegex(ValueError, "actors differ|fabricated or are stale"):
                    reports.recompute_report_v2(forged)

    def test_legacy_unknown_and_relative_path_reports_fail_before_source_work(self):
        report = reports.score_file_v2(self.fixture.input, self.output)
        for field, value in (("schema", old.REPORT_SCHEMA), ("schema", "compass.broad-query-review/99"),
                ("inputPath", "relative.json")):
            with self.subTest(field=field, value=value):
                forged = copy.deepcopy(report)
                forged[field] = value
                with patch("benchmarks.agent_query.broad_query_assess_v2.SourceReader.git") as git:
                    with self.assertRaises(ValueError): reports.recompute_report_v2(forged)
                    git.assert_not_called()

    def test_final_encoding_to_publication_guards_cover_input_raw_actor_and_dependency(self):
        for target in ("input", "raw", "actor", "dependency"):
            with self.subTest(target=target):
                fixture = Fixture(self)
                output = fixture.bundle.root / "guarded-report.json"
                if target == "input": path = fixture.input
                elif target == "raw":
                    capture = old._json((fixture.bundle.root / fixture.observations[0]["captures"][0]["path"]).read_bytes())
                    path = fixture.bundle.root / capture["stdout"]["path"]
                elif target == "actor": path = fixture.bundle.root / fixture.run["protocolBinding"]["files"][0]["artifact"]["path"]
                else: path = Path(fixture.run["tools"][0]["dependencies"][0]["original"]["path"])
                encode = reports.encode_report
                reached = []
                def mutate_after_encoding(value):
                    encoded = encode(value)
                    reached.append(True)
                    path.write_bytes(path.read_bytes() + b" ")
                    return encoded
                with patch.object(reports, "encode_report", side_effect=mutate_after_encoding):
                    with self.assertRaises(ValueError): reports.score_file_v2(fixture.input, output)
                self.assertEqual(reached, [True])
                self.assertFalse(output.exists())

    def test_report_file_changed_during_check_is_rejected(self):
        reports.score_file_v2(self.fixture.input, self.output)
        original = reports.recompute_report_v2
        def changed(report):
            result = original(report)
            self.output.write_bytes(self.output.read_bytes() + b" ")
            return result
        with patch.object(reports, "recompute_report_v2", side_effect=changed):
            with self.assertRaises(ValueError): reports.check_file_v2(self.output)

    def test_output_must_remain_a_new_sibling_and_encoding_failure_publishes_nothing(self):
        with self.assertRaisesRegex(ValueError, "siblings"):
            reports.score_file_v2(self.fixture.input, self.fixture.outer / "outside.json")
        with patch.object(reports.old, "MAX_JSON_BYTES", 8):
            with self.assertRaisesRegex(ValueError, "report exceeds"):
                reports.encode_report(dict(value="longer than capacity"))
        with patch.object(reports, "encode_report", side_effect=ValueError("synthetic report encoding bound")):
            with self.assertRaisesRegex(ValueError, "encoding bound"):
                reports.score_file_v2(self.fixture.input, self.output)
        self.assertFalse(self.output.exists())

    def test_escaped_expansion_is_rejected_before_scalar_escape_allocation(self):
        escape = json.encoder.encode_basestring_ascii
        for scalar in ("\u0100" * 6, "\x01" * 6, "\U0001f600" * 3, '"' * 16):
            for value in (scalar, {scalar: 0}):
                with self.subTest(value_type=type(value).__name__, scalar=repr(scalar)), \
                        patch.object(reports.old, "MAX_JSON_BYTES", 32), \
                        patch.object(json.encoder, "encode_basestring_ascii", wraps=escape) as escaping:
                    with self.assertRaisesRegex(ValueError, "report exceeds"):
                        reports.encode_report(value)
                    escaping.assert_not_called()

    def test_long_valid_scalars_are_chunked_without_a_stricter_string_limit(self):
        value = "\u0100" * 3000 + "\U0001f600" + "\x00"
        expected = (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n").encode()
        escape = json.encoder.encode_basestring_ascii
        with patch.object(reports.old, "MAX_JSON_BYTES", len(expected)), \
                patch.object(json.encoder, "encode_basestring_ascii", wraps=escape) as escaping:
            self.assertEqual(reports.encode_report(value), expected)
            self.assertGreater(escaping.call_count, 1)
            self.assertTrue(all(len(call.args[0]) <= 1024 for call in escaping.call_args_list))
        with patch.object(reports.old, "MAX_JSON_BYTES", len(expected) - 1), \
                patch.object(json.encoder, "encode_basestring_ascii", wraps=escape) as escaping:
            with self.assertRaises(ValueError): reports.encode_report(value)
            escaping.assert_not_called()

    def test_exact_canonical_bytes_and_final_lf_boundaries_match_stdlib(self):
        cases = [None, True, False, 0, 9, 10, -10, 10**127, -(10**127), 1.25, -0.0, 1e-100,
            "", ''.join(chr(code) for code in range(256)) + '\ud800\U0001f600',
            {"z": [False, {}, [1, "\t"]], "\u0100": "\U0001f600", "a": dict(b=None)}, (1, 2)]
        for value in cases:
            expected = (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True, allow_nan=False) + "\n").encode()
            with self.subTest(value_type=type(value).__name__, bytes=len(expected)):
                with patch.object(reports.old, "MAX_JSON_BYTES", len(expected)):
                    self.assertEqual(reports.encode_report(value), expected)
                with patch.object(reports.old, "MAX_JSON_BYTES", len(expected) - 1):
                    with self.assertRaises(ValueError): reports.encode_report(value)

    def test_bad_wrapper_shape_is_rejected_before_recanonicalizing_content(self):
        with patch.object(reports, "encode_report", side_effect=AssertionError("unexpected scalar encoding")) as encode:
            with self.assertRaisesRegex(ValueError, "report schema"):
                reports.recompute_report_v2(dict(schema="wrong", assessment="\u0100" * 10000))
            encode.assert_not_called()


if __name__ == "__main__": unittest.main()
