"""Tests for combining source reviews; no external source or tool required."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from benchmarks.agent_query.broad_query_combine import accepted_review_tasks, read_artifact


class SourceReviewIntegrityTests(unittest.TestCase):
    def test_counts_cannot_replace_explicit_accepted_task_dispositions(self):
        with self.assertRaisesRegex(ValueError, 'explicit accepted'):
            accepted_review_tasks(dict(summary=dict(accepted=48)))

    def test_uncertain_and_rejected_tasks_are_not_promoted(self):
        report = dict(tasks=[dict(taskId='yes',verdict='accepted'),
            dict(taskId='maybe',verdict='uncertain'),dict(taskId='no',verdict='rejected')])
        self.assertEqual(accepted_review_tasks(report), {'yes'})

    def test_source_binding_is_limited_to_its_actual_list(self):
        report = dict(schema='compass.broad-query-independent-source-review-binding/1',
            status='accepted_for_listed_8_tasks_only',taskIds=['one','two'])
        self.assertEqual(accepted_review_tasks(report), {'one','two'})
        report['status']='pending'
        with self.assertRaisesRegex(ValueError, 'unaccepted'):
            accepted_review_tasks(report)

    def test_artifact_digest_mismatch_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'record.json').write_text('{}')
            with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                read_artifact(root,dict(artifact='record.json',sha256='0'*64))

    def test_parent_and_symlink_escapes_fail_before_read(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);inside=root/'inside';inside.mkdir()
            data=b'{}';outside=root/'outside.json';outside.write_bytes(data)
            record=dict(artifact='../outside.json',sha256=hashlib.sha256(data).hexdigest())
            with self.assertRaisesRegex(ValueError, 'contained'):
                read_artifact(inside,record)
            (inside/'link.json').symlink_to(outside)
            record['artifact']='link.json'
            with self.assertRaisesRegex(ValueError, 'containment'):
                read_artifact(inside,record)


if __name__=='__main__':
    unittest.main()
