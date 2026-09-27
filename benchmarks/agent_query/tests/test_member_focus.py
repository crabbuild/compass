import hashlib
import json
from pathlib import Path
import unittest

from benchmarks.agent_query.member_focus import canonical, focus_groups, focus_terms, name_terms


class MemberFocusTests(unittest.TestCase):
    def row(self, label, line=1, file='x'):
        return dict(label=label, file=file, line=line)

    def test_frozen_lexicon_matches_recorded_production_sources(self):
        lexicon = json.loads(Path('benchmarks/agent_query/member_focus_lexicon.json').read_text())
        for file, expected in lexicon['sourceSha256'].items():
            self.assertEqual(hashlib.sha256(Path(file).read_bytes()).hexdigest(), expected)

    def test_terms_match_the_five_captured_native_focus_questions(self):
        questions = json.loads(Path('benchmarks/agent_query/responsibility_questions_panel_a.json').read_text())
        prior = json.loads(Path('benchmarks/agent_query/explanation_focus_review.json').read_text())
        for case in questions['cases']:
            expected = next(r['focusTerms'] for r in prior['results']
                            if r['repository'] == case['repository'] and r['arm'] == 'focused')
            self.assertEqual(focus_terms(case['question']), expected)

    def test_case_snake_camel_morphology_and_unicode(self):
        terms = set(focus_terms('checking loops'))
        for name in ['checkLoop', '.check_loop()', 'CHECK_LOOP']:
            self.assertEqual(name_terms(name) & terms, {'check', 'loop'})
        self.assertEqual(name_terms('Résumé'), {'resume'})
        self.assertEqual(focus_terms('résumé'), ['resume'])
        self.assertEqual(focus_terms('路由处理'), ['处理', '由处', '路由', '路由处理'])
        for raw, expected in [('routing', 'route'), ('dependencies', 'dependency'),
                              ('mapped', 'map'), ('using', 'use')]:
            self.assertEqual(canonical(raw), expected)

    def test_duplicate_query_terms_do_not_add_weight(self):
        self.assertEqual(focus_terms('loop loop loops'), ['loop'])
        self.assertEqual(focus_groups([self.row('loopLoop')], 'loop')['groups'][0]['score'], 1)

    def test_duplicate_and_alternate_labels_use_maximum_not_union(self):
        rows = [self.row('check'), self.row('loop'), self.row('check')]
        report = focus_groups(rows, 'check loop')
        self.assertEqual(len(report['groups']), 1)
        self.assertEqual(report['groups'][0]['score'], 1)
        self.assertEqual([r['label'] for r in report['groups'][0]['labels']], ['check', 'loop'])

    def test_highest_score_first_then_source_order_independent_of_rows(self):
        rows = [self.row('loop', 3), self.row('loop', 1), self.row('checkLoop', 2)]
        expected = focus_groups(rows, 'check loop')
        self.assertEqual([g['line'] for g in expected['groups']], [2, 1, 3])
        self.assertEqual(focus_groups(list(reversed(rows)), 'check loop'), expected)

    def test_zero_matches_preserve_all_groups_and_source_order(self):
        rows = [self.row('first', 10), self.row('second', 1), self.row('third', 1, 'a')]
        report = focus_groups(rows, 'absent')
        self.assertEqual([(g['file'], g['line']) for g in report['groups']], [('a', 1), ('x', 1), ('x', 10)])
        self.assertTrue(all(g['score'] == 0 for g in report['groups']))

    def test_paths_kinds_and_other_tool_metadata_do_not_rank(self):
        row = self.row('plain', file='loop/check.rs')
        row.update(kind='loop', sourceText='loop', id='loop', score=999)
        self.assertEqual(focus_groups([row], 'loop')['groups'][0]['score'], 0)

    def test_empty_invalid_and_excessive_queries_fail(self):
        for query in ['', '!!!', None, 'x' * 4097, ' '.join(f'term{i}' for i in range(33))]:
            with self.subTest(query=query), self.assertRaises(ValueError):
                focus_terms(query)
        self.assertEqual(focus_terms('With'), ['with'])

    def test_invalid_labels_and_anchors_fail(self):
        for row in [None, {}, self.row('', 1), self.row('loop', True),
                    self.row('loop', 0), self.row('loop', file=''), self.row('x' * 4097)]:
            with self.subTest(row=row), self.assertRaises(ValueError):
                focus_groups([row], 'loop')

    def test_row_and_aggregate_metadata_bounds(self):
        with self.assertRaises(ValueError):
            focus_groups([self.row('loop')] * 10001, 'loop')
        with self.assertRaises(ValueError):
            focus_groups([self.row('loop', file='x' * 2000)] * 1000, 'loop')


if __name__ == '__main__':
    unittest.main()
