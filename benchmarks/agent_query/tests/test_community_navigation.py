import unittest

from benchmarks.agent_query.community_navigation import score_navigation, select_label


class CommunityNavigationTests(unittest.TestCase):
    seed = dict(file='a.rs', symbol='run', startLine=10)

    def graph(self):
        return dict(nodes=[dict(id='a', name='Runner.run()', label='Runner.run()'),
                           dict(id='b', name='finish()', label='finish()')],
                    links=[dict(source='a', target='b', kind='calls', relation='calls')])

    def test_selector_uses_exact_file_and_terminal_symbol(self):
        text = 'Community 0 (4 nodes):\n  Runner.run() [a.rs]\n  run_more [a.rs]\n  run [b.rs]\n  Runner [a.rs]'
        result = select_label(text, self.seed)
        self.assertEqual(result['selector'], 'Runner.run()')
        self.assertEqual(len(result['matchingRows']), 1)

    def test_identical_rows_are_retained_without_inventing_an_id(self):
        result = select_label('  run [a.rs]\n  run [a.rs]', self.seed)
        self.assertEqual(result['selector'], 'run')
        self.assertEqual(len(result['matchingRows']), 2)

    def test_distinct_labels_are_ambiguous_not_first_match(self):
        result = select_label('  A.run() [a.rs]\n  B.run() [a.rs]', self.seed)
        self.assertIsNone(result['selector'])
        self.assertEqual(result['status'], 'ambiguous')

    def test_missing_or_incomplete_member_rows_do_not_match(self):
        for text in ['', '  run [a.rs', '  run [b.rs]', '  runner [a.rs]']:
            self.assertEqual(select_label(text, self.seed)['status'], 'missing')

    def test_unique_seed_and_target_are_supported_for_both_tools(self):
        for tool in ['compass', 'graphify']:
            result = score_navigation('Neighbors of Runner.run():\n  --> finish() [calls] [EXTRACTED] at=a.rs:L12', self.graph(), tool, 'a', 'b', True)
            self.assertTrue(result['seedIdentitySupported'])
            self.assertTrue(result['collaboratorIdentitySupported'])
            self.assertTrue(result['displayedAdjacencyMatches'])

    def test_matching_adjacency_does_not_resolve_duplicate_seed(self):
        graph = self.graph()
        graph['nodes'].append(dict(id='duplicate', name='Runner.run()'))
        result = score_navigation('Neighbors of Runner.run():\n  --> finish() [calls] [exact]', graph, 'compass', 'a', 'b', True)
        self.assertTrue(result['displayedAdjacencyMatches'])
        self.assertTrue(result['collaboratorDisplayed'])
        self.assertFalse(result['seedIdentitySupported'])
        self.assertFalse(result['collaboratorIdentitySupported'])

    def test_duplicate_target_label_is_not_identity_support(self):
        graph = self.graph()
        graph['nodes'].append(dict(id='other', name='finish()'))
        result = score_navigation('Neighbors of Runner.run():\n  --> finish() [calls] [exact]', graph, 'compass', 'a', 'b', True)
        self.assertTrue(result['seedIdentitySupported'])
        self.assertTrue(result['collaboratorDisplayed'])
        self.assertFalse(result['collaboratorIdentitySupported'])

    def test_failure_wrong_direction_or_relation_cannot_reach_collaborator(self):
        for text, success in [
            ('Neighbors of Runner.run():\n  --> finish() [calls] [exact]', False),
            ('Neighbors of Runner.run():\n  <-- finish() [calls] [exact]', True),
            ('Neighbors of Runner.run():\n  --> finish() [contains] [exact]', True),
            ('Ambiguous: run matches 2 nodes.', True),
        ]:
            result = score_navigation(text, self.graph(), 'compass', 'a', 'b', success)
            self.assertFalse(result['collaboratorIdentitySupported'])
            self.assertFalse(result['collaboratorDisplayed'])

    def test_parallel_calls_are_distinct_neighbors_not_occurrence_recall(self):
        graph = self.graph()
        graph['links'] *= 2
        result = score_navigation('Neighbors of Runner.run():\n  --> finish() [calls] [exact]', graph, 'compass', 'a', 'b', True)
        self.assertEqual(result['graphDirectCallRecords'], 2)
        self.assertEqual(result['expectedDisplayedNeighbors'], 1)
        self.assertTrue(result['displayedAdjacencyMatches'])

    def test_missing_target_oracle_does_not_hide_valid_seed_navigation(self):
        result = score_navigation('Neighbors of Runner.run():\n  --> finish() [calls] [exact]',
                                  self.graph(), 'compass', 'a', None, True)
        self.assertTrue(result['seedIdentitySupported'])
        self.assertTrue(result['displayedAdjacencyMatches'])
        self.assertFalse(result['collaboratorDisplayed'])
        self.assertFalse(result['collaboratorIdentitySupported'])
        self.assertEqual(result['collaboratorLabelCandidates'], [])

    def test_mismatch_diagnostics_have_stable_sorted_order(self):
        graph = self.graph()
        graph['nodes'].extend([dict(id='x', name='Zulu()'), dict(id='y', name='Alpha()')])
        graph['links'].extend([dict(source='a', target='x', kind='calls'),
                               dict(source='a', target='y', kind='calls')])
        text = 'Neighbors of Runner.run():\n  --> ZExtra() [calls] [exact]\n  --> AExtra() [calls] [exact]'
        result = score_navigation(text, graph, 'compass', 'a', 'b', True)
        self.assertEqual(result['missingDisplayedNeighbors'],
                         sorted([('-->', 'finish()'), ('-->', 'Zulu()'), ('-->', 'Alpha()')]))
        self.assertEqual(result['extraDisplayedNeighbors'],
                         [('-->', 'AExtra()'), ('-->', 'ZExtra()')])

    def test_duplicate_neighbor_names_preserve_multiplicity_of_identities(self):
        graph = self.graph()
        graph['nodes'].append(dict(id='other', name='finish()'))
        graph['links'].append(dict(source='a', target='other', kind='calls'))
        result = score_navigation('Neighbors of Runner.run():\n  --> finish() [calls] [exact]', graph, 'compass', 'a', 'b', True)
        self.assertEqual(result['expectedDisplayedNeighbors'], 2)
        self.assertFalse(result['displayedAdjacencyMatches'])


if __name__ == '__main__':
    unittest.main()
