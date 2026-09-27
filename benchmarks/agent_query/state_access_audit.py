"""Replay a source-registered, full-graph state-contact coverage diagnostic.

This does not infer read/write effects, runtime aliasing, LCOM, or god objects.
Frozen graphs are compared equally; no public-query success is claimed.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import subprocess

MAX_GRAPH_BYTES = 512 * 1024 * 1024
MAX_SOURCE_BYTES = 4 * 1024 * 1024


def read(path, limit):
    with path.open('rb') as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f'input exceeds {limit} bytes: {path}')
    return data


def digest(data):
    return hashlib.sha256(data).hexdigest()


def anchor(record, tool):
    if tool == 'compass':
        source = record.get('source', {})
        return source.get('file'), source.get('startLine')
    match = re.fullmatch(r'L([1-9][0-9]*)(?:-L?[1-9][0-9]*)?', record.get('source_location', ''))
    return record.get('source_file'), int(match[1]) if match else None


def name(record, tool):
    return record.get('name' if tool == 'compass' else 'label', '').removeprefix('.').removesuffix('()')


def candidates(nodes, tool, file, line, symbol, *, constructor=False):
    return sorted((n for n in nodes if anchor(n, tool) == (file, line)
        and (name(n, tool) == symbol or
             (constructor and n.get('kind') == 'constructor' and name(n, tool) == '<init>'))),
        key=lambda n: n['id'])


def occurrence(edge, tool, file, line):
    if tool == 'compass':
        # The relationship site is the edge occurrence; declaration evidence
        # elsewhere on the record is not a substitute.
        site = edge.get('relationshipSite', {})
        return (site.get('file'), site.get('startLine')) == (file, line)
    return anchor(edge, tool) == (file, line)


def assess(graph, tool, case, group, access):
    nodes = graph['nodes']
    methods = candidates(nodes, tool, case['file'], access['methodLine'], access['method'],
                         constructor=access['method'] == group['owner'])
    states = candidates(nodes, tool, case['file'], group['declarationLine'], group['state'])
    method_ids = {n['id'] for n in methods}
    state_ids = {n['id'] for n in states}
    connecting = [e for e in graph['links'] if
                  (e['source'] in method_ids and e['target'] in state_ids) or
                  (e['source'] in state_ids and e['target'] in method_ids)]
    connecting.sort(key=lambda e: json.dumps(e, sort_keys=True))
    contacts = [e for e in connecting if e['source'] in method_ids and e['target'] in state_ids
                and e.get('kind' if tool == 'compass' else 'relation') in {'references', 'reads', 'writes'}]
    unique = len(methods) == len(states) == 1
    status = ('missing_callable' if not methods else 'ambiguous_callable' if len(methods) != 1 else
              'missing_state' if not states else 'ambiguous_state' if len(states) != 1 else
              'no_contact_edge' if not contacts else 'contact_edge')
    return dict(repository=case['repository'], tool=tool, graphDirected=graph.get('directed'), owner=group['owner'],
                state=group['state'], file=case['file'], declarationLine=group['declarationLine'],
                access=access, methodCandidates=methods, stateCandidates=states,
                connectingRecords=connecting, status=status,
                contactSupported=unique and bool(contacts),
                selectedLineSupported=unique and any(occurrence(e, tool, case['file'], access['line']) for e in contacts))


def git(root, *args):
    return subprocess.check_output(['git', '-C', str(root), *args], text=True, timeout=30).strip()


def evaluate(registration, artifact_root):
    raw = read(registration, MAX_SOURCE_BYTES)
    reg = json.loads(raw)
    if reg['schema'] != 'compass.state-access-development-registration/1':
        raise ValueError('unknown registration schema')
    run_data = read(artifact_root / reg['graphRun'], MAX_SOURCE_BYTES)
    if digest(run_data) != reg['graphRunSha256']:
        raise ValueError('graph run hash mismatch')
    repos = {r['repository']: r for r in json.loads(run_data)['repositories']}
    rows, inputs = [], []
    for case in reg['cases']:
        repo = repos[case['repository']]
        root = Path(repo['source']).resolve()
        path = (root / case['file']).resolve()
        path.relative_to(root)
        if git(root, 'rev-parse', 'HEAD') != case['commit'] or git(root, 'status', '--porcelain'):
            raise ValueError('source checkout pin/status mismatch')
        source = read(path, MAX_SOURCE_BYTES)
        if digest(source) != case['sourceFileSha256']:
            raise ValueError('source hash mismatch')
        lines = source.decode('utf-8').splitlines()
        for group in case['groups']:
            if lines[group['declarationLine']-1] != group['declarationText']:
                raise ValueError('declaration witness mismatch')
            for a in group['accesses']:
                if lines[a['line']-1] != a['text'] or not a['text'][a['column']-1:].startswith(a['expression']):
                    raise ValueError('access witness mismatch')
                if not re.search(r'\b' + re.escape(a['method']) + r'\s*[<(]', lines[a['methodLine']-1]):
                    raise ValueError('method declaration witness mismatch')
        for tool in ('compass', 'graphify'):
            data = read(Path(repo[tool+'Graph']), MAX_GRAPH_BYTES)
            if digest(data) != repo[tool+'GraphSha256']:
                raise ValueError('graph hash mismatch')
            graph = json.loads(data)
            identifiers = {n['id'] for n in graph['nodes']}
            if len(identifiers) != len(graph['nodes']):
                raise ValueError('requires unique graph node IDs')
            if any(e['source'] not in identifiers or e['target'] not in identifiers for e in graph['links']):
                raise ValueError('dangling graph endpoint')
            inputs.append(dict(repository=case['repository'], tool=tool, graphSha256=digest(data),
                               sourceFileSha256=digest(source), graphDirected=graph.get('directed'), bytes=len(data),
                               nodes=len(graph['nodes']), edges=len(graph['links'])))
            rows.extend(assess(graph, tool, case, group, access)
                        for group in case['groups'] for access in group['accesses'])
        # Source files are read-only throughout; do not accept concurrent drift.
        if digest(read(path, MAX_SOURCE_BYTES)) != digest(source) or git(root, 'rev-parse', 'HEAD') != case['commit'] or git(root, 'status', '--porcelain'):
            raise ValueError('source changed during audit')
    summary = {}
    for tool in ('compass', 'graphify'):
        selected = [r for r in rows if r['tool'] == tool]
        summary[tool] = dict(accessSites=len(selected),
            uniqueCallableSites=sum(len(r['methodCandidates']) == 1 for r in selected),
            uniqueStateSlots=len({(r['repository'], r['owner'], r['state']) for r in selected if len(r['stateCandidates']) == 1}),
            contactSupported=sum(r['contactSupported'] for r in selected),
            selectedLineSupported=sum(r['selectedLineSupported'] for r in selected),
            status=dict(sorted(Counter(r['status'] for r in selected).items())))
    return dict(schema='compass.state-access-development-review/1', registrationSha256=digest(raw),
                auditScriptSha256=digest(read(Path(__file__), MAX_SOURCE_BYTES)),
                graphRunSha256=reg['graphRunSha256'], summary=summary, inputs=inputs, results=rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--registration', type=Path, required=True)
    parser.add_argument('--artifact-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--verify', action='store_true', help='recompute and compare an existing result')
    args = parser.parse_args()
    report = evaluate(args.registration, args.artifact_root)
    payload = (json.dumps(report, indent=2) + '\n').encode()
    if args.verify:
        if read(args.output, MAX_GRAPH_BYTES) != payload:
            raise ValueError('saved review differs from recomputed evidence')
    else:
        with args.output.open('xb') as stream:
            stream.write(payload)
    print(json.dumps(report['summary'], indent=2))


if __name__ == '__main__':
    main()
