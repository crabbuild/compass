import unittest
from benchmarks.agent_query.mcp_compare import prepare_questions
from benchmarks.agent_query.mcp_path_audit import audit, parse_path


class PreparedPathTests(unittest.TestCase):
    def question(self, **extra):
        return {'pathQuestions':[{'id':'path', 'arguments':{
            'graphify':{'source':'a','target':'b','max_hops':8,**extra}}}]}

    def test_direction_is_explicit_for_shared_navigation(self):
        with self.assertRaises(ValueError):prepare_questions({},'graphify',self.question())
        result=prepare_questions({},'graphify',self.question(undirected=True))
        self.assertEqual(result[0][1],'shortest_path')

    def test_prepared_endpoint_is_not_changed_by_graph_lookup(self):
        result=prepare_questions({},'graphify',self.question(undirected=True))
        self.assertEqual(result[0][2]['source'],'a')

    def test_invalid_hop_bound_and_unknown_argument_fail(self):
        for extra in [{'undirected':True,'max_hops':True}, {'undirected':True,'other':'bad'}]:
            with self.assertRaises(ValueError):prepare_questions({},'graphify',self.question(**extra))


class PathAuditTests(unittest.TestCase):
    def setUp(self):
        self.graph={'nodes':[{'id':'a','name':'Alpha'},{'id':'b','name':'Beta'},{'id':'c','name':'Gamma'}],
                    'links':[{'source':'a','target':'b','kind':'calls'},
                             {'source':'a','target':'b','kind':'references'}]}

    def row(self,text,source='a',target='b',hops=8):
        return {'repository':'fixture','tool':'compass','question':'q','text':text,
                'arguments':{'source':source,'target':target,'max_hops':hops},'executionSucceeded':True}

    def check(self,text,outcome='path',**kwargs):
        return audit(self.row(text,**kwargs),self.graph,{'expected':{'outcome':outcome}})

    def test_parallel_relations_and_reverse_navigation(self):
        self.assertTrue(self.check('Shortest path (1 hops):\n  Alpha --calls/references [EXTRACTED]--> Beta')['matched'])
        self.assertTrue(self.check('Shortest path (1 hops):\n  Beta <--calls-- Alpha',source='b',target='a')['matched'])
        self.assertFalse(self.check('Shortest path (1 hops):\n  Alpha <--calls-- Beta')['matched'])

    def test_endpoint_echo_and_unsupported_relation_are_not_paths(self):
        self.assertFalse(self.check('Alpha Beta')['matched'])
        self.assertFalse(self.check('Shortest path (1 hops):\n  Alpha --imports--> Beta')['matched'])
        with self.assertRaises(ValueError):parse_path('Shortest path (2 hops):\n  Alpha --calls--> Beta')

    def test_missing_and_ambiguous_identity_are_distinct(self):
        self.assertFalse(self.check('warning: ambiguous\nShortest path (1 hops):\n  Alpha --calls--> Beta','ambiguous')['matched'])
        self.assertTrue(self.check('Ambiguous endpoint: choose an ID','ambiguous')['matched'])
        self.assertTrue(self.check("No node matching source 'missing' found.",'unresolved',source='missing')['matched'])

    def test_limit_does_not_prove_disconnection(self):
        self.assertTrue(self.check('Path exceeds max_hops=0 (1 hops found).','depth-limit',hops=0)['matched'])
        self.assertFalse(self.check("No path found between 'Alpha' and 'Beta'.",'depth-limit',hops=0)['matched'])
        self.assertTrue(self.check("No path found between 'Alpha' and 'Gamma'.",'disconnected',target='c')['matched'])

    def test_colliding_display_label_does_not_identify_a_path(self):
        self.graph['nodes'].append({'id':'d','name':'Beta'})
        checked=self.check('Shortest path (1 hops):\n  Alpha --calls--> Beta')
        self.assertFalse(checked['matched'])
        self.assertIn('identity',checked['failure'])

    def test_graphify_direction_uses_semantic_markers(self):
        graph={'nodes':[{'id':'a','label':'Alpha'},{'id':'b','label':'Beta'}],
               'links':[{'source':'b','target':'a','_src':'a','_tgt':'b','relation':'calls'}]}
        row=self.row('Shortest path (1 hops):\n  Alpha --calls [EXTRACTED]--> Beta')
        row['tool']='graphify'
        self.assertTrue(audit(row,graph,{'expected':{'outcome':'path'}})['matched'])

    def test_label_requests_keep_independent_expected_endpoint_ids(self):
        row=self.row('Shortest path (1 hops):\n  Alpha --calls--> Beta',source='Alpha',target='Beta')
        question={'expected':{'outcome':'path','endpointIds':{'compass':['a','b']}}}
        self.assertTrue(audit(row,self.graph,question)['matched'])
