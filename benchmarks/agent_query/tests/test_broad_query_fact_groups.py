"""Prospective fact alternatives; fixtures establish arithmetic, not source gold."""
from copy import deepcopy
from itertools import combinations
import json
import unittest

from benchmarks.agent_query import broad_query_review as review
from benchmarks.agent_query import broad_query_panels as panels
from benchmarks.agent_query.tests import test_broad_query_review as fixture
from benchmarks.agent_query.tests import test_broad_query_panels as panel_fixture


def grouped_task():
    task = fixture.task()
    task["facts"][0]["essential"] = False
    alternative = fixture.identity("flush", 4)
    task["relevant"].append(dict(identity=alternative, grade=3))
    task["facts"].append(dict(id="alternative", identity=alternative, essential=False,
                              source=fixture.site(4, "    def flush(self):"),
                              statement="Synthetic alternative source witness."))
    task["essentialFactGroups"] = dict(schema=review.FACT_GROUP_SCHEMA,
        groups=[dict(id="persistence", anyOf=[["saves"], ["alternative"]])])
    return task


def grouped_registration():
    registered = fixture.registration()
    registered.update(schema=review.REGISTRATION_SCHEMA_V2, tasks=[grouped_task()])
    return registered


def alternative_answer():
    answer = fixture.response()
    answer["ranked"][0].update(identity=fixture.identity("flush", 4), nodeId="flush")
    answer["claims"][0]["identity"] = fixture.identity("flush", 4)
    judgment = fixture.judgment()
    judgment["claims"][0]["factId"] = "alternative"
    return answer, judgment


