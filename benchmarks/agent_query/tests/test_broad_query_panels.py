"""Synthetic panel integrity cases; no product or independent review is run."""
import copy
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_review as review
from benchmarks.agent_query import broad_query_panels as panels
from benchmarks.agent_query.broad_query_panels import (
    DEFINITION_SCHEMA, INPUT_SCHEMA, assess_panels, main, project_capture, validate_panels,
)
from benchmarks.agent_query.tests import test_broad_query_review as fixtures


class PanelBindingTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.BundleTests("test_complete_diagnostic_remains_ineligible_without_independent_heldout_evidence")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.inputs = {}
        self.registered = {}
        self.base = self.fixture.bundle()
        self.manifest = self.make_panels()

    def read(self, ref):
        return json.loads((self.root / ref["path"]).read_bytes())

    def report(self, role):
        source = self.inputs[role]
        input_ref = self.fixture.put(role + "-input.json", source)
        assessment = review.assess(source, self.root)
        report = dict(schema=review.REPORT_SCHEMA, inputPath=str(self.root / input_ref["path"]),
                      inputSha256=input_ref["sha256"],
                      assessorSha256=review.sha(Path(review.__file__).read_bytes()), assessment=assessment)
        return self.fixture.put(role + "-report.json", report)

    def make_panels(self):
        base_registration = self.read(self.base["registration"])
        for role in ("original", "lookup"):
            reg = copy.deepcopy(base_registration)
            named = fixtures.task("exact")
            named["id"] = "named"
            named["question"] = "How does Store.save delegate persistence?"
            if role == "lookup":
                named["id"] = "named.lookup"
                named["question"] = "Locate the declaration of Store.save in store.py."
                named["facts"] = [dict(id="declaration", essential=True,
                    identity=copy.deepcopy(named["relevant"][0]["identity"]),
                    statement="Declaration identity at the frozen source anchor.",
                    source=dict(file="store.py", line=2, text="    def save(self):"))]
            reg["tasks"].append(named)
            self.registered[role] = reg
            reg_ref = self.fixture.put(role + "-registration.json", reg)
            run = self.read(self.base["run"])
            reviews = self.read(self.base["reviews"])
            for obs, judged in zip(copy.deepcopy(run["observations"]), copy.deepcopy(reviews["observations"])):
                obs["task"] = judged["task"] = named["id"]
                normalized = self.read(obs["normalized"])
                normalized["task"] = named["id"]
                prefix = role + "-" + obs["tool"] + "-" + obs["arm"]
                obs["normalized"] = self.fixture.put(prefix + "-named.json", normalized)
                judged["normalizedSha256"] = obs["normalized"]["sha256"]
                judged["claims"][0]["factId"] = named["facts"][0]["id"]
                run["observations"].append(obs)
                reviews["observations"].append(judged)
            run["registrationSha256"] = reg_ref["sha256"]
            run_ref = self.fixture.put(role + "-run.json", run)
            reviews["runSha256"] = run_ref["sha256"]
            source = copy.deepcopy(self.base)
            source.update(registration=reg_ref, run=run_ref,
                          reviews=self.fixture.put(role + "-reviews.json", reviews))
            self.inputs[role] = source
        definition = dict(schema=DEFINITION_SCHEMA, frozenAt="2026-01-01T00:00:01Z",
                          assessorSha256=dict(panel=review.sha(Path(panels.__file__).read_bytes()),
                                              canonical=review.sha(Path(review.__file__).read_bytes())),
                          registrations={role: self.inputs[role]["registration"] for role in self.inputs},
                          lookupPairs=[dict(original="named", lookup="named.lookup")])
        self.definition = definition
        return dict(schema=INPUT_SCHEMA, definition=self.fixture.put("definition.json", definition),
                    reports={role: self.report(role) for role in self.inputs})

    def refresh_run(self, role, run):
        source = self.inputs[role]
        source["run"] = self.fixture.put(role + "-run.json", run)
        reviews = self.read(source["reviews"])
        reviews["runSha256"] = source["run"]["sha256"]
        source["reviews"] = self.fixture.put(role + "-reviews.json", reviews)
        self.manifest["reports"][role] = self.report(role)

    def test_complete_synthetic_panels_remain_ineligible_and_deduplicate_by_observation_key(self):
        result = assess_panels(self.manifest, self.root)
        self.assertFalse(result["eligibleForSuperiorityClaim"])
        self.assertEqual(result["distinctQuestions"], 3)
        self.assertEqual(result["sharedQuestions"], 1)
        self.assertEqual(result["distinctObservations"], 12)
        # Named and broad fixtures intentionally share capture bytes. Different
        # task keys still incur their own cost; only the common task is reused.
        self.assertEqual(sum(row["workflowWallMs"] for row in result["costs"]), 240)
        self.assertEqual(sum(row["workflowWallMs"] for row in result["buildCosts"]), 20)
        self.assertEqual(len(result["buildCosts"]), 2)
        self.assertTrue(any(f.startswith("original:") for f in result["failures"]))
        self.assertTrue(any(f.startswith("lookup:") for f in result["failures"]))

    def test_a_passing_lookup_panel_cannot_hide_a_failing_original_panel(self):
        def synthetic_gate(report):
            result = copy.deepcopy(report["assessment"])
            if Path(report["inputPath"]).name.startswith("lookup-"):
                result.update(eligibleForSuperiorityClaim=True, failures=[])
            return result
        with patch("benchmarks.agent_query.broad_query_panels.recompute_report", side_effect=synthetic_gate):
            result = assess_panels(self.manifest, self.root)
        self.assertTrue(result["panelEligibility"]["lookup"])
        self.assertFalse(result["eligibleForSuperiorityClaim"])

    def test_both_synthetic_gates_are_required_without_an_always_false_shortcut(self):
        def synthetic_gate(report):
            result = copy.deepcopy(report["assessment"])
            result.update(eligibleForSuperiorityClaim=True, failures=[])
            return result
        with patch("benchmarks.agent_query.broad_query_panels.recompute_report", side_effect=synthetic_gate):
            self.assertTrue(assess_panels(self.manifest, self.root)["eligibleForSuperiorityClaim"])

    def test_unknown_definition_version_fails_before_panel_reads(self):
        self.definition["schema"] = "compass.broad-query-panels/99"
        self.manifest["definition"] = self.fixture.put("definition.json", self.definition)
        with self.assertRaisesRegex(ValueError, "unsupported comparison panel definition"):
            assess_panels(self.manifest, self.root)

    def test_frozen_assessor_digest_is_required(self):
        self.definition["assessorSha256"]["panel"] = "0" * 64
        self.manifest["definition"] = self.fixture.put("definition.json", self.definition)
        with self.assertRaisesRegex(ValueError, "differ from the frozen"):
            assess_panels(self.manifest, self.root)

    def test_input_manifest_change_during_aggregate_is_rejected(self):
        original = panels.recompute_report
        def mutate_after_original_check(report):
            result = original(report)
            if Path(report["inputPath"]).name.startswith("lookup-"):
                path = self.root / "original-input.json"
                data = json.loads(path.read_bytes())
                data["changedDuringAssessment"] = True
                path.write_text(json.dumps(data))
            return result
        with patch("benchmarks.agent_query.broad_query_panels.recompute_report", side_effect=mutate_after_original_check):
            with self.assertRaisesRegex(ValueError, "input changed during aggregate"):
                assess_panels(self.manifest, self.root)

    def test_missing_panel_is_rejected(self):
        del self.manifest["reports"]["original"]
        with self.assertRaisesRegex(ValueError, "exactly original and lookup"):
            assess_panels(self.manifest, self.root)

    def test_stored_eligible_boolean_is_recomputed(self):
        report = self.read(self.manifest["reports"]["lookup"])
        report["assessment"]["eligibleForSuperiorityClaim"] = True
        self.manifest["reports"]["lookup"] = self.fixture.put("lookup-report.json", report)
        with self.assertRaisesRegex(ValueError, "fabricated or are stale"):
            assess_panels(self.manifest, self.root)

    def test_shared_measurement_must_be_identical(self):
        run = self.read(self.inputs["lookup"]["run"])
        run["observations"][0]["peakRssBytes"] += 1
        self.refresh_run("lookup", run)
        with self.assertRaisesRegex(ValueError, "not reused identically"):
            assess_panels(self.manifest, self.root)

    def test_different_graph_bytes_are_rejected(self):
        run = self.read(self.inputs["lookup"]["run"])
        graph = self.read(run["graphs"][0]["artifact"])
        graph["fixtureMetadata"] = "different graph image"
        run["graphs"][0]["artifact"] = self.fixture.put("different-graph.json", graph)
        self.refresh_run("lookup", run)
        with self.assertRaisesRegex(ValueError, "different captured graphs"):
            assess_panels(self.manifest, self.root)

    def test_capture_cannot_precede_panel_freeze(self):
        self.definition["frozenAt"] = "2026-01-01T00:00:03Z"
        self.manifest["definition"] = self.fixture.put("definition.json", self.definition)
        with self.assertRaisesRegex(ValueError, "not frozen before"):
            assess_panels(self.manifest, self.root)

    def test_shared_broad_question_cannot_change(self):
        changed = copy.deepcopy(self.registered)
        changed["lookup"]["tasks"][0]["question"] = "an easier question"
        with self.assertRaisesRegex(ValueError, "shared task"):
            validate_panels(self.definition, changed)

    def test_shared_task_split_and_mechanism_lineage_cannot_change(self):
        for field, value in (("split", "confirmation"), ("mechanisms", ["new/mechanism"])):
            changed = copy.deepcopy(self.registered)
            changed["lookup"]["tasks"][0][field] = value
            with self.assertRaisesRegex(ValueError, "shared task"):
                validate_panels(self.definition, changed)

    def test_lookup_rewrite_cannot_discard_mechanism_lineage(self):
        changed = copy.deepcopy(self.registered)
        changed["original"]["tasks"][-1].update(split="development", mechanisms=["store/persistence"])
        with self.assertRaisesRegex(ValueError, "lookup variants changed"):
            validate_panels(self.definition, changed)

    def test_frozen_capture_plans_cannot_differ(self):
        changed = copy.deepcopy(self.registered)
        changed["original"]["capturePlan"] = {"fixture": "first"}
        changed["lookup"]["capturePlan"] = {"fixture": "other"}
        with self.assertRaisesRegex(ValueError, "different frozen capturePlan"):
            validate_panels(self.definition, changed)

    def test_lookup_variant_cannot_create_an_independent_cluster(self):
        changed = copy.deepcopy(self.registered)
        changed["lookup"]["tasks"][1]["cluster"] = "invented-independent-unit"
        with self.assertRaisesRegex(ValueError, "source group"):
            validate_panels(self.definition, changed)

    def test_lookup_fact_must_still_be_the_declaration_identity(self):
        changed = copy.deepcopy(self.registered)
        changed["lookup"]["tasks"][1]["facts"][0]["source"]["line"] = 3
        with self.assertRaisesRegex(ValueError, "declaration identity"):
            validate_panels(self.definition, changed)

    def test_duplicate_lookup_mapping_is_rejected(self):
        changed = copy.deepcopy(self.definition)
        changed["lookupPairs"] *= 2
        with self.assertRaisesRegex(ValueError, "distinct, unique"):
            validate_panels(changed, self.registered)

    def test_nonobject_lookup_pair_is_a_typed_validation_error(self):
        changed = copy.deepcopy(self.definition)
        changed["lookupPairs"] = [7]
        with self.assertRaisesRegex(ValueError, "lookup task pair"):
            validate_panels(changed, self.registered)

    def test_cli_diagnostic_checks_false_and_rejects_forged_aggregate(self):
        source = self.fixture.put("panels-input.json", self.manifest)
        output = self.root / "panels-report.json"
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            self.assertEqual(main(["--score", str(self.root / source["path"]), "--output", str(output)]), 0)
            self.assertEqual(main(["--check", str(output)]), 1)
            report = json.loads(output.read_bytes())
            report["assessment"]["eligibleForSuperiorityClaim"] = True
            output.write_text(json.dumps(report))
            self.assertEqual(main(["--check", str(output)]), 2)

    def union(self):
        original = self.inputs["original"]
        lookup = self.inputs["lookup"]
        registration = copy.deepcopy(self.registered["original"])
        registration["tasks"].append(copy.deepcopy(self.registered["lookup"]["tasks"][1]))
        ref = self.fixture.put("union-registration.json", registration)
        run = self.read(original["run"])
        extra = [x for x in self.read(lookup["run"])["observations"] if x["task"] == "named.lookup"]
        run["observations"].extend(extra)
        run["registrationSha256"] = ref["sha256"]
        run_ref = self.fixture.put("union-run.json", run)
        reviews = self.read(original["reviews"])
        reviews["observations"].extend(x for x in self.read(lookup["reviews"])["observations"] if x["task"] == "named.lookup")
        reviews["runSha256"] = run_ref["sha256"]
        source = copy.deepcopy(original)
        source.update(registration=ref, run=run_ref, reviews=self.fixture.put("union-reviews.json", reviews))
        return source

    def test_union_projection_reuses_captures_and_produces_canonical_checkable_inputs(self):
        receipt = project_capture(self.union(), self.manifest["definition"], self.root)
        reports = {}
        for role, ref in receipt["inputs"].items():
            source = self.read(ref)
            self.inputs[role] = source
            reports[role] = self.report(role)
        manifest = copy.deepcopy(self.manifest)
        manifest["reports"] = reports
        result = assess_panels(manifest, self.root)
        self.assertEqual(result["distinctObservations"], 12)
        self.assertFalse(result["eligibleForSuperiorityClaim"])
        self.assertFalse(receipt["qualificationEstablished"])
        with self.assertRaisesRegex(ValueError, "already exist"):
            project_capture(self.union(), self.manifest["definition"], self.root)

    def test_union_projection_rejects_a_dropped_observation_before_publication(self):
        source = self.union()
        run = self.read(source["run"])
        run["observations"].pop()
        source["run"] = self.fixture.put("union-run.json", run)
        reviews = self.read(source["reviews"])
        reviews["runSha256"] = source["run"]["sha256"]
        source["reviews"] = self.fixture.put("union-reviews.json", reviews)
        with self.assertRaisesRegex(ValueError, "matrix is incomplete"):
            project_capture(source, self.manifest["definition"], self.root)
        self.assertFalse((self.root / "panels-projection.json").exists())

    def test_cli_projection_uses_the_original_artifact_root(self):
        source = self.fixture.put("union-input.json", self.union())
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            self.assertEqual(main(["--project", str(self.root / source["path"]),
                                   "--definition", str(self.root / self.manifest["definition"]["path"])]), 0)
        receipt = json.loads((self.root / "panels-projection.json").read_bytes())
        self.assertEqual(set(receipt["inputs"]), {"original", "lookup"})
        self.assertTrue((self.root / "panels-original-input.json").exists())
        self.assertFalse(receipt["qualificationEstablished"])


if __name__ == "__main__":
    unittest.main()
