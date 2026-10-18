"""Synthetic native exit contracts, never comparative quality measurements."""
import copy
from contextlib import redirect_stdout
import io
from pathlib import Path
import unittest

from benchmarks.agent_query.broad_query_review import _json, query_outcome
from benchmarks.agent_query.broad_query_capture import collect
from benchmarks.agent_query.tests import test_broad_query_capture as capture_fixture


CONTRACT = dict(schema="compass.broad-query-query-exits/1", noMatchExitCode=4,
                noMatchStdoutLine="No matching nodes found.")
RAW = b"No matching nodes found.\n"


class QueryOutcomeTests(unittest.TestCase):
    def test_no_match_requires_predeclared_exact_code_and_bytes(self):
        tool = dict(queryExitContract=CONTRACT)
        self.assertEqual(query_outcome(tool, 4, RAW), "no_match")
        self.assertEqual(query_outcome({}, 4, RAW), "failure")
        for code, raw in [(1, RAW), (-9, RAW), (4, RAW + b"extra\n"), (4, RAW.rstrip()),
                          (4, b"prefix " + RAW), (4, RAW.replace(b"\n", b"\r\n"))]:
            with self.subTest(code=code, raw=raw):
                self.assertEqual(query_outcome(tool, code, raw), "failure")
        self.assertEqual(query_outcome(tool, 0, b"ordinary output"), "success")

    def test_timeout_and_capture_limits_override_even_a_matching_no_match(self):
        for code in (0, 4):
            for flags in [dict(timed_out=True), dict(limited=True)]:
                self.assertEqual(query_outcome(dict(queryExitContract=CONTRACT), code, RAW, **flags), "failure")

    def test_unknown_malformed_and_unbounded_contracts_fail_even_on_exit_zero(self):
        for change in [dict(schema="compass.broad-query-query-exits/99"),
                       dict(extra=True), dict(noMatchExitCode=0), dict(noMatchExitCode=True),
                       dict(noMatchExitCode=256), dict(noMatchStdoutLine=""),
                       dict(noMatchStdoutLine="x" * 4097), dict(noMatchStdoutLine="bad\nline"),
                       dict(noMatchStdoutLine="   ")]:
            value = dict(CONTRACT, **change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                query_outcome(dict(queryExitContract=value), 0, b"")
        with self.assertRaises(ValueError):
            query_outcome(dict(queryExitContract=None), 0, b"")


class CapturedNoMatchTests(unittest.TestCase):
    """Use the actual collector and canonical assessor with the bounded fake CLI."""

    setUp = capture_fixture.MockCaptureTests.setUp
    put = capture_fixture.MockCaptureTests.put
    spec = capture_fixture.MockCaptureTests.spec
    runner = capture_fixture.MockCaptureTests.runner

    def no_match_spec(self, *, unsupported=False, declared=True):
        spec = self.spec()
        tool = next(t for t in spec["plan"]["tools"] if t["id"] == "graphify")
        tool["argvPrefix"] = ["--mock-comparison"]
        if declared:
            tool["queryExitContract"] = copy.deepcopy(CONTRACT)
        reg = _json(Path(spec["registration"]["path"]).read_bytes())
        reg["capturePlan"] = spec["plan"]
        if unsupported:
            reg["tasks"][0].update(kind="unsupported", expected="abstain", relevant=[], facts=[],
                                   rationale="Synthetic scoped absence in the pinned first-party source.")
        spec["registration"] = self.put("no-match-registration.json", reg)
        return spec

    def no_match_runner(self, argv, **kwargs):
        result = self.runner(argv, **kwargs)
        if "--mock-comparison" in argv:
            kwargs["stdout_path"].write_bytes(RAW)
            result.stdout = RAW.decode()
            result.exit_code = 4
        return result

    def test_no_match_is_an_execution_outcome_not_a_correctness_exemption(self):
        for unsupported in (False, True):
            dest = self.root / ("no-match-" + str(unsupported))
            with redirect_stdout(io.StringIO()):
                collect(self.no_match_spec(unsupported=unsupported), dest, runner=self.no_match_runner)
            run = _json((dest / "run.json").read_bytes())
            tool = next(t for t in run["tools"] if t["id"] == "graphify")
            self.assertEqual(tool["queryExitContract"], CONTRACT)
            assessment = _json((dest / "report-provisional.json").read_bytes())["assessment"]
            self.assertFalse(assessment["eligibleForSuperiorityClaim"])
            for obs in run["observations"]:
                if obs["tool"] != "graphify":
                    continue
                self.assertEqual(_json((dest / obs["normalized"]["path"]).read_bytes())["disposition"], "abstain")
                capture = _json((dest / obs["captures"][0]["path"]).read_bytes())
                self.assertEqual(capture["exit_code"], 4)
                self.assertEqual((dest / capture["stdout"]["path"]).read_bytes(), RAW)
            rows = [r for r in assessment["rows"] if r["tool"] == "graphify"]
            self.assertEqual(len(rows), 2)
            self.assertTrue(all(not r["executionFailed"] for r in rows))
            self.assertTrue(all(r["success"] is unsupported for r in rows))

    def test_legacy_contract_keeps_nonzero_exit_failure(self):
        dest = self.root / "legacy-no-match"
        with redirect_stdout(io.StringIO()):
            collect(self.no_match_spec(declared=False), dest, runner=self.no_match_runner)
        assessment = _json((dest / "report-provisional.json").read_bytes())["assessment"]
        rows = [r for r in assessment["rows"] if r["tool"] == "graphify"]
        self.assertTrue(all(r["executionFailed"] for r in rows))

    def test_limits_timeouts_and_mismatched_bytes_remain_failed_and_retained(self):
        for mode in ("timeout", "limit", "mismatch"):
            def failing_runner(argv, **kwargs):
                result = self.no_match_runner(argv, **kwargs)
                if "--mock-comparison" in argv:
                    if mode == "mismatch":
                        kwargs["stdout_path"].write_bytes(RAW + b"extra output\n")
                    else:
                        setattr(result, "timed_out" if mode == "timeout" else "output_limited", True)
                return result
            dest = self.root / mode
            with redirect_stdout(io.StringIO()):
                collect(self.no_match_spec(unsupported=True), dest, runner=failing_runner)
            report = _json((dest / "report-provisional.json").read_bytes())["assessment"]
            rows = [r for r in report["rows"] if r["tool"] == "graphify"]
            self.assertEqual(len(rows), 2)
            self.assertTrue(all(r["executionFailed"] and not r["success"] for r in rows))

    def test_a_declared_no_match_cannot_be_used_as_a_successful_answer(self):
        spec = self.no_match_spec()
        text = "NODE Store::save [src=store.py loc=L2 community=0]"
        spec["plan"]["tools"][1]["queryExitContract"]["noMatchStdoutLine"] = text
        reg = _json(Path(spec["registration"]["path"]).read_bytes())
        reg["capturePlan"] = spec["plan"]
        spec["registration"] = self.put("contradictory-no-match.json", reg)
        def contradictory(argv, **kwargs):
            result = self.no_match_runner(argv, **kwargs)
            if "--mock-comparison" in argv:
                kwargs["stdout_path"].write_bytes((text + "\n").encode())
            return result
        dest = self.root / "contradictory-no-match"
        with redirect_stdout(io.StringIO()):
            collect(spec, dest, runner=contradictory)
        assessment = _json((dest / "report-provisional.json").read_bytes())["assessment"]
        self.assertTrue(all(r["executionFailed"] and not r["success"]
                            for r in assessment["rows"] if r["tool"] == "graphify"))

    def test_contract_is_bound_to_the_frozen_plan_on_reassessment(self):
        from benchmarks.agent_query.broad_query_review import assess, sha
        dest = self.root / "contract-drift"
        with redirect_stdout(io.StringIO()):
            collect(self.no_match_spec(), dest, runner=self.no_match_runner)
        manifest = _json((dest / "review-input.json").read_bytes())
        run = _json((dest / manifest["run"]["path"]).read_bytes())
        next(t for t in run["tools"] if t["id"] == "graphify")["queryExitContract"]["noMatchExitCode"] = 7
        path = dest / "altered-run.json"
        import json
        data = json.dumps(run).encode()
        path.write_bytes(data)
        manifest["run"] = dict(path=path.name, sha256=sha(data))
        reviews = _json((dest / manifest["reviews"]["path"]).read_bytes())
        reviews["runSha256"] = sha(data)
        path = dest / "altered-reviews.json"
        data = json.dumps(reviews).encode()
        path.write_bytes(data)
        manifest["reviews"] = dict(path=path.name, sha256=sha(data))
        with self.assertRaisesRegex(ValueError, "query exit contract differs"):
            assess(manifest, dest)

    def test_matching_no_match_still_fails_a_normalized_output_budget(self):
        spec = self.no_match_spec(unsupported=True)
        text = "No support: " + "x" * 150
        spec["plan"]["tools"][1]["queryExitContract"]["noMatchStdoutLine"] = text
        reg = _json(Path(spec["registration"]["path"]).read_bytes())
        reg["capturePlan"] = spec["plan"]
        spec["registration"] = self.put("oversized-no-match.json", reg)
        def oversized(argv, **kwargs):
            result = self.no_match_runner(argv, **kwargs)
            if "--mock-comparison" in argv:
                kwargs["stdout_path"].write_bytes((text + "\n").encode())
            return result
        dest = self.root / "oversized-no-match"
        with redirect_stdout(io.StringIO()):
            collect(spec, dest, runner=oversized)
        assessment = _json((dest / "report-provisional.json").read_bytes())["assessment"]
        for row in assessment["rows"]:
            if row["tool"] == "graphify":
                self.assertEqual(row["executionFailed"], row["arm"] == "normalized")
                self.assertEqual(row["success"], row["arm"] == "default")

    def test_invalid_contract_rejects_before_any_process(self):
        spec = self.no_match_spec()
        spec["plan"]["tools"][1]["queryExitContract"]["schema"] += "-unknown"
        reg = _json(Path(spec["registration"]["path"]).read_bytes())
        reg["capturePlan"] = spec["plan"]
        spec["registration"] = self.put("invalid-no-match-registration.json", reg)
        with self.assertRaises(ValueError):
            collect(spec, self.root / "invalid-no-match", runner=self.no_match_runner)
        self.assertEqual(self.calls, [])
