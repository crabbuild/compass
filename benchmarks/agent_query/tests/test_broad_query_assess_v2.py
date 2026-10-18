"""Synthetic file-backed assessment; fake tools and mocked Git, no qualification."""
import copy
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_capture as capture
from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_actors_v2 import freeze_protocol, retain_protocol
from benchmarks.agent_query.broad_query_assess_v2 import assess_file_v2
from benchmarks.agent_query.broad_query_lineage_v2 import SCHEMAS
from benchmarks.agent_query.tests import test_broad_query_capture as capture_tests
from benchmarks.agent_query.tests.test_broad_query_score_v2 import scoring_fixture


class Fixture:
    def __init__(self, test, *, complete=True, dependency_payloads=None, frozen_release=True):
        source = capture_tests.MockCaptureTests()
        source.setUp()
        test.addCleanup(source.doCleanups)
        self.source, self.outer = source.source, source.root
        self.spec = source.spec()
        self.spec["schema"] = SCHEMAS["capture-input"]
        self.plan = self.spec["plan"]
        if dependency_payloads is not None:
            files = [source.put(f"dependency-{number}", data) for number, data in enumerate(dependency_payloads)]
            environment = source.put("environment-v2.json", dict(files=files))
            for tool in self.plan["tools"]:
                tool["environment"] = environment
        late_release = None
        if not frozen_release:
            late_release = self.plan["tools"][1].pop("releaseVerification")
        self.plan["schema"] = SCHEMAS["capture-plan"]
        self.plan.pop("assessorSha256")
        self.plan.pop("normalizerSha256")
        code_root = Path(__file__).resolve().parents[3]
        descriptor, actors = freeze_protocol(code_root)
        self.plan["actorSha256"] = actors
        reg = old._json(capture.external(self.spec["registration"])[1])
        reg["capturePlan"] = self.plan
        self.spec["registration"] = source.put("registration-v2.json", reg)
        self.bundle = capture.Bundle(self.outer / "evidence")
        self.registration = self.bundle.copy("registration.json", self.spec["registration"])
        protocol = retain_protocol(self.bundle, code_root, descriptor, actors)
        modules = {item["module"]: item["artifact"] for item in protocol["files"]}
        normalizer = modules["benchmarks/agent_query/broad_query_normalize_v2.py"]
        assessor = modules["benchmarks/agent_query/broad_query_review.py"]
        tools, graphs = [], []
        for tool in self.plan["tools"]:
            name = tool["id"]
            record = {key: tool[key] for key in ("id", "version", "commit", "nativeOffline", "settings")}
            for key in ("executable", "environment", "releaseVerification"):
                if key in tool:
                    record[key] = self.bundle.copy(f"tools/{name}-{key}", tool[key])
            record["normalizer"] = normalizer
            if name == "graphify" and late_release is not None:
                record["releaseVerification"] = self.bundle.copy("tools/late-release", late_release)
            dependencies = old._json(capture.external(tool["environment"])[1])["files"]
            record["dependencies"] = [dict(original=ref,
                artifact=self.bundle.copy(f"tools/{name}-dep-{number}", ref)) for number, ref in enumerate(dependencies)]
            tools.append(record)
        for entry in self.plan["graphs"]:
            name = entry["tool"]
            record = {key: entry[key] for key in ("repository", "tool", "sourceCommit", "sourceCleanBeforeAndAfter")}
            for key in ("artifact", "sourceInventory"):
                record[key] = self.bundle.copy(f"graphs/{name}-{key}.json", entry[key])
            record["originalBuildCapture"] = self.bundle.copy(f"graphs/{name}-original-build.json", entry["buildCapture"])
            build = old._json(capture.external(entry["buildCapture"])[1])
            for stream in ("stdout", "stderr"):
                build[stream] = self.bundle.copy(f"graphs/{name}-build.{stream}", build[stream])
            record["buildCapture"] = self.put(f"graphs/{name}-build", build)
            graphs.append(record)
        self.observations, self.labels, self.selections, self.reviewed = [], [], [], []
        self.counter = 0
        for arm in old.ARMS:
            for tool in old.TOOLS:
                key = dict(task="save", tool=tool, arm=arm)
                _, raw, _, reviewed, judgment = scoring_fixture()
                binding = dict(key, rawStdoutSha256=old.sha(raw), sourceRegistrationSha256=self.registration["sha256"], actorSha256=actors)
                reviewed.update(binding, normalizationStatus="reviewed")
                initial = copy.deepcopy(reviewed)
                initial.update(normalizationComplete=False, semanticCoverageComplete=False, normalizationStatus="pending_review")
                if not complete:
                    reviewed.update(normalizationComplete=False, semanticCoverageComplete=False, normalizationStatus="pending_review")
                    judgment["semanticCoverageComplete"] = False
                name = f"rows/{tool}-{arm}"
                stderr = (b"123 maximum resident set size\n" if self.plan["measurement"]["style"] == "darwin-l"
                    else b"Maximum resident set size (kbytes): 123\n")
                out = self.bundle.put(name + ".stdout", raw)
                err = self.bundle.put(name + ".stderr", stderr)
                frozen_tool = next(item for item in self.plan["tools"] if item["id"] == tool)
                graph = next(item for item in graphs if item["tool"] == tool)
                argv = capture.launch(self.plan, capture.command(self.plan, frozen_tool, arm,
                    reg["tasks"][0]["question"], self.bundle.root / graph["artifact"]["path"]), self.bundle.root)
                native = self.put(name + "-capture", dict(argv=list(argv), exit_code=0, wall_ms=10,
                    timed_out=False, output_limited=False, stdout_bytes=len(raw), stderr_bytes=len(stderr), stdout=out, stderr=err))
                original_ref = self.put(name + "-initial", initial)
                reviewed_ref = self.put(name + "-reviewed", reviewed)
                self.reviewed.append(reviewed)
                self.observations.append(dict(key, captures=[native], normalized=original_ref,
                    condition="fresh_cli", peakRssBytes=capture.rss(stderr, self.plan["measurement"]["style"])))
                self.selections.append(dict(key, initialNormalizedRef=original_ref, reviewedNormalizedRef=reviewed_ref,
                    rawStdoutSha256=out["sha256"], sourceRegistrationSha256=self.registration["sha256"], actorSha256=actors))
                judgment.update(key, reviewedNormalizedSha256=reviewed_ref["sha256"],
                    independentReview=dict(status="confirmed", reviewer="synthetic-output-reviewer", reviewerType="ai",
                        toolLabelsBlinded=True, sourceAware=True, independentOfAuthor=True))
                self.labels.append(judgment)
        self.run = dict(schema=SCHEMAS["capture"], registrationSha256=self.registration["sha256"],
            startedAt="2026-01-02T00:00:00Z", completedAt="2026-01-02T00:00:01Z", protocolBinding=protocol,
            captureInput=self.put("capture-input", self.spec), assessor=assessor, tools=tools, graphs=graphs,
            arms=reg["arms"], observations=self.observations,
            childEnvironment=capture.child_environment(self.plan, self.bundle.root),
            measurementExecutable=self.bundle.copy("timer", self.plan["measurement"]["executable"]),
            environmentExecutable=self.bundle.copy("env", self.plan["environmentExecutable"]))
        self.manifest = dict(schema=SCHEMAS["review-input"], registration=self.registration,
            sourceRoots=self.spec["sourceRoots"], normalizedSelections=self.selections, graphQualification=[])
        self.publish_run()

    def put(self, name, value):
        return self.bundle.put(name + ".json", value)

    def publish_run(self):
        self.counter += 1
        self.manifest["run"] = self.put(f"run-{self.counter}", self.run)
        for selection in self.selections:
            selection["originalRunSha256"] = self.manifest["run"]["sha256"]
        self.publish_review()

    def publish_review(self):
        self.counter += 1
        rows = []
        for number, label in enumerate(self.labels):
            label["runSha256"] = self.manifest["run"]["sha256"]
            rows.append(dict(task=label["task"], tool=label["tool"], arm=label["arm"],
                judgmentRef=self.put(f"judgments-{self.counter}-{number}", label)))
        self.manifest["reviews"] = self.put(f"review-index-{self.counter}", dict(schema=SCHEMAS["judgments"],
            runSha256=self.manifest["run"]["sha256"], observations=rows))
        self.input = self.bundle.root / f"input-{self.counter}.json"
        self.put(f"input-{self.counter}", self.manifest)

    def assess(self):
        with patch("benchmarks.agent_query.broad_query_assess_v2.SourceReader.git",
                side_effect=lambda root, *args: "a" * 40 + "\n" if "rev-parse" in args else "") as git:
            result = assess_file_v2(self.input)
            self.git_calls = git.call_args_list
            return result


