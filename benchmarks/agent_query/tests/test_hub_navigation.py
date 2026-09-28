import unittest

from benchmarks.agent_query.hub_navigation import check_navigation, selectors, server_paths


class HubNavigationTests(unittest.TestCase):
    def test_server_launch_preserves_the_captured_virtual_environment(self):
        run={'servers':{'graphify':{'executable':'/global/python'}},
             'results':[{'tool':'graphify','argv':['/isolated/env/bin/python']},
                        {'tool':'compass','argv':['/frozen/compass']}]}
        self.assertEqual(server_paths(run)['graphify'],'/isolated/env/bin/python')

    def test_legacy_selector_is_the_returned_label(self):
        row={'response':{'result':{}},'text':'God nodes (most connected):\n  1. run() - 9 edges'}
        self.assertEqual(selectors(row),[(1,'run()','display-label')])

    def test_explicit_selector_preserves_id_bytes(self):
        row={'response':{'result':{'structuredContent':{
            'schema':'compass.mcp.tool-result/1',
            'result':{'schema':'compass.mcp.hubs/1','nodes':[{'id':'A"\\\n','rank':1}]}}}}}
        self.assertEqual(selectors(row),[(1,'A"\\\n','explicit-id')])

    def test_unknown_identity_cannot_be_resolved_by_matching_neighbors(self):
        graph={'nodes':[{'id':'a','name':'Alpha'}],'links':[]}
        result=check_navigation('Neighbors of Alpha:',graph,'compass',None)
        self.assertFalse(result['navigationResolved'])

    def test_neighbor_labels_preserve_multiplicity_between_distinct_nodes(self):
        graph={'nodes':[{'id':'a','name':'Alpha'},{'id':'b','name':'run()'},
                        {'id':'c','name':'run()'}],
               'links':[{'source':'a','target':'b'},{'source':'a','target':'c'},
                        {'source':'a','target':'b'}]}
        one='Neighbors of Alpha:\n  --> run() [calls] [EXTRACTED]'
        self.assertFalse(check_navigation(one,graph,'compass','a')['navigationResolved'])
        two=one+'\n  --> run() [calls] [EXTRACTED]'
        self.assertTrue(check_navigation(two,graph,'compass','a')['navigationResolved'])
        self.assertFalse(check_navigation(two.replace('-->','<--'),graph,'compass','a')['navigationResolved'])
