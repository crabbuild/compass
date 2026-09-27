import unittest

from benchmarks.agent_query.directed_identity import resolver


class DirectedIdentityTests(unittest.TestCase):
    def test_resolvers_use_only_the_same_public_source_coordinates(self):
        witness=dict(file='src/a.rs',line=7,symbol='run',id='oracle-id-must-not-be-used')
        c,req=resolver('compass',witness)
        self.assertEqual(c,dict(file='src/a.rs',startLine=7,symbol='run'))
        self.assertEqual(req,dict(method='search_symbols',arguments=dict(query='run',max_candidates=256,max_nodes=500,max_response_bytes=524288)))
        g,req=resolver('graphify',witness)
        self.assertEqual(c,g)
        self.assertEqual(req,dict(method='get_node',arguments=dict(label='src/a.rs::run')))

    def test_unsupported_public_delimiter_fails_instead_of_changing_scope(self):
        with self.assertRaises(ValueError):resolver('graphify',dict(file='src/a::b.rs',line=7,symbol='run'))

    def test_unknown_tools_fail_explicitly(self):
        with self.assertRaises(ValueError):resolver('other',dict(file='a.rs',line=7,symbol='run'))

    def test_exact_workflow_scopes_compass_without_changing_shared_coordinates(self):
        witness=dict(file='src/a.rs',line=7,symbol='next',id='not-a-public-id')
        coordinate,request=resolver('compass',witness,exact=True)
        self.assertEqual(coordinate,dict(file='src/a.rs',startLine=7,symbol='next'))
        self.assertEqual(request['arguments'],dict(query='next',exact=True,source_file='src/a.rs',start_line=7,max_candidates=256,max_nodes=500,max_response_bytes=524288))

    def test_exact_workflow_keeps_graphify_native_request_and_postcheck_coordinates(self):
        witness=dict(file='src/a.rs',line=7,symbol='next')
        self.assertEqual(resolver('graphify',witness,exact=True),resolver('graphify',witness))


if __name__=='__main__':unittest.main()
