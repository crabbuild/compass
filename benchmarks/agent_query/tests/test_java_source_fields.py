import copy
from collections import Counter
import json
from pathlib import Path
import tempfile
import unittest

from benchmarks.agent_query.java_source_fields import (
    compare, load_capture, position, sha, utf16_to_bytes,
)

FIXTURES = Path('benchmarks/agent_query/fixtures/java_field_bindings')


class JavaSourceFieldsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = json.loads((FIXTURES / 'adversarial-manifest.json').read_text())
        cls.capture = load_capture(FIXTURES / 'adversarial.jsonl', FIXTURES, cls.manifest['files'])
        cls.raw = [json.loads(s) for s in (FIXTURES / 'adversarial.jsonl').read_text().splitlines()]

    def test_utf16_conversion_rejects_split_surrogates_and_non_integer_positions(self):
        offsets = utf16_to_bytes('a🧭λ')
        self.assertEqual(list(offsets), [0, 1, -1, 5, 7])
        for bad in [2, -1, 5, True, None]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                position(offsets, bad)
        self.assertEqual(position(offsets, 4), 7)

    def test_fixture_capture_hash_and_all_44_registered_compiler_cases(self):
        manifest = json.loads((FIXTURES / 'scope-manifest.json').read_text())
        self.assertEqual(sha(FIXTURES / 'scope.jsonl'), manifest['captureSha256'])
        capture = load_capture(FIXTURES / 'scope.jsonl',
                               Path('benchmarks/agent_query/fixtures/java_state_access'), manifest['files'])
        reg = json.loads(Path('benchmarks/agent_query/java_state_scope_registration.json').read_text())
        fields = {(f['line'], f['name']): key for key, f in reg['fields'].items()}
        for case in reg['cases']:
            actual = [fields[r['target']['startLine'], r['target']['name']]
                      for r in capture['references'] if r['line'] == case['line'] and r['target']]
            self.assertEqual(Counter(actual), Counter(case['expectedFields']), case['id'])
        self.assertEqual(len(capture['references']), 45)
        self.assertEqual(sum(not c['expectedFields'] for c in reg['cases']), 8)

    def test_adversarial_source_owners_and_overloads(self):
        refs = self.capture['references']
        rows = {line: [r for r in refs if r['line'] == line] for line in [9, 10, 11, 12, 19, 24, 30]}
        self.assertEqual(rows[9][0]['owner']['kind'], 'field')
        self.assertEqual(rows[10][0]['owner']['kind'], 'class')
        self.assertNotEqual(rows[11][0]['owner']['id'], rows[12][0]['owner']['id'])
        self.assertEqual(rows[19][0]['owner']['name'], 'lambda')
        for line in [24, 30]:
            self.assertEqual(rows[line][0]['owner']['name'], 'get')
            self.assertEqual(rows[line][0]['target']['startLine'], line - 1)

    def test_compound_multiplicity_constant_folding_and_enum_constants(self):
        refs = self.capture['references']
        self.assertEqual([r['name'] for r in refs if r['line'] == 13], ['left', 'right', 'left'])
        self.assertEqual([r['name'] for r in refs if r['line'] == 14], ['LIMIT'])
        self.assertEqual([r['targetKind'] for r in refs if r['line'] == 35], ['enum_constant'])
        self.assertEqual([r['target']['qualified'] for r in refs if r['line'] == 20],
                         ['bindings.Base::left', 'bindings.BindingFixture::left'])

    def test_external_and_unsupported_raw_spelling_are_preserved(self):
        external = [r for r in self.capture['references'] if r['line'] == 15]
        self.assertEqual(len(external), 1)
        self.assertIsNone(external[0]['target'])
        escaped = [r for r in self.capture['references'] if r['line'] == 18][0]
        self.assertEqual(escaped['target']['name'], 'x')
        self.assertIsNone(escaped['startByte'])
        self.assertEqual(len(self.capture['references']), 20)
        self.assertEqual(Counter(r['targetOrigin'] for r in self.capture['references']),
                         {'source': 17, 'external': 1, 'array-length': 1, 'class-literal': 1})
        self.assertEqual(sha(FIXTURES / 'adversarial.jsonl'), self.manifest['captureSha256'])
        self.assertEqual(sha(Path('benchmarks/agent_query/java_oracle/FieldBindings.java')), self.manifest['toolSha256'])

    def mutated(self, mutate):
        rows = copy.deepcopy(self.raw)
        mutate(rows)
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory) / 'capture.jsonl'
            p.write_text('\n'.join(json.dumps(r) for r in rows) + '\n')
            return load_capture(p, FIXTURES, self.manifest['files'])

    def test_incomplete_unknown_schema_and_count_drift_fail(self):
        for mutate in [lambda r: r.pop(), lambda r: r[0].update(schema='unknown/2'),
                       lambda r: r[-1].update(fieldReferences=0),
                       lambda r: r[-1].update(unboundExpressions=1)]:
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                self.mutated(mutate)

    def test_duplicate_occurrence_and_corrupt_anchor_fail(self):
        def duplicate(rows):
            rows.insert(-1, next(r for r in rows if r['type'] == 'fieldReference'))
        def corrupt(rows):
            next(r for r in rows if r['type'] == 'fieldReference')['tokenStartUtf16'] += 1
        for mutate in [duplicate, corrupt]:
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                self.mutated(mutate)

    def test_source_hash_drift_is_not_an_empty_oracle(self):
        with self.assertRaises(ValueError):
            load_capture(FIXTURES / 'adversarial.jsonl', FIXTURES, {'BindingFixture.java': '0' * 64})

    def graph(self):
        row = next(r for r in self.capture['references'] if r['line'] == 11)
        def node(id, decl):
            return dict(id=id, kind=decl['kind'], name=decl['name'], source=dict(
                file=decl['file'], startByte=decl['startByte'], endByte=decl['endByte'],
                startLine=decl['startLine'], endLine=decl['endLine']))
        graph = dict(directed=True, nodes=[node('method', row['owner']), node('field', row['target'])],
                     links=[dict(id='edge', source='method', target='field', kind='references',
                                 relationshipSite=dict(file=row['file'], startByte=row['startByte'],
                                                       endByte=row['endByte'], startLine=row['line']))])
        return graph

    def assess(self, graph, tool='compass'):
        return compare(graph, tool, self.capture, self.manifest['files'])

    def test_exact_target_owner_and_occurrence_are_jointly_required(self):
        result = self.assess(self.graph())
        self.assertEqual(result['contactStatus'], {'verified_target_and_owner': 1})
        self.assertEqual(result['summary']['field']['exactTargetAndOwner'], 1)
        self.assertEqual(result['summary']['field']['unsupportedAnchors'], 1)

    def test_wrong_target_and_wrong_owner_are_distinct(self):
        for which, status in [('field', 'wrong_target'), ('method', 'wrong_owner')]:
            graph = self.graph()
            row = next(r for r in self.capture['references'] if r['line'] == 12)
            decl = row['target'] if which == 'field' else row['owner']
            node = next(n for n in graph['nodes'] if n['id'] == which)
            node.update(name=decl['name'], source=dict(file=decl['file'], startByte=decl['startByte'],
                        endByte=decl['endByte'], startLine=decl['startLine'], endLine=decl['endLine']))
            self.assertEqual(self.assess(graph)['contactStatus'], {status: 1})

    def test_duplicate_declaration_identities_remain_ambiguous(self):
        graph = self.graph()
        duplicate = dict(graph['nodes'][1], id='other-field')
        graph['nodes'].append(duplicate)
        result = self.assess(graph)
        self.assertEqual(result['contactStatus'], {'ambiguous_target': 1})
        self.assertEqual(result['summary']['field']['exactTargetAndOwner'], 0)

    def test_missing_anchor_is_retained_using_owner_file_only_for_scope(self):
        graph = self.graph()
        del graph['links'][0]['relationshipSite']
        self.assertEqual(self.assess(graph)['contactStatus'], {'unanchored': 1})

    def test_graphify_line_contact_is_not_upgraded_to_exact_span(self):
        graph = dict(directed=False, nodes=[
            dict(id='method', label='.read()', _callable=True, source_file='BindingFixture.java', source_location='L11'),
            dict(id='field', label='left', source_file='BindingFixture.java', source_location='L6')],
            links=[dict(source='method', target='field', relation='references', source_file='BindingFixture.java', source_location='L11')])
        result = self.assess(graph, 'graphify')
        self.assertEqual(result['contactStatus'], {'unanchored': 1})
        self.assertEqual(result['summary']['field']['exactTargetAndOwner'], 0)
        graph['links'][0].update(source='field', target='method')
        self.assertEqual(self.assess(graph, 'graphify')['contactStatus'], {'unanchored': 1})

    def test_duplicate_graph_ids_and_dangling_endpoints_fail(self):
        for mutation in [lambda g: g['nodes'].append(g['nodes'][0]),
                         lambda g: g['links'][0].update(target='absent')]:
            graph = self.graph()
            mutation(graph)
            with self.assertRaises(ValueError):
                self.assess(graph)


if __name__ == '__main__':
    unittest.main()
