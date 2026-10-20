"""Synthetic transport feedback only; no actual answer normalization/quality."""
from copy import deepcopy
import unittest
from benchmarks.agent_query.broad_query_protocol import pack_claim_batches, flatten_claim_batches, validate_review_partition


def claims(n):
    return [dict(id=f"claim-{i}", payload=i) for i in range(n)]


class BatchTransportTests(unittest.TestCase):
    def test_all_declared_count_boundaries_round_trip_in_order(self):
        for n in (0, 1, 127, 128, 129, 198, 2048):
            with self.subTest(n=n):
                original = claims(n)
                before = deepcopy(original)
                packed = pack_claim_batches(original)
                self.assertEqual(flatten_claim_batches(packed), original)
                self.assertEqual(original, before)
                self.assertEqual([b["index"] for b in packed["claimBatches"]], list(range((n + 127)//128)))
                self.assertTrue(all(len(b["claims"]) == 128 for b in packed["claimBatches"][:-1]))

    def test_aggregate_plus_one_is_rejected_without_a_prefix(self):
        with self.assertRaises(ValueError):
            pack_claim_batches(claims(2049))
        p = pack_claim_batches(claims(2048))
        p["claimCount"] = 2049
        with self.assertRaises(ValueError):
            flatten_claim_batches(p)

    def test_ids_are_global_across_batch_boundaries(self):
        c = claims(129)
        c[-1]["id"] = c[0]["id"]
        with self.assertRaises(ValueError):
            pack_claim_batches(c)
        p = pack_claim_batches(claims(129))
        p["claimBatches"][-1]["claims"][-1]["id"] = "claim-0"
        with self.assertRaises(ValueError):
            flatten_claim_batches(p)

    def test_indexes_totals_and_packing_are_not_advisory(self):
        base = pack_claim_batches(claims(129))
        variants = []
        for value in (True, -1, 1.0, 2049, "129", None):
            p = deepcopy(base); p["claimCount"] = value; variants.append(p)
        for value in (True, -1, 2, "1"):
            p = deepcopy(base); p["claimBatches"][1]["index"] = value; variants.append(p)
        p = deepcopy(base); p["claimBatches"].reverse(); variants.append(p)
        p = deepcopy(base); p["claimBatches"][0]["claims"].pop(); variants.append(p)
        p = deepcopy(base); p["claimBatches"][1]["claims"] = []; variants.append(p)
        p = deepcopy(base); p["claimBatches"].append(dict(index=2, claims=[])); variants.append(p)
        p = deepcopy(base); p["claimBatches"][0]["extra"] = True; variants.append(p)
        p = deepcopy(base); p["claims"] = []; variants.append(p)
        for p in variants:
            with self.subTest(packing=str(p)[:100]):
                with self.assertRaises(ValueError):
                    flatten_claim_batches(p)

    def test_every_late_review_label_has_the_exact_global_id_partition(self):
        normalized = pack_claim_batches(claims(198))
        reviewed = pack_claim_batches([dict(id=c["id"], verdict="unjudged") for c in claims(198)])
        c, r = validate_review_partition(normalized, reviewed)
        self.assertEqual(len(c), len(r))
        for kind in ("missing", "unknown", "reordered"):
            p = deepcopy(reviewed)
            if kind == "missing": p["claimBatches"][1]["claims"].pop()
            if kind == "unknown": p["claimBatches"][1]["claims"][-1]["id"] = "unknown"
            if kind == "reordered": p["claimBatches"][1]["claims"].reverse()
            with self.subTest(kind=kind):
                with self.assertRaises(ValueError):
                    validate_review_partition(normalized, p)

    def test_malformed_ids_never_bypass_boundary_validation(self):
        for value in ("", "x"*129, "bad\nID", 123, None):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    pack_claim_batches([dict(id=value)])


if __name__ == "__main__":
    unittest.main()