class FactGroupScoringTests(unittest.TestCase):
    def test_either_exactly_owned_supported_witness_satisfies_a_group(self):
        for answer, judgment in [(fixture.response(), fixture.judgment()), alternative_answer()]:
            with self.subTest(owner=answer["ranked"][0]["nodeId"]):
                result = review.score_task(grouped_task(), answer, judgment)
                self.assertTrue(result["success"])
                self.assertEqual(result["claimPrecision"], 1)
                self.assertEqual(result["essentialFactGroupRecall"], 1)
                self.assertEqual(result["sourceFactRecall"], .5)
                self.assertEqual(len(result["essentialFactGroups"][0]["recovered"]), 1)

    def test_alternative_does_not_bypass_exact_fact_owner(self):
        answer, judgment = alternative_answer()
        judgment["claims"][0]["factId"] = "saves"
        result = review.score_task(grouped_task(), answer, judgment)
        self.assertFalse(result["success"])
        self.assertEqual(result["unsupportedClaims"], 1)
        self.assertEqual(result["essentialFactGroupRecall"], 0)

    def test_alternative_retains_direction_and_occurrence_requirements(self):
        task = grouped_task()
        fact = task["facts"][1]
        fact["relation"] = dict(source=fact["identity"], target=fixture.identity(),
                                kind="calls", direction="source_to_target",
                                occurrences=[fixture.site()])
        answer, judgment = alternative_answer()
        for relation in [None, dict(fixture.relation()), dict(fact["relation"], occurrences=[])]:
            with self.subTest(relation=relation):
                answer["claims"][0]["relation"] = relation
                self.assertFalse(review.score_task(task, answer, judgment)["success"])
        answer["claims"][0]["relation"] = deepcopy(fact["relation"])
        self.assertTrue(review.score_task(task, answer, judgment)["success"])

    def test_flat_essential_facts_remain_required_alongside_every_group(self):
        task = grouped_task()
        required = deepcopy(task["facts"][0])
        required.update(id="also-required", essential=True)
        task["facts"].append(required)
        answer, judgment = alternative_answer()
        self.assertFalse(review.score_task(task, answer, judgment)["success"])
        answer["claims"].append(dict(id="required", identity=fixture.identity()))
        judgment["claims"].append(dict(id="required", factId="also-required", verdict="supported",
                                       reason="Separate synthetic requirement."))
        self.assertTrue(review.score_task(task, answer, judgment)["success"])

    def test_all_groups_required_and_no_fact_reuse(self):
        task = grouped_task()
        for name, line in [("third", 12), ("fourth", 16)]:
            identity = fixture.identity(name, line)
            task["relevant"].append(dict(identity=identity, grade=3))
            task["facts"].append(dict(id=name, identity=identity, essential=False,
                                       source=fixture.site(line), statement="Another synthetic fact."))
        task["essentialFactGroups"]["groups"].append(dict(id="second", anyOf=[["third"], ["fourth"]]))
        result = review.score_task(task, fixture.response(), fixture.judgment())
        self.assertFalse(result["success"])
        self.assertEqual(result["essentialFactGroupRecall"], .5)
        task["essentialFactGroups"]["groups"][1]["anyOf"][0] = ["saves"]
        with self.assertRaisesRegex(ValueError, "multiple essential"):
            review.score_task(task, fixture.response(), fixture.judgment())

    def test_unjudged_unsupported_and_limit_answers_cannot_earn_group_success(self):
        for verdict in ["unjudged", "unsupported"]:
            judgment = fixture.judgment()
            judgment["claims"][0]["verdict"] = verdict
            self.assertFalse(review.score_task(grouped_task(), fixture.response(), judgment)["success"])
        answer = fixture.response()
        answer["disposition"] = "limit"
        self.assertFalse(review.score_task(grouped_task(), answer, fixture.judgment())["success"])

    def test_partial_alternatives_cannot_be_combined_into_a_complete_path(self):
        task = grouped_task()
        for name, line in [("third", 12), ("fourth", 16)]:
            identity = fixture.identity(name, line)
            task["relevant"].append(dict(identity=identity, grade=3))
            task["facts"].append(dict(id=name, identity=identity, essential=False,
                                       source=fixture.site(line), statement="Synthetic path hop."))
        task["essentialFactGroups"]["groups"][0]["anyOf"] = [["saves", "third"], ["alternative", "fourth"]]
        answer, judgment = fixture.response(), fixture.judgment()
        answer["claims"].append(dict(id="fourth", identity=fixture.identity("fourth", 16)))
        judgment["claims"].append(dict(id="fourth", factId="fourth", verdict="supported", reason="Second path's final hop."))
        mixed = review.score_task(task, answer, judgment)
        self.assertEqual(mixed["claimPrecision"], 1)
        self.assertFalse(mixed["success"])
        self.assertEqual(mixed["essentialFactGroups"][0]["satisfiedAlternativeIndexes"], [])
        answer["claims"].append(dict(id="third", identity=fixture.identity("third", 12)))
        judgment["claims"].append(dict(id="third", factId="third", verdict="supported", reason="Complete the first path."))
        complete = review.score_task(task, answer, judgment)
        self.assertTrue(complete["success"])
        self.assertEqual(complete["essentialFactGroups"][0]["satisfiedAlternativeIndexes"], [0])

    def test_shared_prefix_is_required_and_all_completed_alternatives_reported(self):
        task = grouped_task()
        third = fixture.identity("third", 12)
        task["relevant"].append(dict(identity=third, grade=3))
        task["facts"].append(dict(id="third", identity=third, essential=False,
                                   source=fixture.site(12), statement="Synthetic alternate final hop."))
        task["essentialFactGroups"]["groups"][0]["anyOf"] = [["saves", "alternative"], ["saves", "third"]]
        answer, judgment = alternative_answer()
        self.assertFalse(review.score_task(task, answer, judgment)["success"])
        answer["claims"].append(dict(id="prefix", identity=fixture.identity()))
        judgment["claims"].append(dict(id="prefix", factId="saves", verdict="supported", reason="Shared prefix is printed."))
        self.assertTrue(review.score_task(task, answer, judgment)["success"])
        answer["claims"].append(dict(id="third", identity=third))
        judgment["claims"].append(dict(id="third", factId="third", verdict="supported", reason="Other final hop is printed."))
        result = review.score_task(task, answer, judgment)
        self.assertEqual(result["essentialFactGroups"][0]["satisfiedAlternativeIndexes"], [0, 1])
        self.assertEqual(result["essentialFactGroups"][0]["recovered"], ["alternative", "saves", "third"])

    def test_satisfied_alternative_does_not_excuse_an_unsupported_extra_claim(self):
        answer, judgment = alternative_answer()
        answer["claims"].append(dict(id="extra", identity=fixture.identity()))
        judgment["claims"].append(dict(id="extra", verdict="unsupported", reason="Synthetic false extra assertion."))
        result = review.score_task(grouped_task(), answer, judgment)
        self.assertEqual(result["essentialFactGroupRecall"], 1)
        self.assertFalse(result["success"])

    def test_graph_ledger_and_legacy_result_are_not_replaced_by_groups(self):
        result = review.score_task(grouped_task(), fixture.response(), fixture.judgment(),
                                   review.GraphIndex(fixture.graph(), "compass"))
        self.assertEqual(len(result["ledger"]), 2)
        self.assertEqual([row["graph"] for row in result["ledger"]], ["present", "present"])
        self.assertNotIn("essentialFactGroups", review.score_task(fixture.task(), fixture.response(), fixture.judgment()))


