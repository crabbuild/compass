"""Synthetic exact resource/byte-line controls, not actual source judgments."""
import hashlib
import unittest
from unittest.mock import patch
from benchmarks.agent_query.broad_query_protocol import (PrintedSpanBudget, BundleWork, ProtocolLimit,
    ProtocolIntegrityError, MAX_DISTINCT_HASH_BYTES, MAX_DISTINCT_SPANS,
    MAX_SPAN_CHECKS, MAX_LINE_LINKS, MAX_BUNDLE_HASH_BYTES, MAX_BUNDLE_LINE_LINKS)


def span(raw,start,end):
    return dict(startByte=start,endByte=end,
                sha256=hashlib.sha256(memoryview(raw)[start:end]).hexdigest())


class PrintedWorkTests(unittest.TestCase):
    def test_lf_crlf_final_cr_and_unicode_line_separator_are_byte_defined(self):
        raw = "first\r\n \t\r\nα\u2028界\n\r".encode()
        budget = PrintedSpanBudget(raw)
        ledger = budget.ledger()
        self.assertEqual([x["physicalLine"] for x in ledger],[1,3,4])
        self.assertEqual([raw[x["span"]["startByte"]:x["span"]["endByte"]] for x in ledger],
                         [b"first","α\u2028界".encode(),b"\r"])
        self.assertEqual(PrintedSpanBudget(b"\n\r\n \t").ledger(),[])
        self.assertEqual(PrintedSpanBudget(b"").ledger(),[])

    def test_utf8_boundaries_and_conflicting_cached_sha_fail_closed(self):
        raw = "α界🙂\n".encode()
        budget = PrintedSpanBudget(raw)
        s = span(raw,0,2)
        budget.validate(s)
        before = budget.hashed_bytes
        for _ in range(100):budget.validate(s)
        self.assertEqual(budget.hashed_bytes,before)
        with self.assertRaises(ProtocolIntegrityError):budget.validate(dict(s,sha256="0"*64))
        with self.assertRaises(ProtocolIntegrityError):budget.ledger()
        for start,end in [(1,2),(0,1),(3,5),(5,6)]:
            with self.subTest(start=start,end=end):
                with self.assertRaises(ProtocolIntegrityError):
                    PrintedSpanBudget(raw).validate(span(raw,start,end))

    def test_inclusive256mib_hash_budget_then_no_over_bound_hash(self):
        raw = (b"x"*65535+b"\n")*513
        budget = PrintedSpanBudget(raw)
        width = 32*1024**2
        for start in range(6):budget.validate(span(raw,start,start+width))
        remaining = MAX_DISTINCT_HASH_BYTES-budget.hashed_bytes
        budget.validate(span(raw,6,6+remaining))
        self.assertEqual(budget.hashed_bytes,MAX_DISTINCT_HASH_BYTES)
        extra = span(raw,20,21)
        before = len(budget._digests)
        with patch("benchmarks.agent_query.broad_query_protocol.hashlib.sha256",side_effect=AssertionError("over-bound hash executed")):
            with self.assertRaises(ProtocolLimit) as failure:budget.validate(extra)
        self.assertEqual(failure.exception.code,"SPAN_BYTE_WORK_LIMIT")
        self.assertEqual(budget.hashed_bytes,MAX_DISTINCT_HASH_BYTES)
        self.assertEqual(len(budget._digests),before)
        self.assertEqual(failure.exception.resource["countAtLeast"],MAX_DISTINCT_HASH_BYTES)
        self.assertEqual(failure.exception.resource["attemptedValue"],MAX_DISTINCT_HASH_BYTES+1)

    def test_cache_and_check_boundaries_allow_equality(self):
        raw = b"x\n"*65536
        budget = PrintedSpanBudget(raw)
        extra_count = MAX_DISTINCT_SPANS-len(budget._digests)
        for i in range(extra_count):budget.validate(span(raw,2*i,2*i+2))
        self.assertEqual(len(budget._digests),MAX_DISTINCT_SPANS)
        with self.assertRaises(ProtocolLimit)as failure:budget.validate(span(raw,1,2))
        self.assertEqual(failure.exception.code,"SPAN_CACHE_LIMIT")
        self.assertEqual(len(budget._digests),MAX_DISTINCT_SPANS)
        budget = PrintedSpanBudget(b"x")
        s = span(b"x",0,1)
        for _ in range(MAX_SPAN_CHECKS-budget.checks):budget.validate(s)
        self.assertEqual(budget.checks,MAX_SPAN_CHECKS)
        with self.assertRaises(ProtocolLimit):budget.validate(s)
        self.assertEqual(budget.checks,MAX_SPAN_CHECKS)

    def test_indexed_join_exact_links_and_reviewed_phase_first_excess(self):
        raw = b"x\n"*32
        budget = PrintedSpanBudget(raw)
        s = span(raw,0,len(raw))
        for i in range(8192):
            self.assertEqual(len(budget.join("initial",f"c{i//4}",f"occ{i}",s)),32)
        self.assertEqual(budget.line_links,MAX_LINE_LINKS)
        self.assertEqual(budget.bundle.line_links,MAX_LINE_LINKS)
        # Repeating an identical reference validates its supplied SHA but adds no links.
        budget.join("initial","c0","occ0",s)
        self.assertEqual(budget.line_links,MAX_LINE_LINKS)
        with self.assertRaises(ProtocolLimit)as failure:
            budget.join("reviewed","c0","new",span(raw,0,1))
        self.assertEqual(failure.exception.code,"COVERAGE_LINK_LIMIT")
        self.assertEqual(budget.line_links,MAX_LINE_LINKS)
        with self.assertRaises(ProtocolLimit):budget.ledger()

    def test_bundle_scalar_counter_exact_boundaries_are_accounting_fixtures(self):
        # Synthetic counter preconditions; this does not claim8GiB was hashed.
        bundle = BundleWork(hash_bytes=MAX_BUNDLE_HASH_BYTES-1,line_links=MAX_BUNDLE_LINE_LINKS-1)
        bundle.require_hash_capacity(1);bundle.hash_bytes+=1
        bundle.require_link_capacity();bundle.line_links+=1
        self.assertEqual(bundle.hash_bytes,MAX_BUNDLE_HASH_BYTES)
        self.assertEqual(bundle.line_links,MAX_BUNDLE_LINE_LINKS)
        with self.assertRaises(ProtocolLimit):bundle.require_hash_capacity(1)
        with self.assertRaises(ProtocolLimit):bundle.require_link_capacity()
        self.assertEqual(bundle.hash_bytes,MAX_BUNDLE_HASH_BYTES)
        self.assertEqual(bundle.line_links,MAX_BUNDLE_LINE_LINKS)

    def test_bundle_counter_constructor_rejects_invalid_preconditions(self):
        for kwargs in [dict(hash_bytes=-1),dict(hash_bytes=True),dict(hash_bytes=MAX_BUNDLE_HASH_BYTES+1),
                       dict(line_links=-1),dict(line_links=1.0),dict(line_links=MAX_BUNDLE_LINE_LINKS+1)]:
            with self.subTest(kwargs=kwargs):
                with self.assertRaises(ProtocolIntegrityError):BundleWork(**kwargs)

    def test_physical_line_caps_and_join_identity_fail_explicitly(self):
        PrintedSpanBudget(b"x"*65535+b"\n")
        PrintedSpanBudget(b"x"*65534+b"\r\n")
        PrintedSpanBudget(b"x"*65536)
        for raw in [b"x"*65536+b"\n",b"x"*65535+b"\r\n",b"\n"*65537]:
            with self.subTest(size=len(raw)):
                with self.assertRaises(ProtocolLimit):PrintedSpanBudget(raw)
        raw=b"a\nb\n";budget=PrintedSpanBudget(raw)
        budget.join("initial","c1","same",span(raw,0,1))
        with self.assertRaises(ProtocolIntegrityError):
            budget.join("initial","c2","same",span(raw,2,3))
        with self.assertRaises(ProtocolIntegrityError):
            PrintedSpanBudget(b" \t").join("reviewed","c1","o1",span(b" \t",0,1))


if __name__ == "__main__":unittest.main()
