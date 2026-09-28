import unittest
from benchmarks.agent_query.mcp_audit import audit


class McpAuditTests(unittest.TestCase):
    def test_new_panel_repository_must_be_captured_exactly_once(self):
        from benchmarks.agent_query.mcp_compare import captured_repository
        record = {'repository': 'chi'}
        self.assertEqual(captured_repository({'repositories': [record]}, 'chi'), record)
        for records in [[], [record, record]]:
            with self.assertRaises(ValueError):
                captured_repository({'repositories': records}, 'chi')

    def test_repository_keys_cannot_escape_raw_capture_directory(self):
        from benchmarks.agent_query.mcp_compare import captured_repository
        for name in ['../chi', '/chi', 'chi/other', 'chi\\other', '.', '', 'a' * 81, None]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                captured_repository({'repositories': [{'repository': name}]}, name)

    def test_connection_failure_does_not_remove_remaining_questions(self):
        from benchmarks.agent_query.mcp_compare import skipped_results
        questions=[('hubs','god_nodes',{'top_n':10}),('community','get_community',{'community_id':0})]
        rows=skipped_results('cobra','compass',questions,'digest',['compass'])
        self.assertEqual([r['question'] for r in rows],['hubs','community'])
        self.assertTrue(all(not r['executionSucceeded'] and r['captureError'] for r in rows))

    def setUp(self):
        self.graph={'nodes':[{'id':'a','name':'Alpha','community':{'id':2},'source':{'file':'a.rs','startLine':1}},
                             {'id':'b','name':'Beta','community':{'id':2},'source':{'file':'b.rs','startLine':1}}],
                    'links':[{'source':'a','target':'b','kind':'calls'}]}

    def row(self,kind,text,arguments=None):
        return {'repository':'fixture','tool':'compass','question':kind,'text':text,
                'arguments':arguments or {},'executionSucceeded':True}

    def test_missing_real_community_fails(self):
        r=audit(self.row('community','Community 2 not found.',{'community_id':2}),self.graph)
        self.assertFalse(r['membershipMatches']);self.assertEqual(r['missingMembers'],2)

    def test_membership_multiset_and_header(self):
        text='Community 2 — Core (2 nodes):\n  Alpha [a.rs]\n  Beta [b.rs]'
        self.assertTrue(audit(self.row('community',text,{'community_id':2}),self.graph)['membershipMatches'])
        self.assertFalse(audit(self.row('community',text.replace('Beta [b.rs]','Alpha [a.rs]'),{'community_id':2}),self.graph)['membershipMatches'])

    def test_unambiguous_display_does_not_prove_overloaded_identity(self):
        self.graph['nodes'][1]['name']='Alpha'
        r=audit(self.row('hubs','God nodes (most connected):\n  1. Alpha - 1 edges'),self.graph)
        self.assertEqual(r['verifiedIdentities'],0)
        self.assertEqual(r['matchingDegrees'],0)

    def test_degree_collapses_parallel_pairs_and_counts_self_loop_twice(self):
        self.graph['links'] += [dict(self.graph['links'][0]),{'source':'a','target':'a','kind':'calls'}]
        r=audit(self.row('hubs','God nodes (most connected):\n  1. Alpha - 3 edges'),self.graph)
        self.assertEqual(r['matchingDegrees'],1)

    def test_explicit_hub_identity_must_exist_and_match_source(self):
        self.graph['nodes'][1]['name']='Alpha'
        row=self.row('hubs','God nodes (most connected):\n  1. Alpha - 1 edges')
        record={'id':'a','label':'Alpha','degree':1,'rank':1,'sourceFile':'a.rs','startLine':1}
        row['response']={'result':{'structuredContent':{'schema':'compass.mcp.tool-result/1',
            'result':{'schema':'compass.mcp.hubs/1','nodes':[record]}}}}
        checked=audit(row,self.graph)
        self.assertEqual(checked['explicitIdentities'],1)
        self.assertEqual(checked['matchingSourceAnchors'],1)
        record['sourceFile']='b.rs'
        self.assertEqual(audit(row,self.graph)['matchingSourceAnchors'],0)
        record['id']='missing'
        self.assertEqual(audit(row,self.graph)['verifiedIdentities'],0)

    def test_structured_hubs_cannot_hide_missing_or_duplicate_rows(self):
        row=self.row('hubs','God nodes (most connected):\n  1. Alpha - 1 edges\n  2. Alpha - 1 edges')
        nodes=[{'id':'a','label':'Alpha','degree':1,'rank':rank,'sourceFile':'a.rs','startLine':1} for rank in (1,2)]
        row['response']={'result':{'structuredContent':{'schema':'compass.mcp.tool-result/1',
            'result':{'schema':'compass.mcp.hubs/1','nodes':nodes}}}}
        self.assertEqual(audit(row,self.graph)['verifiedIdentities'],1)
        nodes.pop()
        with self.assertRaises(ValueError):audit(row,self.graph)

    def test_silent_selection_is_not_ambiguity(self):
        r=audit(self.row('ambiguous-neighbors','Neighbors of ambiguous_name:\n  --> Alpha [calls] [EXTRACTED]'),self.graph)
        self.assertFalse(r['ambiguityPreserved'])
        self.assertTrue(audit(self.row('ambiguous-neighbors','Ambiguous: choose an ID'),self.graph)['ambiguityPreserved'])

    def test_neighbor_direction_and_missing_relationship(self):
        args={'label':'a'}
        self.assertTrue(audit(self.row('neighbors','Neighbors of Alpha:\n  --> Beta [calls] [EXTRACTED]',args),self.graph)['displayedPairsMatch'])
        self.assertFalse(audit(self.row('neighbors','Neighbors of Alpha:\n  <-- Beta [calls] [EXTRACTED]',args),self.graph)['displayedPairsMatch'])

    def test_stats_dont_accept_missing_communities(self):
        r=audit(self.row('stats','Nodes: 2\nEdges: 1\nCommunities: 0'),self.graph)
        self.assertFalse(r['graphCountsMatch'])
