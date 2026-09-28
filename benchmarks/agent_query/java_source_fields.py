"""Replay public-JDK source field bindings and compare frozen native graphs.

Development evidence only. Exact occurrences, targets and owners are distinct
from graph consistency, line-only support, read/write effects and design quality.
The compiler is needed for capture, never for offline replay or product runtime.
"""
import argparse
from array import array
from bisect import bisect_right
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path

from benchmarks.agent_query.state_access_audit import read, name, anchor, MAX_GRAPH_BYTES

MAX_SOURCE = 4 * 1024 * 1024
MAX_CAPTURE = 16 * 1024 * 1024
MAX_RECORDS = 100000


def sha(path, limit=MAX_CAPTURE):
    return hashlib.sha256(read(Path(path), limit)).hexdigest()


def utf16_to_bytes(text):
    """Map Java character positions to UTF-8 bytes; reject split surrogates."""
    result = array('q', [0])
    total = 0
    for char in text:
        if ord(char) > 0xffff:
            result.append(-1)
        total += len(char.encode('utf-8'))
        result.append(total)
    return result


def position(mapping, value):
    if type(value) is not int or not 0 <= value < len(mapping) or mapping[value] < 0:
        raise ValueError('invalid UTF-16 source boundary')
    return mapping[value]


def load_capture(path, root, source_hashes):
    if not 0 < len(source_hashes) <= 512:
        raise ValueError('source count limit')
    sources, mappings, lines = {}, {}, {}
    root = root.resolve()
    total = 0
    for file, digest in source_hashes.items():
        p = (root / file).resolve()
        p.relative_to(root)
        raw = read(p, MAX_SOURCE)
        total += len(raw)
        if total > 64 * 1024 * 1024 or hashlib.sha256(raw).hexdigest() != digest:
            raise ValueError('source drift or byte limit')
        sources[file] = raw
        mappings[file] = utf16_to_bytes(raw.decode('utf-8'))
        lines[file] = [0] + [i + 1 for i, byte in enumerate(raw) if byte == 10]
    rows = [json.loads(line) for line in read(path, MAX_CAPTURE).splitlines()]
    if not 2 <= len(rows) <= MAX_RECORDS:
        raise ValueError('capture record count')
    header, complete = rows[0], rows[-1]
    if (header.get('type') != 'header' or header.get('schema') != 'compass.javac-field-bindings/1'
            or header.get('positionEncoding') != 'UTF-16 code units'
            or header.get('files') != len(sources) or header.get('sourceBytes') != total
            or complete.get('type') != 'complete' or complete.get('unboundExpressions') != 0):
        raise ValueError('incomplete or incompatible compiler capture')

    def declaration(raw):
        if raw is None:
            return None
        item = dict(raw)
        file = item['file']
        mapping = mappings[file]
        start, end = position(mapping, item['startUtf16']), position(mapping, item['endUtf16'])
        if start >= end:
            raise ValueError('empty declaration')
        expected_id = f"{file}:{item['startUtf16']}:{item['endUtf16']}:{item['kind']}:{item['name']}"
        if (item['id'] != expected_id or item['startLine'] != bisect_right(lines[file], start)
                or item['endLine'] != bisect_right(lines[file], end - 1)):
            raise ValueError('declaration identity/line mismatch')
        item.update(startByte=start, endByte=end)
        return item

    declarations, references, seen = {}, [], set()
    for raw in rows[1:-1]:
        if raw.get('type') == 'declaration':
            item = declaration(raw['declaration'])
            if item['id'] in declarations:
                raise ValueError('duplicate declaration')
            declarations[item['id']] = item
        elif raw.get('type') == 'fieldReference':
            item = dict(raw)
            file = item['file']
            mapping = mappings[file]
            a, b = position(mapping, item['expressionStartUtf16']), position(mapping, item['expressionEndUtf16'])
            key = (file, a, b)
            if a >= b or key in seen or item['line'] != bisect_right(lines[file], b - 1):
                raise ValueError('duplicate or invalid field occurrence')
            seen.add(key)
            if item['anchorStatus'] == 'exact':
                start, end = position(mapping, item['tokenStartUtf16']), position(mapping, item['tokenEndUtf16'])
                if not a <= start < end <= b or sources[file][start:end].decode() != item['name']:
                    raise ValueError('field token mismatch')
                item.update(startByte=start, endByte=end)
            elif item['anchorStatus'] == 'unsupported-raw-spelling':
                if item['tokenStartUtf16'] is not None or item['tokenEndUtf16'] is not None:
                    raise ValueError('unsupported anchor supplied coordinates')
                item.update(startByte=None, endByte=None)
            else:
                raise ValueError('unknown anchor status')
            item['target'] = declaration(item['target'])
            item['owner'] = declaration(item['owner'])
            if item.get('targetOrigin') not in {'source', 'external', 'array-length', 'class-literal'} or (item['targetOrigin'] == 'source') != bool(item['target']):
                raise ValueError('invalid target origin')
            item.update(expressionStartByte=a, expressionEndByte=b)
            if item['target'] and item['target']['kind'] != item['targetKind']:
                raise ValueError('target kind mismatch')
            if item['owner'] and not (item['owner']['file'] == file
                    and item['owner']['startByte'] <= a < b <= item['owner']['endByte']):
                raise ValueError('source owner does not contain occurrence')
            references.append(item)
        else:
            raise ValueError('unknown capture record')
    if complete.get('declarations') != len(declarations) or complete.get('fieldReferences') != len(references):
        raise ValueError('compiler completion count mismatch')
    for row in references:
        for key in ('target', 'owner'):
            if row[key] and declarations.get(row[key]['id']) != row[key]:
                raise ValueError('unregistered compiler declaration')
    return dict(header=header, complete=complete, declarations=declarations, references=references)


