"""File-backed synthetic lineage, not captured executions or source judgments."""
import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.agent_query.broad_query_actors_v2 import freeze_protocol, retain_protocol
from benchmarks.agent_query.broad_query_capture import Bundle
from benchmarks.agent_query.broad_query_lineage_v2 import load_review_lineage, SCHEMAS
from benchmarks.agent_query.broad_query_normalize_v2 import normalize_v2
from benchmarks.agent_query.broad_query_protocol import flatten_claim_batches, pack_claim_batches
from benchmarks.agent_query.broad_query_review import ARMS, TOOLS, Artifacts, GraphIndex
from benchmarks.agent_query.tests.test_broad_query_actors_v2 import source_tree
from benchmarks.agent_query.tests.test_broad_query_review import graph, registration


class Fixture:
    def __init__(self, directory, *, rejected=False, task_ids=("save",)):
        self.root = Path(directory).resolve() / "product"; self.root.mkdir(); source_tree(self.root)
        descriptor, self.actors = freeze_protocol(self.root)
        self.bundle = Bundle(Path(directory) / "evidence")
        protocol = retain_protocol(self.bundle, self.root, descriptor, self.actors)
        self.registration = registration()
        template = self.registration["tasks"][0]
        self.registration["tasks"] = [dict(copy.deepcopy(template), id=identifier, cluster=identifier)
                                      for identifier in task_ids]
        self.registration["arms"] = [dict(id="default"),dict(id="normalized",measure="utf8_bytes",budgetBytes=512,overflow="fail")]
        self.registration["capturePlan"] = dict(schema=SCHEMAS["capture-plan"],
            arms=self.registration["arms"],actorSha256=self.actors)
        self.registration_ref = self.put("registration",self.registration)
        self.initials, observations, self.labels, self.selections = [], [], [], []
        for task in sorted(self.registration["tasks"], key=lambda task: task["id"]):
            for arm in ARMS:
                for tool in TOOLS:
                    name = f"{task['id']}-{tool}-{arm}"
                    key = dict(task=task["id"],tool=tool,arm=arm)
                    raw = (b"\xff" if rejected else b"NODE Store::save [function] store.py:2:1-3:28\n"
                        if tool == "compass" else b"NODE Store.save() [src=store.py loc=L2]\n")
                    stdout = self.bundle.put(name+".stdout",raw)
                    stderr = self.bundle.put(name+".stderr",b"synthetic stderr\n")
                    capture = self.put(name+"-capture",dict(argv=["synthetic","question"],exit_code=0,
                        wall_ms=1,timed_out=False,output_limited=False,stdout=stdout,stderr=stderr,
                        stdout_bytes=len(raw),stderr_bytes=17))
                    normalized = normalize_v2(raw,tool,GraphIndex(graph(tool),tool),task=task["id"],arm=arm,
                        source_registration_sha256=self.registration_ref["sha256"],actors=self.actors)
                    initial_ref = self.put(name+"-initial",normalized)
                    self.initials.append(normalized)
                    observations.append(dict(key,captures=[capture],normalized=initial_ref))
                    claims = flatten_claim_batches(normalized)
                    labels = [dict(id=claim["id"],verdict="unjudged",reason="Pending synthetic review.",
                        printedOccurrences=[dict(id=o["id"],verdict="unjudged",reason="Pending synthetic occurrence.")
                                            for o in claim["printedOccurrences"]]) for claim in claims]
                    self.labels.append(dict(key,schema=SCHEMAS["judgments"],reviewedNormalizedSha256=initial_ref["sha256"],
                        candidates=[],semanticCoverageComplete=False,emptyConfirmed=False,**pack_claim_batches(labels)))
                    self.selections.append(dict(key,initialNormalizedRef=initial_ref,reviewedNormalizedRef=initial_ref,
                        rawStdoutSha256=stdout["sha256"],sourceRegistrationSha256=self.registration_ref["sha256"],actorSha256=self.actors))
        self.run = dict(schema=SCHEMAS["capture"],registrationSha256=self.registration_ref["sha256"],
            startedAt="2026-01-02T00:00:00Z",completedAt="2026-01-02T00:00:01Z",protocolBinding=protocol,
            arms=self.registration["arms"],observations=observations)
        self.run_ref = self.put("run",self.run)
        self.reviews = dict(schema=SCHEMAS["judgments"],runSha256=self.run_ref["sha256"],observations=[])
        for label in self.labels: label["runSha256"] = self.run_ref["sha256"]
        for selection in self.selections: selection["originalRunSha256"] = self.run_ref["sha256"]
        self.manifest = dict(schema=SCHEMAS["review-input"],registration=self.registration_ref,run=self.run_ref,
            reviews=None,normalizedSelections=self.selections)
        self.counter = 0
        self.rewrite_reviews()

    def put(self, name, value):
        return self.bundle.put(name+".json",value)

    def rewrite_reviews(self):
        self.counter += 1
        index = []
        for number, label in enumerate(self.labels):
            reference = self.put(f"judgment-{self.counter}-{number}",label)
            index.append(dict(task=label["task"],tool=label["tool"],arm=label["arm"],judgmentRef=reference))
        self.reviews["observations"] = index
        self.manifest["reviews"] = self.put(f"reviews-{self.counter}",self.reviews)

    def alternate_run(self):
        # A separate synthetic input variant; the original run stays on disk.
        self.counter += 1
        self.manifest["run"] = self.put(f"run-{self.counter}",self.run)
        self.reviews["runSha256"] = self.manifest["run"]["sha256"]
        for label in self.labels: label["runSha256"] = self.manifest["run"]["sha256"]
        for selection in self.selections: selection["originalRunSha256"] = self.manifest["run"]["sha256"]
        self.rewrite_reviews()

    def change_initial(self, document):
        self.counter += 1; ref = self.put(f"changed-initial-{self.counter}",document)
        self.run["observations"][0]["normalized"] = ref
        self.selections[0]["initialNormalizedRef"] = ref
        self.selections[0]["reviewedNormalizedRef"] = ref
        self.labels[0]["reviewedNormalizedSha256"] = ref["sha256"]
        self.alternate_run()

    def load(self, *, consume=True):
        context = load_review_lineage(self.manifest,Artifacts(self.bundle.root),self.root)
        if consume:
            self.loaded = list(context.iter_observations())  # Four tiny fixture rows only.
        return context


