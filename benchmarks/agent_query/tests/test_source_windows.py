import copy
from pathlib import Path
import tempfile
import unittest

from benchmarks.agent_query.source_windows import score_windows, sha, source_windows


class SourceWindowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = b'first\nsecond\nthird\nfourth\n'
        (self.root / 'x').write_bytes(self.data)
        self.rows = [dict(file='x', line=1), dict(file='x', line=3)]

    def case(self, text='second', first=2, last=2):
        return dict(file='x', sourceFileSha256=sha(self.data), facts=[dict(
            id='test', witnesses=[dict(startLine=first, endLine=last, text=text)])])

    def test_order_grouping_and_shared_raw_byte_budget(self):
        rows = [self.rows[1], self.rows[0], dict(self.rows[0], label='duplicate')]
        r = source_windows(self.root, rows, 16)
        self.assertEqual(r['sourceBytes'], 16)
        self.assertEqual([(w['startByte'], w['endByte']) for w in r['windows']], [(0, 13), (13, 16)])
        self.assertEqual(len(r['windows'][0]['members']), 2)
        self.assertTrue(r['windows'][1]['partial'])
        self.assertEqual(r['sourceReadBytes'], len(self.data))

    def test_quota_exhaustion_retains_omissions(self):
        r = source_windows(self.root, self.rows, 6)
        self.assertEqual(r['omittedGroups'], [dict(file='x', line=3)])
        self.assertFalse(score_windows(self.root, self.case(), r)[0]['sufficientSourceEvidence'])

    def test_reranking_keeps_original_interval_ends(self):
        rows = [dict(file='x', line=i) for i in [1, 2, 3, 4]]
        order = [('x', 3), ('x', 1), ('x', 4), ('x', 2)]
        control = source_windows(self.root, rows, 8, ordered_groups=order)
        self.assertEqual([(w['startByte'], w['endByte'], w['requestedEndByte'])
                          for w in control['windows']], [(13, 19, 19), (0, 2, 6)])
        self.assertEqual(control['omittedGroups'], [dict(file='x', line=4), dict(file='x', line=2)])
        self.assertTrue(score_windows(self.root, self.case('third', 3, 3), control)[0]['sufficientSourceEvidence'])

    def test_reranking_retains_last_anchor_cap_and_other_file_boundaries(self):
        (self.root / 'y').write_bytes(b'y' * 5000)
        rows = self.rows + [dict(file='y', line=1)]
        control = source_windows(self.root, rows, 32000,
                                 ordered_groups=[('y', 1), ('x', 3), ('x', 1)])
        self.assertEqual(control['windows'][0]['endByte'], 4096)
        self.assertEqual(control['windows'][1]['endByte'], len(self.data))
        self.assertEqual(control['windows'][2]['endByte'], 13)

    def test_reordered_groups_must_be_an_exact_permutation(self):
        for order in [[], [('x', 1), ('x', 1)], [('x', 1), ('other', 3)],
                      [('x', True), ('x', 3)], [('x', 1), ['x']], 'bad']:
            with self.subTest(order=order), self.assertRaises(ValueError):
                source_windows(self.root, self.rows, 10, ordered_groups=order)

    def test_explicit_source_order_is_identical_to_default(self):
        self.assertEqual(source_windows(self.root, self.rows, 16),
                         source_windows(self.root, self.rows, 16,
                                        ordered_groups=[('x', 1), ('x', 3)]))

    def test_last_window_has_4096_byte_cap(self):
        (self.root / 'x').write_bytes(b'a' * 5000)
        r = source_windows(self.root, [self.rows[0]], 32000)
        self.assertEqual(r['sourceBytes'], 4096)
        self.assertFalse(r['windows'][0]['partial'])

    def test_missing_and_invalid_anchors_fail_even_after_budget(self):
        for bad in [dict(file='x', line=100), dict(file='x', line=True),
                    dict(file='x'), dict(file='../x', line=1), dict(file='/x', line=1)]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                source_windows(self.root, self.rows + [bad], 1)
        for budget in [True, 0, -1, 1048577]:
            with self.assertRaises(ValueError):
                source_windows(self.root, self.rows, budget)

    def test_symlink_escape_fails(self):
        with tempfile.TemporaryDirectory() as other:
            target = Path(other) / 'outside'
            target.write_bytes(b'outside')
            (self.root / 'link').symlink_to(target)
            with self.assertRaises(ValueError):
                source_windows(self.root, [dict(file='link', line=1)], 10)

    def test_fact_requires_all_witness_lines(self):
        for budget, expected in [(11, False), (13, True)]:
            r = source_windows(self.root, self.rows, budget)
            judgment = score_windows(self.root, self.case(), r)[0]
            self.assertEqual(judgment['sufficientSourceEvidence'], expected)

    def test_utf8_cut_is_retained_without_inventing_complete_character(self):
        self.data = '🧭\n'.encode()
        (self.root / 'x').write_bytes(self.data)
        r = source_windows(self.root, [self.rows[0]], 2)
        j = score_windows(self.root, self.case('🧭', 1, 1), r)[0]
        self.assertFalse(j['sufficientSourceEvidence'])
        self.assertEqual(j['missingAfterIndentNormalization'][0]['returned'], '�')
        self.assertEqual(r['sourceBytes'], 2)

    def test_source_drift_and_witness_drift_fail(self):
        r = source_windows(self.root, self.rows, 100)
        with self.assertRaises(ValueError):
            score_windows(self.root, self.case('wrong'), r)
        (self.root / 'x').write_bytes(b'changed')
        with self.assertRaises(ValueError):
            score_windows(self.root, self.case(), r)

    def test_interval_hash_counts_and_line_tampering_fail(self):
        original = source_windows(self.root, self.rows, 100)
        for change in [dict(endByte=999), dict(startByte=True), dict(sourceSha256='0'),
                       dict(sourceBytes=0), dict(startLine=2)]:
            r = copy.deepcopy(original)
            r['windows'][0].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                score_windows(self.root, self.case(), r)

    def test_header_allowance_needs_identity_and_all_other_lines(self):
        self.data = b'\n' * 454 + b'class _AtomicFile:\n    def name(self): pass\n'
        (self.root / 'x').write_bytes(self.data)
        case = self.case('class _AtomicFile:\n    def name(self): pass', 455, 456)
        case['facts'][0]['id'] = 'click-1'
        r = source_windows(self.root, [dict(file='x', line=456)], 100)
        self.assertFalse(score_windows(self.root, case, r)[0]['sufficientSourceEvidence'])
        self.assertTrue(score_windows(self.root, case, r, identity_verified=True)[0]['sufficientSourceEvidence'])
        r = source_windows(self.root, [dict(file='x', line=456)], 1)
        self.assertFalse(score_windows(self.root, case, r, identity_verified=True)[0]['sufficientSourceEvidence'])

    def test_file_and_row_bounds(self):
        with self.assertRaises(ValueError):
            source_windows(self.root, [self.rows[0]] * 10001, 10)
        (self.root / 'x').write_bytes(b'a' * 4194305)
        with self.assertRaises(ValueError):
            source_windows(self.root, self.rows, 10)


if __name__ == '__main__':
    unittest.main()
