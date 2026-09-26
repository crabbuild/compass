"""Independently check captured MCP responses against preregistered graph facts.

These are graph-consistency diagnostics, not whole-graph source precision or
proof of functional clustering or god-object quality.
"""
from collections import Counter
import argparse
import json
from pathlib import Path
import re

from benchmarks.agent_query.mcp_compare import community
from benchmarks.agent_query.path_audit import read_bounded, MAX_GRAPH_BYTES
from benchmarks.agent_query.runner import _node_anchor, _sha256_file


def label(n, tool):
    return n.get('name', n.get('label', n['id'])) if tool == 'compass' else n.get('label', n['id'])


def audit(row, graph):
    tool, kind = row['tool'], row['question']
    result = {'repository':row['repository'],'tool':tool,'question':kind,
              'executionSucceeded':row['executionSucceeded']}
    if not row['executionSucceeded']:
        return result
    text = row['text']
    nodes = {n['id']:n for n in graph['nodes']}
    names = {}
    for n in nodes.values():
        names.setdefault(label(n,tool),[]).append(n)
    communities = {community(n,tool) for n in nodes.values()} - {None}
    if kind == 'stats':
        actual = {k:int(v) for k,v in re.findall(r'^(Nodes|Edges|Communities): (\d+)$',text,re.M)}
        expected = {'Nodes':len(nodes),'Edges':len(graph['links']),'Communities':len(communities)}
        result.update(expected=expected,actual=actual,graphCountsMatch=actual==expected)
    elif kind == 'community':
        cid = row['arguments']['community_id']
        members = [n for n in nodes.values() if community(n,tool)==cid]
        expected = Counter((label(n,tool),_node_anchor(n,tool)[0] or '') for n in members)
        actual = Counter(re.findall(r'^  (.*) \[(.*)\]$',text,re.M))
        header = re.match(r'Community \d+.* \((\d+) nodes\):',text)
        result.update(expectedMembers=len(members),returnedMembers=sum(actual.values()),
                      missingMembers=sum((expected-actual).values()),extraMembers=sum((actual-expected).values()),
                      membershipMatches=actual==expected and header is not None and int(header[1])==len(members))
    elif kind == 'missing-community':
        cid=row['arguments']['community_id']
        result['absenceMatches'] = cid not in communities and text==f'Community {cid} not found.'
    elif kind == 'ambiguous-neighbors':
        result['ambiguityPreserved'] = 'ambig' in text.lower() and not text.startswith('Neighbors of ')
    elif kind == 'hubs':
        degree=Counter()
        for a,b in {(e['source'],e['target']) for e in graph['links']}:
            degree[a]+=1;degree[b]+=1
        hubs=[]
        displayed=re.findall(r'^  (\d+)\. (.*) - (\d+) edges$',text,re.M)
        structured=row.get('response',{}).get('result',{}).get('structuredContent')
        records=None
        if structured is not None:
            if structured.get('schema')!='compass.mcp.tool-result/1' or structured.get('result',{}).get('schema')!='compass.mcp.hubs/1':
                raise ValueError('unsupported structured hub result')
            records=structured['result']['nodes']
            if not isinstance(records,list) or len(records)!=len(displayed):
                raise ValueError('structured and displayed hub counts disagree')
        used_ids=set()
        for position,(rank,name,count) in enumerate(displayed):
            matches=names.get(name,[])
            entry={'rank':int(rank),'label':name,'degree':int(count),'identityCandidates':len(matches)}
            if records is not None:
                record=records[position]
                identifier=record.get('id')
                entry['explicitId']=identifier
                valid=(isinstance(identifier,str) and identifier in nodes and identifier not in used_ids
                       and label(nodes[identifier],tool)==record.get('label')
                       and record.get('label')==name and record.get('degree')==int(count)
                       and type(record.get('degree')) is int
                       and type(record.get('rank')) is int and record.get('rank')==int(rank)==position+1)
                matches=[nodes[identifier]] if valid else []
                entry['identityCandidates']=len(matches)
                if valid:used_ids.add(identifier)
            if len(matches)==1:
                n=matches[0];file,line,_=_node_anchor(n,tool)
                entry.update(id=n['id'],file=file,line=line,expectedDegree=degree[n['id']],degreeMatches=degree[n['id']]==int(count))
                if records is not None:
                    entry['sourceAnchorMatches']=(record.get('sourceFile')==file and record.get('startLine')==line)
            hubs.append(entry)
        result.update(hubs=hubs,returned=len(hubs),verifiedIdentities=sum(x['identityCandidates']==1 for x in hubs),
                      matchingDegrees=sum(x.get('degreeMatches',False) for x in hubs),
                      explicitIdentities=sum('explicitId' in x and x['identityCandidates']==1 for x in hubs),
                      matchingSourceAnchors=sum(x.get('sourceAnchorMatches',False) for x in hubs))
    elif kind == 'neighbors':
        seed=row['arguments']['label']
        actual=set(re.findall(r'^  (-->|<--) (.*?) \[([^\]]*)\] \[[^\]]*\]',text,re.M))
        expected=set();semantic=set()
        for e in graph['links']:
            rel=e.get('kind' if tool=='compass' else 'relation','')
            if rel!='calls':continue
            a,b=e['source'],e['target']
            if a==seed:expected.add(('-->',label(nodes[b],tool),rel))
            if b==seed:expected.add(('<--',label(nodes[a],tool),rel))
            a,b=(e.get('_src',a),e.get('_tgt',b)) if tool=='graphify' else (a,b)
            if a==seed:semantic.add(('-->',label(nodes[b],tool),rel))
            if b==seed:semantic.add(('<--',label(nodes[a],tool),rel))
        result.update(expectedDisplayedTriples=len(expected),returnedTriples=len(actual),
                      missing=sorted(expected-actual),extra=sorted(actual-expected),displayedPairsMatch=actual==expected,
                      semanticDirectionsMatch=actual==semantic,
                      ambiguousNeighborLabels=sorted({name for _,name,_ in actual if len(names.get(name,[]))!=1}))
    return result


