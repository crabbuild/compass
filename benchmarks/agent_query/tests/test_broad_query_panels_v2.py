"""Synthetic panels share one immutable union; no comparative qualification."""
import copy
from pathlib import Path
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query import broad_query_capture as capture
from benchmarks.agent_query.broad_query_panels_v2 import DEFINITION_SCHEMA, BINDING_SCHEMA
from benchmarks.agent_query.broad_query_report_v2 import score_file_v2, check_file_v2
from benchmarks.agent_query.tests import test_broad_query_collect_v2 as collection
from benchmarks.agent_query.tests.test_broad_query_calibration import calibration_registration


class PanelFixture:
    def __init__(self, test):
        self.case = collection.CollectorTests()
        self.case.setUp()
        test.addCleanup(self.case.doCleanups)
        self.registrations = {}
        broad = copy.deepcopy(self.case.registration["tasks"][0])
        named = dict(copy.deepcopy(broad), id="named", kind="exact", cluster="save-control",
            question="How does Store.save delegate persistence?")
        lookup = copy.deepcopy(named)
        lookup.update(id="named.lookup", question="Locate the declaration of Store.save in store.py.")
        lookup["facts"] = [dict(id="declaration", essential=True,
            identity=copy.deepcopy(lookup["relevant"][0]["identity"]),
            statement="Declaration identity at the frozen source anchor.",
            source=dict(file="store.py", line=2, text="    def save(self):"))]
        for role, task in (("original", named), ("lookup", lookup)):
            source = copy.deepcopy(self.case.registration)
            source["tasks"] = [copy.deepcopy(broad), copy.deepcopy(task)]
            self.registrations[role] = source
        self.case.registration["tasks"] = [broad, named, lookup]
        self.case.registration["frozenAt"] = "2026-01-01T00:00:02Z"
        self.definition = dict(schema=DEFINITION_SCHEMA, frozenAt="2026-01-01T00:00:01Z",
            actorSha256=self.case.spec["plan"]["actorSha256"],
            lookupPairs=[dict(original="named", lookup="named.lookup")])
        self.publish()

    def publish(self):
        references = {role: self.case.fixture.put(role + "-source.json", source)
            for role, source in self.registrations.items()}
        self.definition["registrations"] = {role: ref["sha256"] for role, ref in references.items()}
        definition = self.case.fixture.put("panels-definition.json", self.definition)
        self.case.registration["comparisonPanels"] = dict(schema=BINDING_SCHEMA, definitionSha256=definition["sha256"])
        self.case.spec["panels"] = dict(definition=definition, registrations=references)
        self.case.refreeze()

    def collect(self):
        result = self.case.collect()
        self.manifest = old._json((self.case.destination / "review-input.json").read_bytes())
        self.run = self.case.document(self.manifest["run"])
        return old._json(Path(result["report"]).read_bytes())["assessment"]

    def calibrate(self, *, alias=False):
        prior = copy.deepcopy(self.case.registration)
        prior.pop("comparisonPanels")
        prior["frozenAt"] = "2026-01-01T00:00:00Z"
        if alias:
            prior["tasks"].append(dict(copy.deepcopy(prior["tasks"][0]), id="save-alias"))
        source_ref = self.case.fixture.put("prior-union-source.json", prior)
        run_ref = self.case.fixture.put("prior-union-run.json", dict(schema=old.RUN_SCHEMA,
            registrationSha256=source_ref["sha256"], startedAt="2026-01-01T00:00:01Z",
            completedAt="2026-01-01T00:00:01Z", observations=[dict(task=task["id"], tool=tool, arm=arm)
                for task in prior["tasks"] for tool in old.TOOLS for arm in old.ARMS]))
        self.registrations = {role: calibration_registration(source, source_ref, run_ref)
            for role, source in self.registrations.items()}
        self.case.registration = calibration_registration(self.case.registration, source_ref, run_ref)
        self.case.registration["frozenAt"] = "2026-01-01T00:00:05Z"
        self.definition["frozenAt"] = "2026-01-01T00:00:04Z"
        self.case.spec["calibrationEvidence"] = {source_ref["sha256"]: source_ref, run_ref["sha256"]: run_ref}
        self.publish()


