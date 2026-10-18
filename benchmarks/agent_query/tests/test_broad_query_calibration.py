"""Exposed development recapture is provenance-bound and never confirmation."""
from copy import deepcopy
from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_review as review
from benchmarks.agent_query import broad_query_panels as panels
from benchmarks.agent_query.broad_query_capture import collect
from benchmarks.agent_query.tests import test_broad_query_capture as capture_fixture
from benchmarks.agent_query.tests import test_broad_query_panels as panel_fixture


def calibration_registration(registration, prior_registration, prior_run):
    registered = deepcopy(registration)
    registered.update(schema=review.REGISTRATION_SCHEMA_V2,
                      frozenAt="2026-01-01T00:00:03Z")
    registered["sourceReview"]["blindToOutputs"] = False
    registered["independentReview"].update(priorOutputsObserved=False,
                                           reviewedAt="2026-01-01T00:00:02Z")
    definition = dict(repositories=registered["repositories"], tasks=registered["tasks"])
    registered["independentReview"]["sourceDefinitionSha256"] = review.sha(
        json.dumps(definition, sort_keys=True, separators=(",", ":"), allow_nan=False).encode())
    registered["developmentCalibration"] = dict(
        schema="compass.broad-query-development-calibration/1",
        purpose="paired_development_calibration", priorOutputExposure=True,
        priorRegistrationSha256=prior_registration["sha256"],
        priorRunSha256=prior_run["sha256"],
        selection=[dict(task=t["id"], priorTask=t["id"]) for t in registered["tasks"]],
        selectionRationale="All synthetic questions selected before new capture; no winner filtering.")
    return registered


class DevelopmentCalibrationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = capture_fixture.MockCaptureTests("test_end_to_end_mocked_capture_report_and_recompute_reject_qualification")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.spec = self.fixture.spec()
        old = json.loads(Path(self.spec["registration"]["path"]).read_bytes())
        self.prior = deepcopy(old)
        self.prior_run = dict(schema=review.RUN_SCHEMA,
            registrationSha256=self.spec["registration"]["sha256"],
            startedAt="2026-01-01T00:00:01Z", completedAt="2026-01-01T00:00:01Z",
            observations=[dict(task=t["id"], tool=tool, arm=arm)
                          for t in old["tasks"] for tool in review.TOOLS for arm in review.ARMS])
        self.run_ref = self.fixture.put("prior-run.json", self.prior_run)
        self.registered = calibration_registration(old, self.spec["registration"], self.run_ref)
        self.evidence = {self.spec["registration"]["sha256"]: self.spec["registration"],
                         self.run_ref["sha256"]: self.run_ref}

    def publish(self):
        self.spec["registration"] = self.fixture.put("new-registration.json", self.registered)
        self.spec["calibrationEvidence"] = self.evidence

    def verify(self):
        return review.verify_calibration_evidence(self.registered, self.evidence,
                                                  lambda ref: Path(ref["path"]).read_bytes())

    def change_prior_run(self, change):
        change(self.prior_run)
        self.run_ref = self.fixture.put("changed-prior-run.json", self.prior_run)
        profile = self.registered["developmentCalibration"]
        self.evidence.pop(profile["priorRunSha256"])
        profile["priorRunSha256"] = self.run_ref["sha256"]
        self.evidence[self.run_ref["sha256"]] = self.run_ref

    def repin_history(self):
        previous = self.registered["developmentCalibration"]
        self.evidence.pop(previous["priorRegistrationSha256"])
        ref = self.fixture.put("repinned-historical-source.json", self.prior)
        previous["priorRegistrationSha256"] = ref["sha256"]
        self.evidence[ref["sha256"]] = ref
        self.change_prior_run(lambda r: r.update(registrationSha256=ref["sha256"]))

    def test_truthful_exposed_recapture_retains_history_and_stays_ineligible(self):
        self.publish()
        destination = self.fixture.root / "development-recapture"
        with redirect_stdout(io.StringIO()):
            collect(self.spec, destination, runner=self.fixture.runner)
        manifest = json.loads((destination / "review-input.json").read_bytes())
        self.assertEqual(set(manifest["calibrationEvidence"]), set(self.evidence))
        for digest, ref in manifest["calibrationEvidence"].items():
            self.assertEqual(review.sha((destination / ref["path"]).read_bytes()), digest)
        report = json.loads((destination / "report-provisional.json").read_bytes())
        self.assertFalse(report["assessment"]["eligibleForSuperiorityClaim"])
        self.assertTrue(any("development calibration" in f for f in report["assessment"]["failures"]))
        ref = next(iter(manifest["calibrationEvidence"].values()))
        (destination / ref["path"]).write_bytes(b"changed history")
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            review.assess(manifest, destination)

    def test_legacy_and_heldout_cannot_opt_into_exposed_development(self):
        for changes in [dict(schema=review.REGISTRATION_SCHEMA), dict(designation="heldout"),
                        dict(qualificationEligible=True)]:
            registered = deepcopy(self.registered)
            registered.update(changes)
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                review.validate_registration(registered)

    def test_profile_requires_truthful_flags_version_and_complete_selection(self):
        for field, value in [("schema", "compass.broad-query-development-calibration/2"),
                             ("purpose", "confirmation"), ("priorOutputExposure", False),
                             ("selection", []), ("selectionRationale", ""), ("fresh", True)]:
            registered = deepcopy(self.registered)
            registered["developmentCalibration"][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                review.validate_registration(registered)
        registered = deepcopy(self.registered)
        registered["sourceReview"]["blindToOutputs"] = True
        with self.assertRaises(ValueError):
            review.validate_registration(registered)

    def test_source_reannotation_does_not_rewrite_historical_question_or_source(self):
        self.registered["tasks"][0]["facts"][0]["statement"] = "Prospectively corrected synthetic source fact."
        self.verify()
        for field, value in [("question", "an easier question"), ("kind", "exact"),
                             ("expected", "abstain"), ("repository", "missing")]:
            original = self.registered["tasks"][0][field]
            self.registered["tasks"][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.verify()
            self.registered["tasks"][0][field] = original
        for field in ("commit", "url", "files"):
            registered = deepcopy(self.registered)
            registered["repositories"][0][field] = "b" * 40 if field == "commit" else [] if field == "files" else "https://example.invalid/other"
            with self.subTest(field=field), self.assertRaises(ValueError):
                review.verify_calibration_evidence(registered, self.evidence,
                                                   lambda ref: Path(ref["path"]).read_bytes())

    def test_selection_is_explicit_and_cannot_invent_or_reuse_a_historical_task(self):
        for selection in [[dict(task="save", priorTask="missing")],
                          [dict(task="save", priorTask="save")] * 2,
                          [dict(task="missing", priorTask="save")]]:
            self.registered["developmentCalibration"]["selection"] = selection
            with self.subTest(selection=selection), self.assertRaises(ValueError):
                self.verify()

    def test_exact_subset_and_id_only_renames_preserve_the_historical_denominator(self):
        extra = deepcopy(self.prior["tasks"][0])
        extra.update(id="unselected", question="A different historical question")
        self.prior["tasks"].append(extra)
        self.prior_run["observations"].extend(
            dict(task="unselected", tool=tool, arm=arm) for tool in review.TOOLS for arm in review.ARMS)
        self.repin_history()
        self.registered["tasks"][0].update(id="renamed-question", repository="renamed-repo")
        self.registered["repositories"][0]["id"] = "renamed-repo"
        self.registered["developmentCalibration"]["selection"] = [dict(task="renamed-question", priorTask="save")]
        self.verify()
        self.assertEqual(len(self.prior["tasks"]), 2)
        self.assertEqual(len(self.registered["tasks"]), 1)

    def test_historical_run_must_be_linked_complete_unique_and_earlier(self):
        for label, change in [
            ("wrong registration", lambda r: r.update(registrationSha256="f" * 64)),
            ("missing arm", lambda r: r["observations"].pop()),
            ("duplicate", lambda r: r["observations"].append(deepcopy(r["observations"][0]))),
            ("later", lambda r: r.update(completedAt="2026-01-01T00:00:04Z")),
            ("schema", lambda r: r.update(schema="compass.broad-query-capture/999"))]:
            original = deepcopy(self.prior_run)
            self.change_prior_run(change)
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.verify()
            self.change_prior_run(lambda r: (r.clear(), r.update(original)))

    def test_missing_extra_or_mutated_evidence_rejects_before_any_subprocess(self):
        for label in ("missing", "extra", "mutated"):
            evidence = deepcopy(self.evidence)
            if label == "missing":
                evidence.pop(self.run_ref["sha256"])
            elif label == "extra":
                evidence["c" * 64] = self.run_ref
            else:
                evidence[self.run_ref["sha256"]] = self.fixture.put("wrong-history.json", b"{}")
            self.publish()
            self.spec["calibrationEvidence"] = evidence
            destination = self.fixture.root / ("rejected-" + label)
            with self.subTest(label=label), self.assertRaises(ValueError):
                collect(self.spec, destination, runner=self.fixture.runner)
            self.assertEqual(self.fixture.calls, [])
            self.assertFalse(destination.exists())

    def test_metadata_document_limit_and_calibration_chains_are_rejected(self):
        with patch.object(review, "MAX_JSON_BYTES", 1), self.assertRaisesRegex(ValueError, "byte limit"):
            review.verify_calibration_evidence(self.registered, self.evidence, lambda ref: b"xx")
        self.prior["developmentCalibration"] = deepcopy(self.registered["developmentCalibration"])
        self.repin_history()
        with self.assertRaisesRegex(ValueError, "calibration chain"):
            self.verify()

    def test_independent_source_reviewer_cannot_have_seen_selected_answers(self):
        for changes in [dict(priorOutputsObserved=True), dict(priorOutputsObserved=None),
                        dict(reviewedAt="2026-01-01T00:00:04Z"),
                        dict(reviewedAt="2026-01-01T00:00:02"), dict(status="pending"),
                        dict(sourceAware=False), dict(toolLabelsBlinded=False),
                        dict(reviewer=self.registered["sourceReview"]["author"])]:
            original = deepcopy(self.registered["independentReview"])
            self.registered["independentReview"].update(changes)
            self.publish()
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                collect(self.spec, self.fixture.root / "review-rejected", runner=self.fixture.runner)
            self.assertEqual(self.fixture.calls, [])
            self.registered["independentReview"] = original

    def test_source_review_cannot_be_carried_over_a_changed_oracle(self):
        self.registered["tasks"][0]["facts"][0]["statement"] = "Changed after approval."
        self.publish()
        with self.assertRaisesRegex(ValueError, "independent source review"):
            collect(self.spec, self.fixture.root / "changed-oracle", runner=self.fixture.runner)
        self.assertEqual(self.fixture.calls, [])

    def test_metadata_changes_during_queries_or_scoring_prevent_completion(self):
        from benchmarks.agent_query import broad_query_capture as capture
        original_main = capture.review_main
        for stage in ("query-original", "score-original", "score-copy"):
            destination = self.fixture.root / stage
            original_path = Path(self.run_ref["path"])
            original_bytes = original_path.read_bytes()
            self.publish()

            def mutate():
                path = (destination / "provenance" / "calibration" / self.run_ref["sha256"]
                        if stage == "score-copy" else original_path)
                path.write_bytes(b"changed during collection")

            def runner(argv, **kwargs):
                result = self.fixture.runner(argv, **kwargs)
                if stage == "query-original" and "query" in argv:
                    mutate()
                return result

            def scoring(argv):
                status = original_main(argv)
                if stage.startswith("score-"):
                    mutate()
                return status

            with self.subTest(stage=stage), redirect_stdout(io.StringIO()), \
                    patch.object(capture, "review_main", scoring), self.assertRaises(ValueError):
                collect(self.spec, destination, runner=runner)
            self.assertFalse((destination / "COMPLETE.json").exists())
            original_path.write_bytes(original_bytes)

    def test_actor_drift_during_provisional_scoring_prevents_completion(self):
        from benchmarks.agent_query import broad_query_capture as capture
        original_main, original_reader = capture.review_main, capture.read_bounded
        scored = False

        def scoring(argv):
            nonlocal scored
            result = original_main(argv)
            scored = True
            return result

        def reader(path, maximum):
            data = original_reader(path, maximum)
            return data + b"drift" if scored and Path(path) == Path(capture.__file__) else data

        self.publish()
        destination = self.fixture.root / "actor-drift"
        with redirect_stdout(io.StringIO()), patch.object(capture, "review_main", scoring), \
                patch.object(capture, "read_bounded", reader), self.assertRaisesRegex(ValueError, "input changed"):
            collect(self.spec, destination, runner=self.fixture.runner)
        self.assertFalse((destination / "COMPLETE.json").exists())

    def test_retained_actor_changes_after_scoring_prevent_completion(self):
        from benchmarks.agent_query import broad_query_capture as capture
        original_main = capture.review_main
        for actor in ("broad_query_capture.py", "broad_query_review.py"):
            self.publish()
            destination = self.fixture.root / ("retained-" + actor)

            def scoring(argv):
                status = original_main(argv)
                (destination / "provenance" / actor).write_bytes(b"changed after provisional scoring")
                return status

            with self.subTest(actor=actor):
                with redirect_stdout(io.StringIO()), patch.object(capture, "review_main", scoring), \
                        self.assertRaises(ValueError):
                    collect(self.spec, destination, runner=self.fixture.runner)
                self.assertFalse((destination / "COMPLETE.json").exists())

    def test_calibration_run_requires_its_exact_retained_assessor(self):
        self.publish()
        destination = self.fixture.root / "assessor-binding"
        with redirect_stdout(io.StringIO()):
            collect(self.spec, destination, runner=self.fixture.runner)
        manifest = json.loads((destination / "review-input.json").read_bytes())
        run = json.loads((destination / manifest["run"]["path"]).read_bytes())
        self.assertEqual(run["assessor"]["sha256"], self.spec["plan"]["assessorSha256"])
        for name in ("missing", "substituted"):
            changed = deepcopy(run)
            if name == "missing":
                changed.pop("assessor")
            else:
                changed["assessor"] = deepcopy(changed["tools"][0]["normalizer"])
            data = json.dumps(changed, sort_keys=True).encode()
            relative = name + "-assessor-run.json"
            (destination / relative).write_bytes(data)
            altered = deepcopy(manifest)
            altered["run"] = dict(path=relative, sha256=review.sha(data))
            judged = json.loads((destination / manifest["reviews"]["path"]).read_bytes())
            judged["runSha256"] = altered["run"]["sha256"]
            data = json.dumps(judged, sort_keys=True).encode()
            relative = name + "-assessor-reviews.json"
            (destination / relative).write_bytes(data)
            altered["reviews"] = dict(path=relative, sha256=review.sha(data))
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "assessor"):
                review.assess(altered, destination)

    def test_unprofiled_development_still_requires_source_first_authoring(self):
        self.registered.pop("developmentCalibration")
        self.publish()
        self.spec.pop("calibrationEvidence")
        with self.assertRaisesRegex(ValueError, "output-blinded"):
            collect(self.spec, self.fixture.root / "legacy-rejected", runner=self.fixture.runner)
        self.assertEqual(self.fixture.calls, [])


class CalibrationPanelTests(unittest.TestCase):
    def setUp(self):
        self.panel = panel_fixture.PanelBindingTests(
            "test_union_projection_reuses_captures_and_produces_canonical_checkable_inputs")
        self.panel.setUp()
        self.addCleanup(self.panel.doCleanups)
        p = self.panel
        historical = deepcopy(p.registered["original"])
        historical["tasks"].append(deepcopy(p.registered["lookup"]["tasks"][1]))
        historical["frozenAt"] = "2025-12-01T00:00:00Z"
        prior_ref = p.fixture.put("historical-source.json", historical)
        prior_run = dict(schema=review.RUN_SCHEMA, registrationSha256=prior_ref["sha256"],
                         startedAt="2025-12-01T00:00:01Z", completedAt="2025-12-01T00:00:02Z",
                         observations=[dict(task=t["id"], tool=tool, arm=arm)
                                       for t in historical["tasks"] for tool in review.TOOLS for arm in review.ARMS])
        run_ref = p.fixture.put("historical-run.json", prior_run)
        self.evidence = {prior_ref["sha256"]: prior_ref, run_ref["sha256"]: run_ref}
        for role in panels.ROLES:
            reg = calibration_registration(p.registered[role], prior_ref, run_ref)
            reg["frozenAt"] = "2026-01-01T00:00:00Z"
            reg["independentReview"]["reviewedAt"] = "2025-12-01T00:00:03Z"
            p.registered[role] = reg
            ref = p.fixture.put(role + "-calibration.json", reg)
            p.definition["registrations"][role] = ref
            p.inputs[role].update(registration=ref, calibrationEvidence=self.evidence)
            run = p.read(p.inputs[role]["run"])
            run["registrationSha256"] = ref["sha256"]
            run["assessor"] = p.fixture.put("retained-assessor.py", Path(review.__file__).read_bytes())
            p.refresh_run(role, run)
        p.manifest["definition"] = p.fixture.put("calibration-definition.json", p.definition)

    def union(self):
        p = self.panel
        source = p.union()
        reg = p.read(source["registration"])
        extra = next(item for item in p.registered["lookup"]["developmentCalibration"]["selection"]
                     if item["task"] == "named.lookup")
        reg["developmentCalibration"]["selection"].append(deepcopy(extra))
        source["registration"] = p.fixture.put("calibration-union-registration.json", reg)
        run = p.read(source["run"])
        run["registrationSha256"] = source["registration"]["sha256"]
        source["run"] = p.fixture.put("calibration-union-run.json", run)
        judgments = p.read(source["reviews"])
        judgments["runSha256"] = source["run"]["sha256"]
        source["reviews"] = p.fixture.put("calibration-union-judgments.json", judgments)
        return source

    def test_projection_restricts_selection_and_preserves_verified_evidence(self):
        p = self.panel
        # Selection list order is not semantic; exact current->prior mapping is.
        p.registered["lookup"]["developmentCalibration"]["selection"].reverse()
        panels.validate_panels(p.definition, p.registered)
        receipt = panels.project_capture(self.union(), p.manifest["definition"], p.root)
        for role, ref in receipt["inputs"].items():
            manifest = p.read(ref)
            self.assertEqual(manifest["calibrationEvidence"], self.evidence)
            assessment = review.assess(manifest, p.root)
            self.assertFalse(assessment["eligibleForSuperiorityClaim"])
            self.assertTrue(any("development calibration" in f for f in assessment["failures"]))

    def test_profile_omission_and_changed_common_mapping_are_rejected(self):
        for change in (lambda r: r.pop("developmentCalibration"),
                       lambda r: r["developmentCalibration"]["selection"][0].update(priorTask="elsewhere")):
            registers = deepcopy(self.panel.registered)
            change(registers["lookup"])
            with self.assertRaisesRegex(ValueError, "calibration"):
                panels.validate_panels(self.panel.definition, registers)

    def test_missing_union_evidence_rejects_before_projection_publication(self):
        source = self.union()
        source.pop("calibrationEvidence")
        with self.assertRaisesRegex(ValueError, "calibration evidence"):
            panels.project_capture(source, self.panel.manifest["definition"], self.panel.root)
        self.assertFalse((self.panel.root / "panels-projection.json").exists())


if __name__ == "__main__":
    unittest.main()
