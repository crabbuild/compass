"""Compiler-backed Java field-scope development oracle, separate from product CI.

Requires a supplied installed JDK only when capturing. Never executes Java fixture
code. Verification replays bounded, hashed class files and javap transcripts.
This is not a general Java bytecode analyzer or a real-repository scorecard.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import struct

from benchmarks.agent_query.runner import run_bounded
from benchmarks.agent_query.state_access_audit import MAX_GRAPH_BYTES, candidates, occurrence, read

LIMIT = 4 * 1024 * 1024
MAX_CLASSES = 64


def support_hashes():
    return {name: sha(Path(__file__).with_name(name))
            for name in ('runner.py', 'state_access_audit.py')}


def sha(path):
    return hashlib.sha256(read(path, LIMIT)).hexdigest()


def registration(path):
    data = json.loads(read(path, LIMIT))
    if data['schema'] != 'compass.java-state-scope-registration/1':
        raise ValueError('unknown registration schema')
    source = Path(data['source'])
    if sha(source) != data['sourceSha256']:
        raise ValueError('source hash mismatch')
    lines = read(source, LIMIT).decode().splitlines()
    ids, sites = set(), set()
    for case in data['cases']:
        if case['id'] in ids or case['line'] in sites:
            raise ValueError('duplicate case identity or source line')
        ids.add(case['id'])
        sites.add(case['line'])
        if lines[case['line'] - 1] != case['text']:
            raise ValueError('case source witness mismatch')
        if any(field not in data['fields'] for field in case['expectedFields']):
            raise ValueError('unknown expected field')
    for field in data['fields'].values():
        if lines[field['line'] - 1] != field['text']:
            raise ValueError('field source witness mismatch')
    return data


class ClassReader:
    """Read only constant pool, ancestry and field metadata; no code execution."""
    def __init__(self, data):
        if len(data) > LIMIT:
            raise ValueError('oversized class')
        self.data, self.position = data, 0

    def take(self, size):
        end = self.position + size
        if end > len(self.data):
            raise ValueError('truncated class')
        data, self.position = self.data[self.position:end], end
        return data

    def u2(self):
        return struct.unpack('>H', self.take(2))[0]

    def parse(self):
        if self.take(4) != b'\xca\xfe\xba\xbe':
            raise ValueError('invalid class magic')
        minor, major = self.u2(), self.u2()
        pool = [None] * self.u2()
        index = 1
        while index < len(pool):
            tag = self.take(1)[0]
            if tag == 1:
                # Fixture identifiers are ASCII; reject unsupported modified UTF-8.
                pool[index] = self.take(self.u2()).decode('utf-8')
            elif tag in (7, 8, 16, 19, 20):
                pool[index] = (tag, self.u2())
            elif tag in (3, 4, 9, 10, 11, 12, 17, 18):
                self.take(4)
            elif tag in (5, 6):
                self.take(8)
                index += 1
            elif tag == 15:
                self.take(3)
            else:
                raise ValueError(f'unsupported constant tag {tag}')
            index += 1

        def string(index):
            if not 0 < index < len(pool) or not isinstance(pool[index], str):
                raise ValueError('invalid string constant')
            return pool[index]

        def class_name(index):
            if index == 0:
                return None
            if not 0 < index < len(pool) or not isinstance(pool[index], tuple) or pool[index][0] != 7:
                raise ValueError('invalid class constant')
            return string(pool[index][1]).replace('/', '.')

        self.u2()  # Class flags are irrelevant to field identity.
        owner, parent = class_name(self.u2()), class_name(self.u2())
        interfaces = [class_name(self.u2()) for _ in range(self.u2())]
        fields = []
        for _ in range(self.u2()):
            flags, name, descriptor = self.u2(), string(self.u2()), string(self.u2())
            synthetic = bool(flags & 0x1000)
            for _ in range(self.u2()):
                attribute = string(self.u2())
                size = struct.unpack('>I', self.take(4))[0]
                self.take(size)
                synthetic |= attribute == 'Synthetic'
            fields.append(dict(name=name, descriptor=descriptor, synthetic=synthetic))
        return dict(owner=owner, parent=parent, interfaces=interfaces, fields=fields,
                    major=major, minor=minor)


def parse_javap(text, owner):
    """Associate each field instruction with the preceding bytecode line entry."""
    methods, current = [], None
    for line in text.splitlines():
        # -p -c -l -s emits two-space declaration headers and deeper code.
        if re.match(r'^  \S', line):
            if '(' in line and line.endswith(';'):
                current = dict(method=line.strip(), instructions=[], lines=[], offsets=[])
                methods.append(current)
            elif line == '  static {};':
                current = dict(method='<clinit>', instructions=[], lines=[], offsets=[])
                methods.append(current)
            else:
                current = None
        if current is None:
            continue
        instruction = re.match(r'^\s+(\d+):\s+(\w+)\b', line)
        if instruction:
            offset, opcode = int(instruction[1]), instruction[2]
            current['offsets'].append(offset)
            if opcode in {'getfield', 'putfield', 'getstatic', 'putstatic'}:
                field = re.search(r'// Field ([^: ]+):([^ ]+)\s*$', line)
                if not field:
                    raise ValueError('unrecognized field instruction')
                reference = field[1].replace('/', '.')
                target_owner, _, name = reference.rpartition('.')
                current['instructions'].append(dict(offset=offset, opcode=opcode,
                    owner=target_owner or owner, name=name, descriptor=field[2], raw=line))
        entry = re.fullmatch(r'\s+line (\d+): (\d+)', line)
        if entry:
            current['lines'].append((int(entry[2]), int(entry[1])))
    if not methods:
        raise ValueError('no methods in javap output')
    occurrences, covered = [], set()
    for method in methods:
        offsets = method['offsets']
        if offsets != sorted(set(offsets)):
            raise ValueError('duplicate or unordered instruction offsets')
        line_map = sorted(method['lines'])
        if len({offset for offset, _ in line_map}) != len(line_map):
            raise ValueError('ambiguous line mapping')
        if any(offset not in offsets for offset, _ in line_map):
            raise ValueError('line mapping outside instructions')
        covered.update(line for _, line in line_map)
        for instruction in method['instructions']:
            preceding = [entry for entry in line_map if entry[0] <= instruction['offset']]
            if not preceding:
                raise ValueError('field instruction missing source line')
            occurrences.append(dict(instruction, line=preceding[-1][1],
                                    method=method['method'], bytecodeClass=owner))
    return occurrences, covered


def field_declaration(classes, owner, name, descriptor):
    """JVM field resolution: declared field, interfaces, then superclass.

    This fixture has no interface fields. Reject multiple interface matches
    instead of treating inheritance order as source-level disambiguation.
    """
    def find(current, visiting):
        if current in visiting or len(visiting) >= MAX_CLASSES:
            raise ValueError('cyclic or excessive field hierarchy')
        if current not in classes:
            return []
        cls = classes[current]
        direct = [dict(field, owner=current) for field in cls['fields']
                  if field['name'] == name and field['descriptor'] == descriptor]
        if direct:
            return direct
        visiting = visiting | {current}
        interfaces = [field for interface in cls['interfaces'] for field in find(interface, visiting)]
        return interfaces or find(cls['parent'], visiting)
    result = find(owner, set())
    if len(result) != 1:
        raise ValueError(f'field declaration missing or ambiguous: {owner}.{name}:{descriptor}')
    return result[0]


def capture(reg_path, directory, java_home):
    reg = registration(reg_path)
    directory.mkdir(parents=True, exist_ok=False)
    commands = []

    def run(name, argv):
        result = run_bounded(tuple(map(str, argv)), cwd=Path.cwd(), timeout_seconds=60,
            stdout_path=directory / f'{name}.stdout', stderr_path=directory / f'{name}.stderr')
        commands.append(dict(argv=list(result.argv), exitCode=result.exit_code,
            timedOut=result.timed_out, outputLimited=result.output_limited,
            stdout=f'{name}.stdout', stderr=f'{name}.stderr'))
        if result.exit_code or result.timed_out or result.output_limited:
            raise ValueError(f'command failed; preserved logs: {name}')

    javac, javap = java_home / 'bin/javac', java_home / 'bin/javap'
    run('compiler-version', [javac, '-version'])
    run('javac', [javac, '--release', '17', '-proc:none', '-implicit:none',
                  '-g:lines,vars,source', '-d', directory / 'classes', Path(reg['source']).resolve()])
    files = sorted((directory / 'classes').rglob('*.class'))
    if not 0 < len(files) <= MAX_CLASSES:
        raise ValueError('invalid class count')
    artifacts = []
    for index, path in enumerate(files):
        cls = ClassReader(read(path, LIMIT)).parse()
        name = f'class-{index:02}'
        run(name, [javap, '-p', '-c', '-l', '-s', '-classpath', directory / 'classes', cls['owner']])
        artifacts.append(dict(owner=cls['owner'], binary=str(path.relative_to(directory)),
                              disassembly=f'{name}.stdout'))
    hashes = {str(path.relative_to(directory)): sha(path)
              for path in sorted(directory.rglob('*')) if path.is_file()}
    manifest = dict(schema='compass.java-state-scope-capture/1',
        registrationSha256=sha(reg_path), sourceSha256=reg['sourceSha256'],
        jdkRelease=read(java_home / 'release', LIMIT).decode(),
        compilerSha256=sha(javac), disassemblerSha256=sha(javap),
        commands=commands, classes=artifacts, hashes=hashes)
    (directory / 'capture.json').write_text(json.dumps(manifest, indent=2) + '\n')
    registration(reg_path)  # Reject concurrent fixture changes.


def evaluate(reg_path, directory):
    reg = registration(reg_path)
    manifest = json.loads(read(directory / 'capture.json', LIMIT))
    if (manifest['schema'] != 'compass.java-state-scope-capture/1'
            or manifest['registrationSha256'] != sha(reg_path)
            or manifest['sourceSha256'] != reg['sourceSha256']):
        raise ValueError('capture registration mismatch')
    for name, expected in manifest['hashes'].items():
        path = (directory / name).resolve()
        path.relative_to(directory.resolve())
        if sha(path) != expected:
            raise ValueError('capture file hash mismatch')
    if not manifest['commands'] or any(c['exitCode'] or c['timedOut'] or c['outputLimited']
                                       for c in manifest['commands']):
        raise ValueError('unsuccessful capture')
    classes, occurrences, covered = {}, [], set()
    if not 0 < len(manifest['classes']) <= MAX_CLASSES:
        raise ValueError('invalid class count')
    for artifact in manifest['classes']:
        if any(artifact[k] not in manifest['hashes'] for k in ('binary', 'disassembly')):
            raise ValueError('unhashed class input')
        cls = ClassReader(read(directory / artifact['binary'], LIMIT)).parse()
        if cls['owner'] != artifact['owner'] or cls['owner'] in classes:
            raise ValueError('class identity mismatch')
        classes[cls['owner']] = cls
        found, lines = parse_javap(read(directory / artifact['disassembly'], LIMIT).decode(), cls['owner'])
        occurrences.extend(found)
        covered.update(lines)
    inventory = {(v['owner'], v['name']): k for k, v in reg['fields'].items()}
    compiled = {(owner, field['name']) for owner, cls in classes.items()
                for field in cls['fields'] if not field['synthetic']}
    if compiled != set(inventory) or len(inventory) != len(reg['fields']):
        raise ValueError('compiled field inventory differs from registration')
    for occurrence in occurrences:
        declaration = field_declaration(classes, occurrence['owner'], occurrence['name'], occurrence['descriptor'])
        occurrence['declaration'] = declaration
        occurrence['field'] = inventory.get((declaration['owner'], declaration['name']))
        if occurrence['field'] is None and not declaration['synthetic']:
            raise ValueError('unexpected nonsynthetic field')
    rows = []
    for case in reg['cases']:
        if case['line'] not in covered:
            raise ValueError(f'case has no compiled source line: {case["id"]}')
        records = [o for o in occurrences if o['line'] == case['line']]
        actual = [o['field'] for o in records if o['field'] is not None]
        rows.append(dict(case, observedFields=actual, compilerInstructions=records,
                         agrees=Counter(actual) == Counter(case['expectedFields'])))
    return dict(schema='compass.java-state-scope-oracle/1', registrationSha256=sha(reg_path),
        scriptSha256=sha(Path(__file__)), supportSha256=support_hashes(), captureSha256=sha(directory / 'capture.json'),
        compiler=manifest['jdkRelease'], classes=classes,
        summary=dict(cases=len(rows), agreed=sum(row['agrees'] for row in rows),
            positiveCases=sum(bool(row['expectedFields']) for row in rows),
            negativeCases=sum(not row['expectedFields'] for row in rows),
            fieldOccurrences=sum(len(row['expectedFields']) for row in rows)), cases=rows)



def graph_inventory(graph, tool, reg):
    """Inventory field endpoints and occurrence targets, without caller scoring.

    This is a diagnostic prerequisite, not a positive field-edge precision score:
    caller ownership still requires its own source-coordinate/native regression.
    """
    nodes = {node['id']: node for node in graph['nodes']}
    if len(nodes) != len(graph['nodes']):
        raise ValueError('duplicate graph node ID')
    if any(edge['source'] not in nodes or edge['target'] not in nodes for edge in graph['links']):
        raise ValueError('dangling graph endpoint')
    file = Path(reg['source']).name
    fields = {key: candidates(graph['nodes'], tool, file, field['line'], field['name'])
              for key, field in reg['fields'].items()}
    if tool == 'compass':
        fields = {key: [node for node in choices if node.get('kind') == 'field']
                  for key, choices in fields.items()}
    unique = {choices[0]['id']: key for key, choices in fields.items() if len(choices) == 1}
    all_ids = {node['id'] for choices in fields.values() for node in choices}
    contacts = sorted((edge for edge in graph['links'] if edge['target'] in all_ids
                       and edge.get('kind' if tool == 'compass' else 'relation')
                       in {'references', 'reads', 'writes'}), key=lambda e: json.dumps(e, sort_keys=True))
    rows = []
    for case in reg['cases']:
        records = [edge for edge in contacts if occurrence(edge, tool, file, case['line'])]
        observed = [unique[edge['target']] for edge in records if edge['target'] in unique]
        expected = Counter(case['expectedFields'])
        actual = Counter(observed)
        rows.append(dict(id=case['id'], line=case['line'], expectedFields=case['expectedFields'],
            observedUniqueTargets=observed, connectingRecords=records,
            targetOccurrencesMatched=sum((actual & expected).values()),
            unexpectedTargets=list((actual - expected).elements())))
    return dict(graphDirected=graph.get('directed'), nodes=len(nodes), edges=len(graph['links']),
        fieldCandidates=fields, allContactRecords=contacts, cases=rows,
        summary=dict(fieldDeclarations=len(fields), uniqueFields=len(unique),
            missingFields=sum(not choices for choices in fields.values()),
            ambiguousFields=sum(len(choices) > 1 for choices in fields.values()),
            fieldContactRecords=len(contacts), registeredFieldOccurrences=sum(len(c['expectedFields']) for c in reg['cases']),
            occurrenceTargetsMatched=sum(r['targetOccurrencesMatched'] for r in rows),
            negativeCases=sum(not c['expectedFields'] for c in reg['cases']),
            negativeCasesWithContact=sum(not c['expectedFields'] and bool(c['connectingRecords']) for c in rows),
            callerOwnershipScored=False, edgePrecision=None))


def compare_graphs(reg_path, oracle_path, manifest_path):
    reg = registration(reg_path)
    oracle = json.loads(read(oracle_path, LIMIT))
    if (oracle['registrationSha256'] != sha(reg_path)
            or oracle['summary']['cases'] != len(reg['cases'])
            or oracle['summary']['agreed'] != len(reg['cases'])):
        raise ValueError('compiler oracle does not confirm the registration')
    manifest = json.loads(read(manifest_path, LIMIT))
    if manifest['registrationSha256'] != sha(reg_path) or manifest['sourceSha256'] != reg['sourceSha256']:
        raise ValueError('graph registration mismatch')
    reports = {}
    for tool in ('compass', 'graphify'):
        metadata = manifest['tools'][tool]
        raw = read(Path(metadata['graph']), MAX_GRAPH_BYTES)
        if hashlib.sha256(raw).hexdigest() != metadata['graphSha256'] or metadata['exitCode'] != 0:
            raise ValueError('graph capture mismatch')
        reports[tool] = graph_inventory(json.loads(raw), tool, reg)
    return dict(schema='compass.java-state-scope-baseline/1', registrationSha256=sha(reg_path),
        scriptSha256=sha(Path(__file__)), supportSha256=support_hashes(), oracleSha256=sha(oracle_path),
        graphManifestSha256=sha(manifest_path), scope=reg['scope'],
        limitations=['Target/line diagnostic only; caller ownership and positive edge precision are unscored.',
            'The compiler confirms this nonconstant fixture, not a general source-access oracle.',
            'Graphify stored endpoint order is preserved; an undirected graph is not directed path proof.',
            'Synthetic controls do not replace the five real-repository comparisons or prove god-object quality.'],
        inputs=manifest, tools=reports)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--registration', type=Path, required=True)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--graph-manifest', type=Path)
    parser.add_argument('--oracle', type=Path, help='verified compiler review for graph comparison')
    parser.add_argument('--java-home', type=Path, help='capture into a new directory using this installed JDK')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    if bool(args.graph_manifest) != bool(args.oracle):
        parser.error('--graph-manifest and --oracle must be supplied together')
    if args.graph_manifest and args.java_home:
        parser.error('graph comparison never recaptures the compiler')
    if args.java_home:
        if args.verify:
            parser.error('verification never recaptures')
        capture(args.registration, args.artifacts, args.java_home)
    if args.graph_manifest:
        # Recompute the compiler evidence, not just its saved summary.
        actual = (json.dumps(evaluate(args.registration, args.artifacts), indent=2) + '\n').encode()
        if read(args.oracle, LIMIT) != actual:
            raise ValueError('compiler oracle differs on replay')
        report = compare_graphs(args.registration, args.oracle, args.graph_manifest)
    else:
        report = evaluate(args.registration, args.artifacts)
    payload = (json.dumps(report, indent=2) + '\n').encode()
    if args.verify:
        if read(args.output, LIMIT) != payload:
            raise ValueError('oracle review differs on replay')
    else:
        with args.output.open('xb') as stream:
            stream.write(payload)
    summary = ({tool: result['summary'] for tool, result in report['tools'].items()}
               if args.graph_manifest else report['summary'])
    print(json.dumps(summary, indent=2))
    if not args.graph_manifest and report['summary']['agreed'] != report['summary']['cases']:
        raise SystemExit('source/compiler disagreement; do not score this oracle')


if __name__ == '__main__':
    main()
