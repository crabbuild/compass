"""Focused tests for source-oracle integrity, not product query quality."""
import tempfile
import unittest
from pathlib import Path

from benchmarks.agent_query.broad_query_corpus import TASKS
from benchmarks.agent_query.broad_query_oracle import SourceReader, anchor, make_task, publish_exclusive


class OracleIntegrityTests(unittest.TestCase):
    def test_python_identity_rejects_a_docstring_example(self):
        lines = ['class Actual:', '    def run(self):', '        """',
                 '        def run(self):', '            pass', '        """', '        return 1']
        spec = dict(file='sample.py', symbol='run', owner='Actual', declaration='        def run(',
                    occurrence=0, witnesses=[])
        with self.assertRaisesRegex(ValueError, 'owner/name'):
            anchor(spec, {'sample.py': lines})

    def test_python_owner_must_match(self):
        spec = dict(file='sample.py', symbol='run', owner='Other', declaration='    def run(',
                    occurrence=0, witnesses=[])
        with self.assertRaisesRegex(ValueError, 'owner/name'):
            anchor(spec, {'sample.py': ['class Actual:', '    def run(self):', '        return 1']})

    def test_witness_cannot_drift_into_the_next_function(self):
        spec = dict(file='sample.go', symbol='First', owner='', declaration='func First(',
                    occurrence=0, witnesses=[dict(needle='onlySecond()', statement='incorrect')])
        with self.assertRaisesRegex(ValueError, 'too far'):
            anchor(spec, {'sample.go': ['func First() {', '}', 'func Second() {', '    onlySecond()', '}']})

    def test_publication_never_overwrites_a_freeze(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'freeze.json'
            publish_exclusive(path, b'original')
            with self.assertRaises(FileExistsError):
                publish_exclusive(path, b'replacement')
            self.assertEqual(path.read_bytes(), b'original')
            self.assertEqual([p.name for p in path.parent.iterdir()], ['freeze.json'])

    def test_source_containment_rejects_parent_escape(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reader = SourceReader(root, root)
            with self.assertRaisesRegex(ValueError, 'escapes'):
                reader.contained(root, '../escape.py')

    def test_panel_has_full_balance_and_two_call_hops(self):
        self.assertEqual(len(TASKS), 160)
        for task in TASKS:
            if task['kind'] == 'multihop':
                self.assertGreaterEqual(len(task['edges']), 2, task['id'])
            if task['kind'] == 'unsupported':
                self.assertFalse(task['anchors'])
                self.assertTrue(task['rationale'])

    def test_relation_direction_keeps_authored_source_and_target(self):
        source = dict(file='sample.go', startLine=1, symbol='First', owner='')
        target = dict(file='sample.go', startLine=4, symbol='Second', owner='')
        spec = dict(id='sample.call', repository='sample', cluster='sample.call', kind='directed',
                    question='What is called?', anchors=['first', 'second'],
                    edges=[('first', 'second', 'Second()')], rationale=None)
        resolved = {('sample', key): dict(identity=identity, endLine=end)
                    for key, identity, end in [('first', source, 3), ('second', target, 5)]}
        result = make_task(spec, resolved, {'sample': {'sample.go': ['func First() {', 'Second()', '// Second() is documented here', 'func Second() {}']}})
        self.assertFalse(result['facts'][0]['requireOccurrences'])
        relation = result['facts'][0]['relation']
        self.assertEqual(relation['source'], source)
        self.assertEqual(relation['target'], target)
        self.assertEqual(relation['direction'], 'source_to_target')
        self.assertEqual(relation['occurrences'][0]['line'], 2)
        self.assertEqual(len(relation['occurrences']), 1)


if __name__ == '__main__':
    unittest.main()
