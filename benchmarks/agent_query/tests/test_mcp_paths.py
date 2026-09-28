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

    def test_prepared_questions_have_a_fixed_count_bound_and_unique_ids(self):
        witness=self.question(undirected=True)
        witness['pathQuestions'] *= 2
        with self.assertRaises(ValueError):prepare_questions({},'graphify',witness)
        witness['pathQuestions'] *= 20
        with self.assertRaises(ValueError):prepare_questions({},'graphify',witness)


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

    def test_failed_execution_cannot_pass_from_error_text(self):
        row=self.row("No node matching source 'missing' found.",source='missing')
        row['executionSucceeded']=False
        self.assertFalse(audit(row,self.graph,{'expected':{'outcome':'unresolved'}})['matched'])

    def test_structured_negative_status_must_agree_with_text(self):
        for outcome,text,target,hops,status in [
            ('depth-limit','No path found within max_hops=0.','b',0,'depth_limit'),
            ('disconnected',"No path found between 'Alpha' and 'Gamma'.",'c',8,'disconnected')]:
            row=self.row(text,target=target,hops=hops)
            payload={'schema':'compass.mcp.path/1','source':'a','target':target,'maxHops':hops,'status':status}
            row['response']={'result':{'structuredContent':{'schema':'compass.mcp.tool-result/1','result':payload}}}
            question={'expected':{'outcome':outcome}}
            self.assertTrue(audit(row,self.graph,question)['matched'])
            payload['status']='found'
            self.assertFalse(audit(row,self.graph,question)['matched'])

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

    def test_structured_ids_disambiguate_but_must_match_the_recorded_edges(self):
        self.graph['nodes'].append({'id':'d','name':'Beta'})
        row=self.row('Shortest path (1 hops):\n  Alpha --calls--> Beta')
        payload={'schema':'compass.mcp.path/1','status':'found','source':'a','target':'b','maxHops':8,'hops':1,
                 'nodes':[{'id':'a','label':'Alpha'},{'id':'b','label':'Beta'}],
                 'steps':[{'from':'a','to':'b','source':'a','target':'b','direction':'forward','relation':'calls','edgeId':None}]}
        row['response']={'result':{'structuredContent':{'schema':'compass.mcp.tool-result/1','result':payload}}}
        question={'expected':{'outcome':'path'}}
        self.assertTrue(audit(row,self.graph,question)['matched'])
        payload['steps'][0]['target']='d'
        self.assertFalse(audit(row,self.graph,question)['matched'])
