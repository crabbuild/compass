"""Bounded community-to-neighbor development capture with output-only selectors.

Scoring consults frozen graphs after requests. Displayed name matches are kept
separate from conservative exact-identity support and source-call requirements.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import shutil
import time

from benchmarks.agent_query.community_tasks import (
    MAX_JSON_BYTES, MAX_SOURCE_BYTES, audit_graph, digest, read_bounded, verify_source,
)
from benchmarks.agent_query.mcp_audit import audit as audit_membership, label
from benchmarks.agent_query.mcp_compare import captured_repository, verify_environment
from benchmarks.agent_query.mcp_transport import StdioMcp
from benchmarks.agent_query.runner import _sha256_file, _terminal_symbol


MEMBER = re.compile(r"^  (.*) \[(.*)\]$", re.M)
NEIGHBOR = re.compile(r"^  (-->|<--) (.*?) \[([^\]]*)\] \[[^\]]*\](?: at=.*)?$", re.M)
ERRORS = (OSError, RuntimeError, ValueError, TimeoutError)


def select_label(text: str, seed: dict) -> dict:
    """Only public task coordinates and returned text may select the follow-up."""
    rows = [dict(label=name, file=file) for name, file in MEMBER.findall(text)
            if file == seed['file'] and _terminal_symbol(name) == seed['symbol']]
    choices = sorted({row['label'] for row in rows})
    return dict(matchingRows=rows, distinctLabels=choices,
                selector=choices[0] if len(choices) == 1 else None,
                status='selected' if len(choices) == 1 else 'missing' if not choices else 'ambiguous')


def score_navigation(text: str, graph: dict, tool: str, seed: str, target: str,
                     succeeded: bool) -> dict:
    """Never identify a duplicate label by its conveniently matching neighbors."""
    nodes = {node['id']: node for node in graph['nodes']}
    names = defaultdict(list)
    for node in nodes.values():
        names[label(node, tool)].append(node['id'])
    seed_name, target_name = label(nodes[seed], tool), label(nodes[target], tool)
    heading = text.splitlines()[0] if text else ''
    header_matches = succeeded and heading == f'Neighbors of {seed_name}:'
    actual = Counter((direction, name) for direction, name, relation in NEIGHBOR.findall(text)
                     if 'calls' in relation.lower())
    pairs = set()
    direct_edges = []
    for edge in graph['links']:
        relation = edge.get('kind' if tool == 'compass' else 'relation', '')
        if 'calls' not in relation.lower():
            continue
        a, b = edge['source'], edge['target']
        if a == seed:
            pairs.add(('-->', b))
            if b == target:
                direct_edges.append(edge)
        if b == seed:
            pairs.add(('<--', a))
    expected = Counter((direction, label(nodes[identifier], tool)) for direction, identifier in pairs)
    identity = header_matches and names[seed_name] == [seed]
    displayed = header_matches and actual['-->', target_name] > 0
    return dict(seedHeadingMatches=header_matches,
                ambiguityReported=bool(re.search(r'\bambiguous\b', text, re.I)),
                seedLabelCandidates=sorted(names[seed_name]), seedIdentitySupported=identity,
                expectedDisplayedNeighbors=sum(expected.values()), returnedDisplayedNeighbors=sum(actual.values()),
                displayedAdjacencyMatches=header_matches and expected == actual,
                missingDisplayedNeighbors=list((expected-actual).elements()),
                extraDisplayedNeighbors=list((actual-expected).elements()),
                collaboratorDisplayed=displayed,
                collaboratorLabelCandidates=sorted(names[target_name]),
                collaboratorIdentitySupported=displayed and identity and names[target_name] == [target],
                graphDirectCallRecords=len(direct_edges))


def call(session, method, arguments):
    started = time.monotonic()
    row = dict(method=method, arguments=arguments, executionSucceeded=False)
    try:
        packet = session.send('tools/call', dict(name=method, arguments=arguments))
        result = packet.get('result', {})
        text = '\n'.join(item['text'] for item in result.get('content', []) if item.get('type') == 'text')
        raw = read_bounded(session.directory / f'{packet["id"]:02}.response.jsonl', 1048576)
        if packet not in [json.loads(line) for line in raw.splitlines() if line]:
            raise ValueError('response differs from saved transcript')
        row.update(response=packet, text=text, textBytes=len(text.encode()), wireResponseBytes=len(raw),
                   executionSucceeded='error' not in packet and not result.get('isError', False))
    except ERRORS as error:
        row['captureError'] = str(error)
    row['elapsedSeconds'] = time.monotonic() - started
    return row


def execute(args):
    policy_bytes = read_bounded(args.policy, MAX_SOURCE_BYTES)
    registration_bytes = read_bounded(args.registration, MAX_SOURCE_BYTES)
    run_bytes = read_bounded(args.run, MAX_JSON_BYTES)
    policy, registration, run = map(json.loads, [policy_bytes, registration_bytes, run_bytes])
    if policy.get('schema') != 'compass.community-navigation-policy/1':
        raise ValueError('unsupported workflow policy')
    if registration.get('schema') != 'compass.community-task-pairs/1':
        raise ValueError('unsupported task registration')
    if digest(registration_bytes) != policy['registrationSha256'] or digest(run_bytes) != registration['sourceRunSha256']:
        raise ValueError('registered input hash differs')
    if policy['bounds'] != dict(requestTimeoutSeconds=60, maxResponseBytes=1048576,
                               maxSessionBytes=67108864, graphifyTokenBudget=262144):
        raise ValueError('unsupported workflow bounds')
    repositories = registration['repositories']
    if not 1 <= len(repositories) <= 32 or len({r['repository'] for r in repositories}) != len(repositories):
        raise ValueError('invalid repository list')
    direct = policy['directCallTasks']
    all_tasks = [t['id'] for r in repositories for t in r['tasks']]
    if len(set(all_tasks)) != len(all_tasks) or len(set(direct)) != len(direct) or not set(direct) <= set(all_tasks):
        raise ValueError('invalid direct-call task list')
    verify_environment(args)
    args.output.mkdir(parents=True, exist_ok=False)
    report = dict(schema='compass.community-navigation-capture/1', complete=False,
                  policySha256=digest(policy_bytes), registrationSha256=digest(registration_bytes),
                  sourceRunSha256=digest(run_bytes), sourceRun=str(args.run.resolve()),
                  graphifyEnvironmentSha256=_sha256_file(args.graphify_environment),
                  servers={}, supportFiles={}, sessions=[], results=[])
    for tool, path in [('compass', args.compass), ('graphify', args.graphify_python)]:
        report['servers'][tool] = dict(path=str(path), sha256=_sha256_file(path))
    for path in [args.policy, args.registration, args.graphify_environment, *[
            Path(__file__).with_name(name) for name in ['community_navigation.py', 'community_tasks.py',
                'mcp_transport.py', 'mcp_compare.py', 'mcp_audit.py', 'runner.py']]]:
        shutil.copy2(path, args.output/path.name)
        report['supportFiles'][path.name] = _sha256_file(path)
    def save():
        (args.output/'capture.json').write_text(json.dumps(report, indent=2)+'\n')
    save()
    for repository in repositories:
        name = repository['repository']
        previous = captured_repository(run, name)
        root = Path(previous['source'])
        verify_source(repository, root)
        for tool in ['compass', 'graphify']:
            path = Path(previous[tool+'Graph'])
            data = read_bounded(path, registration['policy']['bounds']['maxGraphBytes'])
            graph_hash = digest(data)
            if graph_hash != repository['graphSha256'][tool]:
                raise ValueError('registered graph hash differs')
            graph = json.loads(data)
            prepared = audit_graph(graph, tool, repository['tasks'], registration['policy']['bounds'])
            declarations = {d['id']: d for d in prepared['declarations']}
            argv = [str(args.compass), 'serve'] if tool == 'compass' else [str(args.graphify_python), '-m', 'graphify.serve']
            argv += ['--graph', str(path), '--transport', 'stdio']
            budget = {'token_budget':262144} if tool == 'graphify' else {}
            directory = args.output/'raw'/name/tool
            session_row = dict(repository=name, tool=tool, argv=argv)
            failure = None
            started = time.monotonic()
            try:
                with StdioMcp(argv, root, directory, timeout=60, max_bytes=1048576) as session:
                    try:
                        session.initialize()
                        listing = session.send('tools/list', {})
                        advertised = {t['name'] for t in listing.get('result', {}).get('tools', [])}
                        if not {'get_community', 'get_neighbors'} <= advertised:
                            raise ValueError('required public tools unavailable')
                    except ERRORS as error:
                        failure = str(error)
                    session_row['startupSeconds'] = time.monotonic() - started
                    for task in repository['tasks']:
                        if len(task['declarations']) != 2:
                            raise ValueError('workflow requires paired declarations')
                        seed, target = [declarations[d['id']] for d in task['declarations']]
                        row = dict(repository=name, tool=tool, task=task['id'], seed=seed, target=target,
                                   graphSha256=graph_hash, directCallRequired=task['id'] in direct)
                        if failure is not None:
                            row['captureError'] = 'connection unavailable: '+failure
                        elif seed['status'] != 'resolved' or target['status'] != 'resolved':
                            row['inputUnresolved'] = True
                        else:
                            row['splitCommunity'] = seed['community'] != target['community']
                            community = call(session, 'get_community', dict(community_id=seed['community'], **budget))
                            row['communityCall'] = community
                            if 'captureError' in community:
                                failure = community['captureError']
                            elif community['executionSucceeded']:
                                # The selector function has no graph access.
                                selection = select_label(community['text'], task['declarations'][0])
                                row['selection'] = selection
                                if selection['selector'] is not None:
                                    neighbors = call(session, 'get_neighbors', dict(label=selection['selector'], relation_filter='calls', **budget))
                                    row['neighborCall'] = neighbors
                                    if 'captureError' in neighbors:
                                        failure = neighbors['captureError']
                                    row['audit'] = score_navigation(neighbors.get('text', ''), graph, tool,
                                        seed['matchedNodeIds'][0], target['matchedNodeIds'][0], neighbors['executionSucceeded'])
                                # Scoring is after requests, never selector preparation.
                                row['communityAudit'] = audit_membership(dict(repository=name, tool=tool,
                                    question='community', **community), graph)
                        report['results'].append(row)
                        save()
                        print(name, tool, task['id'], row.get('selection', {}).get('status'),
                              row.get('audit', {}).get('seedIdentitySupported', False),
                              row.get('audit', {}).get('collaboratorIdentitySupported', False), flush=True)
            except OSError as error:
                session_row['launchError'] = str(error)
                completed = {r['task'] for r in report['results'] if r['repository'] == name and r['tool'] == tool}
                for task in repository['tasks']:
                    if task['id'] not in completed:
                        report['results'].append(dict(repository=name, tool=tool, task=task['id'],
                            directCallRequired=task['id'] in direct, captureError=str(error)))
            session_row.update(elapsedSeconds=time.monotonic()-started,
                              wireFiles={p.name:p.stat().st_size for p in sorted(directory.glob('*')) if p.is_file()})
            report['sessions'].append(session_row)
            save()
            if _sha256_file(path) != graph_hash:
                raise ValueError('graph changed during workflow')
        verify_source(repository, root)
    verify_environment(args)
    for server in report['servers'].values():
        if _sha256_file(Path(server['path'])) != server['sha256']:
            raise ValueError('server executable changed')
    report['complete'] = True
    save()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['policy', 'registration', 'run', 'output', 'compass', 'graphify-python', 'graphify-environment']:
        parser.add_argument('--'+name, type=Path, required=True)
    execute(parser.parse_args())
