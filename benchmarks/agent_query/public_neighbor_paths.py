"""Bounded BFS using only public MCP results; no graph or source-file access.

Returned paths are candidates until independently checked against graph records
and source. In particular, Graphify's label resolver can conflate declarations.
"""
from collections import deque
import re
import time


class WorkflowStop(ValueError):
    pass


def compass_node(node):
    source = node.get('source') or {}
    result = dict(id=node.get('id'), label=node.get('name'),
                  file=source.get('file'), line=source.get('startLine'))
    return checked_node(result)


def checked_node(node):
    if (any(not isinstance(node.get(k), str) or not node[k] for k in ['id', 'label', 'file'])
            or type(node.get('line')) is not int or node['line'] < 1):
        raise WorkflowStop('missing-public-node-identity')
    return node


def graphify_node(call, expected_label=None):
    if not call.get('executionSucceeded'):
        return None
    text = call.get('text', '')
    names = re.findall(r'^Node: (.+)$', text, re.M)
    ids = re.findall(r'^  ID: (.+)$', text, re.M)
    sources = re.findall(r'^  Source: (.+) L([1-9][0-9]*)(?:-L[1-9][0-9]*)?$', text, re.M)
    if len(names) != 1 or len(ids) != 1 or len(sources) != 1:
        return None
    if expected_label is not None and names[0] != expected_label:
        return None
    return checked_node(dict(id=ids[0], label=names[0], file=sources[0][0], line=int(sources[0][1])))


def neighbors(tool, call, seed):
    """Return outgoing groups, retaining public records for later adjudication."""
    if not call.get('executionSucceeded'):
        raise WorkflowStop('neighbor-tool-error')
    if tool == 'compass':
        env = call.get('response', {}).get('result', {}).get('structuredContent') or {}
        body = env.get('result') or {}
        if (env.get('schema') != 'compass.mcp.tool-result/1'
                or env.get('transportTruncation', {}).get('truncated') is not False
                or body.get('schema') != 'compass.query.neighbors/1'
                or body.get('truncated') is not False
                or body.get('directionBasis') != 'stored-endpoints'
                or body.get('graphDirected') is not True
                or body.get('relationFilter') != 'calls'
                or body.get('seed', {}).get('id') != seed['id']):
            raise WorkflowStop('invalid-or-truncated-neighbors')
        groups = body.get('neighbors')
        if not isinstance(groups, list) or len(groups) > 20000:
            raise WorkflowStop('invalid-neighbor-groups')
        result = []
        for group in groups:
            if group.get('direction') not in {'incoming', 'outgoing'}:
                raise WorkflowStop('invalid-neighbor-direction')
            if group['direction'] != 'outgoing':
                continue
            node = compass_node(group.get('node') or {})
            records = group.get('edges')
            if not isinstance(records, list) or not records or len(records) > 10000:
                raise WorkflowStop('invalid-neighbor-records')
            kept = []
            for edge in records:
                if (edge.get('source') != seed['id'] or edge.get('target') != node['id']
                        or edge.get('kind') != 'calls'):
                    raise WorkflowStop('invalid-call-record')
                evidence = edge.get('evidence') or []
                if evidence and all(e.get('origin') == 'ast' and e.get('confidence') == 'exact' for e in evidence):
                    kept.append(edge)
            if kept:
                result.append(dict(node=node, records=kept))
        return result
    if tool != 'graphify':
        raise ValueError('unsupported tool')
    lines = call.get('text', '').splitlines()
    if not lines or lines[0] != 'Neighbors of '+seed['label']+':':
        raise WorkflowStop('invalid-or-truncated-neighbors')
    result = []
    pattern = r'^  (-->|<--) (.+?) \[([^\]]+)\] \[([^\]]*)\](?: at=(.*))?$'
    for line in lines[1:]:
        match = re.fullmatch(pattern, line)
        if match is None:
            raise WorkflowStop('invalid-or-truncated-neighbor-line')
        direction, label, relation, confidence, site = match.groups()
        if direction == '-->' and relation == 'calls' and confidence == 'EXTRACTED':
            result.append(dict(label=label, records=[dict(line=line, site=site, relation=relation,
                                                        confidence=confidence)]))
    return result


