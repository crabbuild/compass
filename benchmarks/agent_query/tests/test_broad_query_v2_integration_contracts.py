"""Synthetic integration prerequisites, not independent source/quality evidence."""
import copy
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_review as legacy
from benchmarks.agent_query.broad_query_score_v2 import _Witnesses, score_task_v2
from benchmarks.agent_query.tests.test_broad_query_review import graph, identity, relation, site
from benchmarks.agent_query.tests.test_broad_query_score_v2 import scoring_fixture


def grouped(selected, *, shared=False, flat=False):
    expected, raw, budget, normalized, judgment = scoring_fixture(129)
    expected["facts"][0]["essential"] = False
    by_id = {"saves": expected["facts"][0]}
    for number, name in enumerate(("left", "right", "join", "flat"), 10):
        declaration = identity(name, number)
        fact = dict(id=name, identity=declaration, essential=name == "flat" and flat,
                    source=site(), statement="Synthetic witness-set member.")
        expected["facts"].append(fact)
        expected["relevant"].append(dict(identity=declaration, grade=3))
        by_id[name] = fact
    expected["essentialFactGroups"] = dict(schema=legacy.FACT_GROUP_SCHEMA,
        groups=[dict(id="path", anyOf=[["saves", "left"], ["saves" if shared else "right", "join"]])])
    claims = [claim for batch in normalized["claimBatches"] for claim in batch["claims"]]
    labels = [label for batch in judgment["claimBatches"] for label in batch["claims"]]
    # One complete alternative crosses the128-claim transport boundary.
    for index, fact_id in ((0, "saves"), (128, "left"), (127, "right"), (126, "join"), (125, "flat")):
        if fact_id in selected:
            labels[index]["factId"] = fact_id
            claims[index]["identity"] = by_id[fact_id]["identity"]
        elif index == 0:
            labels[index].pop("factId")
            witness = dict(file="store.py", line=1000, text="Synthetic extra assertion.")
            labels[index]["source"] = witness
            for occurrence in labels[index]["printedOccurrences"]:
                occurrence["source"] = witness
    return expected, raw, budget, normalized, judgment


class V2TaskIntegrationContracts(unittest.TestCase):
    def test_complete_alternative_crosses_batches_but_mixed_partial_paths_do_not(self):
        for selected, success, satisfied in (({"saves", "join"}, False, []),
                ({"saves", "left"}, True, [0]), ({"right", "join"}, True, [1])):
            expected, raw, budget, n, j = grouped(selected)
            result = score_task_v2(expected, n, j, None, raw, budget)
            with self.subTest(selected=selected):
                self.assertEqual(result["success"], success)
                self.assertEqual(result["essentialFactGroups"][0]["satisfiedAlternativeIndexes"], satisfied)
                self.assertEqual(result["claimPrecision"], 1)
                self.assertEqual(result["supportedClaims"], 129)

    def test_shared_prefix_and_flat_essential_conjunction_remain_required(self):
        for selected, flat, success, satisfied in (({"left", "join"}, False, False, []),
                ({"saves", "left", "join"}, False, True, [0, 1]),
                ({"saves", "left"}, True, False, [0]),
                ({"saves", "left", "flat"}, True, True, [0])):
            expected, raw, budget, n, j = grouped(selected, shared=True, flat=flat)
            result = score_task_v2(expected, n, j, None, raw, budget)
            with self.subTest(selected=selected, flat=flat):
                self.assertEqual(result["success"], success)
                self.assertEqual(result["essentialFactGroups"][0]["satisfiedAlternativeIndexes"], satisfied)

    def test_both_graph_adapters_preserve_exact_missing_and_ambiguous_identity(self):
        for tool in ("compass", "graphify"):
            for shape in ("exact", "missing", "ambiguous"):
                expected, raw, budget, n, j = scoring_fixture()
                data = graph(tool)
                if shape == "missing": data["nodes"] = data["nodes"][1:]; data["edges"] = []
                if shape == "ambiguous":
                    duplicate = copy.deepcopy(data["nodes"][0]); duplicate["id"] = "other-save"
                    data["nodes"].append(duplicate)
                index = legacy.GraphIndex(data, tool)
                with self.subTest(tool=tool, shape=shape):
                    if shape == "exact":
                        result = score_task_v2(expected, n, j, index, raw, budget)
                        self.assertTrue(result["success"])
                        self.assertEqual(result["ledger"][0]["graph"], "present")
                    else:
                        with self.assertRaisesRegex(ValueError, "uniquely resolve"):
                            score_task_v2(expected, n, j, index, raw, budget)
                        with self.assertRaises(ValueError): budget.ledger()

    def test_graph_relation_gaps_remain_separate_from_source_answer_correctness(self):
        for tool in ("compass", "graphify"):
            for shape in ("exact", "reverse", "wrong_site"):
                expected, raw, budget, n, j = scoring_fixture()
                expected["facts"][0]["relation"] = relation()
                n["claimBatches"][0]["claims"][0]["relation"] = relation()
                data = graph(tool)
                if shape == "reverse":
                    edge = data["edges"][0]; edge["source"], edge["target"] = edge["target"], edge["source"]
                if shape == "wrong_site":
                    if tool == "compass": data["edges"][0]["relationshipSite"]["startLine"] = 999
                    else: data["edges"][0]["source_location"] = "L999"
                result = score_task_v2(expected, n, j, legacy.GraphIndex(data, tool), raw, budget)
                with self.subTest(tool=tool, shape=shape):
                    self.assertTrue(result["success"])
                    self.assertEqual(result["sourceFactRecall"], 1)
                    self.assertEqual(result["ledger"][0]["graph"], "present" if shape == "exact" else "missing")


