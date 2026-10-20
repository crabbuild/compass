"""Synthetic normalizer2 contracts; these are not measured tool answers."""
import copy
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_capture as old
from benchmarks.agent_query import broad_query_normalize_v2 as normalizer
from benchmarks.agent_query.broad_query_normalize_v2 import normalize_v2, _json_size
from benchmarks.agent_query.broad_query_protocol import PrintedSpanBudget, ProtocolLimit, flatten_claim_batches, pack_claim_batches
from benchmarks.agent_query.broad_query_coverage import validate_printed_partition, JUDGMENTS_SCHEMA
from benchmarks.agent_query.broad_query_review import GraphIndex
from benchmarks.agent_query.tests.test_broad_query_review import graph

SAVE=b"NODE Store::save [function] store.py:2:1-3:28\n"
FLUSH=b"NODE Store::flush [function] store.py:4:1-5:12\n"


def normalize(raw, tool="compass", fixture=None, **kwargs):
    return normalize_v2(raw,tool,GraphIndex(fixture or graph(tool),tool),task="synthetic",arm="default",
        source_registration_sha256="a"*64,actors={"normalizer":"b"*64,"assessor":"c"*64},**kwargs)


def raw_claims(count):
    if not count: return b"header only\n"
    if count == 1: return SAVE
    return SAVE+FLUSH+b"".join(f"EDGE Store::save --calls--> Store::flush [exact] at=store.py:L{i+1}\n".encode() for i in range(count-2))


