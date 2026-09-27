"""Output-only follow-ups and post-request identity/projection checks.

Direct identity is different from globally unique labels and from a second,
source-assisted resolver call. Graph consistency does not score source precision.
"""
from collections import Counter, defaultdict
import json

from benchmarks.agent_query.community_identity import SEARCH_LIMITS, selected_id
from benchmarks.agent_query.community_navigation import NEIGHBOR
from benchmarks.agent_query.runner import _terminal_symbol


def destination_request(tool, neighbor_call, target):
    """No graph argument: only displayed outgoing calls and public coordinates."""
    if not neighbor_call.get('executionSucceeded'):
        return None
    labels = sorted({name for direction, name, relation in NEIGHBOR.findall(neighbor_call.get('text', ''))
                     if direction == '-->' and 'calls' in relation.lower()
                     and _terminal_symbol(name) == target['symbol']})
    if not labels:
        return None
    if tool == 'compass':
        return dict(method='search_symbols', arguments=dict(query=target['symbol'], **SEARCH_LIMITS))
    if tool == 'graphify':
        if '::' in target['file']:
            return None
        return dict(method='get_node', arguments=dict(label=target['file']+'::'+target['symbol']))
    raise ValueError('unsupported tool')


def followup_score(tool, call, target, expected_id):
    choice = selected_id(tool, call, target)
    return dict(selection=choice, sourceMatchedDestination=expected_id is not None and choice['selector'] == expected_id)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False)


def audit_direct(call, graph, tool, seed, target):
    result = dict(status='unavailable', explicitDestinationSupported=False,
                  fullProjectionMatches=False, returnedGroups=0, returnedRecordAppearances=0)
    if not call.get('executionSucceeded'):
        result['status'] = 'tool-error'
        return result
    envelope = call.get('response', {}).get('result', {}).get('structuredContent')
    # Graphify 0.9.67 emits labels/relations/sites, with no neighbor identity
    # field. Absence is unavailable, never guessed from global label uniqueness.
    if envelope is None:
        return result
    result['status'] = 'invalid'
    if (not isinstance(envelope, dict) or envelope.get('schema') != 'compass.mcp.tool-result/1'
            or envelope.get('transportTruncation', {}).get('truncated') is not False):
        return result
    body = envelope.get('result')
    if (not isinstance(body, dict) or body.get('schema') != 'compass.query.neighbors/1'
            or body.get('truncated') is not False or body.get('directionBasis') != 'stored-endpoints'
            or body.get('relationFilter') != 'calls'):
        return result
    nodes = {n['id']: n for n in graph['nodes']}
    if body.get('seed') != nodes.get(seed) or body.get('graphDirected') is not graph.get('directed', False):
        return result
    groups = body.get('neighbors')
    if not isinstance(groups, list) or len(groups) > 20000:
        return result
    expected = defaultdict(Counter)
    for edge in graph['links']:
        if 'calls' not in edge.get('kind' if tool == 'compass' else 'relation', '').lower():
            continue
        if edge['source'] == seed:
            expected['outgoing', edge['target']][canonical(edge)] += 1
        if edge['target'] == seed:
            expected['incoming', edge['source']][canonical(edge)] += 1
    observed = {}
    for group in groups:
        if not isinstance(group, dict) or group.get('direction') not in {'incoming', 'outgoing'}:
            return result
        node, edges = group.get('node'), group.get('edges')
        if not isinstance(node, dict) or not isinstance(node.get('id'), str) or node != nodes.get(node['id']):
            return result
        key = group['direction'], node['id']
        if key in observed or not isinstance(edges, list) or not edges or len(edges) > 10000:
            return result
        if any(not isinstance(edge, dict) for edge in edges):
            return result
        observed[key] = Counter(map(canonical, edges))
    result.update(status='checked', fullProjectionMatches=observed == expected,
                  returnedGroups=len(groups), returnedRecordAppearances=sum(sum(v.values()) for v in observed.values()))
    result['explicitDestinationSupported'] = (observed == expected and target is not None
                                             and bool(observed.get(('outgoing', target))))
    return result
