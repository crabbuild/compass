import copy
import unittest

from benchmarks.agent_query.neighbor_identity import audit_direct, destination_request


class NeighborIdentityTests(unittest.TestCase):
    def fixture(self):
        graph=dict(directed=True,nodes=[dict(id='a',name='run'),dict(id='b',name='close'),dict(id='c',name='close')],
                   links=[dict(id='call',source='a',target='b',kind='calls',evidence=[dict(origin='ast')])])
        body=dict(schema='compass.query.neighbors/1', directionBasis='stored-endpoints',graphDirected=True,
                  seed=graph['nodes'][0], relationFilter='calls', truncated=False,
                  neighbors=[dict(direction='outgoing',node=graph['nodes'][1],edges=graph['links'])])
        call=dict(executionSucceeded=True,response=dict(result=dict(structuredContent=dict(
            schema='compass.mcp.tool-result/1',transportTruncation=dict(truncated=False),result=copy.deepcopy(body)))))
        return graph,call

    def test_explicit_identity_is_supported_even_when_label_collides(self):
        graph,call=self.fixture()
        result=audit_direct(call,graph,'compass','a','b')
        self.assertTrue(result['explicitDestinationSupported'])
        self.assertTrue(result['fullProjectionMatches'])

    def test_mutated_ids_records_anchors_direction_and_truncation_cannot_pass(self):
        for variant in ['id','edge','direction','seed','node','duplicate','missing','truncated','schema','transport']:
            graph,call=self.fixture(); envelope=call['response']['result']['structuredContent']; b=envelope['result']
            if variant=='id': b['neighbors'][0]['node']['id']='c'
            elif variant=='edge': b['neighbors'][0]['edges'][0]['evidence'][0]['origin']='invented'
            elif variant=='direction': b['neighbors'][0]['direction']='incoming'
            elif variant=='seed': b['seed']['id']='c'
            elif variant=='node': b['neighbors'][0]['node']['source']=dict(file='invented.rs',startLine=1)
            elif variant=='duplicate': b['neighbors'].append(copy.deepcopy(b['neighbors'][0]))
            elif variant=='missing': b['neighbors'][0]['edges']=[]
            elif variant=='truncated': b['truncated']=True
            elif variant=='schema': b['schema']='compass.query.neighbors/99'
            else: envelope['transportTruncation']['truncated']=True
            self.assertFalse(audit_direct(call,graph,'compass','a','b')['explicitDestinationSupported'],variant)

    def test_legacy_text_does_not_turn_unique_labels_into_explicit_ids(self):
        graph,call=self.fixture()
        call['response']['result'].pop('structuredContent')
        call['text']='Neighbors of run:\n  --> close [calls] [EXTRACTED]'
        for tool in ['compass','graphify']:
            self.assertFalse(audit_direct(call,graph,tool,'a','b')['explicitDestinationSupported'])

    def test_parallel_record_loss_and_extra_records_fail_projection(self):
        graph,call=self.fixture()
        graph['links'].append(copy.deepcopy(graph['links'][0]))
        self.assertFalse(audit_direct(call,graph,'compass','a','b')['fullProjectionMatches'])
        call['response']['result']['structuredContent']['result']['neighbors'][0]['edges'].append(copy.deepcopy(graph['links'][0]))
        self.assertTrue(audit_direct(call,graph,'compass','a','b')['fullProjectionMatches'])

    def test_followup_uses_only_returned_outgoing_symbol_and_public_coordinates(self):
        target=dict(file='src/a.py',symbol='close',startLine=10)
        call=dict(executionSucceeded=True,text='Neighbors of run:\n  --> close() [calls] [EXTRACTED]')
        self.assertEqual(destination_request('compass',call,target)['arguments']['query'],'close')
        self.assertEqual(destination_request('graphify',call,target)['arguments']['label'],'src/a.py::close')
        for text in ['  <-- close() [calls] [EXTRACTED]','  --> close() [contains] [EXTRACTED]', '  --> other() [calls] [EXTRACTED]']:
            self.assertIsNone(destination_request('compass',dict(executionSucceeded=True,text=text),target))
        self.assertIsNone(destination_request('compass',dict(executionSucceeded=False,text=call['text']),target))


if __name__=='__main__': unittest.main()
