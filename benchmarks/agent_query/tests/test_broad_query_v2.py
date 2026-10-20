"""Public developer CLI contracts with fake native tools; no quality claim."""
from contextlib import redirect_stdout, redirect_stderr
import copy
import io
import json
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_v2 as cli
from benchmarks.agent_query import broad_query_capture as legacy
from benchmarks.agent_query.broad_query_collect_v2 import collect_file_v2
from benchmarks.agent_query.broad_query_actors_v2 import freeze_protocol, ROLES
from benchmarks.agent_query.broad_query_review import sha
from benchmarks.agent_query.tests.test_broad_query_panels_v2 import PanelFixture
from benchmarks.agent_query.tests import test_broad_query_collect_v2 as collector_tests


class CommandTests(unittest.TestCase):
    def invoke(self, args):
        output, error = io.StringIO(), io.StringIO()
        with redirect_stdout(output), redirect_stderr(error):
            code = cli.main(args)
        return code, output.getvalue(), error.getvalue()

    def test_capture_score_and_check_preserve_one_union_run_and_fail_qualification_honestly(self):
        fixture = PanelFixture(self)
        case = fixture.case
        spec = case.root / "capture-invocation.json"
        spec.write_bytes(legacy.encoded(case.spec))
        def capture(path, output):
            return collect_file_v2(path, output, runner=case.runner)
        with patch.object(cli, "collect_file_v2", side_effect=capture), \
                patch("benchmarks.agent_query.broad_query_assess_v2.SourceReader.git",
                    side_effect=lambda root, *args: "a" * 40 + "\n" if "rev-parse" in args else ""):
            code, output, error = self.invoke(["--capture", str(spec), "--output", str(case.destination)])
            self.assertEqual((code, error), (0, ""))
            captured = json.loads(output)
            self.assertFalse(captured["qualificationEstablished"])
            self.assertEqual(captured["observations"], 12)
            review = case.destination / "review-input.json"
            report = case.destination / "new-report.json"
            code, output, error = self.invoke(["--score", str(review), "--output", str(report)])
            self.assertEqual((code, error), (0, ""))
            self.assertFalse(json.loads(output)["eligibleForSuperiorityClaim"])
            code, output, error = self.invoke(["--check", str(report)])
            self.assertEqual((code, error), (1, ""))
            status = json.loads(output)
            self.assertEqual(set(status["panelEligibility"]), {"original", "lookup", "union"})
            self.assertLessEqual(len(status["failures"]), 10)
            self.assertGreater(status["failureCount"], 0)
            self.assertEqual(len(case.queries), 12)
            stored = json.loads(report.read_bytes())
            stored["assessment"]["eligibleForSuperiorityClaim"] = True
            forged = case.destination / "forged.json"
            forged.write_bytes(legacy.encoded(stored))
            code, output, error = self.invoke(["--check", str(forged)])
            self.assertEqual((code, output), (2, ""))
            self.assertIn("fabricated or are stale", error)

    def test_unknown_and_legacy_capture_versions_execute_nothing(self):
        fixture = PanelFixture(self)
        case = fixture.case
        for schema in (legacy.INPUT_SCHEMA, "compass.broad-query-capture-input/99"):
            path = case.root / "unsupported.json"
            path.write_bytes(legacy.encoded(dict(schema=schema)))
            code, output, error = self.invoke(["--capture", str(path), "--output", str(case.destination)])
            self.assertEqual((code, output), (2, ""))
            self.assertIn("unsupported or mixed protocol2", error)
            self.assertEqual(case.calls, [])
            self.assertFalse(case.destination.exists())

    def test_actual_invocation_drift_has_exit_two_and_no_complete_marker(self):
        fixture = PanelFixture(self)
        case = fixture.case
        path = case.root / "actual-invocation.json"
        path.write_bytes(legacy.encoded(case.spec))
        def drift(argv, captured):
            if argv[0] == "git" and "status" in argv and "-after-" in str(captured):
                path.write_bytes(path.read_bytes() + b" ")
        case.after = drift
        with patch.object(cli, "collect_file_v2", side_effect=lambda path, output:
                collect_file_v2(path, output, runner=case.runner)):
            code, output, error = self.invoke(["--capture", str(path), "--output", str(case.destination)])
        self.assertEqual((code, output), (2, ""))
        self.assertTrue(error)
        self.assertEqual(len(case.queries), 12)
        self.assertFalse((case.destination / "COMPLETE.json").exists())

    def test_frozen_roles_bind_the_actual_public_entry_and_domain_owners(self):
        fixture = PanelFixture(self)
        descriptor, actors = freeze_protocol(fixture.case.code_root)
        self.assertEqual(ROLES["capture"], "benchmarks/agent_query/broad_query_v2.py")
        for role, module in ROLES.items():
            self.assertEqual(actors[role], sha((fixture.case.code_root / module).read_bytes()))
        self.assertIn("reporter", actors)
        self.assertEqual(descriptor["roles"], ROLES)

    def test_incomplete_capture_has_bounded_error_no_retry_and_no_complete(self):
        fixture = PanelFixture(self)
        case = fixture.case
        path = case.root / "incomplete-invocation.json"
        path.write_bytes(legacy.encoded(case.spec))
        attempts = []
        def incomplete(argv, **kwargs):
            attempts.append(tuple(argv))
            raise RuntimeError("incomplete subprocess output capture\n" + "\x1b\0😀" * 10000)
        with patch.object(cli, "collect_file_v2", side_effect=lambda path, output:
                collect_file_v2(path, output, runner=incomplete)):
            code, output, error = self.invoke(["--capture", str(path), "--output", str(case.destination)])
        self.assertEqual((code, output), (2, ""))
        self.assertEqual(len(attempts), 1)
        self.assertEqual(error.count("\n"), 1)
        self.assertNotIn("\x1b", error)
        self.assertNotIn("\0", error)
        self.assertIn("incomplete subprocess output capture\\u000a", error)
        self.assertLessEqual(len(error.encode()), 2048 + len("broad-query protocol2 rejected: \n"))
        self.assertFalse((case.destination / "COMPLETE.json").exists())

    def test_malformed_frozen_records_reject_before_any_native_command(self):
        case = collector_tests.CollectorTests()
        case.setUp()
        self.addCleanup(case.doCleanups)
        baseline, registration = copy.deepcopy(case.spec), copy.deepcopy(case.registration)
        for field in ("tools", "arms", "graphs", "measurement", "settings", "environment", "childEnvironment"):
            for malformed in ("not an object", None):
                with self.subTest(field=field, malformed=malformed):
                    case.spec, case.registration = copy.deepcopy(baseline), copy.deepcopy(registration)
                    plan = case.spec["plan"]
                    if field in ("tools", "arms", "graphs"):
                        plan[field][0] = malformed
                    elif field == "settings":
                        plan["tools"][0]["settings"] = malformed
                    elif field == "environment":
                        plan["tools"][0]["environment"] = case.fixture.put("malformed-environment", malformed)
                    else:
                        plan[field] = malformed
                    case.registration["arms"] = plan["arms"]
                    case.refreeze()
                    path = case.root / "malformed-invocation.json"
                    path.write_bytes(legacy.encoded(case.spec))
                    with patch.object(cli, "collect_file_v2", side_effect=lambda path, output:
                            collect_file_v2(path, output, runner=case.runner)):
                        code, output, error = self.invoke(["--capture", str(path), "--output", str(case.destination)])
                    self.assertEqual((code, output), (2, ""))
                    self.assertIn("broad-query protocol2 rejected:", error)
                    self.assertEqual(case.calls, [])
                    self.assertFalse((case.destination / "COMPLETE.json").exists())

    def test_error_preview_bounds_unicode_and_cancellation_still_propagates(self):
        for detail in ("😀" * 10000, "\r\n\x1b\0" * 10000, "ascii" * 10000):
            preview = cli._error_preview(ValueError(detail))
            self.assertLessEqual(len(preview.encode()), 2048)
            self.assertTrue(preview.isprintable())
        with patch.object(cli, "collect_file_v2", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.invoke(["--capture", "input", "--output", "destination"])

    def test_argument_errors_do_not_collect_or_score(self):
        for args in (["--capture", "input"], ["--score", "input"], ["--check", "report", "--output", "bad"]):
            with self.subTest(args=args), patch.object(cli, "collect_file_v2") as collect, \
                    patch.object(cli, "score_file_v2") as score, redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error: cli.main(args)
                self.assertEqual(error.exception.code, 2)
                collect.assert_not_called()
                score.assert_not_called()


if __name__ == "__main__": unittest.main()