class AssessmentTests(unittest.TestCase):
    def test_capture_input_custody_rejects_malformed_digest_object_and_late_retained_drift(self):
        for change in ("shape", "digest", "object", "late"):
            with self.subTest(change=change):
                fixture = Fixture(self)
                original = fixture.outer / "original-invocation.json"
                value = copy.deepcopy(fixture.spec)
                if change == "object": value["plan"]["timeoutSeconds"] += 1
                data = capture.encoded(value)
                original.write_bytes(data)
                invocation = dict(original=dict(path=str(original), sha256=old.sha(data)),
                    artifact=fixture.bundle.put("retained-invocation.json", data))
                if change == "shape": invocation["unexpected"] = True
                if change == "digest": invocation["artifact"] = fixture.bundle.put("wrong-invocation.json", b"{}")
                fixture.run["captureInputInvocation"] = invocation
                fixture.publish_run()
                if change == "late":
                    finish = old._finish_assessment
                    reached = []
                    def mutate_after_assessment(*args, **kwargs):
                        report = finish(*args, **kwargs)
                        reached.append(True)
                        (fixture.bundle.root / invocation["artifact"]["path"]).write_bytes(b"changed")
                        return report
                    with patch.object(old, "_finish_assessment", side_effect=mutate_after_assessment):
                        with self.assertRaises(ValueError): fixture.assess()
                    self.assertEqual(reached, [True])
                else:
                    with self.assertRaisesRegex(ValueError, "capture-input invocation"):
                        fixture.assess()

    def test_truncated_default_prefix_keeps_all_rows_and_blocks_formal_precision(self):
        fixture = Fixture(self)
        row = fixture.observations[0]
        capture = fixture.bundle.root / row["captures"][0]["path"]
        record = old._json(capture.read_bytes())
        record["output_limited"] = True
        row["captures"] = [fixture.put("truncated-native-capture", record)]
        fixture.publish_run()
        report = fixture.assess()
        self.assertEqual(len(report["rows"]), 4)
        self.assertEqual(len(report["costs"]), 4)
        self.assertIsNone(report["comparisons"]["claimPrecisionEmptyZero"]["estimate"])
        truncated = next(row for row in report["rows"] if row["nativeOutputTruncated"])
        self.assertIsNone(truncated["printedAssertionCount"])
        self.assertEqual(truncated["precisionView"]["knownSupported"], 1)
        self.assertFalse(report["eligibleForSuperiorityClaim"])

    def test_release_evidence_presence_and_digest_must_match_the_pre_output_freeze(self):
        for change in ("late", "missing", "digest"):
            with self.subTest(change=change):
                fixture = Fixture(self, frozen_release=change != "late")
                if change == "missing": fixture.run["tools"][1].pop("releaseVerification")
                if change == "digest":
                    fixture.run["tools"][1]["releaseVerification"] = fixture.put("different-release", dict(
                        version="0.0-fixture", commit="b" * 40, checkedAt="2026-01-01T00:00:00Z",
                        source="https://github.com/Graphify-Labs/graphify/releases"))
                if change != "late": fixture.publish_run()
                with self.assertRaisesRegex(ValueError, "release evidence"):
                    fixture.assess()

    def test_complete_source_answers_keep_full_matrix_and_all_existing_qualification_gates(self):
        fixture = Fixture(self)
        report = fixture.assess()
        self.assertEqual(len(report["rows"]), 4)
        self.assertEqual(len(report["costs"]), 4)
        self.assertTrue(all(row["success"] for row in report["rows"]))
        self.assertEqual(report["comparisons"]["claimPrecisionEmptyZero"]["estimate"], 0)
        self.assertFalse(report["eligibleForSuperiorityClaim"])
        self.assertIn("insufficient independent repositories", report["failures"])
        self.assertIn("existing independent production graph-qualification gate is incomplete", report["failures"])
        self.assertIn("prospective power analysis is missing; sample size alone cannot establish power", report["failures"])
        self.assertTrue(report["sourceStateVerified"])
        self.assertEqual(report["printedWork"]["completedLineLinks"], 8)
        self.assertEqual(len(fixture.git_calls), 4)
        for call in fixture.git_calls:
            self.assertIn("--no-optional-locks", call.args)
            self.assertIn("core.fsmonitor=false", call.args)
            self.assertIn("core.untrackedCache=false", call.args)

    def test_incomplete_rows_keep_unknown_precision_in_comparison_and_every_summary(self):
        fixture = Fixture(self, complete=False)
        report = fixture.assess()
        comparison = report["comparisons"]["claimPrecisionEmptyZero"]
        self.assertIsNone(comparison["estimate"])
        self.assertIsNone(comparison["lower"])
        self.assertEqual(comparison["rowsRetained"], 1)
        self.assertEqual(comparison["missingPairRows"], 1)
        self.assertFalse(report["eligibleForSuperiorityClaim"])
        for tool in old.TOOLS:
            for arm in old.ARMS:
                summary = report["summaries"][tool][arm]
                self.assertEqual(summary["tasks"], 1)
                self.assertIsNone(summary["claimPrecision"])
                self.assertEqual(summary["precision"]["unknownRows"], 1)
                self.assertIsNone(summary["byRepository"]["sample"]["claimPrecision"])
                self.assertIsNone(summary["byLanguage"]["python"]["claimPrecision"])

    def test_unknown_default_row_cannot_establish_formal_precision_on_complete_normalized_pairs(self):
        fixture = Fixture(self)
        reviewed = copy.deepcopy(fixture.reviewed[0])
        reviewed.update(normalizationComplete=False, semanticCoverageComplete=False, normalizationStatus="pending_review")
        reference = fixture.put("default-incomplete", reviewed)
        fixture.selections[0]["reviewedNormalizedRef"] = reference
        fixture.labels[0].update(reviewedNormalizedSha256=reference["sha256"], semanticCoverageComplete=False)
        fixture.publish_review()
        report = fixture.assess()
        comparison = report["comparisons"]["claimPrecisionEmptyZero"]
        self.assertIsNone(comparison["estimate"])
        self.assertEqual(comparison["missingPairRows"], 0)
        self.assertEqual(comparison["rowsRetained"], 1)
        self.assertFalse(report["eligibleForSuperiorityClaim"])

    def test_missing_dependency_and_frozen_tool_or_graph_mismatch_abort(self):
        for change in ("dependency", "tool", "graph"):
            with self.subTest(change=change):
                fixture = Fixture(self)
                if change == "dependency": fixture.run["tools"][0]["dependencies"] = []
                elif change == "tool": fixture.run["tools"][0]["settings"]["queryInterface"] = "changed"
                else: fixture.run["graphs"][0]["sourceCommit"] = "c" * 40
                fixture.publish_run()
                with self.assertRaises(ValueError): fixture.assess()

    def test_original_dependency_or_retained_stream_drift_is_an_integrity_failure(self):
        for target in ("dependency", "raw"):
            with self.subTest(target=target):
                fixture = Fixture(self)
                if target == "dependency":
                    path = Path(fixture.run["tools"][0]["dependencies"][0]["original"]["path"])
                else:
                    native = old._json((fixture.bundle.root / fixture.observations[0]["captures"][0]["path"]).read_bytes())
                    path = fixture.bundle.root / native["stdout"]["path"]
                path.write_bytes(path.read_bytes() + b"changed")
                with self.assertRaises(ValueError): fixture.assess()

    def test_rejected_initial_keeps_native_matrix_costs_and_unknown_formal_precision(self):
        fixture = Fixture(self)
        index = 2  # Compass normalized arm, after both default rows.
        original = old._json((fixture.bundle.root / fixture.observations[index]["normalized"]["path"]).read_bytes())
        keys = ("schema", "task", "tool", "arm", "rawStdoutSha256", "sourceRegistrationSha256", "actorSha256")
        rejected = {key: original[key] for key in keys}
        rejected.update(normalizationStatus="rejected", normalizationComplete=False, semanticCoverageComplete=False,
            disposition="error", rankingKind="unavailable", admission=None, ranked=[], claimCount=0, claimBatches=[],
            printedAssertionCount=None, fullMeasuredPrecision=None, falseEmpty=None, success=False,
            normalizationFailure=dict(code="PARSER_REJECTION", resource=None))
        reference = fixture.put("original-parser-rejection", rejected)
        fixture.observations[index]["normalized"] = reference
        fixture.selections[index]["initialNormalizedRef"] = reference
        fixture.publish_run()
        report = fixture.assess()
        self.assertEqual(len(report["rows"]), 4)
        self.assertEqual(len(report["costs"]), 4)
        row = next(row for row in report["rows"] if row["tool"] == "compass" and row["arm"] == "normalized")
        self.assertTrue(row["initialNormalizationRejected"])
        self.assertFalse(row["success"])
        self.assertIsNone(row["claimPrecision"])
        self.assertIsNone(report["comparisons"]["claimPrecisionEmptyZero"]["estimate"])
        self.assertEqual(report["costSummaries"]["compass"]["normalized"]["workflowBytes"], 2)

    def test_late_original_dependency_drift_prevents_returning_a_completed_prefix(self):
        fixture = Fixture(self)
        from benchmarks.agent_query import broad_query_assess_v2 as module
        original = module.score_bound_observation
        calls = []
        def changed(*args, **kwargs):
            result = original(*args, **kwargs)
            calls.append(result)
            if len(calls) == 4:
                path = Path(fixture.run["tools"][0]["dependencies"][0]["original"]["path"])
                path.write_bytes(path.read_bytes() + b"changed after all rows")
            return result
        with patch.object(module, "score_bound_observation", side_effect=changed):
            with self.assertRaises(ValueError): fixture.assess()
        self.assertEqual(len(calls), 4)

    def test_current_git_pin_and_cleanliness_are_checked_before_scoring(self):
        fixture = Fixture(self)
        for head, status in (("b" * 40, ""), ("a" * 40, " M store.py")):
            with patch("benchmarks.agent_query.broad_query_assess_v2.SourceReader.git",
                    side_effect=lambda root, *args: head if "rev-parse" in args else status):
                with self.assertRaises(ValueError): assess_file_v2(fixture.input)

    def test_actual_manifest_drift_after_scoring_prevents_returning_a_report(self):
        fixture = Fixture(self)
        from benchmarks.agent_query import broad_query_assess_v2 as module
        original = module.assess_v2
        def changed(manifest, root):
            result = original(manifest, root)
            fixture.input.write_bytes(fixture.input.read_bytes() + b"\n")
            return result
        with patch.object(module, "assess_v2", side_effect=changed):
            with self.assertRaisesRegex(ValueError, "manifest changed"):
                fixture.assess()


if __name__ == "__main__": unittest.main()
