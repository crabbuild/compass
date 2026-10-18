"""Synthetic partition checks; no claim of source truth or real quality."""
import copy
import hashlib
import unittest

from benchmarks.agent_query.broad_query_protocol import PrintedSpanBudget, pack_claim_batches, ProtocolLimit
from benchmarks.agent_query.broad_query_coverage import validate_printed_partition, NORMALIZED_SCHEMA, JUDGMENTS_SCHEMA


def fixture(claim_count=2, occurrences_per_claim=1):
    raw = b"p\n" * (claim_count * occurrences_per_claim)
    budget = PrintedSpanBudget(raw)
    claims, labels, ledger, lines = [], [], budget.ledger(), []
    for index in range(claim_count):
        claim_id = f"c-{index:04}"
        occurrences, reviews = [], []
        for number in range(occurrences_per_claim):
            line = index * occurrences_per_claim + number
            occurrence_id = f"o-{line:05}"
            occurrences.append(dict(id=occurrence_id, span=ledger[line]["span"]))
            reviews.append(dict(id=occurrence_id, verdict="supported", reason="Synthetic occurrence label."))
            ledger[line]["claimIds"] = [claim_id]
            lines.append(dict(index=line, classification="assertions", claimIds=[claim_id], reason="Synthetic printed assertion."))
        claims.append(dict(id=claim_id, span=occurrences[0]["span"], printedOccurrences=occurrences))
        labels.append(dict(id=claim_id, verdict="supported", reason="Synthetic claim label.", printedOccurrences=reviews))
    normalized = dict(schema=NORMALIZED_SCHEMA, semanticCoverageComplete=True, lineLedger=ledger,
                      **pack_claim_batches(claims))
    judgment = dict(schema=JUDGMENTS_SCHEMA, semanticCoverageComplete=True, lineCoverage=lines,
                    **pack_claim_batches(labels))
    return raw, budget, normalized, judgment