class FactGroupRegistrationTests(unittest.TestCase):
    def test_v2_opt_in_and_v1_no_silent_reinterpretation(self):
        registered = grouped_registration()
        self.assertEqual(review.validate_registration(registered), registered)
        registered["schema"] = review.REGISTRATION_SCHEMA
        with self.assertRaisesRegex(ValueError, "version 2"):
            review.validate_registration(registered)

    def test_unknown_versions_and_nonbroad_tasks_fail(self):
        registered = grouped_registration()
        registered["schema"] = "compass.broad-query-registration/99"
        with self.assertRaisesRegex(ValueError, "schema"):
            review.validate_registration(registered)
        for kind in ["exact", "directed", "multihop", "ambiguous", "unsupported"]:
            registered = grouped_registration()
            registered["tasks"][0]["kind"] = kind
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, "broad answerable"):
                review.validate_registration(registered)

    def test_group_contract_rejects_ambiguous_or_unbounded_membership(self):
        mutations = [
            lambda t: t["essentialFactGroups"].update(schema="unknown/99"),
            lambda t: t["essentialFactGroups"].update(groups=[]),
            lambda t: t["essentialFactGroups"].update(groups=t["essentialFactGroups"]["groups"] * 65),
            lambda t: t["essentialFactGroups"].update(extra=True),
            lambda t: t["essentialFactGroups"]["groups"][0].update(anyOf=[["saves"]]),
            lambda t: t["essentialFactGroups"]["groups"][0].update(anyOf=[[], ["alternative"]]),
            lambda t: t["essentialFactGroups"]["groups"][0].update(anyOf=[["saves", "saves"], ["alternative"]]),
            lambda t: t["essentialFactGroups"]["groups"][0].update(anyOf=[["saves"], ["saves"]]),
            lambda t: t["essentialFactGroups"]["groups"][0].update(anyOf=[["saves"], ["saves", "alternative"]]),
            lambda t: t["essentialFactGroups"]["groups"][0].update(anyOf=[["saves"], ["unknown"]]),
            lambda t: t["essentialFactGroups"]["groups"][0].update(id="saves"),
            lambda t: t["essentialFactGroups"]["groups"].append(deepcopy(t["essentialFactGroups"]["groups"][0])),
            lambda t: t["facts"][0].update(essential=True),
            lambda t: t["facts"][1].update(identity=fixture.identity()),
            lambda t: t["facts"][1]["source"].update(file="unregistered.py"),
        ]
        for number, mutate in enumerate(mutations):
            with self.subTest(case=number):
                registered = grouped_registration()
                mutate(registered["tasks"][0])
                with self.assertRaises(ValueError):
                    review.validate_registration(registered)

    def test_aggregate_alternative_references_are_bounded(self):
        registered = grouped_registration()
        task = registered["tasks"][0]
        task.update(relevant=[], facts=[])
        for number in range(10):
            identity = fixture.identity("f" + str(number), number + 1)
            task["relevant"].append(dict(identity=identity, grade=3))
            task["facts"].append(dict(id="f" + str(number), identity=identity, essential=False,
                                       source=fixture.site(number + 1), statement="Synthetic witness."))
        task["essentialFactGroups"]["groups"][0]["anyOf"] = [list(members) for members in
            list(combinations([fact["id"] for fact in task["facts"]], 5))[:52]]
        with self.assertRaisesRegex(ValueError, "alternative fact references"):
            review.validate_registration(registered)


