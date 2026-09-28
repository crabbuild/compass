import copy
import itertools
import unittest

from benchmarks.agent_query.public_neighbor_paths import (
    PublicWalk, WorkflowStop, graphify_node, neighbors,
)


BOUNDS = dict(maxDepth=8, maxExpandedNodes=128, maxRequests=512,
              maxOutgoingGroups=8192, maxWorkflowResponseBytes=16777216,
              workflowTimeoutSeconds=180, graphifyTokenBudget=262144)


def node(identifier):
    return dict(id=identifier, label=identifier, file='fixture.rs', line=ord(identifier[0]))


def raw_node(identifier):
    n = node(identifier)
    return dict(id=identifier, name=n['label'], source=dict(file=n['file'], startLine=n['line']))


def compass_response(seed, targets):
    body = dict(schema='compass.query.neighbors/1', directionBasis='stored-endpoints',
                graphDirected=True, relationFilter='calls', truncated=False,
                seed=raw_node(seed), neighbors=[])
    for target in targets:
        body['neighbors'].append(dict(direction='outgoing', node=raw_node(target), edges=[
            dict(source=seed, target=target, kind='calls', id=seed+target,
                 evidence=[dict(origin='ast', confidence='exact')])]))
    return dict(executionSucceeded=True, wireResponseBytes=100, response=dict(result=dict(
        structuredContent=dict(schema='compass.mcp.tool-result/1', result=body,
                               transportTruncation=dict(truncated=False)))))


def graphify_response(text):
    return dict(executionSucceeded=True, text=text, wireResponseBytes=100)


