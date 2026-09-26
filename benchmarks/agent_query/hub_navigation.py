"""One-follow-up hub navigation diagnostic; selectors come only from tool output.

This development diagnostic checks displayed adjacency against stored graphs,
not source precision or god-object design quality. See COVERAGE_PLAN.md.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
import shutil
import time
from types import SimpleNamespace

from benchmarks.agent_query.mcp_audit import audit, label, main as audit_capture
from benchmarks.agent_query.mcp_compare import verify_environment
from benchmarks.agent_query.mcp_transport import StdioMcp
from benchmarks.agent_query.path_audit import read_bounded, MAX_GRAPH_BYTES
from benchmarks.agent_query.runner import _sha256_file, _verify_source, load_suite


def selectors(row):
    """Never consult a graph or inferred degree to choose a selector."""
    structured = row['response'].get('result', {}).get('structuredContent')
    if structured is not None:
        if structured.get('schema') != 'compass.mcp.tool-result/1' or structured.get('result', {}).get('schema') != 'compass.mcp.hubs/1':
            raise ValueError('unsupported structured hub response')
        return [(n['rank'], n['id'], 'explicit-id') for n in structured['result']['nodes']]
    return [(int(rank), name, 'display-label') for rank, name in
            re.findall(r'^  (\d+)\. (.*) - \d+ edges$', row['text'], re.M)]


def check_navigation(text, graph, tool, seed):
    """Seed is oracle identity, never a substituted request selector."""
    nodes = {n['id']: n for n in graph['nodes']}
    if seed not in nodes:
        return {'identityKnown': False, 'navigationResolved': False,
                'ambiguityReported': 'ambig' in text.lower()}
    expected = set()
    for edge in graph['links']:
        a, b = edge['source'], edge['target']
        if a == seed:
            expected.add(('-->', b))
        if b == seed:
            expected.add(('<--', a))
    expected_labels = Counter((direction, label(nodes[n], tool)) for direction, n in expected)
    actual = Counter(re.findall(r'^  (-->|<--) (.*?) \[[^\]]*\] \[[^\]]*\]', text, re.M))
    header = text.splitlines()[0] if text else ''
    header_matches = header == f'Neighbors of {label(nodes[seed], tool)}:'
    return {'identityKnown': True, 'seed': seed,
            'expectedDisplayedNeighbors': sum(expected_labels.values()),
            'returnedDisplayedNeighbors': sum(actual.values()),
            'displayedAdjacencyMatches': expected_labels == actual,
            'navigationResolved': header_matches and expected_labels == actual}


def main(args):
    args.output.mkdir(parents=True, exist_ok=False)
    # Verify the source capture, graphs, and complete raw RPC transcripts first.
    audit_capture(SimpleNamespace(run=args.run, output=args.output/'input-audit.json'))
    run = json.loads(read_bounded(args.run/'run.json'))
    source_run = Path(run['sourceRun'])
    source = json.loads(read_bounded(source_run/'run.json'))
    suite = load_suite(source_run/'suite.toml')
    if suite.digest != source['suiteDigest']:
        raise ValueError('source suite changed')
    environment = SimpleNamespace(
        graphify_python=Path(run['servers']['graphify']['executable']),
        graphify_environment=args.run/'graphify-environment.json')
    verify_environment(environment)
    for record in run['servers'].values():
        if _sha256_file(Path(record['executable'])) != record['executableSha256']:
            raise ValueError('captured server executable changed')
    shutil.copy2(__file__, args.output/'hub_navigation.py')
    report = {'scope': __doc__, 'inputRun': str(args.run.resolve()),
              'inputRunSha256': _sha256_file(args.run/'run.json'),
              'collectorSha256': _sha256_file(Path(__file__)), 'results': [], 'complete': False}
    report['supportFiles'] = {}
    for name in ['mcp_audit.py', 'mcp_transport.py', 'mcp_compare.py', 'runner.py', 'path_audit.py', 'COVERAGE_PLAN.md']:
        path = Path(__file__).with_name(name)
        shutil.copy2(path, args.output/name)
        report['supportFiles'][name] = _sha256_file(path)
    def save():
        (args.output/'run.json').write_text(json.dumps(report, indent=2)+'\n')
    save()
    for row in run['results']:
        if row['question'] != 'hubs':
            continue
        if not row['executionSucceeded']:
            raise ValueError('hub question failed; retain it as an incomplete input workflow')
        name, tool = row['repository'], row['tool']
        repo = next(r for r in source['repositories'] if r['repository'] == name)
        pinned = next(r for r in suite.repositories if r.name == name)
        root, graph_path = Path(repo['source']), Path(repo[tool+'Graph'])
        _verify_source(pinned, root)
        digest = _sha256_file(graph_path)
        if digest != repo[tool+'GraphSha256']:
            raise ValueError('input graph changed')
        graph = json.loads(read_bounded(graph_path, MAX_GRAPH_BYTES))
        checked = audit(row, graph)
        choices = selectors(row)
        if len(choices) != 10 or len(checked['hubs']) != 10:
            raise ValueError('expected ten captured hubs; input workflow incomplete')
        binary = run['servers'][tool]['executable']
        argv = [binary, 'serve'] if tool == 'compass' else [binary, '-m', 'graphify.serve']
        argv += ['--graph', str(graph_path), '--transport', 'stdio']
        failure = None
        with StdioMcp(argv, root, args.output/'raw'/name/tool) as session:
            try:
                session.initialize()
            except (OSError, RuntimeError, ValueError, TimeoutError) as error:
                failure = str(error)
            for (rank, selector, mode), oracle in zip(choices, checked['hubs']):
                params = {'label': selector}
                if tool == 'graphify':
                    params['token_budget'] = 262144
                result = {'repository': name, 'tool': tool, 'rank': rank, 'selectorMode': mode,
                          'arguments': params, 'argv': argv, 'graphSha256': digest,
                          'executionSucceeded': False, 'navigationResolved': False}
                if failure is not None:
                    result['captureError'] = f'connection unavailable: {failure}'
                else:
                    started = time.monotonic()
                    try:
                        packet = session.send('tools/call', {'name': 'get_neighbors', 'arguments': params})
                        answer = packet.get('result', {})
                        text = '\n'.join(c['text'] for c in answer.get('content', []) if c.get('type') == 'text')
                        result.update(response=packet, text=text, textBytes=len(text.encode()),
                                      executionSucceeded='error' not in packet and not answer.get('isError', False))
                        raw = session.directory/f'{packet["id"]:02}.response.jsonl'
                        result['wireResponseBytes'] = raw.stat().st_size
                        if result['executionSucceeded']:
                            result.update(check_navigation(text, graph, tool, oracle.get('id')))
                    except (OSError, RuntimeError, ValueError, TimeoutError) as error:
                        failure = str(error)
                        result['captureError'] = failure
                    result['elapsedSeconds'] = time.monotonic() - started
                report['results'].append(result)
                save()
                print(name, tool, rank, result['navigationResolved'], flush=True)
        _verify_source(pinned, root)
        if _sha256_file(graph_path) != digest:
            raise ValueError('graph changed during navigation')
    verify_environment(environment)
    for record in run['servers'].values():
        if _sha256_file(Path(record['executable'])) != record['executableSha256']:
            raise ValueError('server executable changed during navigation')
    report['complete'] = True
    save()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    main(parser.parse_args())