class ReviewLineageTests(unittest.TestCase):
    def test_multiple_tasks_use_canonical_task_arm_tool_order_independent_of_registration_order(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory, task_ids=("zeta", "alpha"))
            context = fixture.load()
            self.assertEqual([row.key for row in fixture.loaded],
                [(task, tool, arm) for task in ("alpha", "zeta") for arm in ARMS for tool in TOOLS])
            context.recheck()
            # Reorder all three indexes consistently: agreement with each
            # other still cannot replace the source-frozen canonical order.
            fixture.run["observations"].reverse()
            fixture.selections.reverse()
            fixture.labels.reverse()
            fixture.alternate_run()
            with self.assertRaisesRegex(ValueError, "observations"):
                fixture.load()

    def test_complete_finite_pending_lineage_retains_every_native_row_and_stream(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            context = fixture.load()
            self.assertEqual(len(fixture.loaded),4)
            self.assertEqual([row.key for row in fixture.loaded],
                [("save",tool,arm) for arm in ARMS for tool in TOOLS])
            self.assertTrue(all(row.stderr == b"synthetic stderr\n" for row in fixture.loaded))
            context.recheck(); fixture.bundle.recheck()

    def test_reviewed_partition_can_change_without_rewriting_original_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            original = (fixture.bundle.root / fixture.run_ref["path"]).read_bytes()
            corrected = copy.deepcopy(fixture.initials[0])
            corrected["normalizationStatus"] = "reviewed"
            corrected["claimBatches"][0]["claims"][0]["id"] = "reviewed-claim"
            corrected["claimBatches"][0]["claims"][0]["printedOccurrences"][0]["id"] = "reviewed-occurrence"
            reviewed_ref = fixture.put("reviewed-corrected",corrected)
            fixture.selections[0]["reviewedNormalizedRef"] = reviewed_ref
            fixture.labels[0]["reviewedNormalizedSha256"] = reviewed_ref["sha256"]
            label = fixture.labels[0]["claimBatches"][0]["claims"][0]
            label["id"] = "reviewed-claim"; label["printedOccurrences"][0]["id"] = "reviewed-occurrence"
            fixture.rewrite_reviews()
            context = fixture.load()
            self.assertEqual(fixture.loaded[0].initial["claimBatches"][0]["claims"][0]["id"],"claim-0001")
            self.assertEqual(fixture.loaded[0].reviewed["claimBatches"][0]["claims"][0]["id"],"reviewed-claim")
            self.assertEqual((fixture.bundle.root / fixture.run_ref["path"]).read_bytes(),original)

    def test_judgments_must_match_exact_reviewed_claim_and_occurrence_partition(self):
        for change in ("sha","run","claim","occurrence","missing"):
            with self.subTest(change=change),tempfile.TemporaryDirectory() as directory:
                fixture=Fixture(directory);label=fixture.labels[-1]
                if change == "sha": label["reviewedNormalizedSha256"]="0"*64
                elif change == "run": label["runSha256"]="0"*64
                elif change == "claim": label["claimBatches"][0]["claims"][0]["id"]="foreign"
                elif change == "occurrence": label["claimBatches"][0]["claims"][0]["printedOccurrences"][0]["id"]="foreign"
                else: label["claimBatches"][0]["claims"][0]["printedOccurrences"]=[]
                fixture.rewrite_reviews()
                with self.assertRaises(ValueError): fixture.load()

    def test_initial_reference_cannot_be_replaced_by_equal_byte_alias(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture=Fixture(directory)
            fixture.selections[0]["initialNormalizedRef"]=fixture.put("alias",fixture.initials[0])
            with self.assertRaisesRegex(ValueError,"immutable original run"): fixture.load()

    def test_full_matrix_rejects_late_missing_duplicate_reordered_or_foreign_rows(self):
        for table in ("selections","judgments","capture"):
            for change in ("missing","duplicate","reorder","foreign"):
                with self.subTest(table=table,change=change),tempfile.TemporaryDirectory() as directory:
                    fixture=Fixture(directory)
                    rows=fixture.selections if table == "selections" else fixture.labels if table == "judgments" else fixture.run["observations"]
                    if change == "missing": rows.pop()
                    elif change == "duplicate": rows[-1]=copy.deepcopy(rows[0])
                    elif change == "reorder": rows.reverse()
                    else: rows[-1]["task"]="foreign"
                    if table == "judgments": fixture.rewrite_reviews()
                    if table == "capture": fixture.alternate_run()
                    with self.assertRaisesRegex(ValueError,"matrix|observations"): fixture.load()

    def test_selection_bindings_and_transitive_references_cannot_drift(self):
        for field in ("originalRunSha256","rawStdoutSha256","sourceRegistrationSha256","actorSha256","parentRef"):
            with self.subTest(field=field),tempfile.TemporaryDirectory() as directory:
                fixture=Fixture(directory)
                fixture.selections[-1][field]="0"*64
                with self.assertRaises(ValueError): fixture.load()
        with tempfile.TemporaryDirectory() as directory:
            fixture=Fixture(directory);fixture.manifest["amendments"]=[]
            with self.assertRaises(ValueError): fixture.load()

    def test_normalization_cannot_bind_future_run_wrong_source_or_foreign_actors(self):
        for field,value in (("originalRunSha256","0"*64),("rawStdoutSha256","0"*64),
                ("sourceRegistrationSha256","0"*64),("actorSha256",{}),("task","foreign"),
                ("normalizationComplete",True),("normalizationStatus","reviewed")):
            with self.subTest(field=field),tempfile.TemporaryDirectory() as directory:
                fixture=Fixture(directory);document=copy.deepcopy(fixture.initials[0]);document[field]=value
                fixture.change_initial(document)
                with self.assertRaises(ValueError): fixture.load()

    def test_mixed_and_unknown_schemas_are_rejected_without_legacy_conversion(self):
        for target in ("manifest","run","reviews","initial","row"):
            for major in (1,99):
                with self.subTest(target=target,major=major),tempfile.TemporaryDirectory() as directory:
                    fixture=Fixture(directory)
                    if target == "manifest": fixture.manifest["schema"]=f"compass.broad-query-review-input/{major}"
                    elif target == "run": fixture.run["schema"]=f"compass.broad-query-capture/{major}";fixture.alternate_run()
                    elif target == "reviews": fixture.reviews["schema"]=f"compass.broad-query-judgments/{major}";fixture.rewrite_reviews()
                    elif target == "row": fixture.labels[-1]["schema"]=f"compass.broad-query-judgments/{major}";fixture.rewrite_reviews()
                    else:
                        document=copy.deepcopy(fixture.initials[0]);document["schema"]=f"compass.broad-query-normalized/{major}";fixture.change_initial(document)
                    with self.assertRaises(ValueError): fixture.load()

    def test_rejected_rows_keep_complete_matrix_with_unknown_assertion_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture=Fixture(directory,rejected=True);context=fixture.load()
            self.assertEqual(len(fixture.loaded),4)
            for row in fixture.loaded:
                self.assertEqual(row.stdout,b"\xff")
                self.assertEqual(row.initial["claimCount"],0)
                self.assertIsNone(row.initial["printedAssertionCount"])
                self.assertIsNone(row.initial["falseEmpty"])
                self.assertFalse(row.initial["success"])
            context.recheck()

    def test_shared_capture_paths_and_free_followups_are_rejected(self):
        for change in ("reuse","followup"):
            with self.subTest(change=change),tempfile.TemporaryDirectory() as directory:
                fixture=Fixture(directory)
                first=fixture.run["observations"][0]["captures"][0]
                if change == "reuse": fixture.run["observations"][-1]["captures"]=[first]
                else: fixture.run["observations"][-1]["captures"].append(first)
                fixture.alternate_run()
                with self.assertRaises(ValueError): fixture.load()

    def test_raw_and_original_or_retained_actor_drift_fail_final_recheck(self):
        for target in ("raw","original_actor","retained_actor"):
            with self.subTest(target=target),tempfile.TemporaryDirectory() as directory:
                fixture=Fixture(directory);context=fixture.load()
                if target == "raw":
                    ref=fixture.loaded[0].capture["stdout"];path=fixture.bundle.root/ref["path"]
                elif target == "original_actor": path=fixture.root/"benchmarks/agent_query/broad_query_review.py"
                else: path=fixture.bundle.root/fixture.run["protocolBinding"]["files"][0]["artifact"]["path"]
                path.write_bytes(path.read_bytes()+b"\n")
                with self.assertRaises(ValueError): context.recheck()

    def test_loading_is_lazy_and_equal_initial_reviewed_reference_is_read_once(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            artifacts = Artifacts(fixture.bundle.root)
            with patch.object(artifacts, "read", wraps=artifacts.read) as read:
                context = load_review_lineage(fixture.manifest, artifacts, fixture.root)
                header_paths = {call.args[0]["path"] for call in read.call_args_list}
                deferred = []
                for observed, selection, index in zip(fixture.run["observations"],
                        fixture.selections, fixture.reviews["observations"]):
                    deferred.extend([observed["captures"][0]["path"],
                        selection["initialNormalizedRef"]["path"], index["judgmentRef"]["path"]])
                self.assertTrue(header_paths.isdisjoint(deferred))
                read.reset_mock()
                stream = context.iter_observations()
                first = next(stream)
                paths = [call.args[0]["path"] for call in read.call_args_list]
                self.assertEqual(paths.count(deferred[1]), 1)
                self.assertTrue(set(paths).isdisjoint(deferred[3:]))
                self.assertIs(first.initial, first.reviewed)
                stream.close()
                with self.assertRaises(ValueError): context.recheck()

    def test_only_full_successful_traversal_allows_final_recheck(self):
        for stop in ("not_started", "not_resumed", "closed", "complete"):
            with self.subTest(stop=stop), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(directory)
                context = fixture.load(consume=False)
                if stop == "not_started":
                    with self.assertRaises(ValueError): context.recheck()
                    stream = context.iter_observations()
                    stream.close()
                else:
                    stream = context.iter_observations()
                    if stop == "complete":
                        for _ in stream: pass
                        context.recheck()
                    else:
                        next(stream)
                        if stop == "closed": stream.close()
                        with self.assertRaises(ValueError): context.recheck()
                        stream.close()
                with self.assertRaises(ValueError): context.iter_observations()
                if stop != "complete":
                    with self.assertRaises(ValueError): context.recheck()

    def test_late_row_failure_prevents_publication_of_valid_prefix(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.labels[-1]["runSha256"] = "0" * 64
            fixture.rewrite_reviews()
            context = fixture.load(consume=False)
            stream = context.iter_observations()
            for _ in range(3): next(stream)
            with self.assertRaises(ValueError): next(stream)
            with self.assertRaises(ValueError): context.recheck()
            with self.assertRaises(ValueError): context.iter_observations()

    def test_failed_final_recheck_cannot_be_retried_after_restoring_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            context = fixture.load()
            path = fixture.bundle.root / fixture.loaded[0].capture["stdout"]["path"]
            original = path.read_bytes()
            path.write_bytes(original + b"\n")
            with self.assertRaises(ValueError): context.recheck()
            path.write_bytes(original)
            with self.assertRaises(ValueError): context.recheck()

    def test_embedded_judgments_cannot_bypass_bounded_document_index(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.reviews["observations"] = copy.deepcopy(fixture.labels)
            fixture.manifest["reviews"] = fixture.put("embedded-judgments", fixture.reviews)
            with self.assertRaisesRegex(ValueError, "bounded observation document"):
                fixture.load()

    def test_rejection_cannot_retain_ranked_or_coverage_prefix_or_success(self):
        for field, value in (("ranked", [{"rank": 1}]), ("lineLedger", [{}]),
                ("reviewQueue", [{}]), ("admission", {}), ("rankingKind", "retrieval"),
                ("disposition", "answered"), ("success", True)):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(directory, rejected=True)
                document = copy.deepcopy(fixture.initials[0])
                document[field] = value
                fixture.change_initial(document)
                with self.assertRaisesRegex(ValueError, "rejection"):
                    fixture.load()

    def test_nonrejected_normalization_cannot_predeclare_scoring_results(self):
        for field, value in (("fullMeasuredPrecision", 1), ("printedAssertionCount", 0),
                ("falseEmpty", False), ("success", True)):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(directory)
                document = copy.deepcopy(fixture.initials[0])
                document[field] = value
                fixture.change_initial(document)
                with self.assertRaisesRegex(ValueError, "scoring or rejection fields"):
                    fixture.load()


if __name__ == "__main__": unittest.main()
