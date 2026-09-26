"""Capture preregistered, public MCP operations on existing paired graphs.

This collector does not synthesize tool answers. Input IDs/community IDs are
prepared symmetrically from captured graphs; that preparation is not scored as
node retrieval. It requires separately supplied local server executables.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import shutil
import time
from collections import Counter

from benchmarks.agent_query.mcp_transport import StdioMcp
from benchmarks.agent_query.path_audit import read_bounded, MAX_GRAPH_BYTES, check_source
from benchmarks.agent_query.runner import _sha256_file, _node_anchor, _verify_source, load_suite


def community(node, tool):
    value = node.get('community')
    return value.get('id') if tool == 'compass' and isinstance(value, dict) else value


def prepare_questions(graph, tool, witness):
    matches = [n for n in graph['nodes'] if _node_anchor(n, tool)[:2] == (witness['file'], witness['line'])
               and witness['symbol'] in _node_anchor(n, tool)[2]]
    if len(matches) != 1:
        raise ValueError(f"{tool}: declaration input has {len(matches)} identities")
    counts = Counter(community(n, tool) for n in graph['nodes'] if community(n, tool) is not None)
    if not counts or any(type(k) is not int or k < 0 for k in counts):
        raise ValueError('invalid or missing stored community IDs')
    largest = min(counts, key=lambda k: (-counts[k], k))
    budget = {'token_budget': 262144} if tool == 'graphify' else {}
    queries = [
        ('stats', 'graph_stats', {}),
        ('hubs', 'god_nodes', {'top_n': 10}),
        ('community', 'get_community', {'community_id': largest, **budget}),
        ('missing-community', 'get_community', {'community_id': max(counts)+1, **budget}),
        ('neighbors', 'get_neighbors', {'label': matches[0]['id'], 'relation_filter': 'calls', **budget}),
    ]
    if witness['ambiguousLabel']:
        queries.append(('ambiguous-neighbors', 'get_neighbors', {'label': witness['ambiguousLabel'], **budget}))
    return queries


def skipped_results(repository, tool, questions, graph_digest, argv):
    return [{'repository':repository, 'tool':tool, 'question':identifier,
             'method':method, 'arguments':params, 'graphSha256':graph_digest,
             'argv':argv, 'executionSucceeded':False,
             'captureError':'not executed after the MCP connection failed'}
            for identifier,method,params in questions]


def verify_environment(args):
    manifest = json.loads(read_bounded(args.graphify_environment))
    roots = list(args.graphify_python.parent.parent.glob('lib/python*/site-packages/graphify'))
    if len(roots) != 1:
        raise ValueError('cannot identify Graphify package root')
    for record in manifest['files']:
        relative = Path(record['file'])
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('invalid environment file path')
        if _sha256_file(roots[0]/relative) != record['mcpEnvironmentSha256']:
            raise ValueError('Graphify package changed since environment capture')


def execute(args):
    verify_environment(args)
    source_run = json.loads(read_bounded(args.run/'run.json'))
    suite = load_suite(args.run/'suite.toml')
    if suite.digest != source_run['suiteDigest']:
        raise ValueError('captured suite digest mismatch')
    manifest = json.loads(read_bounded(args.inputs))
    if manifest['schema'] != 'compass.mcp-comparison-inputs/1':
        raise ValueError('unsupported MCP input schema')
    args.output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(args.inputs, args.output/'inputs.json')
    shutil.copy2(args.graphify_environment, args.output/'graphify-environment.json')
    for name in ['mcp_compare.py', 'mcp_transport.py']:
        shutil.copy2(Path(__file__).with_name(name), args.output/name)
    report = {'schema':'compass.mcp-comparison-capture/1', 'sourceRunSha256':_sha256_file(args.run/'run.json'),
              'inputSha256':_sha256_file(args.inputs), 'sourceRun':str(args.run.resolve()),
              'collectorSha256':_sha256_file(Path(__file__)),
              'transportSha256':_sha256_file(Path(__file__).with_name('mcp_transport.py')), 'servers':{}, 'results':[]}
    for tool, binary in [('compass',args.compass),('graphify',args.graphify_python)]:
        report['servers'][tool] = {'executable':str(binary.resolve()), 'executableSha256':_sha256_file(binary)}
    for witness in manifest['repositories']:
        name = witness['name']
        if name not in {'cobra','flask','gson','zod','axum'}:
            raise ValueError('unsupported repository key')
        repo = next(r for r in source_run['repositories'] if r['repository'] == name)
        pinned = next(r for r in suite.repositories if r.name == name)
        source = Path(repo['source'])
        _verify_source(pinned,source)
        check_source(source, {'file':witness['file'],'line':witness['line'],'text':witness['sourceText']})
        for tool in ['compass','graphify']:
            graph_path = Path(repo[tool+'Graph'])
            digest = _sha256_file(graph_path)
            if digest != repo[tool+'GraphSha256']:
                raise ValueError('captured graph changed')
            graph = json.loads(read_bounded(graph_path,MAX_GRAPH_BYTES))
            questions = prepare_questions(graph,tool,witness)
            argv = [str(args.compass),'serve'] if tool == 'compass' else [str(args.graphify_python),'-m','graphify.serve']
            argv += ['--graph',str(graph_path),'--transport','stdio']
            directory = args.output/'raw'/name/tool
            with StdioMcp(argv,source,directory) as session:
                session.initialize()
                listing = session.send('tools/list',{})
                advertised = {t['name'] for t in listing.get('result',{}).get('tools',[])}
                for position, (identifier, method, params) in enumerate(questions):
                    if method not in advertised:
                        raise ValueError(f'{tool} did not advertise {method}')
                    started = time.monotonic()
                    row = {'repository':name,'tool':tool,'question':identifier,'method':method,
                           'arguments':params,'graphSha256':digest,'argv':argv}
                    try:
                        response = session.send('tools/call',{'name':method,'arguments':params})
                        result = response.get('result',{})
                        text = '\n'.join(c['text'] for c in result.get('content',[]) if c.get('type') == 'text')
                        row.update({'response':response,'text':text,'textBytes':len(text.encode()),
                                    'protocolBytes':len(json.dumps(response).encode()),
                                    'executionSucceeded': 'error' not in response and not result.get('isError',False)})
                    except (ValueError,RuntimeError,TimeoutError,OSError) as error:
                        row.update({'executionSucceeded':False,'captureError':str(error)})
                    row['wallMs'] = round((time.monotonic()-started)*1000)
                    report['results'].append(row)
                    (args.output/'run.json').write_text(json.dumps(report,indent=2,sort_keys=True)+'\n')
                    print(name,tool,identifier,row['executionSucceeded'],row.get('textBytes'),flush=True)
                    if 'captureError' in row:
                        report['results'].extend(skipped_results(name,tool,questions[position+1:],digest,argv))
                        break  # A desynchronized connection cannot be reused.
            if _sha256_file(graph_path) != digest:
                raise ValueError('graph changed during MCP questions')
        _verify_source(pinned,source)
    verify_environment(args)
    report['graphifyEnvironmentSha256'] = _sha256_file(args.graphify_environment)
    report['complete'] = True
    (args.output/'run.json').write_text(json.dumps(report,indent=2,sort_keys=True)+'\n')


if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--run',type=Path,required=True)
    p.add_argument('--inputs',type=Path,default=Path(__file__).with_name('suite_mcp.json'))
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--compass',type=Path,required=True)
    p.add_argument('--graphify-python',type=Path,required=True)
    p.add_argument('--graphify-environment',type=Path,required=True)
    execute(p.parse_args())