class Version2NormalizerTests(unittest.TestCase):
    def test_helper_navigation_and_preview_lines_remain_pending_source_review(self):
        raw = (b"Call-linked lookups: 2/40; indexed recall probes: 17/291.\n"
               b"1. Store::save @ store.py:2 [relevance=low]\n"
               b"   Requested operation-name leads are reachable through exact AST call paths. Alternatives are bounded inspection candidates, not a unique interpretation or proof of requested behavior\n"
               b"2. Store::flush @ store.py:4 [relevance=low]\n"
               b"   Call path (execution unverified): save -> flush; navigation hint publish -> flush\n"
               b"   Helper code preview (behavior unverified) @ other.py:99: return value\n")
        result = normalize(raw)
        claims = flatten_claim_batches(result)
        self.assertEqual(result["rankingKind"], "retrieval")
        self.assertEqual(len(result["ranked"]), 2)
        self.assertEqual(len(claims), 2)
        self.assertTrue(all(c["assertionKind"] == "declaration_identity_only" for c in claims))
        self.assertFalse(result["normalizationComplete"])
        self.assertFalse(result["semanticCoverageComplete"])
        self.assertEqual(len(result["lineLedger"]), 6)
        pending = [entry for entry in result["reviewQueue"] if entry["kind"] == "unparsed_printed_text"]
        self.assertEqual(len(pending), 4)
        for entry in pending:
            span = entry["span"]
            self.assertTrue(raw[span["startByte"]:span["endByte"]])
        self.assertFalse(any("relation" in c for c in claims))

        self.assertEqual(result["normalizationStatus"], "pending_review")

    def test_resolved_ownerless_and_conflicting_groups_summarize_once_for_both_endpoints(self):
        for case in ("ownerless_same","ownerless_distinct","conflicting","resolved_same","resolved_distinct"):
            count={"gets":0,"calls":0}
            class TrackedRecord(dict):
                def get(self,key,*default):
                    if key == "resolvedIdentity": count["gets"]+=1
                    return super().get(key,*default)
            original=normalizer._endpoint_summaries
            def summaries(labels):
                count["calls"]+=1
                return original({label:[TrackedRecord(item) for item in items] for label,items in labels.items()})
            fixture=graph("graphify" if case.startswith("ownerless") else "compass")
            if case.startswith("ownerless"):
                raw=b"NODE .save() [src=store.py loc=L2]\n"*200
                if case.endswith("distinct"):
                    raw+=b"NODE .flush() [src=store.py loc=L4]\n"*200
                target=b".flush()" if case.endswith("distinct") else b".save()"
                raw+=(b"EDGE .save() --calls [exact]--> "+target+b"\n")*400
                tool="graphify"
            else:
                raw=SAVE*200+FLUSH*200
                if case == "conflicting":
                    extra=copy.deepcopy(fixture["nodes"][0]);extra["id"]="other";extra["source"]["startLine"]=6;fixture["nodes"].append(extra)
                    raw+=SAVE.replace(b":2:1-3:28",b":6:1-7:28")
                target=b"Store::save" if case.endswith("same") else b"Store::flush"
                raw+=(b"EDGE Store::save --calls--> "+target+b" [exact]\n")*400
                tool="compass"
            with patch.object(normalizer,"_endpoint_summaries",side_effect=summaries):
                result=normalize(raw,tool,fixture=fixture)
            with self.subTest(case=case):
                self.assertEqual(count["calls"],1)
                self.assertLessEqual(count["gets"],410)
                self.assertEqual(len(result["reviewQueue"]),raw.count(b"\n"))
                self.assertFalse(result["normalizationComplete"])
                relations=[c for c in flatten_claim_batches(result) if "relation" in c]
                self.assertEqual(len(relations),0 if case.startswith("ownerless") or case == "conflicting" else 1)

    def test_repeated_edges_do_not_rescan_every_endpoint_declaration(self):
        counts={"equalities":0}
        class TrackedIdentity(dict):
            def __eq__(self, other):
                counts["equalities"]+=1
                return super().__eq__(other)
        original=old.Identity.record
        def record(identity): return TrackedIdentity(original(identity))
        edge=b"EDGE Store::save --calls--> Store::flush [exact]\n"
        with patch.object(old.Identity,"record",record):
            result=normalize(SAVE*200+FLUSH+edge*400)
        claims=flatten_claim_batches(result)
        self.assertEqual(len(claims),3)
        self.assertEqual(sum(len(c["printedOccurrences"]) for c in claims),601)
        self.assertLessEqual(counts["equalities"],250)

    def test_canonical_all_batch_boundaries_preserve_pending_source_review(self):
        for count in (0,1,127,128,129,198,2048):
            with self.subTest(count=count):
                raw=raw_claims(count);result=normalize(raw)
                self.assertEqual(result["claimCount"],count)
                claims=flatten_claim_batches(result)
                self.assertEqual(len(claims),count)
                self.assertFalse(result["normalizationComplete"])
                self.assertFalse(result["semanticCoverageComplete"])
                self.assertEqual(result["normalizationStatus"],"pending_review")
                self.assertEqual(result["rawStdoutSha256"],old.sha(raw))
                self.assertNotIn("claims",result)

    def test_first_claim_excess_rejects_whole_row_without_asserting_empty(self):
        result=normalize(raw_claims(2049))
        self.assertEqual(result["normalizationStatus"],"rejected")
        self.assertEqual(result["normalizationFailure"]["code"],"CLAIM_LIMIT")
        self.assertEqual(result["claimCount"],0)
        self.assertEqual(result["claimBatches"],[])
        self.assertIsNone(result["printedAssertionCount"])
        self.assertIsNone(result["fullMeasuredPrecision"])
        self.assertIsNone(result["falseEmpty"])
        self.assertFalse(result["success"])

    def test_every_repeated_printed_occurrence_is_retained_and_bounded(self):
        exact=normalize(SAVE*8192)
        claims=flatten_claim_batches(exact)
        self.assertEqual(len(claims),1)
        self.assertEqual(len(claims[0]["printedOccurrences"]),8192)
        excess=normalize(SAVE*8193)
        self.assertEqual(excess["normalizationFailure"]["code"],"OCCURRENCE_LIMIT")
        self.assertEqual(excess["claimCount"],0)

    def test_citation_distinctions_remain_separate_and_repeats_retain_occurrences(self):
        edge=b"EDGE Store::save --calls--> Store::flush [exact] at=store.py:L3\n"
        result=normalize(SAVE+FLUSH+edge+edge+edge.replace(b"L3",b"L8"))
        relations=[c for c in flatten_claim_batches(result) if "relation" in c]
        self.assertEqual(len(relations),2)
        self.assertEqual([len(c["printedOccurrences"]) for c in relations],[2,1])
        self.assertEqual(relations[0]["printedOccurrences"][0]["printedSite"],[c for c in relations[0]["printedOccurrences"]][1]["printedSite"])
        self.assertTrue(all(c["relation"]["occurrences"] == [] for c in relations))

    def test_initial_partition_joins_exact_ledger_and_all_unparsed_text(self):
        raw=b" \t\r\n"+SAVE+b"unparsed behavior \xe2\x80\xa8 still same line\r\n"+FLUSH+b"final CR\r"
        result=normalize(raw)
        claims=flatten_claim_batches(result)
        labels=[dict(id=c["id"],verdict="unjudged",reason="Synthetic pending claim.",
            printedOccurrences=[dict(id=o["id"],verdict="unjudged",reason="Synthetic pending occurrence.") for o in c["printedOccurrences"]]) for c in claims]
        lines=[dict(index=line["index"],classification="assertions" if line["claimIds"] else "unresolved",
            claimIds=line["claimIds"],reason="Synthetic initial print partition.") for line in result["lineLedger"]]
        j=dict(schema=JUDGMENTS_SCHEMA,semanticCoverageComplete=False,lineCoverage=lines,**pack_claim_batches(labels))
        checked=validate_printed_partition(result,j,PrintedSpanBudget(raw),phase="initial")
        self.assertFalse(checked["semanticCoverageComplete"])
        self.assertEqual(checked["unresolvedLineCount"],2)
        self.assertEqual([l["physicalLine"] for l in result["lineLedger"]],[2,3,4,5])
        self.assertEqual(len(result["reviewQueue"]),4)
        self.assertTrue(raw[result["lineLedger"][-1]["span"]["startByte"]:].endswith(b"\r"))

    def test_missing_owners_and_ambiguous_graph_anchors_never_auto_select(self):
        raw=b"NODE .save() [src=store.py loc=L2]\n"
        self.assertEqual(normalize(raw,"graphify")["claimCount"],0)
        fixture=graph();other=copy.deepcopy(fixture["nodes"][0]);other["id"]="duplicate";fixture["nodes"].append(other)
        result=normalize(SAVE,fixture=fixture)
        self.assertEqual(result["ranked"],[])
        self.assertEqual(result["claimCount"],0)
        self.assertEqual(result["reviewQueue"][0]["graphMatchCount"],2)

    def test_empty_no_match_failure_and_limits_never_infer_semantic_empty(self):
        for raw, flags, disposition in [(b"",{},"error"),(b"no_match\n",{},"abstain"),
                (b"",dict(no_match_outcome=True),"abstain"),(SAVE,dict(failed=True),"error"),
                (SAVE,dict(limited=True),"limit")]:
            result=normalize(raw,**flags)
            self.assertEqual(result["disposition"],disposition)
            self.assertFalse(result["normalizationComplete"])
            self.assertFalse(result["semanticCoverageComplete"])
        self.assertEqual(normalize(b"\xff")["normalizationFailure"]["code"],"INVALID_UTF8")

    def test_exact_json_byte_cap_and_first_excess_use_bounded_typed_error(self):
        framing=len(old.encoded(dict(x="")))
        content="x"*(old.MAX_JSON_BYTES-framing)
        _json_size(dict(x=content))
        with self.assertRaises(ProtocolLimit) as error: _json_size(dict(x=content+"x"))
        self.assertEqual(error.exception.code,"JSON_BYTE_LIMIT")
        self.assertFalse(error.exception.resource["exactCountKnown"])
        with patch("benchmarks.agent_query.broad_query_normalize_v2._json_size",side_effect=error.exception):
            rejected=normalize(SAVE)
        self.assertEqual(rejected["normalizationFailure"]["code"],"JSON_BYTE_LIMIT")
        self.assertEqual(rejected["claimCount"],0)

    def test_invalid_frozen_bindings_abort_instead_of_becoming_native_no_match(self):
        graph_index=GraphIndex(graph(),"compass")
        base=dict(task="synthetic",arm="default",source_registration_sha256="a"*64,actors={"normalizer":"b"*64})
        for field,value in (("task",""),("arm","unknown"),("source_registration_sha256","bad"),("actors",{}),
                            ("actors",{"normalizer":[]}), ("failed",1)):
            values=dict(base);values[field]=value
            with self.subTest(field=field,value=value),self.assertRaises(ValueError):
                normalize_v2(SAVE,"compass",graph_index,**values)

    def test_v1_parser_keeps_its_historical_claim_limit(self):
        raw=raw_claims(129)
        # V1 groups citation-only connectivity into one suggestion; use129
        # uniquely printed declarations to characterize its actual128 cap.
        fixture=graph();fixture["nodes"]=[];fixture["edges"]=[];raw=b""
        for index in range(129):
            fixture["nodes"].append(dict(id=f"n{index}",name=f"f{index}",qualifiedName=f"Store::f{index}",source=dict(file="store.py",startLine=index+1)))
            raw+=f"NODE Store::f{index} [function] store.py:{index+1}:1-{index+1}:2\n".encode()
        with self.assertRaisesRegex(ValueError,"printed claims"):
            old.normalize(raw,"compass",GraphIndex(fixture,"compass"),task="synthetic",arm="default")


if __name__ == "__main__": unittest.main()
