import copy
import unittest

from benchmarks.agent_query.community_identity import resolver_request, selected_id


class CommunityIdentityTests(unittest.TestCase):
    seed = dict(file='src/a.rs', startLine=10, symbol='run')

    def compass(self, nodes=None):
        if nodes is None:
            nodes = [dict(id='a', name='run()', source=dict(file='src/a.rs', startLine=10))]
        return dict(executionSucceeded=True, response=dict(result=dict(structuredContent=dict(
            schema='compass.mcp.tool-result/1', transportTruncation=dict(truncated=False),
            result=dict(schema='compass.query/1', operation='search', truncated=False,
                        nodes=nodes, results=[dict(nodeId=n['id']) for n in nodes], diagnostics=[])))))

    def test_member_variants_use_common_symbol_without_selecting_declaration(self):
        text = '  run [src/a.rs]\n  .run() [src/a.rs]'
        c = resolver_request('compass', text, self.seed)
        self.assertEqual(c['selection']['status'], 'ambiguous')
        self.assertEqual(c['request']['arguments']['query'], 'run')
        g = resolver_request('graphify', text, self.seed)
        self.assertEqual(g['request']['arguments']['label'], 'src/a.rs::run')
        self.assertIsNone(resolver_request('compass', '  run [other.rs]', self.seed)['request'])

    def test_exact_source_match_returns_observed_id_for_both_tools(self):
        self.assertEqual(selected_id('compass', self.compass(), self.seed)['selector'], 'a')
        g = dict(executionSucceeded=True, text='Node: .run()\n  ID: actual_id\n  Source: src/a.rs L10\n  Degree: 4')
        self.assertEqual(selected_id('graphify', g, self.seed)['selector'], 'actual_id')

    def test_duplicate_exact_anchor_is_ambiguous_not_ranked(self):
        nodes = self.compass()['response']['result']['structuredContent']['result']['nodes']
        nodes.append(dict(id='b', name='run()', source=dict(file='src/a.rs', startLine=10)))
        for order in [nodes, list(reversed(nodes))]:
            r = selected_id('compass', self.compass(order), self.seed)
            self.assertEqual(r['matchedIds'], ['a', 'b'])
            self.assertEqual(r['status'], 'ambiguous')
            self.assertIsNone(r['selector'])

    def test_partial_source_symbol_or_different_start_cannot_select_id(self):
        for change in [dict(name='runner()'), dict(source=dict(file='src/b.rs', startLine=10)),
                       dict(source=dict(file='src/a.rs', startLine=1, endLine=20)),
                       dict(source=dict(file='src/a.rs', startLine=True))]:
            node = dict(id='a', name='run()', source=dict(file='src/a.rs', startLine=10))
            node.update(change)
            self.assertIsNone(selected_id('compass', self.compass([node]), self.seed)['selector'])
        for source in ['src/a.rs L9', 'src/b.rs L10', 'src/a.rs unknown']:
            r = selected_id('graphify', dict(executionSucceeded=True,text='Node: run()\n  ID: a\n  Source: '+source),self.seed)
            self.assertIsNone(r['selector'])

    def test_truncation_unknown_schema_and_tool_errors_fail_closed(self):
        for layer in ['transport', 'semantic', 'diagnostic', 'schema', 'failure']:
            call = self.compass()
            e = call['response']['result']['structuredContent']; b = e['result']
            if layer == 'transport': e['transportTruncation']['truncated'] = True
            elif layer == 'semantic': b['truncated'] = True
            elif layer == 'diagnostic': b['diagnostics'] = [dict(code='bounded_truncation')]
            elif layer == 'schema': b['schema'] = 'compass.query/99'
            else: call['executionSucceeded'] = False
            self.assertIsNone(selected_id('compass', call, self.seed)['selector'])

    def test_only_returned_search_hits_can_supply_ids(self):
        call = self.compass()
        call['response']['result']['structuredContent']['result']['results'] = []
        self.assertIsNone(selected_id('compass',call,self.seed)['selector'])

    def test_missing_hit_nodes_duplicate_ids_and_invalid_diagnostics_are_rejected(self):
        for variant in ['missing', 'duplicate', 'diagnostic']:
            call=self.compass(); b=call['response']['result']['structuredContent']['result']
            if variant=='missing': b['results']=[dict(nodeId='not-returned')]
            elif variant=='duplicate': b['nodes'].append(copy.deepcopy(b['nodes'][0]))
            else: b['diagnostics']=['not-an-object']
            self.assertEqual(selected_id('compass',call,self.seed)['status'],'invalid')

    def test_multiple_graphify_ids_and_ambiguity_text_are_unresolved(self):
        for text in ['Ambiguous: run matches 2 nodes.',
                     'Node: run()\n  ID: a\n  ID: b\n  Source: src/a.rs L10']:
            self.assertIsNone(selected_id('graphify',dict(executionSucceeded=True,text=text),self.seed)['selector'])


if __name__ == '__main__':
    unittest.main()
