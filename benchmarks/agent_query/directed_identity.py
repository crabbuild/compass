"""Capture source-assisted directed paths and separate Compass depth diagnostics.

Request construction uses public resolver outputs and frozen source coordinates;
no graph ID or route is consulted to choose endpoints. All failures are retained.
"""
import argparse
import dataclasses
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

from benchmarks.agent_query.community_identity import SEARCH_LIMITS, selected_id
from benchmarks.agent_query.community_navigation import call
from benchmarks.agent_query.community_tasks import read_bounded
from benchmarks.agent_query.mcp_compare import verify_environment
from benchmarks.agent_query.mcp_transport import StdioMcp
from benchmarks.agent_query.runner import run_bounded, _sha256_file


def resolver(tool, witness, *, exact=False):
    coordinate = dict(file=witness['file'], startLine=witness['line'], symbol=witness['symbol'])
    if tool == 'compass':
        arguments = dict(query=coordinate['symbol'], **SEARCH_LIMITS)
        if exact:
            arguments.update(exact=True, source_file=coordinate['file'], start_line=coordinate['startLine'])
        return coordinate, dict(method='search_symbols', arguments=arguments)
    if tool == 'graphify' and '::' not in coordinate['file']:
        return coordinate, dict(method='get_node', arguments=dict(label=coordinate['file']+'::'+coordinate['symbol']))
    raise ValueError('unsupported resolver input')


