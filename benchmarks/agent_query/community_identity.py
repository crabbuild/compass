"""Select public source-anchored resolver inputs/IDs without graph access."""
import re

from benchmarks.agent_query.community_navigation import select_label
from benchmarks.agent_query.runner import _node_anchor, _terminal_symbol

SEARCH_LIMITS = dict(max_candidates=256, max_nodes=500, max_response_bytes=524288)


def resolver_request(tool, text, seed):
    selection = select_label(text, seed)
    # Several presentation variants may refer to the same terminal symbol. No
    # declaration is chosen here: the public resolver must return its anchors.
    if not selection['matchingRows']:
        return dict(selection=selection, request=None)
    symbol = seed['symbol']
    if tool == 'compass':
        request = dict(method='search_symbols', arguments=dict(query=symbol, **SEARCH_LIMITS))
    elif tool == 'graphify':
        if '::' in seed['file']:
            return dict(selection=selection, request=None, reason='unsupported path delimiter')
        request = dict(method='get_node', arguments=dict(label=seed['file']+'::'+symbol))
    else:
        raise ValueError('unsupported tool')
    return dict(selection=selection, request=request)


def selected_id(tool, call, seed):
    result = dict(status='invalid', matchedIds=[], selector=None, returnedAnchors=[])
    if not call.get('executionSucceeded'):
        result['status'] = 'tool-error'
        return result
    if tool == 'compass':
        envelope = call.get('response', {}).get('result', {}).get('structuredContent')
        if not isinstance(envelope, dict) or envelope.get('schema') != 'compass.mcp.tool-result/1':
            return result
        body = envelope.get('result')
        transport = envelope.get('transportTruncation')
        if (not isinstance(body, dict) or body.get('schema') != 'compass.query/1'
                or body.get('operation') != 'search' or not isinstance(transport, dict)):
            return result
        if body.get('truncated') is not False or transport.get('truncated') is not False:
            result['status'] = 'truncated-or-unknown'
            return result
        diagnostics = body.get('diagnostics', [])
        if not isinstance(diagnostics, list) or len(diagnostics) > 1024 or any(not isinstance(d, dict) for d in diagnostics):
            return result
        if any(d.get('code') == 'bounded_truncation' for d in diagnostics):
            result['status'] = 'truncated-or-unknown'
            return result
        nodes, hits = body.get('nodes'), body.get('results')
        if not isinstance(nodes, list) or not isinstance(hits, list) or len(nodes) > 500 or len(hits) > 256:
            return result
        if any(not isinstance(n, dict) or not isinstance(n.get('id'), str) or not n['id'] for n in nodes):
            return result
        ids = [n['id'] for n in nodes]
        if len(ids) != len(set(ids)) or any(not isinstance(h, dict) or h.get('nodeId') not in ids for h in hits):
            return result
        hit_ids = {h['nodeId'] for h in hits}
        anchors = []
        for node in nodes:
            if node['id'] in hit_ids:
                file, line, names = _node_anchor(node, 'compass')
                anchors.append(dict(id=node['id'], file=file, line=line, symbols=sorted(names)))
    elif tool == 'graphify':
        text = call.get('text', '')
        labels = re.findall(r'^Node: (.*)$', text, re.M)
        ids = re.findall(r'^  ID: (.+)$', text, re.M)
        sources = re.findall(r'^  Source: (.+) L([1-9][0-9]*)(?:-L[1-9][0-9]*)?$', text, re.M)
        if len(labels) != 1 or len(ids) != 1 or len(sources) != 1:
            result['status'] = 'unresolved-response'
            return result
        anchors = [dict(id=ids[0], file=sources[0][0], line=int(sources[0][1]),
                        symbols=[_terminal_symbol(labels[0])])]
    else:
        raise ValueError('unsupported tool')
    result['returnedAnchors'] = anchors
    result['matchedIds'] = sorted({a['id'] for a in anchors if a['file'] == seed['file']
                                  and a['line'] == seed['startLine'] and seed['symbol'] in a['symbols']})
    matches = result['matchedIds']
    result['status'] = 'resolved' if len(matches) == 1 else 'anchor-mismatch' if not matches else 'ambiguous'
    result['selector'] = matches[0] if len(matches) == 1 else None
    return result
