"""Audit MCP navigation against stored topology and separate reviewed source paths."""
import argparse
from collections import deque
import json
from pathlib import Path
import re
from types import SimpleNamespace

from benchmarks.agent_query.mcp_audit import label, main as verify_capture
from benchmarks.agent_query.path_audit import read_bounded, MAX_GRAPH_BYTES
from benchmarks.agent_query.runner import _node_anchor, _sha256_file

REL = r'[a-z_]+(?:/[a-z_]+)*'
CONF = r'(?: \[[A-Z_]+(?:/[A-Z_]+)*\])?'
EDGE = re.compile(r'--('+REL+')'+CONF+r'-->|<--('+REL+')'+CONF+r'--')


def shortest_distance(graph, source, target):
    adjacency = {n['id']: set() for n in graph['nodes']}
    for e in graph['links']:
        adjacency[e['source']].add(e['target'])
        adjacency[e['target']].add(e['source'])
    if source not in adjacency or target not in adjacency:
        return None
    queue = deque([(source, 0)])
    seen = {source}
    while queue:
        node, distance = queue.popleft()
        if node == target:
            return distance
        for other in adjacency[node]:
            if other not in seen:
                seen.add(other)
                queue.append((other, distance+1))
    return None


def parse_path(text):
    headers = list(re.finditer(r'^Shortest path \((\d+) hops\):\n  (.+)$', text, re.M))
    if len(headers) != 1:
        raise ValueError('expected one actual path body')
    h = headers[0]
    if text[h.end():].strip():
        raise ValueError('unexpected trailing answer')
    hops = int(h[1])
    if hops > 64:
        raise ValueError('hop count outside audit bound')
    body = h[2]; labels = []; steps = []; offset = 0
    for match in EDGE.finditer(body):
        labels.append(body[offset:match.start()].strip())
        steps.append({'relations': (match[1] or match[2]).split('/'),
                      'direction': 'forward' if match[1] else 'reverse'})
        offset = match.end()
    labels.append(body[offset:].strip())
    if hops != len(steps) or any(not x for x in labels):
        raise ValueError('path body differs from printed hop count')
    return labels, steps


def audit(row, graph, question):
    tool = row['tool']; args = row['arguments']; expected = question['expected']['outcome']
    text = row.get('text', '') or row.get('response', {}).get('error', {}).get('message', '')
    result = {'repository': row['repository'], 'tool': tool, 'question': row['question'],
              'expected': expected, 'matched': False, 'executionSucceeded': row['executionSucceeded']}
    if 'captureError' in row:
        result['failure'] = row['captureError']; return result
    positive = 'Shortest path (' in text
    if expected == 'unresolved':
        result['matched'] = not positive and 'No node matching source' in text and args['source'] in text
        return result
    if expected == 'ambiguous':
        result['matched'] = not positive and 'ambig' in text.lower()
        result['selectedDespiteAmbiguity'] = positive
        return result
    distance = shortest_distance(graph, args['source'], args['target'])
    result['storedShortestHops'] = distance
    if expected == 'depth-limit':
        result['matched'] = distance is not None and distance > args['max_hops'] and not positive and 'max_hops' in text
        return result
    if expected == 'disconnected':
        nodes = {n['id']: n for n in graph['nodes']}
        expected_text = f"No path found between '{label(nodes[args['source']],tool)}' and '{label(nodes[args['target']],tool)}'."
        result['matched'] = distance is None and text == expected_text
        return result
    try:
        labels, steps = parse_path(text)
    except ValueError as error:
        result['failure'] = str(error); return result
    names = {}
    for node in graph['nodes']:
        names.setdefault(label(node, tool), []).append(node)
    if any(len(names.get(name, [])) != 1 for name in labels):
        result['failure'] = 'path contains an unverified display identity'; return result
    nodes = [names[name][0] for name in labels]
    ids = [n['id'] for n in nodes]
    result['nodeIds'] = ids
    failures = []
    if ids[0] != args['source'] or ids[-1] != args['target']:
        failures.append('wrong endpoint identity')
    if len(steps) != distance or len(steps) > args['max_hops']:
        failures.append('path is not minimum-hop within the requested bound')
    for left, right, step in zip(ids, ids[1:], steps):
        source, target = (left, right) if step['direction'] == 'forward' else (right, left)
        relations = set()
        for edge in graph['links']:
            a, b = edge['source'], edge['target']
            if tool == 'graphify':
                a, b = edge.get('_src', a), edge.get('_tgt', b)
            if (a, b) == (source, target):
                relations.add(edge.get('kind' if tool == 'compass' else 'relation', 'related'))
        if not set(step['relations']) <= relations:
            failures.append('printed relation/direction lacks a stored witness')
    result.update(matched=not failures, failures=failures, steps=steps)
    witness = question['expected'].get('sourceWitness')
    if witness:
        result['matchesReviewedSourceRoute'] = (
            not failures and len(nodes) == len(witness['nodes'])
            and all(_node_anchor(n, tool)[:2] == (a['file'], a['line']) for n,a in zip(nodes,witness['nodes']))
            and all(s['direction'] == w['direction'] and set(s['relations']) <= set(w['relations'])
                    for s,w in zip(steps,witness['steps'])))
    return result


def main(args):
    verify_capture(SimpleNamespace(run=args.run, output=args.output.with_suffix('.capture.json')))
    run = json.loads(read_bounded(args.run/'run.json'))
    inputs = json.loads(read_bounded(args.run/'inputs.json'))
    source = json.loads(read_bounded(Path(run['sourceRun'])/'run.json'))
    graphs = {}; results = []
    for row in run['results']:
        key = row['repository'], row['tool']
        if key not in graphs:
            repo = next(r for r in source['repositories'] if r['repository'] == key[0])
            graphs[key] = json.loads(read_bounded(Path(repo[key[1]+'Graph']), MAX_GRAPH_BYTES))
        spec = next(r for r in inputs['repositories'] if r['name'] == key[0])
        question = next(q for q in spec['pathQuestions'] if q['id'] == row['question'])
        if row['arguments'] != question['arguments'][key[1]]:
            raise ValueError('request arguments differ from preregistration')
        checked = audit(row, graphs[key], question); results.append(checked)
        print(key, row['question'], checked['matched'], checked.get('failure', checked.get('failures', [])))
    report = {'scope': __doc__, 'runSha256': _sha256_file(args.run/'run.json'),
              'auditorSha256': _sha256_file(Path(__file__)), 'results': results}
    with args.output.open('x') as stream:
        json.dump(report, stream, indent=2)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    main(parser.parse_args())