class PublicNeighborPathTests(unittest.TestCase):
    def walk(self, edges, **bounds):
        def invoke(method, args):
            self.assertEqual(method, 'get_neighbors')
            self.assertEqual(args['relation_filter'], 'calls')
            return compass_response(args['label'], edges.get(args['label'], []))
        return PublicWalk('compass', invoke, BOUNDS | bounds)

    def test_breadth_first_directed_deterministic_and_cycles(self):
        for ordering in [['d', 'b'], ['b', 'd']]:
            walk = self.walk(dict(a=ordering, b=['a', 'c'], c=['z'], d=['z']))
            result = walk.walk(node('a'), node('z'))
            self.assertEqual([n['id'] for n in result['nodes']], ['a', 'd', 'z'])
            self.assertEqual(walk.expanded, 3)

    def test_same_node_needs_no_request(self):
        walk = self.walk({})
        self.assertEqual(walk.walk(node('a'), node('a'))['steps'], [])
        self.assertEqual(walk.metrics()['requests'], 0)

    def test_depth_limit_and_closed_cycle_never_prove_global_absence(self):
        for edges, frontier in [(dict(a=['b'], b=['z']), True), (dict(a=['a']), False)]:
            result = self.walk(edges, maxDepth=1).walk(node('a'), node('z'))
            self.assertEqual(result['status'], 'no-path-in-public-projection')
            self.assertEqual(result['depthFrontier'], frontier)
            self.assertFalse(result['globalAbsenceProven'])

    def test_request_node_group_byte_and_time_bounds(self):
        for bounds, reason in [({'maxRequests': 1}, 'request-limit'),
                               ({'maxExpandedNodes': 1}, 'expanded-node-limit'),
                               ({'maxOutgoingGroups': 0}, 'outgoing-group-limit'),
                               ({'maxWorkflowResponseBytes': 99}, 'workflow-response-byte-limit'),
                               ({'workflowTimeoutSeconds': 0}, 'workflow-time-limit')]:
            with self.subTest(bounds=bounds), self.assertRaisesRegex(WorkflowStop, reason):
                self.walk(dict(a=['b'], b=['z']), **bounds).walk(node('a'), node('z'))

    def test_endpoint_requests_share_budget(self):
        walk = self.walk(dict(a=['z']), maxRequests=1)
        walk.request('get_neighbors', dict(label='a', relation_filter='calls'))
        with self.assertRaisesRegex(WorkflowStop, 'request-limit'):
            walk.walk(node('a'), node('z'))

    def test_transport_failure_stops_without_retry(self):
        walk = PublicWalk('compass', lambda *_: dict(captureError='closed'), BOUNDS)
        with self.assertRaisesRegex(WorkflowStop, 'transport-error'):
            walk.walk(node('a'), node('z'))
        self.assertEqual(len(walk.calls), 1)

    def test_incoming_and_heuristic_edges_excluded(self):
        response = compass_response('a', ['b', 'c'])
        groups = response['response']['result']['structuredContent']['result']['neighbors']
        groups[0]['direction'] = 'incoming'
        groups[1]['edges'][0]['evidence'][0]['confidence'] = 'heuristic'
        self.assertEqual(neighbors('compass', response, node('a')), [])

    def test_bad_identity_direction_filter_record_or_truncation_stops(self):
        for field in ['seed', 'direction', 'filter', 'edge', 'transport', 'truncated']:
            response = compass_response('a', ['z'])
            env = response['response']['result']['structuredContent']; body = env['result']
            if field == 'seed': body['seed']['id'] = 'wrong'
            elif field == 'direction': body['directionBasis'] = 'undirected'
            elif field == 'filter': body['relationFilter'] = ''
            elif field == 'edge': body['neighbors'][0]['edges'][0]['target'] = 'wrong'
            elif field == 'transport': env['transportTruncation']['truncated'] = True
            else: body['truncated'] = True
            with self.subTest(field=field), self.assertRaises(WorkflowStop):
                neighbors('compass', response, node('a'))

    def test_parallel_records_retained(self):
        response = compass_response('a', ['z'])
        group = response['response']['result']['structuredContent']['result']['neighbors'][0]
        extra = copy.deepcopy(group['edges'][0]); extra['id'] = 'second'; group['edges'].append(extra)
        self.assertEqual(len(neighbors('compass', response, node('a'))[0]['records']), 2)

    def test_graphify_resolver_refuses_ambiguity_fuzzy_labels_and_missing_anchors(self):
        valid = graphify_response('Node: close()\n  ID: abc\n  Source: a.py L10-L20')
        self.assertEqual(graphify_node(valid, 'close()')['id'], 'abc')
        self.assertIsNone(graphify_node(valid, 'close'))
        for text in ['Ambiguous: close\n id: abc', 'Node: close()\n  ID: abc',
                     valid['text']+'\nNode: close()']:
            self.assertIsNone(graphify_node(graphify_response(text), 'close()'))

    def test_graphify_text_direction_relation_confidence_and_truncation(self):
        text = ('Neighbors of a:\n  --> z [calls] [EXTRACTED] at=a.py:L3\n'
                '  <-- c [calls] [EXTRACTED]\n  --> d [calls] [INFERRED]\n'
                '  --> e [calls_extra] [EXTRACTED]')
        self.assertEqual([g['label'] for g in neighbors('graphify', graphify_response(text), node('a'))], ['z'])
        for malformed in ['[!] TRUNCATED\n'+text, text+'\n... truncated', text.replace('Neighbors of a', 'Neighbors of b')]:
            with self.assertRaises(WorkflowStop):
                neighbors('graphify', graphify_response(malformed), node('a'))

    def test_graphify_labels_resolved_once_failures_cached_and_costed(self):
        def invoke(method, args):
            if method == 'get_node':
                return graphify_response('Node: b\n  ID: b\n  Source: fixture.rs L98') if args['label'] == 'b' else graphify_response('Ambiguous: missing')
            self.assertEqual(args['relation_filter'], 'calls')
            return graphify_response('Neighbors of '+args['label']+':\n  --> b [calls] [EXTRACTED]\n  --> missing [calls] [EXTRACTED]')
        walk = PublicWalk('graphify', invoke, BOUNDS)
        result = walk.walk(node('a'), node('z'))
        self.assertEqual(len(walk.calls), 4)  # two expansions, two cached resolutions
        self.assertEqual(walk.bytes, 400)
        self.assertTrue(result['identityIncomplete'])
        self.assertEqual(len(walk.identity_gaps), 2)

    def test_graphify_success_retains_sites_and_charges_bridge_lookups(self):
        def invoke(method, args):
            if method == 'get_node':
                n = node(args['label'])
                return graphify_response(f"Node: {n['label']}\n  ID: {n['id']}\n  Source: fixture.rs L{n['line']}")
            target = dict(a='b', b='z')[args['label']]
            return graphify_response(f"Neighbors of {args['label']}:\n  --> {target} [calls] [EXTRACTED] at=fixture.rs:L2")
        walk = PublicWalk('graphify', invoke, BOUNDS)
        result = walk.walk(node('a'), node('z'))
        self.assertEqual([n['id'] for n in result['nodes']], ['a', 'b', 'z'])
        self.assertEqual(result['steps'][0]['records'][0]['site'], 'fixture.rs:L2')
        self.assertEqual(walk.metrics()['requests'], 4)

    def test_independent_simple_path_oracle_all_four_node_directed_graphs(self):
        # All 4096 directed simple graphs, including cycles. Enumerate simple
        # paths by permutations independently of the production FIFO walk.
        vertices = 'abcd'
        pairs = [(a, b) for a in vertices for b in vertices if a != b]
        for mask in range(1 << len(pairs)):
            edges = {v: [] for v in vertices}
            for i, (a, b) in enumerate(pairs):
                if mask & (1 << i): edges[a].append(b)
            lengths = []
            for count in range(3):
                for middle in itertools.permutations('bc', count):
                    route = ('a', *middle, 'd')
                    if all(b in edges[a] for a, b in zip(route, route[1:])):
                        lengths.append(len(route)-1)
            for depth in [1, 2, 3]:
                result = self.walk(edges, maxDepth=depth).walk(node('a'), node('d'))
                expected = min(lengths) if lengths and min(lengths) <= depth else None
                actual = len(result['steps']) if result['status'] == 'candidate-path' else None
                self.assertEqual(actual, expected, (mask, depth))


if __name__ == '__main__':
    unittest.main()
