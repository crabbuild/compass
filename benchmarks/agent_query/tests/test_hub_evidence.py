import copy
import hashlib
from pathlib import Path
import tempfile
import unittest

from benchmarks.agent_query.hub_evidence_audit import connectivity, check_summary, check_review, check_text_rows, check_request


class HubEvidenceTests(unittest.TestCase):
    def test_requests_cannot_be_relabelled_after_capture(self):
        row=dict(response=dict(id=5),arguments=dict(top_n=10))
        request=dict(jsonrpc='2.0',method='tools/call',id=5,params=dict(name='god_nodes',arguments=dict(top_n=10)))
        check_request(request,row)
        for changed in [dict(top_n=1),dict(top_n=True),dict(top_n=10,extra='bad')]:
            request['params']['arguments']=changed
            with self.assertRaises(ValueError):check_request(request,row)

    def graph(self, directed=True):
        return {'directed': directed, 'nodes':[{'id':'a'}, {'id':'b'}], 'links':[
            {'source':'a','target':'b','kind':'calls'},
            {'source':'a','target':'b','kind':'calls'},
            {'source':'b','target':'a','kind':'references'},
            {'source':'a','target':'a','kind':'calls'},
            {'source':'a','target':'missing','kind':'calls'}]}

    def test_parallel_and_loop_records_are_not_pair_degree(self):
        result=connectivity(self.graph(),'compass','a')
        self.assertEqual(result['edgeRecords'],4)
        self.assertEqual(result['selfLoopRecords'],1)
        self.assertEqual(result['relations'][0],dict(relation='calls',edgeRecords=3,
            incomingRecords=1,outgoingRecords=3,undirectedRecords=0))

    def test_undirected_does_not_invent_arrow_directions(self):
        graph=self.graph(False)
        for edge in graph['links']:
            edge['relation']=edge.pop('kind')
        result=connectivity(graph,'graphify','a')
        self.assertEqual(result['relations'][0],dict(relation='calls',edgeRecords=3,
            incomingRecords=0,outgoingRecords=0,undirectedRecords=3))

    def test_cap_reports_omitted_records_and_stable_order(self):
        graph=self.graph()
        graph['links']=[dict(source='a',target='b',kind=f'r{i:02}') for i in range(20)]
        graph['links'].append(dict(source='a',target='b',kind='r19'))
        result=connectivity(graph,'compass','a')
        self.assertEqual(result['edgeRecords'],21)
        self.assertEqual(result['omittedRelationKinds'],4)
        self.assertEqual(result['omittedRelationRecords'],4)
        self.assertEqual([r['relation'] for r in result['relations'][:2]],['r19','r00'])
        graph['links'].reverse()
        self.assertEqual(result,connectivity(graph,'compass','a'))

    def test_bad_count_direction_and_boolean_counter_fail(self):
        expected=connectivity(self.graph(),'compass','a')
        self.assertTrue(check_summary(expected,expected))
        for key,value in [('edgeRecords',5),('selfLoopRecords',True),('directed',False)]:
            actual=copy.deepcopy(expected);actual[key]=value
            self.assertFalse(check_summary(actual,expected))
        actual=copy.deepcopy(expected);actual['relations'][0]['incomingRecords']=0
        self.assertFalse(check_summary(actual,expected))

    def test_text_rows_must_belong_to_the_right_hub(self):
        graph=self.graph();graph['links']=graph['links'][:1]
        expected=connectivity(graph,'compass','a')
        block='\n    kind: function | incident records: 1 | self-loops: 0\n    relation "calls": incoming 0, outgoing 1, undirected 0'
        text='  1. A - 1 edges'+block+'\n  2. B - 1 edges'
        self.assertTrue(check_text_rows(text,1,expected))
        self.assertFalse(check_text_rows(text,2,expected))
        self.assertFalse(check_text_rows(text.replace('outgoing 1','outgoing 2'),1,expected))

    def test_review_requires_exact_source_and_contains_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);data=b'class Service:\n    pass\n';(root/'a.py').write_bytes(data)
            node={'id':'a','name':'Service','source':{'file':'a.py','startLine':1}}
            review=dict(id='a',file='a.py',line=1,sourceText=data.decode().rstrip('\n'),
                sourceFileSha256=hashlib.sha256(data).hexdigest())
            check_review(review,node,'compass',root)
            review['sourceText']='wrong'
            with self.assertRaises(ValueError):check_review(review,node,'compass',root)
            node['source']['file']='../escape.py';review['file']='../escape.py'
            with self.assertRaises(ValueError):check_review(review,node,'compass',root)


if __name__ == '__main__':
    unittest.main()
