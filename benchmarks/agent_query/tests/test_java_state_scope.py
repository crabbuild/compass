import copy
import json
from pathlib import Path
import struct
import tempfile
import unittest

from benchmarks.agent_query.java_state_scope_audit import (
    ClassReader, field_declaration, graph_inventory, parse_javap, registration,
)


DISASSEMBLY = '''Compiled from "Fixture.java"
class audit.Box {
  int value;
    descriptor: I

  int get();
    descriptor: ()I
    Code:
       0: aload_0
       1: getfield      #7 // Field value:I
       4: aload_0
       5: getfield      #7 // Field audit/Other.value:I
       8: iadd
       9: ireturn
    LineNumberTable:
      line 10: 0
      line 11: 4

  private int lambda$get$0();
    descriptor: ()I
    Code:
       0: aload_0
       1: getfield      #7 // Field value:I
       4: ireturn
    LineNumberTable:
      line 12: 0
}
'''


def cls(parent=None, fields=(), interfaces=()):
    return dict(parent=parent, interfaces=list(interfaces), fields=[
        dict(name=name, descriptor='I', synthetic=False) for name in fields])


class JavaScopeTests(unittest.TestCase):
    def test_registered_source_is_unchanged_and_all_targets_exist(self):
        reg = registration(Path('benchmarks/agent_query/java_state_scope_registration.json'))
        self.assertEqual(len(reg['cases']), 44)
        self.assertEqual(sum(not c['expectedFields'] for c in reg['cases']), 8)
        self.assertEqual(sum(len(c['expectedFields']) for c in reg['cases']), 45)

    def test_field_bytecodes_keep_offset_owner_and_lambda_method(self):
        rows, covered = parse_javap(DISASSEMBLY, 'audit.Box')
        self.assertEqual(covered, {10, 11, 12})
        self.assertEqual([(r['owner'], r['line'], r['offset']) for r in rows],
                         [('audit.Box', 10, 1), ('audit.Other', 11, 5), ('audit.Box', 12, 1)])
        self.assertIn('lambda$get$0', rows[-1]['method'])

    def test_multiplicity_and_repeated_source_line_are_preserved(self):
        rows, _ = parse_javap(DISASSEMBLY.replace('line 11: 4', 'line 10: 4'), 'audit.Box')
        self.assertEqual(sum(r['line'] == 10 for r in rows), 2)

    def test_field_hiding_uses_declared_bytecode_owner(self):
        classes = {'Base': cls(fields=['value']), 'Sub': cls('Base', ['value']), 'Inner': cls('Base')}
        self.assertEqual(field_declaration(classes, 'Base', 'value', 'I')['owner'], 'Base')
        self.assertEqual(field_declaration(classes, 'Sub', 'value', 'I')['owner'], 'Sub')
        self.assertEqual(field_declaration(classes, 'Inner', 'value', 'I')['owner'], 'Base')

    def test_ambiguous_missing_and_cyclic_fields_fail(self):
        for classes, owner in [
            ({'A': cls('B'), 'B': cls('A')}, 'A'),
            ({'A': cls()}, 'A'),
            ({'A': cls(interfaces=['B', 'C']), 'B': cls(fields=['value']), 'C': cls(fields=['value'])}, 'A'),
        ]:
            with self.subTest(classes=classes), self.assertRaises(ValueError):
                field_declaration(classes, owner, 'value', 'I')

    def test_missing_or_ambiguous_line_tables_fail(self):
        for text in [DISASSEMBLY.replace('      line 10: 0\n', ''),
                     DISASSEMBLY.replace('line 11: 4', 'line 11: 0'),
                     DISASSEMBLY.replace('line 11: 4', 'line 11: 3')]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                parse_javap(text, 'audit.Box')

    def test_unrecognized_field_instruction_is_not_dropped(self):
        with self.assertRaises(ValueError):
            parse_javap(DISASSEMBLY.replace('// Field value:I', '// Unexpected value:I'), 'audit.Box')

    def test_empty_or_unordered_bytecode_fails(self):
        for text in ['', DISASSEMBLY.replace('       5: getfield', '       1: getfield')]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                parse_javap(text, 'audit.Box')

    def test_class_metadata_and_synthetic_field_flags(self):
        def utf(text):
            raw = text.encode()
            return b'\1' + struct.pack('>H', len(raw)) + raw
        # Pool: class name, Class entry, field name, field descriptor.
        data = (b'\xca\xfe\xba\xbe' + struct.pack('>HHH', 0, 61, 5)
                + utf('audit/Box') + b'\7\0\1' + utf('this$0') + utf('Laudit/Outer;')
                + struct.pack('>HHHHH', 0, 2, 0, 0, 1)
                + struct.pack('>HHHH', 0x1010, 3, 4, 0))
        result = ClassReader(data).parse()
        self.assertEqual(result['owner'], 'audit.Box')
        self.assertTrue(result['fields'][0]['synthetic'])
        self.assertEqual(result['major'], 61)
        with self.assertRaises(ValueError):
            ClassReader(data[:-1]).parse()
        with self.assertRaises(ValueError):
            ClassReader(b'bad!').parse()


    def test_graph_inventory_never_scores_empty_negatives_as_precision(self):
        reg = dict(source='Fixture.java', fields=dict(value=dict(owner='Box', name='value', line=2)),
                   cases=[dict(id='positive', line=5, expectedFields=['value']),
                          dict(id='negative', line=6, expectedFields=[])])
        graph = dict(directed=True, nodes=[dict(id='field', name='value', kind='field',
                                               source=dict(file='Fixture.java', startLine=2))], links=[])
        result = graph_inventory(graph, 'compass', reg)
        self.assertEqual(result['summary']['uniqueFields'], 1)
        self.assertEqual(result['summary']['occurrenceTargetsMatched'], 0)
        self.assertEqual(result['summary']['negativeCasesWithContact'], 0)
        self.assertIsNone(result['summary']['edgePrecision'])
        self.assertFalse(result['summary']['callerOwnershipScored'])

    def test_graph_inventory_retains_duplicates_and_wrong_targets(self):
        reg = dict(source='Fixture.java', fields=dict(value=dict(name='value', line=2)),
                   cases=[dict(id='positive', line=5, expectedFields=['value']),
                          dict(id='negative', line=6, expectedFields=[])])
        graph = dict(directed=True, nodes=[
            dict(id='field', name='value', kind='field', source=dict(file='Fixture.java', startLine=2)),
            dict(id='method', name='get', kind='method', source=dict(file='Fixture.java', startLine=4))], links=[
            dict(id='one', source='method', target='field', kind='references',
                 relationshipSite=dict(file='Fixture.java', startLine=5)),
            dict(id='two', source='method', target='field', kind='references',
                 relationshipSite=dict(file='Fixture.java', startLine=5)),
            dict(id='bad', source='method', target='field', kind='references',
                 relationshipSite=dict(file='Fixture.java', startLine=6))])
        result = graph_inventory(graph, 'compass', reg)
        self.assertEqual(len(result['allContactRecords']), 3)
        self.assertEqual(result['summary']['occurrenceTargetsMatched'], 1)
        self.assertEqual(result['summary']['negativeCasesWithContact'], 1)
        self.assertEqual(result['cases'][0]['unexpectedTargets'], ['value'])
        self.assertEqual(result['cases'][1]['unexpectedTargets'], ['value'])
        graph['nodes'].append(dict(graph['nodes'][0], id='duplicate'))
        result = graph_inventory(graph, 'compass', reg)
        self.assertEqual(result['summary']['ambiguousFields'], 1)
        self.assertEqual(result['summary']['occurrenceTargetsMatched'], 0)

    def test_graphify_direction_and_absent_occurrence_remain_visible(self):
        reg = dict(source='Fixture.java', fields=dict(value=dict(name='value', line=2)),
                   cases=[dict(id='positive', line=5, expectedFields=['value'])])
        graph = dict(directed=False, nodes=[
            dict(id='f', label='value', source_file='Fixture.java', source_location='L2'),
            dict(id='m', label='get', source_file='Fixture.java', source_location='L4')], links=[
            dict(source='m', target='f', relation='references')])
        result = graph_inventory(graph, 'graphify', reg)
        self.assertFalse(result['graphDirected'])
        self.assertEqual(result['summary']['fieldContactRecords'], 1)
        self.assertEqual(result['summary']['occurrenceTargetsMatched'], 0)
        graph['links'][0].update(source_file='Fixture.java', source_location='L5')
        self.assertEqual(graph_inventory(graph, 'graphify', reg)['summary']['occurrenceTargetsMatched'], 1)
        graph['links'][0].update(source='f', target='m')
        self.assertEqual(graph_inventory(graph, 'graphify', reg)['summary']['occurrenceTargetsMatched'], 0)

    def test_invalid_graph_identity_fails(self):
        reg = dict(source='Fixture.java', fields={}, cases=[])
        for graph in [dict(nodes=[dict(id='x'), dict(id='x')], links=[]),
                      dict(nodes=[dict(id='x')], links=[dict(source='x', target='missing')])]:
            with self.subTest(graph=graph), self.assertRaises(ValueError):
                graph_inventory(graph, 'compass', reg)


    def test_registration_refuses_changed_source_or_unknown_fields(self):
        original = json.loads(Path('benchmarks/agent_query/java_state_scope_registration.json').read_text())
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'registration.json'
            for mutation in ['hash', 'target', 'duplicate', 'line']:
                data = copy.deepcopy(original)
                if mutation == 'hash':
                    data['sourceSha256'] = 'wrong'
                elif mutation == 'target':
                    data['cases'][0]['expectedFields'] = ['invented']
                elif mutation == 'duplicate':
                    data['cases'].append(data['cases'][0])
                else:
                    data['cases'][0]['text'] = 'wrong'
                path.write_text(json.dumps(data))
                with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                    registration(path)


if __name__ == '__main__':
    unittest.main()