def execute(args):
    registration = json.loads(read_bounded(args.registration, 1048576))
    witnesses = json.loads(read_bounded(args.witnesses, 1048576))
    run = json.loads(read_bounded(args.run, 16777216))
    if registration['schema'] not in {'compass.directed-identity-development-registration/1', 'compass.longer-path-registration/1'}:
        raise ValueError('unsupported registration')
    exact = registration['schema'] == 'compass.longer-path-registration/1'
    if exact and _sha256_file(args.compass) != registration['baselineBinarySha256']:
        raise ValueError('registered binary digest mismatch')
    if _sha256_file(args.run) != registration['graphRunSha256']:
        raise ValueError('graph input digest mismatch')
    if _sha256_file(args.witnesses) != registration['sourceWitnesses'][args.witnesses.name]:
        raise ValueError('source witness digest mismatch')
    verify_environment(args)
    repositories = {r['repository']: r for r in run['repositories']}
    if len(witnesses['witnesses']) != 5:
        raise ValueError('expected the frozen five source chains')
    def inputs():
        for w in witnesses['witnesses']:
            repo = repositories[w['repository']]; root = Path(repo['source'])
            if subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True, timeout=10).strip() != w['commit']:
                raise ValueError('source revision mismatch')
            if subprocess.check_output(['git', '-C', str(root), 'status', '--porcelain'], text=True, timeout=10).strip():
                raise ValueError('source checkout is dirty')
            for tool in ['compass', 'graphify']:
                if _sha256_file(Path(repo[tool+'Graph'])) != repo[tool+'GraphSha256']:
                    raise ValueError('graph digest mismatch')
            routes = [w] + w.get('alternativeWitnesses', [])
            anchors = [a for route in routes for a in route['nodes'] + [s['site'] for s in route['steps']]]
            for anchor in anchors:
                path = (root/anchor['file']).resolve()
                if not path.is_relative_to(root.resolve()): raise ValueError('source path escapes root')
                data = read_bounded(path, 4194304)
                if hashlib.sha256(data).hexdigest() != anchor['fileSha256']:
                    raise ValueError('source file digest mismatch')
                if anchor['text'] not in data.decode().splitlines()[anchor['line']-1]:
                    raise ValueError('source witness text mismatch')
    inputs()
    args.output.mkdir(parents=True, exist_ok=False)
    files = [args.registration, args.witnesses, args.graphify_environment, Path(__file__)]
    files += [Path(__file__).with_name(n) for n in ['community_identity.py','community_navigation.py','mcp_compare.py','mcp_transport.py','runner.py']]
    support = {}
    for p in files:
        shutil.copy2(p,args.output/p.name); support[p.name]=_sha256_file(p)
    binaries={name:dict(path=str(path),sha256=_sha256_file(path)) for name,path in [('compass',args.compass),('graphify-python',args.graphify_python)]}
    report=dict(schema='compass.directed-identity-capture/1',complete=False,sourceRun=str(args.run),sourceRunSha256=_sha256_file(args.run),supportFiles=support,binaries=binaries,results=[])
    def save(): (args.output/'capture.json').write_text(json.dumps(report,indent=2)+'\n')
    save()
    for witness in witnesses['witnesses']:
        repo=repositories[witness['repository']]; root=Path(repo['source'])
        for tool in ['compass','graphify']:
            row=dict(question=witness['id'],repository=witness['repository'],tool=tool,graphSha256=repo[tool+'GraphSha256'],resolvers=[],paths=[])
            argv=[str(args.compass),'serve'] if tool=='compass' else [str(args.graphify_python),'-m','graphify.serve']
            argv+=['--graph',repo[tool+'Graph'],'--transport','stdio']
            directory=args.output/'raw'/witness['id']/tool
            try:
                with StdioMcp(argv,root,directory/'mcp',timeout=60,max_bytes=1048576) as session:
                    session.initialize()
                    listing=session.send('tools/list',{})
                    expected='search_symbols' if tool=='compass' else 'get_node'
                    if expected not in {t['name'] for t in listing.get('result',{}).get('tools',[])}:
                        raise ValueError('public resolver unavailable')
                    for endpoint in [witness['nodes'][0],witness['nodes'][-1]]:
                        coordinate,request=resolver(tool,endpoint,exact=exact)
                        captured=call(session,request['method'],request['arguments'])
                        row['resolvers'].append(dict(coordinate=coordinate,call=captured,selection=selected_id(tool,captured,coordinate)))
            except (OSError,RuntimeError,ValueError,TimeoutError) as error:
                row['resolverError']=str(error)
            selectors=[r['selection']['selector'] for r in row['resolvers']]
            row['endpointsResolved']=len(selectors)==2 and all(s is not None for s in selectors)
            if row['endpointsResolved']:
                plans=[('forward',selectors[0],selectors[1],8),('reverse',selectors[1],selectors[0],8)]
                if tool=='compass':plans += [('depth-'+str(depth),selectors[0],selectors[1],depth) for depth in range(1,len(witness['steps']))]
                for direction,source,target,depth in plans:
                    argv=([str(args.compass),'node',source,target,'--max-depth',str(depth),'--max-paths','1','--format','json'] if tool=='compass' else [str(args.graphify_python),'-m','graphify','path',source,target,'--directed'])
                    argv+=['--graph',repo[tool+'Graph']]
                    stdout=directory/(direction+'.stdout');stderr=directory/(direction+'.stderr')
                    capture=run_bounded(tuple(argv),cwd=root,timeout_seconds=60,stdout_path=stdout,stderr_path=stderr)
                    record=dataclasses.asdict(capture);record.pop('stdout');record.pop('stderr')
                    record.update(direction=direction,maxDepth=depth if tool=='compass' else None,stdoutPath=str(stdout),stderrPath=str(stderr),stdoutSha256=_sha256_file(stdout),stderrSha256=_sha256_file(stderr))
                    row['paths'].append(record)
            report['results'].append(row);save()
            print(witness['id'],tool,row['endpointsResolved'],[(p['direction'],p['exit_code']) for p in row['paths']],flush=True)
    inputs();verify_environment(args)
    for binary in binaries.values():
        if _sha256_file(Path(binary['path']))!=binary['sha256']:raise ValueError('executable changed')
    report['complete']=True;save()


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['registration','witnesses','run','output','compass','graphify-python','graphify-environment']:
        p.add_argument('--'+name,type=Path,required=True)
    execute(p.parse_args())
