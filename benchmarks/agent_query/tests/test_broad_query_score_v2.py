"""Synthetic source-scoring contracts, never real answer qualification."""
import copy
import unittest

from benchmarks.agent_query import broad_query_review as legacy
from benchmarks.agent_query.broad_query_protocol import PrintedSpanBudget, pack_claim_batches
from benchmarks.agent_query.broad_query_score_v2 import score_task_v2, ExpressivenessCollision
from benchmarks.agent_query.tests.test_broad_query_review import task, identity, relation, site
from benchmarks.agent_query.tests.test_protocol2_printed_coverage import fixture


def scoring_fixture(count=1, occurrences=1):
    raw,budget,n,j=fixture(count,occurrences)
    expected=task()
    claims=[c for b in n["claimBatches"] for c in b["claims"]]
    labels=[c for b in j["claimBatches"] for c in b["claims"]]
    for index,(claim,label) in enumerate(zip(claims,labels)):
        claim["identity"]=identity()
        if not index:
            label["factId"]="saves"
        else:
            witness=dict(file="store.py",line=index+10,text=f"synthetic extra assertion{index}")
            label["source"]=witness
            for occurrence in label["printedOccurrences"]: occurrence["source"]=witness
    n.update(disposition="answered" if count else "abstain",rankingKind="retrieval" if count else "unavailable",
        admission=None,ranked=[dict(identity=identity(),nodeId="save",firstPage=True,span=claims[0]["span"])] if count else [],
        normalizationComplete=True)
    j.update(candidates=[],emptyConfirmed=not count)
    return expected,raw,budget,n,j


