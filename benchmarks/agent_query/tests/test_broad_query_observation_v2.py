"""Synthetic execution/coverage/scoring integration, no qualification evidence."""
import copy
from dataclasses import replace
import tempfile
import unittest

from benchmarks.agent_query.broad_query_normalize_v2 import normalize_v2
from benchmarks.agent_query.broad_query_observation_v2 import score_bound_observation, _pending_judgment
from benchmarks.agent_query.broad_query_protocol import (
    BundleWork, MAX_BUNDLE_HASH_BYTES, ProtocolLimit, pack_claim_batches,
)
from benchmarks.agent_query.broad_query_review import GraphIndex, sha
from benchmarks.agent_query.tests.test_broad_query_execution_v2 import Fixture as ExecutionFixture
from benchmarks.agent_query.tests.test_broad_query_review import graph
from benchmarks.agent_query.tests.test_broad_query_score_v2 import scoring_fixture


class Fixture(ExecutionFixture):
    def __init__(self, directory, count=1, *, rejected=False):
        task, raw, _, reviewed, judgment = scoring_fixture(count)
        if rejected: raw = b"\xff"
        super().__init__(directory, arm="default", raw=raw)
        self.task = dict(task, question=self.task["question"])
        self.index = GraphIndex(graph(), "compass")
        self.work = BundleWork()
        if rejected:
            reviewed = normalize_v2(raw, "compass", self.index, task="save", arm="default",
                source_registration_sha256="a" * 64, actors=dict(capture="b" * 64))
            judgment = dict(schema="compass.broad-query-judgments/2", claimCount=0,
                claimBatches=[], candidates=[], semanticCoverageComplete=False, emptyConfirmed=False)
        initial = copy.deepcopy(reviewed)
        if not rejected:
            initial.update(normalizationComplete=False, semanticCoverageComplete=False,
                normalizationStatus="pending_review")
            reviewed["normalizationStatus"] = "reviewed"
        self.observation = replace(self.observation, initial=initial, reviewed=reviewed, judgment=judgment)
        lines = ["class Store:", "    def save(self):", "        return self.flush()",
            "    def flush(self):", "        pass"]
        for number in range(6, max(count + 10, 11)):
            lines.append(f"synthetic extra assertion{number - 10}" if number >= 11 else "")
        self.sources = {"store.py": ("\n".join(lines) + "\n").encode()}

    def score(self):
        return score_bound_observation(self.observation, self.task, self.plan, self.graph,
            self.root, self.index, self.sources, self.work)

    def rejection(self, code="PARSER_REJECTION"):
        document = normalize_v2(b"\xff", "compass", self.index, task="save", arm="default",
            source_registration_sha256="a" * 64, actors=dict(capture="b" * 64))
        document["rawStdoutSha256"] = sha(self.observation.stdout)
        resource = (dict(name="registered_claim_records", bound=2048, countAtLeast=2048,
            exactCountKnown=True, attemptedValue=2049) if code == "CLAIM_LIMIT" else None)
        document["normalizationFailure"] = dict(code=code, resource=resource)
        return document