class PanelTests(unittest.TestCase):
    def test_exposed_calibration_panels_keep_exact_history_restrictions_and_stay_ineligible(self):
        fixture = PanelFixture(self)
        fixture.calibrate()
        report = fixture.collect()
        self.assertFalse(report["eligibleForSuperiorityClaim"])
        for role in ("original", "lookup"):
            self.assertTrue(any("permanently ineligible" in failure for failure in
                report["panelAssessment"]["reports"][role]["failures"]))
        for change in ("selection", "prior_pair"):
            with self.subTest(change=change):
                fixture = PanelFixture(self)
                fixture.calibrate(alias=True)
                if change == "selection":
                    for registration in fixture.registrations.values():
                        next(item for item in registration["developmentCalibration"]["selection"]
                            if item["task"] == "save")["priorTask"] = "save-alias"
                    # Each alternate selection is individually valid history;
                    # only the exact union restriction is violated.
                    for registration in fixture.registrations.values():
                        old.verify_calibration_evidence(registration, fixture.case.spec["calibrationEvidence"],
                            lambda ref: Path(ref["path"]).read_bytes())
                else:
                    fixture.registrations["original"]["developmentCalibration"]["priorRunSha256"] = "0" * 64
                fixture.publish()
                with self.assertRaisesRegex(ValueError, "calibration"):
                    fixture.case.collect()
                self.assertEqual(fixture.case.calls, [])

    def test_missing_frozen_panels_is_explicitly_ineligible(self):
        fixture = PanelFixture(self)
        fixture.case.registration.pop("comparisonPanels")
        fixture.case.spec.pop("panels")
        fixture.case.refreeze()
        report = fixture.collect()
        self.assertFalse(report["eligibleForSuperiorityClaim"])
        self.assertIn("source-frozen original/lookup comparison panels are missing", report["failures"])

    def test_full_panel_statistics_reuse_one_lineage_and_charge_native_costs_once(self):
        fixture = PanelFixture(self)
        from benchmarks.agent_query import broad_query_collect_v2 as module
        from benchmarks.agent_query import broad_query_assess_v2 as assessment
        with patch.object(module, "normalize_v2", wraps=module.normalize_v2) as normalize, \
                patch.object(assessment, "score_bound_observation", wraps=assessment.score_bound_observation) as score:
            report = fixture.collect()
        self.assertEqual(normalize.call_count, 12)
        self.assertEqual(score.call_count, 12)
        self.assertEqual(len(fixture.case.queries), 12)
        panels = report["panelAssessment"]
        self.assertEqual(panels["distinctQuestions"], 3)
        self.assertEqual(panels["sharedQuestions"], 1)
        self.assertEqual(panels["distinctObservations"], 12)
        self.assertEqual(len(panels["reports"]["original"]["rows"]), 8)
        self.assertEqual(len(panels["reports"]["lookup"]["rows"]), 8)
        self.assertEqual(sum(cost["workflowWallMs"] for cost in panels["costs"]), 120)
        self.assertEqual(sum(cost["workflowWallMs"] for cost in panels["buildCosts"]), 10)
        self.assertEqual(len(panels["buildCosts"]), 2)
        self.assertEqual(panels["costs"], report["costs"])
        self.assertFalse(report["eligibleForSuperiorityClaim"])
        for role in ("original", "lookup"):
            self.assertIsNone(panels["reports"][role]["comparisons"]["claimPrecisionEmptyZero"]["estimate"])
        # Every original normalization remains bound to the same union source;
        # it is never rewritten to a panel registration or a projected run.
        for observation in fixture.run["observations"]:
            normalized = fixture.case.document(observation["normalized"])
            self.assertEqual(normalized["sourceRegistrationSha256"], fixture.manifest["registration"]["sha256"])
        self.assertEqual(list(fixture.case.destination.glob("panels-*-run.json")), [])

    def test_all_three_synthetic_gates_are_required_without_an_always_false_shortcut(self):
        for failed in (None, "original", "lookup", "union"):
            with self.subTest(failed=failed):
                fixture = PanelFixture(self)
                finish = old._finish_assessment
                calls = []
                def synthetic_gate(manifest, registration, *args, **kwargs):
                    report = finish(manifest, registration, *args, **kwargs)
                    ids = {task["id"] for task in registration["tasks"]}
                    role = "union" if "comparisonPanels" in registration else "lookup" if "named.lookup" in ids else "original"
                    calls.append(role)
                    report.update(eligibleForSuperiorityClaim=role != failed,
                        failures=["synthetic gate failure"] if role == failed else [])
                    return report
                fixture.collect()
                # Replace only the mathematical gate outcome for this AND
                # composition test; never publish a synthetic passing report.
                with patch.object(old, "_finish_assessment", side_effect=synthetic_gate):
                    from benchmarks.agent_query.broad_query_assess_v2 import assess_file_v2
                    with patch("benchmarks.agent_query.broad_query_assess_v2.SourceReader.git",
                            side_effect=lambda root, *args: "a" * 40 + "\n" if "rev-parse" in args else ""):
                        combined = assess_file_v2(fixture.case.destination / "review-input.json")
                self.assertEqual(calls, ["union", "original", "lookup"])
                self.assertEqual(combined["eligibleForSuperiorityClaim"], failed is None)
                if failed is not None:
                    self.assertIn(failed + ": synthetic gate failure", combined["failures"])

    def test_frozen_panel_controls_and_source_gold_are_required_before_any_process(self):
        for change in ("actor", "late", "shared", "lookup_fact", "missing_role", "drop_union", "blind", "nested", "plan"):
            with self.subTest(change=change):
                fixture = PanelFixture(self)
                if change == "actor": fixture.definition["actorSha256"] = dict(protocol="0" * 64)
                if change == "late": fixture.definition["frozenAt"] = "2026-01-01T00:00:03Z"
                if change == "shared": fixture.registrations["lookup"]["tasks"][0]["rationale"] = "Changed shared source definition."
                if change == "lookup_fact": fixture.registrations["lookup"]["tasks"][1]["facts"][0]["statement"] = "A behavioral shortcut."
                if change == "missing_role": fixture.registrations.pop("original")
                if change == "drop_union": fixture.case.registration["tasks"].pop(0)
                if change == "blind": fixture.registrations["original"]["sourceReview"]["blindToOutputs"] = False
                if change == "nested": fixture.registrations["original"]["comparisonPanels"] = dict(schema=BINDING_SCHEMA, definitionSha256="0" * 64)
                if change == "plan": fixture.registrations["original"]["capturePlan"]["timeoutSeconds"] += 1
                fixture.publish()
                with self.assertRaises(ValueError): fixture.case.collect()
                self.assertEqual(fixture.case.calls, [])
                self.assertFalse((fixture.case.destination / "COMPLETE.json").exists())

    def test_definition_digest_and_retained_panel_drift_cannot_be_substituted(self):
        fixture = PanelFixture(self)
        fixture.case.registration["comparisonPanels"]["definitionSha256"] = "0" * 64
        fixture.case.refreeze()
        with self.assertRaisesRegex(ValueError, "source-frozen digest"): fixture.case.collect()
        self.assertEqual(fixture.case.calls, [])
        fixture = PanelFixture(self)
        def drift(argv, path):
            if argv[0] == "git" and "status" in argv and "-publication-" in str(path):
                (fixture.case.destination / "panels/original-registration.json").write_bytes(b"changed")
        fixture.case.after = drift
        with self.assertRaises(ValueError): fixture.case.collect()
        self.assertTrue((fixture.case.destination / "report-provisional.json").exists())
        self.assertFalse((fixture.case.destination / "COMPLETE.json").exists())

    def test_panel_report_recomputes_without_mutating_or_recapturing_the_union(self):
        fixture = PanelFixture(self)
        fixture.collect()
        run_path = fixture.case.destination / fixture.manifest["run"]["path"]
        original_run = run_path.read_bytes()
        output = fixture.case.destination / "panels-reviewed-report.json"
        with patch("benchmarks.agent_query.broad_query_assess_v2.SourceReader.git",
                side_effect=lambda root, *args: "a" * 40 + "\n" if "rev-parse" in args else ""):
            stored = score_file_v2(fixture.case.destination / "review-input.json", output)
            checked = check_file_v2(output)
        self.assertEqual(checked, stored["assessment"])
        self.assertEqual(run_path.read_bytes(), original_run)
        self.assertEqual(len(fixture.case.queries), 12)


if __name__ == "__main__": unittest.main()
