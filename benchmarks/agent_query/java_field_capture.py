"""Capture the registered real-source field oracle with an installed JDK.

Uses cached, digest-matched dependencies. No downloads, project code execution,
annotation processors or generated project classes. All outputs go to a new
explicit artifact directory; a failed process is retained and never scored.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

from benchmarks.agent_query.java_source_fields import load_capture, sha, MAX_CAPTURE
from benchmarks.agent_query.state_access_audit import read, MAX_GRAPH_BYTES
from benchmarks.agent_query.runner import run_bounded


def capture(registration, root, java_home, classpath, artifacts):
    reg = json.loads(read(registration, MAX_CAPTURE))
    if reg['schema'] != 'compass.java-real-field-registration/1':
        raise ValueError('unknown compiler registration')
    root, java_home = root.resolve(), java_home.resolve()
    suffix = '.exe' if os.name == 'nt' else ''
    java = java_home / 'bin' / ('java' + suffix)
    javac = java_home / 'bin' / ('javac' + suffix)
    def git(*args):
        return subprocess.check_output(['git', '-C', str(root), *args], text=True, timeout=30).strip()
    files = {row['file']: row['sha256'] for row in reg['files']}
    if len(files) != len(reg['files']) or not 0 < len(files) <= 512:
        raise ValueError('duplicate or excessive source files')
    def check_sources():
        if git('rev-parse', 'HEAD') != reg['commit'] or git('status', '--porcelain'):
            raise ValueError('source commit/status drift')
        if sha(root / 'pom.xml') != reg['buildEvidence']['pomSha256']:
            raise ValueError('build configuration drift')
        total = 0
        for file, digest in files.items():
            path = (root / file).resolve()
            path.relative_to(root)
            data = read(path, 4 * 1024 * 1024)
            total += len(data)
            if sha(path) != digest or total > 64 * 1024 * 1024:
                raise ValueError('source digest/size mismatch')
    check_sources()
    libraries = [p.resolve() for p in classpath]
    expected = reg['buildEvidence']['classpath']
    if len(libraries) != len(expected) or any(sha(p) != row['sha256'] for p, row in zip(libraries, expected)):
        raise ValueError('classpath does not match registration')
    artifacts.mkdir(parents=True, exist_ok=False)
    classes = artifacts / 'classes'
    classes.mkdir()
    tool = Path(__file__).parent / 'java_oracle/FieldBindings.java'
    manifest = dict(schema='compass.java-field-capture/1', complete=False,
                    registrationSha256=sha(registration), toolSha256=sha(tool),
                    collectorSha256=sha(__file__), runnerSha256=sha(Path(__file__).with_name('runner.py')),
                    javaSha256=sha(java), javacSha256=sha(javac),
                    modulesSha256=sha(java_home / 'lib/modules', MAX_GRAPH_BYTES),
                    releaseSha256=sha(java_home / 'release'), files=files,
                    classpathSha256={row['coordinate']: sha(p) for p, row in zip(libraries, expected)}, commands=[])
    destination = artifacts / 'manifest.json'
    def save():
        destination.write_text(json.dumps(manifest, indent=2) + '\n')
    def call(name, argv):
        result = run_bounded(tuple(map(str, argv)), cwd=artifacts, timeout_seconds=120,
                             stdout_path=artifacts / (name + '.stdout'), stderr_path=artifacts / (name + '.stderr'))
        manifest['commands'].append(dict(name=name, argv=list(map(str, argv)), exitCode=result.exit_code,
            timedOut=result.timed_out, outputLimited=result.output_limited, milliseconds=result.wall_ms,
            stdoutSha256=sha(artifacts / (name + '.stdout')), stderrSha256=sha(artifacts / (name + '.stderr'))))
        save()
        if result.exit_code or result.timed_out or result.output_limited:
            raise ValueError(f'{name} failed; retained logs are not a scored capture')
    save()
    call('compile', [javac, '--release', '17', '-proc:none', '-d', classes, tool.resolve()])
    manifest['classHashes'] = {str(p.relative_to(classes)): sha(p) for p in sorted(classes.rglob('*.class'))}
    listing = artifacts / 'files.txt'
    listing.write_text('\n'.join(sorted(files)) + '\n')
    call('bindings', [java, '-Xmx1024m', '-cp', classes, 'FieldBindings', root,
                     reg['buildEvidence']['release'], os.pathsep.join(map(str, libraries)), listing])
    load_capture(artifacts / 'bindings.stdout', root, files)
    check_sources()
    manifest.update(complete=True, exitCode=0, timedOut=False, outputLimited=False,
                    stdoutSha256=sha(artifacts / 'bindings.stdout'), stderrSha256=sha(artifacts / 'bindings.stderr'))
    save()
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--registration', type=Path, required=True)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--java-home', type=Path, required=True)
    parser.add_argument('--classpath', type=Path, action='append', required=True)
    parser.add_argument('--artifacts', type=Path, required=True)
    args = parser.parse_args()
    result = capture(args.registration, args.root, args.java_home, args.classpath, args.artifacts.resolve())
    print(json.dumps(dict(complete=result['complete'], files=len(result['files']), stdoutSha256=result['stdoutSha256'])))


if __name__ == '__main__':
    main()