def compatible_kind(node, tool, declaration):
    kind = declaration['kind']
    if tool == 'compass':
        return node.get('kind') == {
            'enum_constant': 'enum_member', 'annotation_type': 'annotation',
        }.get(kind, kind)
    # Only use explicit native flags; absent type metadata is not invented.
    if node.get('_callable_class'):
        return kind in {'class', 'interface', 'enum', 'annotation_type', 'record'}
    if node.get('_callable'):
        return kind in {'method', 'constructor'}
    return True


def join_declarations(graph, tool, declarations):
    by_file_name = defaultdict(list)
    for declaration in declarations.values():
        aliases = {declaration['name']}
        if declaration['kind'] == 'constructor':
            aliases.add(declaration['qualified'].rsplit('::', 1)[0].rsplit('.', 1)[-1])
        for alias in aliases:
            by_file_name[declaration['file'], alias].append(declaration)
    node_to_decls, decl_to_nodes = {}, defaultdict(list)
    for node in graph['nodes']:
        file, line = anchor(node, tool)
        matches = []
        for declaration in by_file_name.get((file, name(node, tool)), []):
            if not compatible_kind(node, tool, declaration):
                continue
            if tool == 'compass':
                point = node.get('source', {}).get('startByte')
                inside = type(point) is int and declaration['startByte'] <= point < declaration['endByte']
            else:
                inside = type(line) is int and declaration['startLine'] <= line <= declaration['endLine']
            if inside:
                matches.append(declaration['id'])
                decl_to_nodes[declaration['id']].append(node['id'])
        node_to_decls[node['id']] = sorted(matches)
    return node_to_decls, {key: sorted(value) for key, value in decl_to_nodes.items()}