class FactGroupPanelTests(unittest.TestCase):
    def setUp(self):
        self.panel = panel_fixture.PanelBindingTests("test_complete_synthetic_panels_remain_ineligible_and_deduplicate_by_observation_key")
        self.panel.setUp()
        self.addCleanup(self.panel.doCleanups)

    def test_shared_groups_and_registration_versions_are_bound(self):
        registered = deepcopy(self.panel.registered)
        for reg in registered.values():
            reg["schema"] = review.REGISTRATION_SCHEMA_V2
            reg["tasks"][0] = grouped_task()
        panels.validate_panels(self.panel.definition, registered)
        registered["lookup"]["tasks"][0]["essentialFactGroups"]["groups"][0]["id"] = "changed"
        with self.assertRaisesRegex(ValueError, "shared task"):
            panels.validate_panels(self.panel.definition, registered)
        registered["lookup"]["tasks"][0] = fixture.task()
        registered["lookup"]["schema"] = review.REGISTRATION_SCHEMA
        with self.assertRaisesRegex(ValueError, "registration schemas"):
            panels.validate_panels(self.panel.definition, registered)


class FactGroupBundleTests(unittest.TestCase):
    def setUp(self):
        self.bundle = fixture.BundleTests("test_complete_diagnostic_remains_ineligible_without_independent_heldout_evidence")
        self.bundle.setUp()
        self.addCleanup(self.bundle.doCleanups)

    def read(self, ref):
        return json.loads((self.bundle.root / ref["path"]).read_bytes())

    def grouped_bundle(self):
        manifest = self.bundle.bundle()
        registered = self.read(manifest["registration"])
        registered.update(schema=review.REGISTRATION_SCHEMA_V2, tasks=[grouped_task()])
        manifest["registration"] = self.bundle.put("registration.json", registered)
        run = self.read(manifest["run"])
        run["registrationSha256"] = manifest["registration"]["sha256"]
        manifest["run"] = self.bundle.put("run.json", run)
        judged = self.read(manifest["reviews"])
        judged["runSha256"] = manifest["run"]["sha256"]
        manifest["reviews"] = self.bundle.put("reviews.json", judged)
        return manifest

    def test_file_backed_both_tools_use_groups_without_earning_qualification(self):
        result = review.assess(self.grouped_bundle(), self.bundle.root)
        self.assertFalse(result["eligibleForSuperiorityClaim"])
        self.assertEqual(len(result["rows"]), 4)
        self.assertTrue(all(row["success"] and row["essentialFactGroupRecall"] == 1
                            for row in result["rows"]))
        self.assertTrue(all(len(row["ledger"]) == 2 for row in result["rows"]))

    def test_changed_group_payload_invalidates_captured_registration_binding(self):
        manifest = self.grouped_bundle()
        registered = self.read(manifest["registration"])
        registered["tasks"][0]["essentialFactGroups"]["groups"][0]["id"] = "after-capture"
        manifest["registration"] = self.bundle.put("registration.json", registered)
        with self.assertRaisesRegex(ValueError, "run is not bound"):
            review.assess(manifest, self.bundle.root)


if __name__ == "__main__":
    unittest.main()
