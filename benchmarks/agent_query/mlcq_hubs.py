"""Score partial externally rated class retrieval, never population precision.

Native ranking identity comes exclusively from its public response. Source
coordinates identify graph coverage; they never disambiguate a returned label.
Callers verify capture provenance and raw transcripts before using this scorer.
"""
from collections import Counter

from benchmarks.agent_query.mcp_audit import audit, label
from benchmarks.agent_query.runner import _node_anchor

CUTOFFS = (10, 50, 100)
PRIMARY = {'unanimous-major-critical', 'unanimous-none'}


def evaluate_ranking(tool, repository, call, graph, samples, witnesses):
    """Retain every registered primary case, including unavailable captures."""
    selected = [s for s in samples if s['repository'] == repository and s['classification'] in PRIMARY]
    by_sample = {w['sampleId']: w for w in witnesses if w['repository'] == repository}
    if len(by_sample) != len(selected) or set(by_sample) != {s['sampleId'] for s in selected}:
        raise ValueError('primary source witness denominator differs')
    if tool not in {'compass', 'graphify'}:
        raise ValueError('unsupported tool')
    checked = None
    nodes = {}
    names = {}
    if call is not None and call.get('executionSucceeded') and graph is not None:
        nodes = {n['id']: n for n in graph['nodes']}
        if len(nodes) != len(graph['nodes']):
            raise ValueError('duplicate graph IDs')
        for n in nodes.values():
            names.setdefault(label(n, tool), []).append(n['id'])
        checked = audit(dict(call, repository=repository, tool=tool, question='hubs'), graph)
        hubs = checked['hubs']
        if not hubs or len(hubs) > 100 or [h['rank'] for h in hubs] != list(range(1, len(hubs)+1)):
            raise ValueError('missing, invalid, or reordered hub ranking')
        if any('explicitId' in h and h['identityCandidates'] != 1 for h in hubs):
            raise ValueError('invalid explicit hub identity')
    cases = []
    for sample in sorted(selected, key=lambda s: s['sampleId']):
        witness = by_sample[sample['sampleId']]
        case = dict(sampleId=sample['sampleId'], classification=sample['classification'])
        statuses = {}
        if checked is None:
            case.update(graphIds=None, rank=None)
            statuses = {str(k): 'capture-unavailable' for k in CUTOFFS}
        else:
            matching = []
            for identifier, node in nodes.items():
                file, line, symbols = _node_anchor(node, tool)
                if (file, line) == (witness['file'], witness['line']) and witness['symbol'] in symbols:
                    matching.append(identifier)
            matching.sort()
            case.update(graphIds=matching, rank=None)
            if len(matching) != 1:
                statuses = {str(k): 'source-missing' if not matching else 'source-ambiguous' for k in CUTOFFS}
            else:
                identifier = matching[0]
                ranks = [h['rank'] for h in checked['hubs'] if h.get('id') == identifier]
                if len(ranks) > 1:
                    raise ValueError('repeated returned hub identity')
                case['rank'] = ranks[0] if ranks else None
                uncertain = [h['rank'] for h in checked['hubs'] if h['identityCandidates'] != 1
                             and identifier in names.get(h['label'], [])]
                case['ambiguousOutputRanks'] = uncertain
                for k in CUTOFFS:
                    statuses[str(k)] = ('retrieved' if ranks and ranks[0] <= k else
                        'ambiguous-output' if any(r <= k for r in uncertain) else 'not-returned')
        case['atCutoff'] = statuses
        cases.append(case)
    totals = {str(k): {rating: dict(Counter(c['atCutoff'][str(k)] for c in cases
                                         if c['classification'] == rating))
                      for rating in sorted(PRIMARY)} for k in CUTOFFS}
    return dict(repository=repository, tool=tool, cases=cases, cutoffs=totals, graphConsistency=checked)