def main(args):
    run=json.loads(read_bounded(args.run/'run.json'))
    if not run.get('complete'):raise ValueError('capture is incomplete')
    old=Path(run['sourceRun'])
    if _sha256_file(old/'run.json')!=run['sourceRunSha256']:raise ValueError('source run changed')
    source=json.loads(read_bounded(old/'run.json'))
    if _sha256_file(args.run/'inputs.json')!=run['inputSha256']:
        raise ValueError('input manifest digest mismatch')
    if _sha256_file(args.run/'mcp_compare.py')!=run['collectorSha256']:
        raise ValueError('collector digest mismatch')
    if 'transportSha256' in run and _sha256_file(args.run/'mcp_transport.py')!=run['transportSha256']:
        raise ValueError('transport digest mismatch')
    results=[]
    graphs={}
    for row in run['results']:
        key=(row['repository'],row['tool'])
        if key[0] not in {'cobra','flask','gson','zod','axum'} or key[1] not in {'compass','graphify'}:
            raise ValueError('invalid capture key')
        if key not in graphs:
            repo=next(r for r in source['repositories'] if r['repository']==key[0])
            p=Path(repo[key[1]+'Graph'])
            if _sha256_file(p)!=row['graphSha256']:raise ValueError('graph digest mismatch')
            graphs[key]=json.loads(read_bounded(p,MAX_GRAPH_BYTES))
        if row['executionSucceeded']:
            packet=row['response']
            identity=packet.get('id')
            if type(identity) is not int or not 1 <= identity <= 100:
                raise ValueError('invalid captured response ID')
            raw=read_bounded(args.run/'raw'/key[0]/key[1]/f'{identity:02}.response.jsonl')
            packets=[json.loads(line) for line in raw.splitlines() if line]
            if not any(p==packet for p in packets):
                raise ValueError('response differs from raw transcript')
            text='\n'.join(c['text'] for c in packet.get('result',{}).get('content',[]) if c.get('type')=='text')
            if text!=row['text'] or len(text.encode())!=row['textBytes']:
                raise ValueError('answer text differs from response')
        checked=audit(row,graphs[key])
        if row['executionSucceeded']:
            checked['wireResponseBytes']=len(raw)
            checked['textBytes']=row['textBytes']
        results.append(checked)
    report={'scope':__doc__,'captureSha256':_sha256_file(args.run/'run.json'),
            'auditorSha256':_sha256_file(Path(__file__)),'results':results}
    with args.output.open('x') as f:json.dump(report,f,indent=2,sort_keys=True)
    for r in results:
        print(r['repository'],r['tool'],r['question'],{k:v for k,v in r.items() if k.endswith('Match') or k.endswith('Matches') or k.endswith('Preserved')})


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--run',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    main(p.parse_args())
