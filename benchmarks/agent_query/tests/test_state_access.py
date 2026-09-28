import copy
from pathlib import Path
import tempfile
import unittest

from benchmarks.agent_query.state_access_audit import anchor, assess, candidates, read


class StateAccessTests(unittest.TestCase):
    def setUp(self):
        self.case = dict(repository='fixture', file='state.py')
        self.group = dict(owner='Box', state='value', declarationLine=2)
        self.access = dict(method='get', methodLine=3, line=4)
        self.graph = dict(directed=True, nodes=[
            dict(id='m', name='.get()', kind='method', source=dict(file='state.py', startLine=3)),
            dict(id='s', name='value', kind='field', source=dict(file='state.py', startLine=2))], links=[
            dict(id='e', source='m', target='s', kind='references',
                 relationshipSite=dict(file='state.py', startLine=4))])

    def score(self, graph=None, tool='compass'):
        return assess(graph or self.graph, tool, self.case, self.group, self.access)

    def test_contact_and_selected_occurrence_are_separate(self):
        row = self.score()
        self.assertTrue(row['contactSupported'])
        self.assertTrue(row['selectedLineSupported'])
        self.graph['links'][0]['relationshipSite']['startLine'] = 5
        row = self.score()
        self.assertTrue(row['contactSupported'])
        self.assertFalse(row['selectedLineSupported'])

    def test_calls_containment_and_reverse_edges_are_not_contacts(self):
        for kind in ('calls', 'contains', 'returns', 'instantiates'):
            with self.subTest(kind=kind):
                self.graph['links'][0]['kind'] = kind
                self.assertFalse(self.score()['contactSupported'])
        self.graph['links'][0].update(kind='references', source='s', target='m')
        row = self.score()
        self.assertEqual(len(row['connectingRecords']), 1)
        self.assertFalse(row['contactSupported'])

    def test_missing_state_is_distinct_from_missing_edge(self):
        self.graph['links'].clear()
        self.assertEqual(self.score()['status'], 'no_contact_edge')
        self.graph['nodes'].pop()
        self.assertEqual(self.score()['status'], 'missing_state')

    def test_duplicate_candidates_are_never_silently_selected(self):
        for index, status in [(0, 'ambiguous_callable'), (1, 'ambiguous_state')]:
            graph = copy.deepcopy(self.graph)
            node = dict(graph['nodes'][index], id='duplicate')
            graph['nodes'].append(node)
            row = self.score(graph)
            self.assertEqual(row['status'], status)
            self.assertFalse(row['contactSupported'])

    def test_shadow_parameter_and_other_file_do_not_replace_field(self):
        self.graph['nodes'][1].update(kind='parameter', source=dict(file='state.py', startLine=3))
        self.assertEqual(self.score()['status'], 'missing_state')
        self.graph['nodes'][1]['source'] = dict(file='other.py', startLine=2)
        self.assertEqual(self.score()['status'], 'missing_state')

    def test_constructor_uses_exact_source_identity(self):
        node = dict(id='ctor', kind='constructor', name='<init>', source=dict(file='Box.java', startLine=8))
        self.assertEqual(candidates([node], 'compass', 'Box.java', 8, 'Box', constructor=True), [node])
        self.assertEqual(candidates([node], 'compass', 'Box.java', 9, 'Box', constructor=True), [])
        self.assertEqual(candidates([node], 'compass', 'Box.java', 8, 'Box'), [])

    def test_parallel_occurrences_retained_and_order_stable(self):
        self.graph['links'].append(dict(self.graph['links'][0], id='other'))
        row = self.score()
        self.assertEqual(len(row['connectingRecords']), 2)
        self.graph['links'].reverse()
        self.graph['nodes'].reverse()
        self.assertEqual(self.score(), row)

    def test_graphify_representation_and_undirected_flag_preserved(self):
        graph = dict(directed=False, nodes=[
            dict(id='m', label='.get()', source_file='state.py', source_location='L3'),
            dict(id='s', label='value', source_file='state.py', source_location='L2')], links=[
            dict(source='m', target='s', relation='references', source_file='state.py', source_location='L4')])
        row = self.score(graph, 'graphify')
        self.assertTrue(row['selectedLineSupported'])
        self.assertFalse(row['graphDirected'])
        # Contact here is stored endpoint order, not a directed graph path.
        graph['links'][0]['source_location'] = 'unknown'
        self.assertFalse(self.score(graph, 'graphify')['selectedLineSupported'])

    def test_missing_edge_occurrence_cannot_use_declaration_evidence(self):
        edge = self.graph['links'][0]
        del edge['relationshipSite']
        edge['evidence'] = [dict(anchors=[dict(file='state.py', startLine=4)])]
        self.assertTrue(self.score()['contactSupported'])
        self.assertFalse(self.score()['selectedLineSupported'])

    def test_bounded_reader_fails_instead_of_truncating(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input'
            path.write_bytes(b'1234')
            self.assertEqual(read(path, 4), b'1234')
            with self.assertRaises(ValueError):
                read(path, 3)

    def test_graphify_anchor_parser_does_not_guess(self):
        self.assertEqual(anchor(dict(source_file='a', source_location='L42-L50'), 'graphify'), ('a', 42))
        for location in ('line 42', 'L0', 'L42 trailing', ''):
            self.assertEqual(anchor(dict(source_file='a', source_location=location), 'graphify'), ('a', None))


if __name__ == '__main__':
    unittest.main()
