import csv
import io
import unittest

from benchmarks.agent_query.mlcq_corpus import FIELDS, MAX_BYTES, census, select_cohorts


def row(identifier, sample, reviewer, severity='none', **changes):
    value = dict(zip(FIELDS, [''] * len(FIELDS)))
    value.update(id=str(identifier), sample_id=str(sample), reviewer_id=str(reviewer),
                 smell='blob', severity=severity, type='class', code_name='org.example.C'+str(sample),
                 repository='git@github.com:example/project.git', commit_hash='a'*40,
                 path='/src/C.java', start_line='10', end_line='20')
    return value | changes


def encode(rows):
    output = io.StringIO(newline='')
    writer = csv.DictWriter(output, fieldnames=FIELDS, delimiter=';'); writer.writeheader(); writer.writerows(rows)
    return output.getvalue().encode()


class MlcqCorpusTests(unittest.TestCase):
    def test_preserves_disagreement_minor_and_strong_consensus(self):
        data = encode([row(1, 1, 1), row(2, 1, 2), row(3, 2, 1, 'major'), row(4, 2, 2, 'critical'),
                       row(5, 3, 1, 'minor'), row(6, 3, 2, 'minor'), row(7, 4, 1), row(8, 4, 2, 'major')])
        r = census(data)
        self.assertEqual([s['classification'] for s in r['samples']],
                         ['unanimous-none', 'unanimous-major-critical', 'minor-or-disagreement', 'minor-or-disagreement'])
        self.assertEqual(r['blobReviews'], 8)
        self.assertEqual(select_cohorts(r), [])

    def test_repeated_reviewer_never_creates_an_independent_vote(self):
        r = census(encode([row(1, 1, 7, 'major'), row(2, 1, 7, 'major')]))['samples'][0]
        self.assertEqual(r['classification'], 'single-reviewer')
        self.assertEqual(r['duplicateReviewerRows'], 1)
        self.assertEqual(len(r['ratings']), 2)

    def test_same_reviewer_conflict_is_preserved_and_excluded(self):
        r = census(encode([row(1, 1, 1), row(2, 1, 1, 'major'), row(3, 1, 2)]))
        self.assertEqual(r['samples'][0]['classification'], 'conflicting-same-reviewer')

    def test_selection_retains_all_secondary_samples_and_is_order_independent(self):
        rows = []
        for sample in range(1, 5):
            for reviewer in [1, 2]:
                rows.append(row(len(rows)+1, sample, reviewer, 'major' if sample <= 2 else 'none'))
        rows.append(row(9, 5, 1, 'minor'))
        first, second = census(encode(rows)), census(encode(rows[::-1]))
        self.assertEqual(select_cohorts(first), select_cohorts(second))
        self.assertEqual(len(select_cohorts(first)), 5)
        self.assertTrue(first['projects'][0]['eligible'])

    def test_identity_conflicts_and_duplicate_review_ids_fail(self):
        for rows in [[row(1, 1, 1), row(2, 1, 2, path='/elsewhere.java')],
                     [row(1, 1, 1), row(1, 2, 1)]]:
            with self.assertRaises(ValueError): census(encode(rows))

    def test_paths_revisions_and_line_bounds_fail_closed(self):
        for changes in [dict(path='/../x.java'), dict(path='//x.java'), dict(path='/a/./b.java'),
                        dict(path='a\\b.java'), dict(commit_hash='main'), dict(start_line='0'),
                        dict(end_line='9'), dict(repository='https://attacker.invalid/x'), dict(severity='high')]:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                census(encode([row(1, 1, 1, **changes)]))

    def test_nonblob_rows_do_not_become_clean_class_samples(self):
        r = census(encode([row(1, 1, 1, smell='long method', type='function')]))
        self.assertEqual(r['totalReviews'], 1)
        self.assertEqual(r['blobSamples'], 0)

    def test_schema_and_byte_bounds(self):
        for data in [b'x;y\n1;2\n', b'x'*(MAX_BYTES+1)]:
            with self.assertRaises(ValueError): census(data)


if __name__ == '__main__': unittest.main()
