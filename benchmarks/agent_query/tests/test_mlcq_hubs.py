import unittest

from benchmarks.agent_query.mlcq_hubs import evaluate_ranking


def node(identifier, name='C', file='C.java', line=1):
    return dict(id=identifier, label=name, source_file=file, source_location=f'L{line}')


def score(nodes, text='  1. C - 1 edges', success=True, rating='unanimous-major-critical'):
    return evaluate_ranking('graphify', 'r', dict(text=text, executionSucceeded=success),
        dict(nodes=nodes, links=[]),
        [dict(repository='r', sampleId=1, classification=rating)],
        [dict(repository='r', sampleId=1, file='C.java', line=1, symbol='C')])


class MlcqHubTests(unittest.TestCase):
    def test_exact_unique_identity_and_degree_are_separate(self):
        result = score([node('c')])
        self.assertEqual(result['cases'][0]['atCutoff']['10'], 'retrieved')
        self.assertFalse(result['graphConsistency']['hubs'][0]['degreeMatches'])

    def test_unlabeled_hubs_are_not_false_positives(self):
        result = score([node('c'), node('d', 'D')], '  1. D - 7 edges')
        self.assertEqual(result['cutoffs']['10']['unanimous-major-critical'], {'not-returned': 1})
        self.assertEqual(result['cutoffs']['10']['unanimous-none'], {})

    def test_duplicate_label_cannot_borrow_source_identity(self):
        result = score([node('c'), node('d', file='Other.java')])
        self.assertEqual(result['cases'][0]['graphIds'], ['c'])
        self.assertEqual(result['cases'][0]['atCutoff']['100'], 'ambiguous-output')
        self.assertIsNone(result['cases'][0]['rank'])

    def test_missing_and_ambiguous_source_are_distinct(self):
        self.assertEqual(score([node('x', file='X.java')])['cases'][0]['atCutoff']['10'], 'source-missing')
        self.assertEqual(score([node('c'), node('d')])['cases'][0]['atCutoff']['10'], 'source-ambiguous')

    def test_failed_capture_keeps_denominator(self):
        result = score([], success=False)
        self.assertEqual(result['cutoffs']['100']['unanimous-major-critical'], {'capture-unavailable': 1})

    def test_none_rating_is_not_treated_as_positive(self):
        result = score([node('c')], rating='unanimous-none')
        self.assertEqual(result['cutoffs']['10']['unanimous-none'], {'retrieved': 1})
        self.assertEqual(result['cutoffs']['10']['unanimous-major-critical'], {})

    def test_cutoffs_do_not_use_future_rank(self):
        nodes = [node(str(i), f'D{i}', f'D{i}.java') for i in range(10)] + [node('c')]
        text = '\n'.join(f"  {i+1}. {n['label']} - 1 edges" for i,n in enumerate(nodes))
        result = score(nodes, text)['cases'][0]
        self.assertEqual(result['rank'], 11)
        self.assertEqual(result['atCutoff'], {'10':'not-returned', '50':'retrieved', '100':'retrieved'})

    def test_compass_explicit_id_resolves_duplicate_display_names(self):
        nodes=[dict(id='c',name='C',source=dict(file='C.java',startLine=1)),
               dict(id='d',name='C',source=dict(file='Other.java',startLine=1))]
        record=dict(id='c',label='C',rank=1,degree=1,sourceFile='C.java',startLine=1)
        call=dict(executionSucceeded=True,text='  1. C - 1 edges',response=dict(result=dict(
            structuredContent=dict(schema='compass.mcp.tool-result/1',result=dict(
                schema='compass.mcp.hubs/1',nodes=[record])))))
        samples=[dict(repository='r',sampleId=1,classification='unanimous-none')]
        witnesses=[dict(repository='r',sampleId=1,file='C.java',line=1,symbol='C')]
        result=evaluate_ranking('compass','r',call,dict(nodes=nodes,links=[]),samples,witnesses)
        self.assertEqual(result['cases'][0]['atCutoff']['10'],'retrieved')
        record['id']='invented'
        with self.assertRaises(ValueError):
            evaluate_ranking('compass','r',call,dict(nodes=nodes,links=[]),samples,witnesses)

    def test_duplicate_graph_ids_and_incomplete_witnesses_fail_closed(self):
        with self.assertRaises(ValueError): score([node('c'),node('c')])
        with self.assertRaises(ValueError):
            evaluate_ranking('graphify','r',None,None,
                [dict(repository='r',sampleId=1,classification='unanimous-none')],[])

    def test_invalid_rank_and_repeated_identity_rejected(self):
        for text in ['', '  2. C - 1 edges', '  1. C - 1 edges\n  2. C - 1 edges']:
            with self.subTest(text=text), self.assertRaises(ValueError): score([node('c')], text)


if __name__ == '__main__': unittest.main()