class PrintedCoverageTests(unittest.TestCase):
    def test_all_batches_and_occurrences_keep_one_global_partition(self):
        for count in (0,1,127,128,129,198,2048):
            with self.subTest(count=count):
                _, budget, normalized, judgment = fixture(count)
                result = validate_printed_partition(normalized,judgment,budget)
                self.assertEqual(len(result["effectiveClaimVerdicts"]),count)
                self.assertEqual(result["printedOccurrenceCount"],count)
                self.assertTrue(result["semanticCoverageComplete"])

    def test_late_unsupported_or_unjudged_occurrence_cannot_hide_in_supported_claim(self):
        for verdict in ("unsupported", "unjudged"):
            _, budget, normalized, judgment = fixture(129,2)
            judgment["claimBatches"][-1]["claims"][-1]["printedOccurrences"][-1]["verdict"] = verdict
            result = validate_printed_partition(normalized,judgment,budget)
            self.assertEqual(result["effectiveClaimVerdicts"][-1]["verdict"],verdict)
            self.assertEqual(result["occurrenceVerdicts"][verdict],1)
            self.assertEqual(len(result["effectiveClaimVerdicts"]),129)

    def test_occurrence_precedence_preserves_pending_even_with_unsupported_claim(self):
        for claim_verdict in ("supported","unsupported","unjudged"):
            for occurrence_verdict in ("supported","unsupported","unjudged"):
                _, budget, n, j = fixture(1)
                label = j["claimBatches"][0]["claims"][0]
                label["verdict"] = claim_verdict
                label["printedOccurrences"][0]["verdict"] = occurrence_verdict
                result = validate_printed_partition(n,j,budget)
                expected = occurrence_verdict if occurrence_verdict != "supported" else claim_verdict
                with self.subTest(claim=claim_verdict, occurrence=occurrence_verdict):
                    self.assertEqual(result["effectiveClaimVerdicts"][0]["verdict"],expected)
                    self.assertEqual(result["occurrenceVerdicts"][occurrence_verdict],1)
        _, budget, n, j = fixture(1,2)
        labels=j["claimBatches"][0]["claims"][0]["printedOccurrences"]
        labels[0]["verdict"]="unjudged";labels[1]["verdict"]="unsupported"
        result=validate_printed_partition(n,j,budget)
        self.assertEqual(result["effectiveClaimVerdicts"][0]["verdict"],"unsupported")
        self.assertEqual(result["occurrenceVerdicts"]["unjudged"],1)

    def test_multiline_and_same_line_subspans_charge_each_occurrence_link(self):
        raw=b"alpha beta\n gamma\n";budget=PrintedSpanBudget(raw)
        _, _, n, j=fixture(1,2)
        def span(start,end):
            return dict(startByte=start,endByte=end,sha256=hashlib.sha256(raw[start:end]).hexdigest())
        claim=n["claimBatches"][0]["claims"][0]
        claim["span"]=span(0,len(raw)-1)
        claim["printedOccurrences"][0]["span"]=claim["span"]
        claim["printedOccurrences"][1]["span"]=span(6,10)
        n["lineLedger"]=[dict(line,claimIds=[claim["id"]]) for line in budget.ledger()]
        j["lineCoverage"]=[dict(index=i,classification="assertions",claimIds=[claim["id"]],reason="Synthetic multiline output.") for i in range(2)]
        result=validate_printed_partition(n,j,budget)
        self.assertEqual(result["printedOccurrenceCount"],2)
        self.assertEqual(budget.line_links,3)
        self.assertEqual(budget.bundle.line_links,3)

    def test_nonassertion_byte_line_conventions_do_not_infer_assertions(self):
        raw=" \t\r\nheader\r\n\t\nline\u2028tail\r".encode()
        budget=PrintedSpanBudget(raw)
        _, _, n, j=fixture(0)
        n["lineLedger"]=[dict(line,claimIds=[]) for line in budget.ledger()]
        j["lineCoverage"]=[dict(index=i,classification="non_assertion",claimIds=[],reason="Synthetic nonassertion review.") for i in range(len(n["lineLedger"]))]
        result=validate_printed_partition(n,j,budget)
        self.assertEqual([line["physicalLine"] for line in n["lineLedger"]],[2,4])
        last=n["lineLedger"][-1]["span"]
        self.assertTrue(raw[last["startByte"]:last["endByte"]].endswith(b"\r"))
        self.assertEqual(result["printedOccurrenceCount"],0)
        self.assertTrue(result["semanticCoverageComplete"])

    def test_late_partition_failure_poisoning_retains_completed_work_in_both_phases(self):
        for phase in ("initial","reviewed"):
            _, budget, n, j=fixture(2)
            j["lineCoverage"][-1]["claimIds"]=[]
            with self.assertRaises(ValueError): validate_printed_partition(n,j,budget,phase=phase)
            self.assertEqual(budget.line_links,2)
            self.assertEqual(budget.bundle.line_links,2)
            completed=budget.bundle.hash_bytes
            _, _, good, labels=fixture(2)
            for retry_phase in ("initial","reviewed"):
                with self.assertRaises(ValueError): validate_printed_partition(good,labels,budget,phase=retry_phase)
            self.assertEqual(budget.bundle.hash_bytes,completed)
            self.assertEqual(budget.bundle.line_links,2)

    def test_every_occurrence_needs_a_label_and_global_unique_id(self):
        for mutation in ("missing", "reordered", "duplicate-id", "alternate-span", "duplicate-span"):
            _, budget, n, j = fixture(2,2)
            occurrences = n["claimBatches"][0]["claims"][0]["printedOccurrences"]
            reviews = j["claimBatches"][0]["claims"][0]["printedOccurrences"]
            if mutation == "missing": reviews.pop()
            elif mutation == "reordered": reviews.reverse()
            elif mutation == "duplicate-id": occurrences[1]["id"] = occurrences[0]["id"]
            elif mutation == "alternate-span": reviews[0]["span"] = occurrences[0]["span"]
            else: occurrences[1]["span"] = occurrences[0]["span"]
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate_printed_partition(n,j,budget)
            with self.assertRaises(ValueError): budget.ledger()

    def test_canonical_order_and_first_span_are_required(self):
        for mutation in ("claim-order", "occurrence-order", "claim-span", "bool-span", "ledger-bool"):
            _, budget, n, j = fixture(2,2)
            claims=n["claimBatches"][0]["claims"]
            if mutation == "claim-order":
                claims.reverse();j["claimBatches"][0]["claims"].reverse()
            elif mutation == "occurrence-order":
                claims[0]["printedOccurrences"].reverse()
                j["claimBatches"][0]["claims"][0]["printedOccurrences"].reverse()
                claims[0]["span"]=claims[0]["printedOccurrences"][0]["span"]
            elif mutation == "claim-span": claims[0]["span"]=claims[1]["span"]
            elif mutation == "bool-span":
                claims[0]["span"]=dict(claims[0]["span"],startByte=False)
            else:
                n["lineLedger"][0]["span"]=dict(n["lineLedger"][0]["span"],startByte=False)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate_printed_partition(n,j,budget)

    def test_two_way_lines_reject_missing_or_invented_links(self):
        for mutation in ("missing-ledger", "missing-review", "normalized-links", "reviewed-links", "nonassertion", "index-bool", "span"):
            _, budget, n, j = fixture()
            if mutation == "missing-ledger": n["lineLedger"].pop()
            elif mutation == "missing-review": j["lineCoverage"].pop()
            elif mutation == "normalized-links": n["lineLedger"][0]["claimIds"]=[]
            elif mutation == "reviewed-links": j["lineCoverage"][0]["claimIds"]=["c-0001"]
            elif mutation == "nonassertion": j["lineCoverage"][0]["classification"]="non_assertion"
            elif mutation == "index-bool": j["lineCoverage"][0]["index"]=False
            else: n["lineLedger"][0]["span"]=n["lineLedger"][1]["span"]
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate_printed_partition(n,j,budget)

    def test_unresolved_or_unattested_never_becomes_semantically_complete(self):
        _, budget, n, j = fixture()
        n["semanticCoverageComplete"]=False;j["semanticCoverageComplete"]=False
        j["lineCoverage"][0]["classification"]="unresolved"
        result=validate_printed_partition(n,j,budget)
        self.assertFalse(result["semanticCoverageComplete"])
        self.assertEqual(result["unresolvedLineCount"],1)
        _, budget, n, j = fixture()
        j["lineCoverage"][0]["classification"]="unresolved"
        with self.assertRaises(ValueError): validate_printed_partition(n,j,budget)
        _, budget, n, j = fixture()
        j["semanticCoverageComplete"]=False
        self.assertFalse(validate_printed_partition(n,j,budget)["semanticCoverageComplete"])

    def test_shared_printed_line_does_not_prove_equivalent_assertions(self):
        _, _, n, j = fixture()
        raw=b"p\n";budget=PrintedSpanBudget(raw)
        claims=n["claimBatches"][0]["claims"]
        claims[1]["span"]=claims[0]["span"]
        claims[1]["printedOccurrences"][0]["span"]=claims[0]["span"]
        n["lineLedger"]=budget.ledger()
        n["lineLedger"][0]["claimIds"]=["c-0000","c-0001"]
        j["lineCoverage"]=j["lineCoverage"][:1]
        j["lineCoverage"][0]["claimIds"]=["c-0000","c-0001"]
        result=validate_printed_partition(n,j,budget)
        self.assertEqual(len(result["claims"]),2)
        self.assertEqual(budget.line_links,2)
        self.assertIn("typed source scoring",result["note"])

    def test_exact8192_occurrences_then_first_excess_before_join(self):
        _, budget, n, j = fixture(2048,4)
        self.assertEqual(validate_printed_partition(n,j,budget)["printedOccurrenceCount"],8192)
        _, budget, n, j = fixture(2048,4)
        n["claimBatches"][-1]["claims"][-1]["printedOccurrences"].append(dict(id="extra",span=n["lineLedger"][-1]["span"]))
        j["claimBatches"][-1]["claims"][-1]["printedOccurrences"].append(dict(id="extra",verdict="unjudged",reason="Synthetic excess."))
        with self.assertRaises(ProtocolLimit) as error: validate_printed_partition(n,j,budget)
        self.assertEqual(error.exception.code,"OCCURRENCE_LIMIT")
        self.assertEqual(error.exception.resource["attemptedValue"],8193)
        self.assertEqual(budget.line_links,0)

    def test_initial_and_reviewed_may_select_different_claim_partitions(self):
        _, budget, initial, labels=fixture(2)
        validate_printed_partition(initial,labels,budget,phase="initial")
        reviewed=copy.deepcopy(initial);labels=copy.deepcopy(labels)
        for index, (claim,label) in enumerate(zip(reviewed["claimBatches"][0]["claims"],labels["claimBatches"][0]["claims"])):
            claim["id"]=label["id"]=f"new-{index}"
            reviewed["lineLedger"][index]["claimIds"]=[claim["id"]]
            labels["lineCoverage"][index]["claimIds"]=[claim["id"]]
        result=validate_printed_partition(reviewed,labels,budget)
        self.assertEqual([r["id"] for r in result["claims"]],["new-0","new-1"])
        self.assertEqual(budget.line_links,4)

    def test_unknown_mixed_and_empty_phase_flags_reject(self):
        for change in ("v1", "unknown", "flag", "phase"):
            _, budget, n, j=fixture(0)
            if change == "v1": n["schema"]="compass.broad-query-normalized/1"
            elif change == "unknown": j["schema"]="compass.broad-query-judgments/99"
            elif change == "flag": j["semanticCoverageComplete"]=1
            with self.subTest(change=change), self.assertRaises(ValueError):
                validate_printed_partition(n,j,budget,phase="third" if change == "phase" else "reviewed")


if __name__ == "__main__": unittest.main()
