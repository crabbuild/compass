"""Audit hub connectivity summaries and optional post-output source-role reviews.

Counts describe each tool's own stored graph. Missing summaries are unavailable,
not incorrect answers; follow-up/CLI workflows are not scored by this diagnostic.
Source roles do not establish cohesion, responsibility count, or god-object defects.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
from types import SimpleNamespace

from benchmarks.agent_query.mcp_audit import audit, main as verify_capture
from benchmarks.agent_query.path_audit import read_bounded, MAX_GRAPH_BYTES
from benchmarks.agent_query.runner import _node_anchor, _sha256_file, _verify_source, load_suite


def connectivity(graph, tool, identifier):
    nodes = {n['id'] for n in graph['nodes']}
    if identifier not in nodes:
        raise ValueError('unknown hub identity')
    directed = graph.get('directed', False)
    relations = {}
    loops = 0
    for edge in graph['links']:
        source, target = edge['source'], edge['target']
        if source not in nodes or target not in nodes or identifier not in (source, target):
            continue
        relation = edge.get('kind' if tool == 'compass' else 'relation', '')
        row = relations.setdefault(relation, dict(relation=relation, edgeRecords=0,
            incomingRecords=0, outgoingRecords=0, undirectedRecords=0))
        row['edgeRecords'] += 1
        loops += source == target
        if directed:
            row['incomingRecords'] += target == identifier
            row['outgoingRecords'] += source == identifier
        else:
            row['undirectedRecords'] += 1
    ordered = sorted(relations.values(), key=lambda row: (-row['edgeRecords'], row['relation']))
    return dict(schema='compass.hub-connectivity/1', directed=directed,
        edgeRecords=sum(row['edgeRecords'] for row in ordered), selfLoopRecords=loops,
        relations=ordered[:16], omittedRelationKinds=max(0, len(ordered)-16),
        omittedRelationRecords=sum(row['edgeRecords'] for row in ordered[16:]))


def check_summary(actual, expected):
    # Canonical JSON distinguishes boolean counters from integers, unlike ==.
    return json.dumps(actual, sort_keys=True) == json.dumps(expected, sort_keys=True)


def check_request(request, row):
    expected = dict(jsonrpc='2.0', method='tools/call', id=row['response']['id'],
        params=dict(name='god_nodes', arguments=row['arguments']))
    if not check_summary(request, expected):
        raise ValueError('hub request differs from raw transcript')


def check_text_rows(text, rank, expected):
    headers = list(re.finditer(r'^  (\d+)\. .* - \d+ edges$', text, re.M))
    matches = [i for i, header in enumerate(headers) if int(header[1]) == rank]
    if len(matches) != 1:
        return False
    position = matches[0]
    end = headers[position+1].start() if position+1 < len(headers) else len(text)
    block = text[headers[position].end():end].splitlines()
    wanted = [f"    relation {json.dumps(r['relation'], ensure_ascii=False)}: incoming {r['incomingRecords']}, outgoing {r['outgoingRecords']}, undirected {r['undirectedRecords']}" for r in expected['relations']]
    actual = [line for line in block if line.startswith('    relation ')]
    totals = f" | incident records: {expected['edgeRecords']} | self-loops: {expected['selfLoopRecords']}"
    omitted = [line for line in block if ' additional relation kinds (' in line]
    omissions = ([f"    {expected['omittedRelationKinds']} additional relation kinds ({expected['omittedRelationRecords']} records) omitted"]
        if expected['omittedRelationKinds'] else [])
    return actual == wanted and omitted == omissions and any(line.startswith('    kind: ') and line.endswith(totals) for line in block)


def check_review(review, node, tool, root):
    file, line, _ = _node_anchor(node, tool)
    if (review['id'], review['file'], review['line']) != (node['id'], file, line):
        raise ValueError('review identity/source differs from captured graph')
    path = (root/file).resolve()
    path.relative_to(root.resolve())
    data = read_bounded(path, 4 * 1024 * 1024)
    if _sha256_file(path) != review['sourceFileSha256']:
        raise ValueError('reviewed source file changed')
    lines = data.decode().splitlines()
    if type(line) is not int or not 1 <= line <= len(lines):
        raise ValueError('invalid review source line')
    if '\n'.join(lines[line-1:line+3]) != review['sourceText']:
        raise ValueError('review excerpt differs from source')


def main(args):
    verify_capture(SimpleNamespace(run=args.run, output=args.output.with_suffix('.capture.json')))
    run = json.loads(read_bounded(args.run/'run.json'))
    source_run = Path(run['sourceRun'])
    source = json.loads(read_bounded(source_run/'run.json'))
    suite = load_suite(source_run/'suite.toml')
    if suite.digest != source['suiteDigest']:
        raise ValueError('source suite changed')
    reviews = None
    if args.reviews:
        manifest = json.loads(read_bounded(args.reviews))
        if manifest['schema'] != 'compass.hub-source-review/1' or manifest['inputRunSha256'] != _sha256_file(args.run/'run.json'):
            raise ValueError('reviews do not identify this source capture')
        reviews = {(r['repository'], r['tool'], r['rank']): r for r in manifest['reviews']}
        if len(reviews) != len(manifest['reviews']) or len(reviews) != 100:
            raise ValueError('review denominator must contain 100 unique hub entries')
    results = []
    for row in run['results']:
        if row['question'] != 'hubs':
            continue
        name, tool = row['repository'], row['tool']
        if row['executionSucceeded']:
            identity = row['response']['id']
            request = json.loads(read_bounded(args.run/'raw'/name/tool/f'{identity:02}.request.json'))
            check_request(request, row)
        repo = next(r for r in source['repositories'] if r['repository'] == name)
        graph = json.loads(read_bounded(Path(repo[tool+'Graph']), MAX_GRAPH_BYTES))
        nodes = {n['id']: n for n in graph['nodes']}
        if reviews is not None:
            pinned = next(r for r in suite.repositories if r.name == name)
            _verify_source(pinned, Path(repo['source']))
            expected_source = next(r for r in manifest['sources'] if r['repository'] == name)
            if any(repo[k] != expected_source[k] for k in ['commit', 'compassGraphSha256', 'graphifyGraphSha256']):
                raise ValueError('review source provenance differs')
        checked = audit(row, graph)
        entries = row.get('response', {}).get('result', {}).get('structuredContent', {}).get('result', {}).get('nodes', [])
        by_rank = {r['rank']: r for r in entries}
        hubs = checked.get('hubs', [])
        if not row['executionSucceeded'] or len(hubs) != 10:
            raise ValueError('capture does not supply the full ten-hub denominator')
        for hub in hubs:
            result = dict(repository=name, tool=tool, rank=hub['rank'],
                label=hub['label'], identityKnown='id' in hub, summaryAvailable=False)
            if 'id' in hub:
                expected = connectivity(graph, tool, hub['id'])
                result.update(id=hub['id'], expected=expected)
                actual = by_rank.get(hub['rank'], {}).get('connectivity')
                if actual is not None:
                    result.update(summaryAvailable=True, summaryMatches=check_summary(actual, expected))
                    result['textRowsMatch'] = check_text_rows(row['text'], hub['rank'], expected)
            if reviews is not None:
                review = reviews[name, tool, hub['rank']]
                if 'id' in hub:
                    if review['role'] not in manifest['roles']:
                        raise ValueError('missing reviewed source role')
                    check_review(review, nodes[hub['id']], tool, Path(repo['source']))
                    result.update(reviewedRole=review['role'], sourceVerified=True)
                elif review['role'] is not None:
                    raise ValueError('ambiguous hub cannot borrow an oracle identity')
                else:
                    result['reviewedRole'] = 'unverified-identity'
            results.append(result)
    if len(results) != 100:
        raise ValueError('expected fifty returned hubs per tool')
    report = dict(scope=__doc__, runSha256=_sha256_file(args.run/'run.json'),
        auditorSha256=_sha256_file(Path(__file__)), results=results)
    if reviews is not None:
        report['reviewsSha256'] = _sha256_file(args.reviews)
    with args.output.open('x') as stream:
        json.dump(report, stream, indent=2)
    for tool in ['compass', 'graphify']:
        rows = [r for r in results if r['tool'] == tool]
        print(tool, dict(total=len(rows), identityKnown=sum(r['identityKnown'] for r in rows),
            summaryAvailable=sum(r['summaryAvailable'] for r in rows),
            summaryMatches=sum(r.get('summaryMatches', False) for r in rows),
            roles=dict(Counter(r.get('reviewedRole', 'not-reviewed') for r in rows))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--reviews', type=Path)
    main(parser.parse_args())
