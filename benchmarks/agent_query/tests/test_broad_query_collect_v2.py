"""Synthetic collector-to-assessor integration; no installed tools or services."""
import copy
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_capture as capture
from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_actors_v2 import freeze_protocol
from benchmarks.agent_query.broad_query_collect_v2 import CaptureBundle, collect_v2, collect_file_v2
from benchmarks.agent_query.broad_query_inputs_v2 import FrozenInputs, BoundedArtifacts
from benchmarks.agent_query.broad_query_lineage_v2 import SCHEMAS, load_review_lineage
from benchmarks.agent_query.broad_query_protocol import ProtocolLimit
from benchmarks.agent_query.tests import test_broad_query_capture as capture_tests
from benchmarks.agent_query.tests.test_broad_query_calibration import calibration_registration


class CollectorTests(unittest.TestCase):
    def setUp(self):
        self.fixture = capture_tests.MockCaptureTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.destination = self.root / "evidence-v2"
        self.spec = self.fixture.spec()
        self.spec["schema"] = SCHEMAS["capture-input"]
        plan = self.spec["plan"]
        plan["schema"] = SCHEMAS["capture-plan"]
        plan.pop("normalizerSha256")
        plan.pop("assessorSha256")
        self.code_root = Path(__file__).resolve().parents[3]
        plan["actorSha256"] = freeze_protocol(self.code_root)[1]
        self.registration = old._json(capture.external(self.spec["registration"])[1])
        self.refreeze()
        self.calls, self.queries = [], []
        self.mode = "normal"
        self.after = None

    def refreeze(self):
        self.registration["capturePlan"] = self.spec["plan"]
        self.spec["registration"] = self.fixture.put("registration-v2.json", self.registration)

    def document(self, reference):
        return old._json((self.destination / reference["path"]).read_bytes())

    def runner(self, argv, *, cwd, timeout_seconds, stdout_path, stderr_path):
        self.calls.append(tuple(argv))
        if argv[0] == "git":
            raw = b"a" * 40 + b"\n" if "rev-parse" in argv else b""
            error = b""
        else:
            self.queries.append(tuple(argv))
            graph = argv[argv.index("--graph") + 1]
            raw = (b"NODE Store.save() [src=store.py loc=L2]\n" if "graphify" in graph
                else b"NODE Store::save [function] store.py:2:1-3:28\n")
            if self.mode == "utf8": raw = b"\xff"
            if self.mode == "line": raw = b"x" * 65537
            if self.mode == "pages": raw = b"x\n" * 65537
            if self.mode == "overflow": raw += b"extra printed details\n" * 20
            if self.mode == "no_match": raw = b"No matching nodes found.\n"
            error = (b"123 maximum resident set size\n" if self.spec["plan"]["measurement"]["style"] == "darwin-l"
                else b"Maximum resident set size (kbytes): 123\n")
            if self.mode == "rss":
                error = (("9" * 5000 + " maximum resident set size\n")
                    if self.spec["plan"]["measurement"]["style"] == "darwin-l" else
                    "Maximum resident set size (kbytes): " + "9" * 5000 + "\n").encode()
        stdout_path.parent.mkdir(parents=True, exist_ok=True)
        stdout_path.write_bytes(raw)
        stderr_path.write_bytes(error)
        if self.after is not None: self.after(argv, stdout_path)
        exit_code = 7 if self.mode == "failure" and argv[0] != "git" else 0
        if self.mode == "no_match" and argv[0] != "git": exit_code = 1
        return SimpleNamespace(argv=argv, exit_code=exit_code,
            timed_out=self.mode == "timeout" and argv[0] != "git",
            output_limited=self.mode == "limit" and argv[0] != "git", wall_ms=10,
            # Deliberately untrusted decoded values/counts. Actual retained raw
            # bytes, not these replacement strings, determine normalization/cost.
            stdout="replacement", stderr="replacement", stdout_bytes=0, stderr_bytes=0)

    def collect(self):
        with patch("benchmarks.agent_query.broad_query_assess_v2.SourceReader.git",
                side_effect=lambda root, *args: "a" * 40 + "\n" if "rev-parse" in args else ""):
            return collect_v2(self.spec, self.destination, runner=self.runner)

    def test_full_once_only_matrix_pending_lineage_and_assessment(self):
        task = self.registration["tasks"][0]
        self.registration["tasks"] = [dict(copy.deepcopy(task), id=name) for name in ("z-save", "a-save")]
        self.spec["plan"]["tools"].reverse()
        self.refreeze()
        result = self.collect()
        self.assertEqual(result["observations"], 8)
        self.assertEqual(len(self.queries), 8)
        manifest = old._json((self.destination / "review-input.json").read_bytes())
        lineage = load_review_lineage(manifest, BoundedArtifacts(self.destination), self.code_root)
        expected = [(task, tool, arm) for task in ("a-save", "z-save") for arm in old.ARMS for tool in old.TOOLS]
        self.assertEqual(lineage.keys, tuple(expected))
        for observation in lineage.iter_observations():
            self.assertEqual(observation.initial, observation.reviewed)
            self.assertFalse(observation.initial["normalizationComplete"])
            self.assertFalse(observation.judgment["semanticCoverageComplete"])
            self.assertEqual(observation.judgment["independentReview"]["status"], "pending")
            self.assertEqual(observation.capture["stdout_bytes"], len(observation.stdout))
        lineage.recheck()
        report = old._json(Path(result["report"]).read_bytes())["assessment"]
        self.assertEqual(len(report["rows"]), 8)
        self.assertEqual(len(report["costs"]), 8)
        self.assertFalse(report["eligibleForSuperiorityClaim"])
        self.assertIsNone(report["comparisons"]["claimPrecisionEmptyZero"]["estimate"])
        marker = old._json((self.destination / "COMPLETE.json").read_bytes())
        self.assertFalse(marker["qualificationEstablished"])
        for argv in self.calls:
            if argv[0] == "git":
                self.assertIn("--no-optional-locks", argv)
                self.assertIn("core.fsmonitor=false", argv)
        with self.assertRaises(FileExistsError): self.collect()
        self.assertEqual(len(self.queries), 8)

    def test_invalid_utf8_and_physical_limits_retain_all_raw_rows_and_costs(self):
        for mode, code in (("utf8", "INVALID_UTF8"), ("line", "LINE_BYTE_LIMIT"), ("pages", "PHYSICAL_LINE_LIMIT")):
            with self.subTest(mode=mode):
                self.mode = mode
                self.destination = self.root / ("rejection-" + mode)
                result = self.collect()
                self.assertEqual(result["observations"], 4)
                manifest = old._json((self.destination / "review-input.json").read_bytes())
                run = self.document(manifest["run"])
                for row in run["observations"]:
                    answer = self.document(row["normalized"])
                    self.assertEqual(answer["normalizationFailure"]["code"], code)
                    self.assertIsNone(answer["printedAssertionCount"])
                    self.assertEqual(answer["ranked"], [])
                report = old._json(Path(result["report"]).read_bytes())["assessment"]
                self.assertEqual(len(report["costs"]), 4)
                self.assertTrue(all(row["evaluationRejected"] for row in report["rows"]))
                self.assertTrue(all(row["claimPrecision"] is None for row in report["rows"]))

    def test_json_parser_limit_is_a_typed_row_rejection_not_bundle_loss(self):
        with patch("benchmarks.agent_query.broad_query_normalize_v2._json_size",
                side_effect=ProtocolLimit("JSON_BYTE_LIMIT", "encoded_json_bytes", old.MAX_JSON_BYTES,
                    old.MAX_JSON_BYTES, old.MAX_JSON_BYTES + 1)):
            result = self.collect()
        report = old._json(Path(result["report"]).read_bytes())["assessment"]
        self.assertEqual(len(report["costs"]), 4)
        self.assertTrue(all(row["evaluationFailure"]["code"] == "JSON_BYTE_LIMIT" for row in report["rows"]))

    def test_failures_timeouts_capture_limits_and_budget_overflow_keep_matrix(self):
        for mode in ("failure", "timeout", "limit", "overflow"):
            with self.subTest(mode=mode):
                self.mode = mode
                self.destination = self.root / ("execution-" + mode)
                result = self.collect()
                report = old._json(Path(result["report"]).read_bytes())["assessment"]
                self.assertEqual(len(report["rows"]), 4)
                self.assertEqual(len(report["costs"]), 4)
                selected = [row for row in report["costs"] if mode != "overflow" or row["arm"] == "normalized"]
                self.assertTrue(all(row["failed"] for row in selected))
                if mode == "overflow":
                    self.assertTrue(all(row["overBudget"] and row["firstPageBytes"] > 128 for row in selected))

    def test_bad_freeze_missing_dependency_and_source_review_execute_nothing(self):
        baseline = copy.deepcopy(self.spec)
        registration = copy.deepcopy(self.registration)
        for change in ("actors", "dependency", "review", "arm", "roots", "schema", "source", "author_blind"):
            with self.subTest(change=change):
                self.spec = copy.deepcopy(baseline)
                self.registration = copy.deepcopy(registration)
                if change == "actors": self.spec["plan"]["actorSha256"]["capture"] = "0" * 64
                if change == "dependency":
                    self.spec["plan"]["tools"][0]["environment"] = self.fixture.put("bad-environment", dict(files=[dict(
                        path=str(self.root / "absent"), sha256="0" * 64)]))
                if change == "review": self.registration["independentReview"]["status"] = "pending"
                if change == "author_blind": self.registration["sourceReview"]["blindToOutputs"] = False
                if change == "arm": self.spec["plan"]["arms"][0]["compassView"] = "ranked"
                if change == "roots": self.spec["sourceRoots"]["unexpected"] = str(self.root)
                if change == "source": self.registration["repositories"][0]["files"][0]["sha256"] = "0" * 64
                self.refreeze()
                if change == "schema": self.spec["schema"] = capture.INPUT_SCHEMA
                with self.assertRaises((ValueError, OSError)): self.collect()
                self.assertEqual(self.calls, [])
                self.assertFalse((self.destination / "COMPLETE.json").exists())

    def test_late_original_or_retained_drift_never_publishes_complete(self):
        for target in ("original", "raw", "actor"):
            with self.subTest(target=target):
                self.destination = self.root / ("drift-" + target)
                original = Path(self.spec["plan"]["tools"][0]["executable"]["path"])
                original_data = original.read_bytes()
                def drift(argv, path):
                    if argv[0] == "git" and "status" in argv and "-after-" in str(path):
                        if target == "original": original.write_bytes(b"changed")
                        elif target == "raw":
                            (self.destination / "queries/save/compass-default.stdout").write_bytes(b"changed")
                        else:
                            copy_path = self.destination / "provenance/protocol/benchmarks/agent_query/broad_query_normalize_v2.py"
                            copy_path.write_bytes(b"changed")
                self.after = drift
                try:
                    with self.assertRaises(ValueError): self.collect()
                    self.assertFalse((self.destination / "COMPLETE.json").exists())
                finally:
                    original.write_bytes(original_data)

    def test_missing_rss_is_retained_as_unavailable(self):
        def remove_rss(argv, path):
            if argv[0] != "git": path.with_suffix(".stderr").write_bytes(b"no measurement\n")
        self.after = remove_rss
        result = self.collect()
        report = old._json(Path(result["report"]).read_bytes())["assessment"]
        self.assertEqual(len(report["costs"]), 4)
        self.assertTrue(all(row["peakRssBytes"] is None for row in report["costs"]))
        self.assertTrue(any("RSS" in failure for failure in report["failures"]))

    def test_large_numeric_rss_preserves_entire_matrix_and_original_stderr_for_both_styles(self):
        self.mode = "rss"
        for style, system in (("darwin-l", "Darwin"), ("gnu-v", "Linux")):
            with self.subTest(style=style):
                self.destination = self.root / ("rss-" + system)
                self.spec["plan"]["measurement"]["style"] = style
                self.refreeze()
                with patch("benchmarks.agent_query.broad_query_collect_v2.platform.system", return_value=system):
                    result = self.collect()
                report = old._json(Path(result["report"]).read_bytes())["assessment"]
                self.assertEqual(len(report["costs"]), 4)
                self.assertTrue(all(row["peakRssBytes"] is None for row in report["costs"]))
                manifest = old._json((self.destination / "review-input.json").read_bytes())
                run = self.document(manifest["run"])
                for row in run["observations"]:
                    record = self.document(row["captures"][0])
                    self.assertIn(b"9" * 5000, (self.destination / record["stderr"]["path"]).read_bytes())
                self.assertFalse(report["eligibleForSuperiorityClaim"])
                self.assertTrue(any("RSS" in failure for failure in report["failures"]))

    def calibration(self):
        prior = self.fixture.put("prior-registration.json", self.registration)
        run = self.fixture.put("prior-run.json", dict(schema=old.RUN_SCHEMA,
            registrationSha256=prior["sha256"], startedAt="2026-01-01T00:00:01Z",
            completedAt="2026-01-01T00:00:01Z", observations=[dict(task=task["id"], tool=tool, arm=arm)
                for task in self.registration["tasks"] for tool in old.TOOLS for arm in old.ARMS]))
        self.registration = calibration_registration(self.registration, prior, run)
        self.spec["calibrationEvidence"] = {prior["sha256"]: prior, run["sha256"]: run}
        self.refreeze()

    def test_truthful_exposed_development_calibration_collects_but_remains_ineligible(self):
        self.calibration()
        self.assertFalse(self.registration["sourceReview"]["blindToOutputs"])
        result = self.collect()
        self.assertEqual(len(self.queries), 4)
        report = old._json(Path(result["report"]).read_bytes())["assessment"]
        self.assertEqual(len(report["costs"]), 4)
        self.assertFalse(report["eligibleForSuperiorityClaim"])
        self.assertTrue(any("permanently ineligible" in failure for failure in report["failures"]))

    def test_calibration_evidence_definition_and_independent_reviewer_remain_mandatory(self):
        self.calibration()
        baseline, registration = copy.deepcopy(self.spec), copy.deepcopy(self.registration)
        for change in ("evidence", "definition", "exposed_reviewer", "self_review"):
            with self.subTest(change=change):
                self.spec, self.registration = copy.deepcopy(baseline), copy.deepcopy(registration)
                if change == "evidence": self.spec["calibrationEvidence"].clear()
                if change == "definition": self.registration["tasks"][0]["rationale"] = "Changed after independent definition review."
                if change == "exposed_reviewer": self.registration["independentReview"]["priorOutputsObserved"] = True
                if change == "self_review":
                    self.registration["independentReview"]["reviewer"] = self.registration["sourceReview"]["author"]
                self.refreeze()
                with self.assertRaises(ValueError): self.collect()
                self.assertEqual(self.calls, [])
                self.assertFalse((self.destination / "COMPLETE.json").exists())

    def test_source_failure_or_mandatory_report_failure_prevents_completion(self):
        for target in ("source", "report"):
            with self.subTest(target=target):
                self.destination = self.root / ("mandatory-" + target)
                if target == "source":
                    def wrong_head(argv, path):
                        if argv[0] == "git" and "rev-parse" in argv: path.write_bytes(b"b" * 40 + b"\n")
                    self.after = wrong_head
                    with self.assertRaisesRegex(ValueError, "source pin"): self.collect()
                    self.assertEqual(self.queries, [])
                    self.after = None
                else:
                    with patch("benchmarks.agent_query.broad_query_collect_v2.score_file_v2",
                            side_effect=ValueError("synthetic mandatory report failure")):
                        with self.assertRaisesRegex(ValueError, "mandatory report"): self.collect()
                self.assertFalse((self.destination / "COMPLETE.json").exists())

    def test_final_publication_drift_is_checked_after_provisional_report(self):
        for target in ("report", "undeclared_sidecar"):
            with self.subTest(target=target):
                self.destination = self.root / ("publication-drift-" + target)
                def change_publication(argv, path):
                    if argv[0] == "git" and "status" in argv and "-publication-" in str(path):
                        changed = (self.destination / "report-provisional.json" if target == "report" else
                            self.destination / "graphs/sample-compass" / old.SOURCE_QUERY_FILE)
                        changed.write_bytes(b"changed")
                self.after = change_publication
                with self.assertRaises(ValueError): self.collect()
                self.assertTrue((self.destination / "report-provisional.json").exists())
                self.assertFalse((self.destination / "COMPLETE.json").exists())

    def test_dependency_capacity_is_checked_before_reading_excess_payload(self):
        from benchmarks.agent_query import broad_query_collect_v2 as module
        reference = self.spec["plan"]["tools"][0]["executable"]
        with patch.object(module, "MAX_DEP_BYTES", 1), \
                patch.object(module.capture, "external", wraps=module.capture.external) as read:
            with self.assertRaises(ValueError): self.collect()
        dependency_reads = [call for call in read.call_args_list if call.args[0] == reference]
        self.assertTrue(any(call.args[1] == 1 for call in dependency_reads))
        self.assertEqual(self.calls, [])

    def test_declared_sidecar_profile_digest_or_symlink_change_after_report_blocks_completion(self):
        entry = next(graph for graph in self.spec["plan"]["graphs"] if graph["tool"] == "compass")
        original = Path(entry["artifact"]["path"]).with_name(old.SOURCE_QUERY_FILE)
        data = capture.encoded(dict(schema="compass.source-query/1", profile=old.SOURCE_QUERY_PROFILES[-1],
            graphDigest=entry["artifact"]["sha256"]))
        original.write_bytes(data)
        entry["sourceQueryArtifact"] = dict(path=str(original), sha256=old.sha(data))
        self.refreeze()
        for change in ("profile", "digest", "symlink"):
            with self.subTest(change=change):
                self.destination = self.root / ("declared-sidecar-" + change)
                def mutate(argv, path):
                    if argv[0] == "git" and "status" in argv and "-publication-" in str(path):
                        retained = self.destination / "graphs/sample-compass" / old.SOURCE_QUERY_FILE
                        if change == "symlink":
                            retained.unlink()
                            retained.symlink_to(original)
                        else:
                            value = old._json(data)
                            value["profile" if change == "profile" else "graphDigest"] = "unknown-profile" if change == "profile" else "0" * 64
                            retained.write_bytes(capture.encoded(value))
                self.after = mutate
                with self.assertRaises(ValueError): self.collect()
                self.assertTrue((self.destination / "report-provisional.json").is_file())
                self.assertFalse((self.destination / "COMPLETE.json").exists())

    def test_native_no_match_does_not_establish_semantic_empty(self):
        self.mode = "no_match"
        for tool in self.spec["plan"]["tools"]:
            tool["queryExitContract"] = dict(schema=old.QUERY_EXIT_SCHEMA,
                noMatchExitCode=1, noMatchStdoutLine="No matching nodes found.")
        self.refreeze()
        result = self.collect()
        manifest = old._json((self.destination / "review-input.json").read_bytes())
        run = self.document(manifest["run"])
        for row in run["observations"]:
            normalized = self.document(row["normalized"])
            self.assertFalse(normalized["semanticCoverageComplete"])
            self.assertEqual(normalized["disposition"], "abstain")
        report = old._json(Path(result["report"]).read_bytes())["assessment"]
        self.assertFalse(report["eligibleForSuperiorityClaim"])
        self.assertTrue(all(row["falseEmpty"] is None for row in report["rows"]))

    def test_actual_capture_input_file_is_retained_and_rechecked(self):
        path = self.root / "capture-spec.json"
        # Preserve original noncanonical formatting too, not just its parsed object.
        import json
        data = json.dumps(self.spec, indent=4).encode()
        path.write_bytes(data)
        with patch("benchmarks.agent_query.broad_query_assess_v2.SourceReader.git",
                side_effect=lambda root, *args: "a" * 40 + "\n" if "rev-parse" in args else ""):
            result = collect_file_v2(path, self.destination, runner=self.runner)
        manifest = old._json((self.destination / "review-input.json").read_bytes())
        run = self.document(manifest["run"])
        invocation = run["captureInputInvocation"]
        self.assertEqual(invocation["original"]["path"], str(path))
        self.assertEqual((self.destination / invocation["artifact"]["path"]).read_bytes(), data)
        report = old._json(Path(result["report"]).read_bytes())["assessment"]
        self.assertFalse(any("invocation file" in failure for failure in report["failures"]))
        self.destination = self.root / "input-drift"
        def drift_input(argv, capture_path):
            if argv[0] == "git" and "status" in argv and "-after-" in str(capture_path): path.write_bytes(data + b" ")
        self.after = drift_input
        with self.assertRaisesRegex(ValueError, "input exceeds"):
            collect_file_v2(path, self.destination, runner=self.runner)
        self.assertFalse((self.destination / "COMPLETE.json").exists())


