"""Freeze source-only expansion batches; no query tool or model is executed."""
import argparse
import ast
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import tempfile

from benchmarks.agent_query.broad_query_expansion_corpus import ANCHORS,TASKS
from benchmarks.agent_query.broad_query_oracle import (SourceReader,anchor,make_task,
    merge_overlapping_clusters,publish_exclusive,read_bounded)

HERE=Path(__file__).resolve().parent
SOURCE_GROUPS={'python-controls':{'requests','urllib3','pluggy','attrs'},
 'go-controls':{'viper','pflag','jwt','validator'},
 'rust-controls':{'mio','env_logger','reqwest','petgraph'},
 'typescript-controls':{'tanstack-query','pinia','reselect','immer'}}
IDS={'TanStack/query':'tanstack-query'}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def python_spec(spec,lines):
    matches=[]
    def visit(node,owners):
        for child in ast.iter_child_nodes(node):
            if isinstance(child,(ast.ClassDef,ast.FunctionDef,ast.AsyncFunctionDef)):
                if isinstance(child,(ast.FunctionDef,ast.AsyncFunctionDef)):
                    overloaded=any(isinstance(d,ast.Name) and d.id=='overload' or isinstance(d,ast.Attribute) and d.attr=='overload' for d in child.decorator_list)
                    stub=len(child.body)==1 and isinstance(child.body[0],ast.Expr) and isinstance(child.body[0].value,ast.Constant) and child.body[0].value.value is Ellipsis
                    if child.name==spec['symbol'] and '.'.join(owners)==spec['owner'] and not overloaded and not stub:
                        matches.append(child)
                visit(child,owners+[child.name])
            else: visit(child,owners)
    visit(ast.parse('\n'.join(lines)),[])
    if len(matches)!=1: raise ValueError(f'non-unique implemented Python declaration: {spec["owner"]}.{spec["symbol"]}')
    line=matches[0].lineno-1;needle=lines[line]
    return {**spec,'declaration':needle,'occurrence':sum(needle in item for item in lines[:line])}


def build(source_root,artifacts,batch,frozen_at):
    time=datetime.fromisoformat(frozen_at.replace('Z','+00:00'))
    if time.utcoffset() is None or time.utcoffset().total_seconds()!=0 or time>datetime.now(timezone.utc): raise ValueError('nonfuture UTC freeze required')
    candidates=json.loads(read_bounded(HERE/'broad_query_expansion_sources.json',4194304))
    pins={}
    for item in candidates['candidates']:
        slug=item['url'].removeprefix('https://github.com/')
        key=IDS.get(slug,slug.split('/')[1])
        pins[key]=dict(slug=slug,commit=item['commit'],language=item['language'])
    initial=json.loads(read_bounded(HERE/'broad_query_registration_reviewed.json',16777216))
    for item in initial['repositories']:
        pins[item['id']]=dict(slug=item['url'].removeprefix('https://github.com/'),commit=item['commit'],language=item['language'])
    selected=[t for t in TASKS if t['kind']=='broad'] if batch=='broad-supplement' else [t for t in TASKS if t['repository'] in SOURCE_GROUPS[batch] and t['kind'] in {'exact','directed'}]
    if batch=='broad-supplement' and len(selected)!=14: raise ValueError('fourteen broad supplements required')
    if batch!='broad-supplement':
        for repo in SOURCE_GROUPS[batch]:
            if Counter(t['kind'] for t in selected if t['repository']==repo)!=Counter(exact=9,directed=3): raise ValueError('each control repository needs9exact+3directed')
    reader=SourceReader(source_root,artifacts);content={};repositories=[]
    for key in sorted({t['repository'] for t in selected}):
        record,lines=reader.repository(key,pins[key]);repositories.append(record);content[key]=lines
    resolved={}
    for task in selected:
        for name in task['anchors']:
            spec=ANCHORS[task['repository'],name]
            if spec['file'].endswith(('.py','.pyi')): spec=python_spec(spec,content[task['repository']][spec['file']])
            resolved[task['repository'],name]=anchor(spec,content[task['repository']])
    rendered=[]
    for spec in selected:
        try: value=make_task(spec,resolved,content)
        except ValueError as error: raise ValueError(spec['id']+': '+str(error)) from error
        if spec.get('selectedDirectWitnessOnly'):
            for fact in value['facts']:
                if 'relation' in fact:
                    fact['relation']['occurrences']=[fact['source']]
                    fact['relation']['occurrenceScope']='selected_direct_line_witness'
            value['sourceWitnessPolicy']='Only the selected direct factory-body call is included. Nested callbacks own their own calls and are excluded; this is not a complete occurrence inventory.'
        value['selectionRationale']=spec['selectionRationale'];rendered.append(value)
    merge_overlapping_clusters(rendered)
    if batch!='broad-supplement':
        if len({t['cluster'] for t in rendered})!=48: raise ValueError('control declarations overlap across units')
        identities=[(t['repository'],tuple(sorted(r['identity'].items()))) for t in rendered for r in t['relevant']]
        if len(identities)!=len(set(identities)): raise ValueError('control endpoint declaration reused')
    for key in content:
        root=reader.contained_repository(pins[key]['slug'])
        if reader.git(root,'status','--porcelain','--untracked-files=all').strip(): raise ValueError('input changed during source freeze')
        if reader.git(root,'rev-parse','HEAD').strip()!=pins[key]['commit']: raise ValueError('input commit changed')
    support={}
    for name in ['broad_query_expansion_corpus.py','broad_query_expansion_oracle.py','broad_query_expansion_sources.json','broad_query_oracle.py']:
        data=read_bounded(HERE/name,4194304);support[name]=digest(data)
        (artifacts/name).write_bytes(data)
    return dict(schema='compass.broad-query-registration/1',designation='heldout',frozenAt=frozen_at,
        sourceReview=dict(status='source_checked',author='Codex source author',reviewerType='ai',blindToOutputs=True,
            method='Pinned implementation bodies/owners/imports inspected; exact controls intentionally name distinct runtime declarations. No held-out graph or query output used.'),
        independentReview=dict(status='pending',reviewer=None,reviewerType=None,sourceAware=False,independentOfAuthor=False,toolLabelsBlinded=True,outputExposure='none'),
        qualificationEligible=False,sourceSupportSha256=support,batch=batch,
        parentSourceRegistrationSha256=digest(read_bounded(HERE/'broad_query_registration_reviewed.json',16777216)),
        repositories=repositories,tasks=rendered,
        samplingDesign='Source-only extension of preserved initial160; exact/direct controls use distinct endpoints and behavior families, no tool-driven filtering. Independent source review required before any capture.')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-root',type=Path,required=True)
    parser.add_argument('--artifacts',type=Path,required=True)
    parser.add_argument('--batch',choices=['broad-supplement',*SOURCE_GROUPS],required=True)
    parser.add_argument('--frozen-at',required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();args.artifacts.mkdir(parents=True,exist_ok=True)
    captures=Path(tempfile.mkdtemp(prefix=args.batch+'-',dir=args.artifacts))
    value=build(args.source_root,captures,args.batch,args.frozen_at)
    data=(json.dumps(value,indent=2,ensure_ascii=False)+'\n').encode();publish_exclusive(args.output,data)
    print(json.dumps(dict(batch=args.batch,tasks=len(value['tasks']),sha256=digest(data),captures=str(captures))))


if __name__=='__main__': main()