class Version2ScoringTests(unittest.TestCase):
    def test_malformed_fact_id_rejects_with_typed_error_and_poisoned_budget(self):
        for value in ([], {}, ["saves"], True, 1, 1.0, "", "x"*129):
            expected,raw,budget,n,j=scoring_fixture()
            j["claimBatches"][0]["claims"][0]["factId"]=value
            with self.subTest(value=value), self.assertRaises(ValueError):
                score_task_v2(expected,n,j,None,raw,budget)
            with self.assertRaises(ValueError): budget.ledger()

    def test_shared_source_semantics_and_complete_measured_precision(self):
        expected,raw,budget,n,j=scoring_fixture(129)
        result=score_task_v2(expected,n,j,None,raw,budget)
        self.assertTrue(result["success"])
        self.assertEqual(result["sourceFactRecall"],1)
        self.assertEqual(result["supportedClaims"],129)
        self.assertEqual(result["precisionWithEmptyZero"],1)
        self.assertEqual(result["printedOccurrenceCount"],129)
        self.assertEqual(result["occurrenceVerdicts"]["supported"],129)
        self.assertNotIn("formalDifference",result)

    def test_v1_public_path_keeps128_limit_after_engine_extraction(self):
        expected,_,_,n,j=scoring_fixture(129)
        old_n=dict(n,claims=[c for b in n["claimBatches"] for c in b["claims"]])
        old_j=dict(j,claims=[c for b in j["claimBatches"] for c in b["claims"]])
        with self.assertRaisesRegex(ValueError,"returned claims"):
            legacy.score_task(expected,old_n,old_j)

    def test_wrong_owner_direction_and_actual_occurrence_are_not_recovered(self):
        for change in ("owner","direction","occurrence"):
            expected,raw,budget,n,j=scoring_fixture()
            claim=n["claimBatches"][0]["claims"][0]
            if change == "owner": claim["identity"]=identity(owner="WrongStore")
            else:
                expected["facts"][0]["relation"]=relation();claim["relation"]=relation()
                if change == "direction":
                    claim["relation"]["source"],claim["relation"]["target"]=claim["relation"]["target"],claim["relation"]["source"]
                else: claim["relation"]["occurrences"][0]["line"]=4
            result=score_task_v2(expected,n,j,None,raw,budget)
            with self.subTest(change=change):
                self.assertFalse(result["success"])
                self.assertEqual(result["sourceFactRecall"],0)
                self.assertEqual(result["unsupportedClaims"],1)
                self.assertEqual(result["occurrenceVerdicts"]["unsupported"],1)

    def test_pending_occurrence_keeps_full_precision_unknown_even_with_unsupported_occurrence(self):
        expected,raw,budget,n,j=scoring_fixture(1,2)
        reviews=j["claimBatches"][0]["claims"][0]["printedOccurrences"]
        reviews[0]["verdict"]="unjudged";reviews[1]["verdict"]="unsupported"
        result=score_task_v2(expected,n,j,None,raw,budget)
        self.assertEqual(result["unsupportedClaims"],1)
        self.assertEqual(result["occurrenceVerdicts"]["unjudged"],1)
        self.assertFalse(result["judgmentComplete"])
        self.assertFalse(result["success"])
        self.assertIsNone(result["claimPrecision"])
        self.assertIsNone(result["precisionWithEmptyZero"])
        self.assertEqual(result["precisionView"]["judgedOnlyPrecision"],0)

    def test_incomplete_normalization_and_semantic_coverage_never_score_complete(self):
        for change in ("normalization","coverage","unresolved"):
            expected,raw,budget,n,j=scoring_fixture()
            if change == "normalization": n["normalizationComplete"]=False
            else:
                n["semanticCoverageComplete"]=False;j["semanticCoverageComplete"]=False
                if change == "unresolved": j["lineCoverage"][0]["classification"]="unresolved"
            result=score_task_v2(expected,n,j,None,raw,budget)
            self.assertFalse(result["success"])
            self.assertIsNone(result["precisionWithEmptyZero"])
            self.assertIsNone(result["falseEmpty"])

    def test_extra_assertions_need_each_supported_occurrence_pinned_witness(self):
        for change in ("valid","missing","wrong"):
            expected,raw,budget,n,j=scoring_fixture(2,2)
            witness=j["claimBatches"][0]["claims"][1]["source"]
            sources={"store.py":b"\n"*10+witness["text"].encode()+b"\n"}
            occurrence=j["claimBatches"][0]["claims"][1]["printedOccurrences"][-1]
            if change == "missing": occurrence.pop("source")
            elif change == "wrong": occurrence["source"]=dict(witness,text="invented source witness")
            if change == "valid":
                self.assertTrue(score_task_v2(expected,n,j,None,raw,budget,sources=sources)["success"])
            else:
                with self.subTest(change=change), self.assertRaises(ValueError):
                    score_task_v2(expected,n,j,None,raw,budget,sources=sources)
                with self.assertRaises(ValueError): budget.ledger()

    def test_cross_batch_typed_collisions_cannot_escape_by_id_or_assertion_kind(self):
        expected,raw,budget,n,j=scoring_fixture(129)
        first=n["claimBatches"][0]["claims"][0]
        last=n["claimBatches"][-1]["claims"][-1]
        last["assertionKind"]="different-arbitrary-kind"
        j["claimBatches"][-1]["claims"][-1]["factId"]="saves"
        with self.assertRaises(ExpressivenessCollision) as error: score_task_v2(expected,n,j,None,raw,budget)
        self.assertEqual(error.exception.code,"EXPRESSIVENESS_COLLISION")
        with self.assertRaises(ValueError): budget.ledger()

    def test_distinct_same_owner_extra_assertions_on_same_source_line_reject(self):
        expected,raw,budget,n,j=scoring_fixture(3)
        labels=j["claimBatches"][0]["claims"]
        labels[2]["source"]=labels[1]["source"]
        for occurrence in labels[2]["printedOccurrences"]: occurrence["source"]=labels[1]["source"]
        with self.assertRaises(ExpressivenessCollision): score_task_v2(expected,n,j,None,raw,budget)

    def test_exact_printed_repetitions_keep_occurrence_counts_but_one_semantic_vote(self):
        expected,raw,budget,n,j=scoring_fixture(1,4)
        result=score_task_v2(expected,n,j,None,raw,budget)
        self.assertEqual(result["supportedClaims"],1)
        self.assertEqual(result["occurrenceVerdicts"]["supported"],4)
        self.assertEqual(result["printedOccurrenceCount"],4)

    def test_empty_confirmation_is_explicit_and_distinct_from_unknown(self):
        expected,raw,budget,n,j=scoring_fixture(0)
        expected["expected"]="abstain";expected["facts"]=[];expected["relevant"]=[]
        result=score_task_v2(expected,n,j,None,raw,budget)
        self.assertTrue(result["success"])
        self.assertIsNone(result["claimPrecision"])
        self.assertEqual(result["precisionWithEmptyZero"],0)
        expected,raw,budget,n,j=scoring_fixture(0)
        n["normalizationComplete"]=False;n["semanticCoverageComplete"]=False
        j["semanticCoverageComplete"]=False;j["emptyConfirmed"]=False
        result=score_task_v2(expected,n,j,None,raw,budget)
        self.assertFalse(result["success"])
        self.assertIsNone(result["precisionWithEmptyZero"])
        self.assertIsNone(result["falseEmpty"])


if __name__ == "__main__": unittest.main()