class BundleTests(unittest.TestCase):
    def setUp(self):
        import tempfile
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.bundle = CaptureBundle(self.root / "bundle", FrozenInputs())

    def test_count_and_bytes_preflight_before_publication_or_subprocess(self):
        self.bundle.put("one", b"x")
        from benchmarks.agent_query import broad_query_collect_v2 as module
        with patch.object(module, "MAX_FILES", 1):
            with self.assertRaisesRegex(ValueError, "count bound"):
                self.bundle.put("two", b"x")
        with patch.object(module, "MAX_BYTES", 1):
            with self.assertRaisesRegex(ValueError, "byte bound"):
                self.bundle.put("two", b"x")
            with patch.object(module.process, "run_bounded") as runner:
                with self.assertRaisesRegex(ValueError, "byte bound"):
                    self.bundle.execute(("unused",), "run.json", self.root, 1, runner)
                runner.assert_not_called()
        self.assertFalse((self.bundle.root / "two").exists())

    def test_no_clobber_and_retained_recheck(self):
        self.bundle.put("one", b"original")
        with self.assertRaises(FileExistsError): self.bundle.put("one", b"replacement")
        (self.bundle.root / "one").write_bytes(b"changed")
        with self.assertRaises(ValueError): self.bundle.recheck()


if __name__ == "__main__": unittest.main()