class WitnessIndexIntegrationContracts(unittest.TestCase):
    def labels(self, sites):
        return [dict(verdict="supported", source=source, printedOccurrences=[
            dict(verdict="supported", source=source)]) for source in sites]

    def test_historical_splitlines_separator_parity_and_repeated_requests(self):
        for separator in ("\n", "\r\n", "\r", "\v", "\f", "\x1c", "\x1d", "\x1e", "\x85", "\u2028", "\u2029"):
            for trailing in (False, True):
                text = separator.join((" first ", "\tsecond\t")) + (separator if trailing else "")
                sites = [dict(file="store.py", line=i, text=line) for i, line in enumerate(text.splitlines(), 1)]
                witnesses = _Witnesses({"store.py": text.encode()}, self.labels(sites))
                for source in sites: witnesses.validate(source)
                self.assertEqual(len(witnesses.lines), 2)
                with self.assertRaises(ValueError): witnesses.validate(dict(file="store.py", line=3, text="absent"))

    def test_missing_source_invalid_utf8_and_absent_lines_poison_public_score(self):
        for source in ({}, {"store.py": b"\xff"}, {"store.py": b"only one line\n"}):
            expected, raw, budget, n, j = scoring_fixture(2)
            with self.assertRaises(ValueError): score_task_v2(expected, n, j, None, raw, budget, sources=source)
            with self.assertRaises(ValueError): budget.ledger()

    def test_source_byte_caps_check_before_scanning_and_each_file_scans_once(self):
        data = b"needle\n" + b" " * (legacy.MAX_SOURCE_BYTES - 7)
        pattern = _Witnesses._line_end
        class Scans:
            count = 0
            def finditer(self, text):
                self.count += 1
                return pattern.finditer(text)
        source = dict(file="store.py", line=1, text="needle")
        scans = Scans()
        with patch.object(_Witnesses, "_line_end", scans):
            index = _Witnesses({"store.py": data}, self.labels([source, source]))
            index.validate(source)
            self.assertEqual(scans.count, 1)
            with self.assertRaises(ValueError): _Witnesses({"store.py": data + b" "}, self.labels([source]))
            self.assertEqual(scans.count, 1)
        # The byte object is shared in memory; the scanner actually processes
        #64 distinct file entries totaling256MiB, not a synthetic counter setup.
        sources = {f"f{i}.py": data for i in range(65)}
        sites = [dict(file=file, line=1, text="needle") for file in sources]
        scans = Scans()
        with patch.object(_Witnesses, "_line_end", scans):
            exact = _Witnesses(sources, self.labels(sites[:64]))
            self.assertEqual(scans.count, 64)
            for source in sites[:64]: exact.validate(source)
            scans.count = 0
            with self.assertRaises(ValueError): _Witnesses(sources, self.labels(sites))
            self.assertEqual(scans.count, 64)


if __name__ == "__main__": unittest.main()