class PublicWalk:
    """One fixed request/byte/time budget, including endpoint resolution."""
    def __init__(self, tool, invoke, bounds, clock=time.monotonic):
        self.tool, self.invoke, self.bounds, self.clock = tool, invoke, bounds, clock
        self.started = clock()
        self.calls, self.bytes, self.expanded, self.groups = [], 0, 0, 0
        self.identity_gaps, self.cache = [], {}

    def request(self, method, arguments):
        if self.clock()-self.started >= self.bounds['workflowTimeoutSeconds']:
            raise WorkflowStop('workflow-time-limit')
        if len(self.calls) >= self.bounds['maxRequests']:
            raise WorkflowStop('request-limit')
        row = self.invoke(method, arguments)
        self.calls.append(row)
        self.bytes += row.get('wireResponseBytes', 0)
        if row.get('captureError'):
            raise WorkflowStop('transport-error')
        if self.bytes > self.bounds['maxWorkflowResponseBytes']:
            raise WorkflowStop('workflow-response-byte-limit')
        if self.clock()-self.started >= self.bounds['workflowTimeoutSeconds']:
            raise WorkflowStop('workflow-time-limit')
        return row

    def walk(self, source, target):
        checked_node(source); checked_node(target)
        if source['id'] == target['id']:
            return dict(status='candidate-path', nodes=[source], steps=[])
        queue = deque([(source, 0)])
        seen, parents, nodes = {source['id']}, {}, {source['id']: source}
        depth_frontier = False
        while queue:
            node, depth = queue.popleft()
            if depth >= self.bounds['maxDepth']:
                depth_frontier = True
                continue
            if self.expanded >= self.bounds['maxExpandedNodes']:
                raise WorkflowStop('expanded-node-limit')
            args = dict(label=node['id'], relation_filter='calls')
            if self.tool == 'graphify':
                args['token_budget'] = self.bounds['graphifyTokenBudget']
            response = self.request('get_neighbors', args)
            self.expanded += 1
            outgoing = neighbors(self.tool, response, node)
            self.groups += len(outgoing)
            if self.groups > self.bounds['maxOutgoingGroups']:
                raise WorkflowStop('outgoing-group-limit')
            destinations = []
            for group in sorted(outgoing, key=lambda g: g.get('label', g.get('node', {}).get('label', ''))):
                if self.tool == 'graphify':
                    label = group['label']
                    if label not in self.cache:
                        resolved = self.request('get_node', dict(label=label))
                        self.cache[label] = graphify_node(resolved, label)
                    destination = self.cache[label]
                    if destination is None:
                        self.identity_gaps.append(dict(source=node['id'], label=label))
                        continue
                else:
                    destination = group['node']
                destinations.append((destination, group['records']))
            destinations.sort(key=lambda item: (item[0]['file'], item[0]['line'], item[0]['label'], item[0]['id']))
            for destination, records in destinations:
                identifier = destination['id']
                if identifier in seen:
                    continue
                seen.add(identifier); nodes[identifier] = destination
                parents[identifier] = dict(source=node['id'], target=identifier, records=records)
                if identifier == target['id']:
                    route, steps, current = [nodes[identifier]], [], identifier
                    while current != source['id']:
                        step = parents[current]; steps.append(step)
                        current = step['source']; route.append(nodes[current])
                    return dict(status='candidate-path', nodes=route[::-1], steps=steps[::-1])
                queue.append((destination, depth+1))
        return dict(status='no-path-in-public-projection', depthFrontier=depth_frontier,
                    identityIncomplete=bool(self.identity_gaps), globalAbsenceProven=False)

    def metrics(self):
        return dict(requests=len(self.calls), responseBytes=self.bytes, expandedNodes=self.expanded,
                    outgoingGroups=self.groups, identityGaps=self.identity_gaps)
