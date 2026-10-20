"""Synthetic artifact-binding controls; no product runs or real judgments."""
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_graph_evidence as evidence
from benchmarks.agent_query import broad_graph_statistics as statistics
from benchmarks.agent_query import broad_query_review as query
from benchmarks.agent_query.tests.test_broad_query_review import graph


class Fixture:
    def __init__(self, root):
        self.root = root
        self.source = root / "source"
        self.source.mkdir()
        self.data = b"class Store:\n    def save(self):\n        return self.flush()\n    def flush(self):\n        pass\n"
        (self.source / "store.py").write_bytes(self.data)
        files = [dict(file="store.py", sha256=query.sha(self.data))]
        self.tools = [dict(id=tool, version="synthetic1", commit="b" * 40, profile="synthetic",
            executable=self.put(tool + "-binary", b"synthetic-not-executed"),
            environment=self.put(tool + "-env.json", {}),
            configuration=self.put(tool + "-config.json", {})) for tool in statistics.TOOLS]
        def span(text, symbol=None):
            start = self.data.index(text)
            result = dict(file="store.py", startByte=start, endByte=start + len(text), sha256=query.sha(text))
            if symbol is not None:
                result.update(symbol=symbol, owner="Store")
            return result
        self.fact = dict(id="save-flush", repository="sample", cluster="save", language="python", relation="calls",
            fact=dict(source=span(b"def save", "save"), target=span(b"def flush", "flush"),
                occurrence=span(b"self.flush()"), requiresDirection=True))
        self.registration = dict(schema=evidence.REGISTRATION_SCHEMA, frozenAt="2026-01-01T00:00:00Z",
            supportSha256=evidence.support_hashes(), repositories=[dict(id="sample", commit="a" * 40, files=files)],
            tools=copy.deepcopy(self.tools), facts=[self.fact], resamples=1000, seed=29)
        self.graphs, self.builds = {}, {}
        entries, observations = [], []
        for tool in statistics.TOOLS:
            self.graphs[tool] = graph(tool)
            artifact = self.put(tool + "-graph.json", self.graphs[tool])
            stdout = b"synthetic build bytes, not an executed build\n"
            self.builds[tool] = dict(argv=["synthetic-builder", tool],
                stdout=self.put(tool + "-stdout", stdout), stderr=self.put(tool + "-stderr", b""),
                stdout_bytes=len(stdout), stderr_bytes=0, wall_ms=1,
                timed_out=False, output_limited=False, exit_code=0)
            entries.append(dict(repository="sample", tool=tool, artifact=artifact,
                sourceCommit="a" * 40, sourceCleanBeforeAndAfter=True,
                sourceInventory=self.put(tool + "-inventory.json", dict(files=files)),
                buildCapture=self.put(tool + "-build.json", self.builds[tool])))
            observations.append(dict(repository="sample", factId=self.fact["id"], tool=tool,
                sourceFactSha256=self.fact_digest(), graphSha256=artifact["sha256"], status="present",
                reason="Synthetic label for an artifact-binding test, not independent source review.",
                edges=[dict(index=0, sha256=evidence.canonical_sha256(self.graphs[tool]["edges"][0]))]))
        self.capture = dict(schema=evidence.CAPTURE_SCHEMA, registrationSha256="", startedAt="2026-01-02T00:00:00Z",
            tools=copy.deepcopy(self.tools), graphs=entries)
        self.reviews = dict(schema=evidence.REVIEW_SCHEMA, captureSha256="", reviewedAt="2026-01-03T00:00:00Z", observations=observations)

    def put(self, file, value):
        data = value if isinstance(value, bytes) else json.dumps(value, sort_keys=True).encode()
        (self.root / file).write_bytes(data)
        return dict(path=file, sha256=query.sha(data))

    def fact_digest(self):
        return evidence.canonical_sha256(dict(relation=self.fact["relation"], **self.fact["fact"]))

    def entry(self, tool):
        return next(e for e in self.capture["graphs"] if e["tool"] == tool)

    def observation(self, tool):
        return next(o for o in self.reviews["observations"] if o["tool"] == tool)

    def publish_graph(self, tool):
        ref = self.put(tool + "-graph.json", self.graphs[tool])
        self.entry(tool)["artifact"] = ref
        item = self.observation(tool)
        item["graphSha256"] = ref["sha256"]
        item["edges"] = [dict(index=0, sha256=evidence.canonical_sha256(self.graphs[tool]["edges"][0]))]

    def publish_input(self):
        registration = self.put("registration.json", self.registration)
        self.capture["registrationSha256"] = registration["sha256"]
        capture = self.put("capture.json", self.capture)
        self.reviews["captureSha256"] = capture["sha256"]
        manifest = dict(schema=evidence.INPUT_SCHEMA, registration=registration, capture=capture,
            reviews=self.put("reviews.json", self.reviews), sourceRoots=dict(sample=str(self.source)))
        return manifest

    def assess(self):
        return evidence.assess_graph_coverage(self.publish_input(), self.root)


class GraphEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.fixture = Fixture(Path(self.directory.name))

    def test_bound_source_graph_build_and_record_bytes_remain_diagnostic(self):
        result = self.fixture.assess()
        self.assertTrue(result["artifactBindingsVerified"])
        self.assertTrue(result["sourceByteAnchorsVerified"])
        self.assertTrue(result["statistics"]["completePairs"])
        self.assertFalse(result["qualificationEstablished"])
        self.assertFalse(result["statistics"]["qualificationEstablished"])

    def test_missing_review_keeps_the_frozen_fact_and_unknown_denominator(self):
        self.fixture.reviews["observations"].pop()
        result = self.fixture.assess()
        self.assertEqual(result["statistics"]["rowsRetained"], 1)
        self.assertEqual(result["statistics"]["coverage"]["graphify"]["unknown"], 1)
        self.assertIsNone(result["statistics"]["pairedClusterWeightedRecallDifference"]["estimate"])

    def test_empty_captured_graphs_keep_every_source_fact_as_missing(self):
        for tool in statistics.TOOLS:
            self.fixture.graphs[tool].update(nodes=[], edges=[])
            ref = self.fixture.put(tool + "-graph.json", self.fixture.graphs[tool])
            self.fixture.entry(tool)["artifact"] = ref
            self.fixture.observation(tool).update(graphSha256=ref["sha256"], status="missing", edges=[])
        result = self.fixture.assess()
        self.assertEqual(result["statistics"]["rowsRetained"], 1)
        for counts in result["statistics"]["coverage"].values():
            self.assertEqual(counts["missing"], 1)
            self.assertEqual(counts["microRecall"], 0)

    def test_distinct_negative_statuses_remain_visible(self):
        for status in ["missing", "ambiguous", "unavailable_contract", "unreviewed"]:
            item = self.fixture.observation("graphify")
            item.update(status=status, edges=[])
            result = self.fixture.assess()
            self.assertEqual(result["statusCounts"]["graphify"], {status: 1})
            expected = None if status == "unreviewed" else 0
            self.assertEqual(result["statistics"]["coverage"]["graphify"]["microRecall"], expected)

    def test_changed_owner_labels_cannot_duplicate_one_physical_source_occurrence(self):
        duplicate = copy.deepcopy(self.fixture.fact)
        duplicate["id"] = "alias"
        duplicate["fact"]["source"]["owner"] = "OtherOwner"
        self.fixture.registration["facts"].append(duplicate)
        with self.assertRaisesRegex(ValueError, "physical relationship"):
            self.fixture.assess()

    def test_source_file_mutation_is_rejected(self):
        (self.fixture.source / "store.py").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "source file digest"):
            self.fixture.assess()

    def test_bad_source_anchor_or_uninventoried_file_is_rejected(self):
        for change in [dict(sha256="0" * 64), dict(file="missing.py"), dict(endByte=10**8)]:
            original = copy.deepcopy(self.fixture.fact["fact"]["source"])
            self.fixture.fact["fact"]["source"].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.fixture.assess()
            self.fixture.fact["fact"]["source"] = original

    def test_observations_cannot_rebind_graph_or_source_fact(self):
        for field in ["graphSha256", "sourceFactSha256"]:
            item = self.fixture.observation("compass")
            original = item[field]
            item[field] = "0" * 64
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "digest differs"):
                self.fixture.assess()
            item[field] = original

    def test_missing_duplicated_or_forged_record_proof_is_rejected(self):
        item = self.fixture.observation("compass")
        original = copy.deepcopy(item["edges"])
        for proofs in [[], original * 2, [dict(index=100, sha256="0" * 64)], [dict(index=0, sha256="0" * 64)]]:
            item["edges"] = proofs
            with self.subTest(proofs=proofs), self.assertRaises(ValueError):
                self.fixture.assess()
        item["edges"] = original

    def test_negative_review_cannot_assert_positive_record_evidence(self):
        self.fixture.observation("compass")["status"] = "missing"
        with self.assertRaisesRegex(ValueError, "non-present"):
            self.fixture.assess()

    def test_undirected_contract_is_unavailable_not_an_invented_reverse_error(self):
        self.fixture.graphs["graphify"]["directed"] = False
        self.fixture.publish_graph("graphify")
        with self.assertRaisesRegex(ValueError, "undirected graph"):
            self.fixture.assess()
        self.fixture.observation("graphify").update(status="unavailable_contract", edges=[])
        result = self.fixture.assess()
        self.assertEqual(result["statusCounts"]["graphify"], dict(unavailable_contract=1))
        self.assertFalse(result["qualificationEstablished"])

    def test_common_undirected_relationship_does_not_assume_storage_orientation(self):
        self.fixture.fact["fact"]["requiresDirection"] = False
        for item in self.fixture.reviews["observations"]:
            item["sourceFactSha256"] = self.fixture.fact_digest()
        graphify = self.fixture.graphs["graphify"]
        graphify["directed"] = False
        edge = graphify["edges"][0]
        edge["source"], edge["target"] = edge["target"], edge["source"]
        self.fixture.publish_graph("graphify")
        self.assertTrue(self.fixture.assess()["statistics"]["completePairs"])

    def test_failed_bounded_build_is_not_absence_or_success(self):
        self.fixture.builds["compass"]["timed_out"] = True
        self.fixture.entry("compass")["buildCapture"] = self.fixture.put("compass-build.json", self.fixture.builds["compass"])
        with self.assertRaisesRegex(ValueError, "successful build"):
            self.fixture.assess()
        self.fixture.observation("compass").update(status="missing", edges=[])
        result = self.fixture.assess()
        self.assertIsNone(result["statistics"]["coverage"]["compass"]["microRecall"])
        self.assertEqual(result["statusCounts"]["compass"], dict(build_failure=1))
        self.assertEqual(result["reviewStatusCounts"]["compass"], dict(missing=1))
        self.assertEqual(len(result["buildFailures"]), 1)
        self.assertTrue(result["buildFailures"][0]["timedOut"])

    def test_tool_version_profile_configuration_and_capture_matrix_are_frozen(self):
        original = copy.deepcopy(self.fixture.capture["tools"])
        for field, value in [("version", "other"), ("profile", "other"),
                ("configuration", self.fixture.put("other-config.json", {"changed": True}))]:
            self.fixture.capture["tools"][0][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "tool bytes/profile"):
                self.fixture.assess()
            self.fixture.capture["tools"] = copy.deepcopy(original)
        self.fixture.capture["graphs"].pop()
        with self.assertRaises(ValueError):
            self.fixture.assess()

    def test_source_commit_inventory_and_clean_attestation_are_bound(self):
        entry = self.fixture.entry("compass")
        for field, value in [("sourceCommit", "c" * 40), ("sourceCleanBeforeAndAfter", False),
                ("sourceInventory", self.fixture.put("other-inventory.json", dict(files=[])))]:
            original = entry[field]
            entry[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.fixture.assess()
            entry[field] = original

    def test_raw_build_stream_counts_are_recomputed(self):
        self.fixture.builds["compass"]["stdout_bytes"] += 1
        self.fixture.entry("compass")["buildCapture"] = self.fixture.put("compass-build.json", self.fixture.builds["compass"])
        with self.assertRaisesRegex(ValueError, "byte counts"):
            self.fixture.assess()

    def test_registration_and_review_chronology_are_checked(self):
        frozen = self.fixture.registration["frozenAt"]
        self.fixture.registration["frozenAt"] = self.fixture.capture["startedAt"]
        with self.assertRaisesRegex(ValueError, "precede"):
            self.fixture.assess()
        self.fixture.registration["frozenAt"] = frozen
        self.fixture.reviews["reviewedAt"] = frozen
        with self.assertRaisesRegex(ValueError, "predates"):
            self.fixture.assess()

    def test_unknown_major_versions_and_changed_support_bytes_are_rejected(self):
        for document in [self.fixture.registration, self.fixture.capture, self.fixture.reviews]:
            original = document["schema"]
            document["schema"] = original.rsplit("/", 1)[0] + "/99"
            with self.subTest(schema=original), self.assertRaises(ValueError):
                self.fixture.assess()
            document["schema"] = original
        self.fixture.registration["supportSha256"]["statistics"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "analysis protocol"):
            self.fixture.assess()

    def test_duplicate_or_unexpected_observations_are_rejected(self):
        duplicate = copy.deepcopy(self.fixture.reviews["observations"][0])
        self.fixture.reviews["observations"].append(duplicate)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.fixture.assess()
        duplicate["factId"] = "not-registered"
        with self.assertRaisesRegex(ValueError, "unexpected"):
            self.fixture.assess()

    def test_proof_and_canonical_record_byte_budgets_fail_explicitly(self):
        for field in ["MAX_RECORD_PROOFS_TOTAL", "MAX_RECORD_BYTES_TOTAL"]:
            with self.subTest(field=field), patch.object(evidence, field, 1), self.assertRaisesRegex(ValueError, "aggregate"):
                self.fixture.assess()

    def test_source_mutation_during_assessment_is_rechecked(self):
        original = statistics.paired_graph_coverage
        def mutate(*args, **kwargs):
            result = original(*args, **kwargs)
            (self.fixture.source / "store.py").write_bytes(self.fixture.data + b"# changed\n")
            return result
        with patch.object(statistics, "paired_graph_coverage", side_effect=mutate), self.assertRaisesRegex(ValueError, "source file changed"):
            self.fixture.assess()

    def test_graph_mutation_during_assessment_is_rechecked(self):
        original = statistics.paired_graph_coverage
        def mutate(*args, **kwargs):
            result = original(*args, **kwargs)
            (self.fixture.root / "compass-graph.json").write_bytes(b"changed")
            return result
        with patch.object(statistics, "paired_graph_coverage", side_effect=mutate), self.assertRaisesRegex(ValueError, "digest mismatch"):
            self.fixture.assess()

    def test_analysis_support_change_after_scoring_is_rejected(self):
        before = evidence.support_hashes()
        changed = dict(before, statistics="0" * 64)
        with patch.object(evidence, "support_hashes", side_effect=[before, changed]), self.assertRaisesRegex(ValueError, "support files changed"):
            self.fixture.assess()

    def test_source_symlink_escape_during_recheck_is_not_hidden_by_matching_bytes(self):
        outside = self.fixture.root / "outside.py"
        outside.write_bytes(self.fixture.data)
        original = statistics.paired_graph_coverage
        def mutate(*args, **kwargs):
            result = original(*args, **kwargs)
            source = self.fixture.source / "store.py"
            source.unlink()
            try:
                source.symlink_to(outside)
            except (OSError, NotImplementedError):
                self.skipTest("symlink creation unavailable")
            return result
        with patch.object(statistics, "paired_graph_coverage", side_effect=mutate), self.assertRaisesRegex(ValueError, "escapes"):
            self.fixture.assess()

    def test_artifact_symlink_escape_during_recheck_is_rejected(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        outside = Path(directory.name) / "graph.json"
        source = self.fixture.root / "compass-graph.json"
        outside.write_bytes(source.read_bytes())
        original = statistics.paired_graph_coverage
        def mutate(*args, **kwargs):
            result = original(*args, **kwargs)
            source.unlink()
            try:
                source.symlink_to(outside)
            except (OSError, NotImplementedError):
                self.skipTest("symlink creation unavailable")
            return result
        with patch.object(statistics, "paired_graph_coverage", side_effect=mutate), self.assertRaisesRegex(ValueError, "escapes"):
            self.fixture.assess()

    def test_late_bad_proof_is_not_skipped_after_an_unreviewed_pair(self):
        self.fixture.observation("compass").update(status="unreviewed", edges=[])
        self.fixture.observation("graphify")["edges"][0]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "record proof"):
            self.fixture.assess()

    def test_source_symlink_escape_is_rejected_before_reading(self):
        outside = self.fixture.root / "outside.py"
        outside.write_bytes(self.fixture.data)
        source = self.fixture.source / "store.py"
        source.unlink()
        try:
            source.symlink_to(outside)
        except (OSError, NotImplementedError):
            self.skipTest("symlink creation unavailable")
        with self.assertRaisesRegex(ValueError, "escapes"):
            self.fixture.assess()

    @unittest.skipUnless(hasattr(os, "mkfifo"), "FIFO fixture needs Unix")
    def test_nonregular_artifact_is_rejected_without_opening_it(self):
        fifo = self.fixture.root / "fifo"
        os.mkfifo(fifo)
        self.fixture.entry("compass")["artifact"] = dict(path="fifo", sha256="0" * 64)
        with self.assertRaisesRegex(ValueError, "regular files"):
            self.fixture.assess()