class ObservationTests(unittest.TestCase):
    def test_initial_aggregate_occurrence_limit_precedes_all_pending_label_allocations(self):
        class Occurrence(dict):
            reads = 0
            def get(self, key, default=None):
                if key == "id": self.reads += 1
                return super().get(key, default)
        for count in (8192, 8193):
            occurrence = Occurrence(id="synthetic")
            initial = dict(lineLedger=[], **pack_claim_batches([
                dict(id="first", printedOccurrences=[occurrence] * 4096),
                dict(id="second", printedOccurrences=[occurrence] * (count - 4096))]))
            if count == 8192:
                labels = _pending_judgment(initial)
                self.assertEqual(occurrence.reads, 8192)
                self.assertEqual(sum(len(c["printedOccurrences"]) for batch in labels["claimBatches"]
                    for c in batch["claims"]), 8192)
            else:
                with self.assertRaises(ProtocolLimit) as caught: _pending_judgment(initial)
                self.assertEqual(caught.exception.code, "OCCURRENCE_LIMIT")
                self.assertEqual(occurrence.reads, 0)
        initial["lineLedger"] = "not a ledger"
        initial["claimBatches"][0]["claims"][1]["printedOccurrences"] = [occurrence] * 4096
        with self.assertRaises(ValueError): _pending_judgment(initial)
        self.assertEqual(occurrence.reads, 0)

    def test_rejected_review_still_checks_initial_spans_and_line_links(self):
        with tempfile.TemporaryDirectory() as directory:
            for change in ("valid", "sha", "first_span", "line_links"):
                fixture = Fixture(directory, 2 if change == "first_span" else 1)
                judgment = dict(schema="compass.broad-query-judgments/2", claimCount=0,
                    claimBatches=[], semanticCoverageComplete=False, emptyConfirmed=False)
                fixture.observation = replace(fixture.observation,
                    reviewed=fixture.rejection(), judgment=judgment)
                initial = fixture.observation.initial
                claim = initial["claimBatches"][0]["claims"][0]
                if change == "sha": claim["span"]["sha256"] = "0" * 64
                elif change == "first_span":
                    # A valid nonempty raw span with a valid digest; only its
                    # equality to this claim's first occurrence is wrong.
                    claim["span"] = copy.deepcopy(initial["claimBatches"][0]["claims"][1]["span"])
                elif change == "line_links": initial["lineLedger"][0]["claimIds"] = []
                if change == "first_span":
                    with self.assertRaisesRegex(ValueError, "first printed occurrence"): fixture.score()
                elif change != "valid":
                    with self.assertRaises(ValueError): fixture.score()
                else:
                    result = fixture.score()
                    self.assertIsNone(result["row"]["claimPrecision"])
                    self.assertTrue(result["row"]["reviewedNormalizationRejected"])
                    self.assertEqual(result["cost"]["workflowWallMs"], 5)
                    self.assertGreater(result["work"]["completedHashBytes"], 0)
                    self.assertEqual(result["work"]["completedLineLinks"], 1)

    def test_original_parser_and_limit_rejections_cannot_be_promoted_by_reviewed_success(self):
        with tempfile.TemporaryDirectory() as directory:
            for code in ("PARSER_REJECTION", "CLAIM_LIMIT"):
                fixture = Fixture(directory)
                fixture.observation = replace(fixture.observation, initial=fixture.rejection(code))
                result = fixture.score()
                row = result["row"]
                self.assertTrue(row["initialNormalizationRejected"])
                self.assertFalse(row["reviewedNormalizationRejected"])
                self.assertEqual(row["initialNormalizationFailure"]["code"], code)
                self.assertIsNone(row["reviewedNormalizationFailure"])
                self.assertFalse(row["success"])
                self.assertIsNone(row["claimPrecision"])
                self.assertIsNone(row["precisionWithEmptyZero"])
                self.assertIsNone(row["falseEmpty"])
                self.assertEqual(row["precisionView"]["knownSupported"], 1)
                self.assertEqual(row["precisionView"]["judgedOnlyPrecision"], 1)
                self.assertFalse(row["precisionView"]["formalPrecisionAvailable"])
                self.assertEqual(result["cost"]["workflowWallMs"], 5)

    def test_source_scoring_and_native_costs_share_one_finite_printed_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            result = fixture.score()
            self.assertTrue(result["row"]["success"])
            self.assertEqual(result["row"]["claimPrecision"], 1)
            self.assertEqual(result["row"]["sourceFactRecall"], 1)
            self.assertFalse(result["row"]["evaluationRejected"])
            self.assertEqual(result["cost"]["workflowBytes"], len(fixture.observation.stdout))
            self.assertEqual(result["work"]["completedHashBytes"], fixture.work.hash_bytes)
            # Initial and reviewed phases each charge a line link, while their
            # identical actual span digest is hashed once in the shared cache.
            self.assertEqual(result["work"]["completedLineLinks"], 2)
            self.assertEqual(fixture.work.hash_bytes, len(fixture.observation.stdout.rstrip(b"\n")))

    def test_late_unsupported_assertion_cannot_preserve_successful_batch_prefix(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory, 129)
            last = fixture.observation.judgment["claimBatches"][-1]["claims"][-1]
            last["printedOccurrences"][0]["verdict"] = "unsupported"
            row = fixture.score()["row"]
            self.assertFalse(row["success"])
            self.assertEqual(row["supportedClaims"], 128)
            self.assertEqual(row["unsupportedClaims"], 1)
            self.assertEqual(row["claimPrecision"], 128 / 129)

    def test_rejected_raw_bytes_keep_costs_and_unknown_precision_without_false_empty(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory, rejected=True)
            result = fixture.score()
            row = result["row"]
            self.assertFalse(row["success"])
            self.assertTrue(row["evaluationRejected"])
            self.assertIsNone(row["claimPrecision"])
            self.assertIsNone(row["precisionWithEmptyZero"])
            self.assertIsNone(row["falseEmpty"])
            self.assertIsNone(row["printedAssertionCount"])
            self.assertEqual(row["evaluationFailure"]["code"], "INVALID_UTF8")
            self.assertEqual(result["cost"]["workflowBytes"], 1)
            self.assertEqual(result["work"]["completedHashBytes"], 0)

    def test_rejected_review_cannot_claim_empty_or_candidate_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            for key, value in (("emptyConfirmed", True), ("semanticCoverageComplete", True),
                    ("candidates", [{}]), ("lineCoverage", [{}])):
                fixture = Fixture(directory, rejected=True)
                fixture.observation.judgment[key] = value
                with self.assertRaises(ValueError): fixture.score()

    def test_bundle_work_exhaustion_retains_failure_without_executing_rejected_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.work = BundleWork(hash_bytes=MAX_BUNDLE_HASH_BYTES)
            result = fixture.score()
            self.assertEqual(result["row"]["evaluationFailure"]["code"], "BUNDLE_HASH_WORK_LIMIT")
            self.assertFalse(result["row"]["success"])
            self.assertIsNone(result["row"]["precisionWithEmptyZero"])
            self.assertEqual(result["work"]["completedHashBytes"], 0)
            self.assertEqual(fixture.work.hash_bytes, MAX_BUNDLE_HASH_BYTES)
            self.assertEqual(result["cost"]["workflowWallMs"], 5)

    def test_typed_collision_discards_scoring_prefix_without_discarding_raw_costs(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory, 2)
            label = fixture.observation.judgment["claimBatches"][0]["claims"][1]
            label["factId"] = "saves"
            label.pop("source")
            row = fixture.score()["row"]
            self.assertTrue(row["evaluationRejected"])
            self.assertEqual(row["evaluationFailure"]["code"], "EXPRESSIVENESS_COLLISION")
            self.assertEqual(row["supportedClaims"], 0)
            self.assertIsNone(row["claimPrecision"])

    def test_initial_span_tamper_and_pinned_source_errors_remain_integrity_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.observation.initial["claimBatches"][0]["claims"][0]["span"]["sha256"] = "0" * 64
            with self.assertRaises(ValueError): fixture.score()
            for source in (b"wrong source\n", b"\xff"):
                fixture = Fixture(directory, 2)
                fixture.sources["store.py"] = source
                with self.assertRaises(ValueError): fixture.score()

    def test_native_failure_overrides_complete_source_answer_without_erasing_precision(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.observation.capture["exit_code"] = 1
            result = fixture.score()
            self.assertFalse(result["row"]["success"])
            self.assertTrue(result["row"]["executionFailed"])
            self.assertEqual(result["row"]["claimPrecision"], 1)
            self.assertEqual(result["cost"]["workflowWallMs"], 5)

    def test_unverified_graph_source_or_missing_shared_bundle_counter_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            for graph_input, sources, work in ((None, fixture.sources, fixture.work),
                    (fixture.index, None, fixture.work), (fixture.index, fixture.sources, None)):
                with self.assertRaises(ValueError):
                    score_bound_observation(fixture.observation, fixture.task, fixture.plan,
                        fixture.graph, fixture.root, graph_input, sources, work)


if __name__ == "__main__": unittest.main()