def compare(graph, tool, capture, files):
    nodes = {node['id']: node for node in graph['nodes']}
    if len(nodes) != len(graph['nodes']) or len(graph['nodes']) > 1000000 or len(graph['links']) > 5000000:
        raise ValueError('duplicate node or graph size limit')
    if any(e['source'] not in nodes or e['target'] not in nodes for e in graph['links']):
        raise ValueError('missing graph endpoint')
    declarations, references = capture['declarations'], capture['references']
    node_join, decl_join = join_declarations(graph, tool, declarations)
    field_kinds = {'field', 'enum_constant'}
    site_index = defaultdict(list)
    for i, row in enumerate(references):
        if row['startByte'] is not None:
            site_index[row['file'], row['startByte'], row['endByte']].append(i)
    contacts, matched = [], set()
    for i, edge in enumerate(graph['links']):
        if edge.get('kind' if tool == 'compass' else 'relation') not in {'references', 'reads', 'writes'}:
            continue
        target, source = nodes[edge['target']], nodes[edge['source']]
        def is_field(node):
            return (node.get('kind') in {'field', 'enum_member'} or node.get('symbol_kind') == 'field'
                    or any(declarations[d]['kind'] in field_kinds for d in node_join[node['id']]))
        if not is_field(target):
            if tool == 'graphify' and graph.get('directed') is False and is_field(source):
                # Inventory an unordered contact in either stored orientation.
                # It still has no exact/directed occurrence proof.
                source, target = target, source
            else:
                continue
        targets, owners = node_join[target['id']], node_join[source['id']]
        site = edge.get('relationshipSite', {}) if tool == 'compass' else {}
        file, line = ((site.get('file'), site.get('startLine')) if tool == 'compass' else anchor(edge, tool))
        if file not in files:
            if file is not None or anchor(source, tool)[0] not in files:
                continue
            file = anchor(source, tool)[0]
        expected = site_index.get((file, site.get('startByte'), site.get('endByte')), [])
        status = 'unanchored' if not site else 'unmatched_occurrence'
        if expected:
            if len(expected) != 1:
                status = 'ambiguous_occurrence'
            elif len(targets) != 1 or len(decl_join.get(targets[0], [])) != 1:
                status = 'unmapped_target' if not targets else 'ambiguous_target'
            else:
                row = references[expected[0]]
                if row['target'] is None or row['target']['id'] != targets[0]:
                    status = 'wrong_target'
                elif len(owners) != 1 or row['owner'] is None or len(decl_join.get(owners[0], [])) != 1:
                    status = 'unmapped_owner' if not owners else 'ambiguous_owner'
                elif row['owner']['id'] != owners[0]:
                    status = 'wrong_owner'
                else:
                    status = 'verified_target_and_owner'
                    matched.add(expected[0])
        contacts.append(dict(index=i, id=edge.get('id'), source=edge['source'], target=edge['target'],
                             file=file, line=line, site=site, graphDirected=graph.get('directed'), targetDeclarations=targets,
                             ownerDeclarations=owners, oracleReferences=expected, status=status))
    internal = [(i, row) for i, row in enumerate(references) if row['target']]
    summary = {}
    for kind in sorted(field_kinds):
        rows = [(i, r) for i, r in internal if r['targetKind'] == kind]
        fields = [d for d in declarations.values() if d['kind'] == kind]
        summary[kind] = dict(occurrences=len(rows), exactTargetAndOwner=sum(i in matched for i, _ in rows),
                             declarations=len(fields), uniqueGraphDeclarations=sum(len(decl_join.get(d['id'], [])) == 1 for d in fields),
                             unsupportedAnchors=sum(r['startByte'] is None for _, r in rows))
    misses = []
    for i, row in internal:
        if i in matched:
            continue
        target_candidates = decl_join.get(row['target']['id'], [])
        owner_candidates = decl_join.get(row['owner']['id'], []) if row['owner'] else []
        misses.append(dict(oracleReference=i, file=row['file'], line=row['line'], name=row['name'],
                           target=row['target']['id'], owner=row['owner']['id'] if row['owner'] else None,
                           targetCandidates=target_candidates, ownerCandidates=owner_candidates))
    return dict(schema='compass.java-source-field-review/1', tool=tool, graphDirected=graph.get('directed'), identityPolicy='name/kind and byte region for Compass; explicit native flags and name/line region for Graphify; ambiguous joins never select a winner',
                summary=summary, contactStatus=dict(sorted(Counter(c['status'] for c in contacts).items())),
                contacts=contacts, misses=misses, declarationJoins=decl_join,
                limits='Exact-span semantic evidence only. Line-only contacts remain unanchored and are not silently expanded into repeated occurrences. No inference about read/write effects, paths, cohesion, god-object defects or overall superiority.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--graph', type=Path)
    parser.add_argument('--tool', choices=['compass', 'graphify'])
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--verify', action='store_true', help='recompute and compare an existing output')
    args = parser.parse_args()
    manifest = json.loads(read(args.manifest, MAX_CAPTURE))
    expected_hash = manifest.get('stdoutSha256', manifest.get('captureSha256'))
    if not expected_hash or sha(args.capture) != expected_hash:
        raise ValueError('capture drift or missing digest')
    if manifest.get('exitCode', 0) != 0 or manifest.get('timedOut') or manifest.get('outputLimited'):
        raise ValueError('failed or incomplete capture process')
    capture = load_capture(args.capture, args.root, manifest['files'])
    if args.graph:
        if not args.tool:
            parser.error('--graph requires --tool')
        report = compare(json.loads(read(args.graph, MAX_GRAPH_BYTES)), args.tool, capture, manifest['files'])
        report.update(graphSha256=sha(args.graph, MAX_GRAPH_BYTES))
    else:
        report = capture
    report.update(captureSha256=sha(args.capture), manifestSha256=sha(args.manifest), auditorSha256=sha(__file__))
    payload = (json.dumps(report, indent=2) + '\n').encode()
    if args.verify:
        if read(args.output, MAX_GRAPH_BYTES) != payload:
            raise ValueError('replay differs from saved result')
    else:
        with args.output.open('xb') as output:
            output.write(payload)


if __name__ == '__main__':
    main()
