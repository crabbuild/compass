"""Freeze/check source-only Plan 029 facts without executing either query tool.

All authored semantic judgments live in broad_query_corpus.py. Matching a snippet
only checks reproducibility, not semantic correctness. Qualification requires an
independent review plus the separate experiment and statistical gates.
"""
from __future__ import annotations

import argparse
import ast
import re
from collections import Counter
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import tempfile

from benchmarks.agent_query.broad_query_corpus import ANCHORS, FACT_SELECTION, REPOSITORIES, TASKS
from benchmarks.agent_query.community_tasks import read_bounded
from benchmarks.agent_query.runner import run_bounded

SCHEMA = 'compass.broad-query-registration/1'
MAX_FILE = 4 * 1024 * 1024
MAX_FILES = 4096
MAX_TOTAL = 128 * 1024 * 1024
EXTENSIONS = {'.rs', '.py', '.pyi', '.go', '.ts', '.tsx', '.js', '.jsx', '.mjs', '.cjs'}
HERE = Path(__file__).resolve().parent


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class SourceReader:
    def __init__(self, root: Path, artifacts: Path):
        self.root = root.resolve(strict=True)
        self.artifacts = artifacts
        self.counter = 0

    def git(self, path: Path, *arguments: str) -> str:
        self.counter += 1
        result = run_bounded(('git', '-C', str(path), *arguments), cwd=self.root,
            timeout_seconds=15, stdout_path=self.artifacts / f'git-{self.counter:03}.stdout',
            stderr_path=self.artifacts / f'git-{self.counter:03}.stderr')
        if result.timed_out or result.output_limited or result.exit_code:
            raise ValueError(f'git verification failed: {arguments[0]} ({result.exit_code})')
        return result.stdout

    def contained(self, root: Path, relative: str) -> Path:
        name = PurePosixPath(relative)
        if name.is_absolute() or '..' in name.parts or '\\' in relative:
            raise ValueError('source path escapes checkout')
        path = (root / relative).resolve(strict=True)
        if not path.is_relative_to(root.resolve()) or not path.is_file():
            raise ValueError('source path escapes checkout or is not a file')
        return path

    def repository(self, key: str, pin: dict) -> tuple[dict, dict[str, list[str]]]:
        root = self.contained_repository(pin['slug'])
        if self.git(root, 'rev-parse', 'HEAD').strip() != pin['commit']:
            raise ValueError(f'{key}: source commit differs from frozen pin')
        if self.git(root, 'status', '--porcelain', '--untracked-files=all').strip():
            raise ValueError(f'{key}: source checkout must be clean')
        paths = self.git(root, 'ls-files', '-z').rstrip('\x00').split('\x00')
        paths = sorted(p for p in paths if PurePosixPath(p).suffix in EXTENSIONS)
        if not paths or len(paths) > MAX_FILES:
            raise ValueError(f'{key}: source inventory count exceeds limit')
        inventory, content, total = [], {}, 0
        for name in paths:
            data = read_bounded(self.contained(root, name), MAX_FILE)
            total += len(data)
            if total > MAX_TOTAL:
                raise ValueError(f'{key}: source inventory bytes exceed limit')
            inventory.append(dict(file=name, sha256=sha(data)))
            content[name] = data.decode('utf-8').splitlines()
        record = dict(id=key, language=pin['language'], url=f'https://github.com/{pin["slug"]}',
            commit=pin['commit'], files=inventory,
            sourceScope=dict(selection='all tracked source files with listed extensions, including tests/examples',
                extensions=sorted(EXTENSIONS), sourceBytes=total, sourceFiles=len(paths)),
            inventorySha256=sha(json.dumps(inventory, sort_keys=True, separators=(',', ':')).encode()))
        return record, content

    def contained_repository(self, slug: str) -> Path:
        parts = PurePosixPath(slug).parts
        if len(parts) != 2 or any(p in {'', '.', '..'} for p in parts):
            raise ValueError('invalid repository slug')
        path = (self.root / slug).resolve(strict=True)
        if not path.is_dir() or not path.is_relative_to(self.root):
            raise ValueError('repository escapes source root')
        return path


def locate(lines: list[str], needle: str, *, start: int = 0, occurrence: int = 0) -> int:
    matches = [i for i in range(start, len(lines)) if needle in lines[i]]
    if occurrence < 0 or occurrence >= len(matches):
        raise ValueError(f'source selector not found: {needle!r}')
    return matches[occurrence]


def declaration_end(spec: dict, lines: list[str], start: int) -> int:
    """Bound witnesses to an authored declaration, without claiming resolution."""
    if spec['file'].endswith(('.py', '.pyi')):
        tree = ast.parse('\n'.join(lines))
        found = []
        def visit(node, owners):
            for child in ast.iter_child_nodes(node):
                if isinstance(child, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
                    if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef)) and child.lineno == start + 1:
                        found.append((child, '.'.join(owners)))
                    visit(child, owners + [child.name])
                else:
                    visit(child, owners)
        visit(tree, [])
        if len(found) != 1 or found[0][1] != spec['owner'] or found[0][0].name != spec['symbol']:
            raise ValueError('Python declaration owner/name differs from authored identity')
        return found[0][0].end_lineno
    indentation = len(lines[start]) - len(lines[start].lstrip())
    declaration = re.compile(r'(?:pub(?:\([^)]*\))?\s+)?(?:(?:unsafe|async|const)\s+)*(?:fn|func)\s+|'
        r'(?:export\s+)?(?:default\s+)?(?:async\s+)?(?:function|class|const)\s+|'
        r'(?:(?:get|set|async)\s+)?#?[A-Za-z_][A-Za-z_0-9]*(?:<[^>]*>)?\(')
    for index in range(start + 1, len(lines)):
        line = lines[index]
        indent = len(line) - len(line.lstrip())
        if indent <= indentation and declaration.match(line.lstrip()):
            return index
    return len(lines)


def anchor(spec: dict, content: dict[str, list[str]]) -> dict:
    lines = content[spec['file']]
    start = locate(lines, spec['declaration'], occurrence=spec['occurrence'])
    end = declaration_end(spec, lines, start)
    identity = dict(file=spec['file'], startLine=start+1, symbol=spec['symbol'], owner=spec['owner'])
    witnesses = []
    for witness in spec['witnesses']:
        line = locate(lines, witness['needle'], start=start)
        # This bound detects accidental matching in a far-away declaration. It
        # deliberately does not pretend to infer a Rust/Go/TS/Python function AST.
        if line >= end or line - start > 300:
            raise ValueError(f'witness too far from {identity}: {witness["needle"]!r}')
        witnesses.append(dict(source=dict(file=spec['file'], line=line+1, text=lines[line].strip()),
            statement=witness['statement']))
    return dict(identity=identity, endLine=end, declaration=dict(file=spec['file'], line=start+1, text=lines[start].strip()),
        witnesses=witnesses)


def make_task(spec: dict, resolved: dict, content: dict) -> dict:
    key = spec['repository']
    selected = {name: resolved[(key, name)] for name in spec['anchors']}
    kind = spec['kind']
    task = {name: spec[name] for name in ['id', 'repository', 'cluster', 'kind', 'question']}
    task['expected'] = 'abstain' if kind == 'unsupported' else 'ambiguous' if kind == 'ambiguous' else 'answer'
    task['relevant'] = [dict(identity=value['identity'], grade=3) for value in selected.values()]
    facts = []
    if kind == 'broad':
        for name, value in selected.items():
            indices = FACT_SELECTION.get(spec['id'], {}).get(name, range(len(value['witnesses'])))
            for i in indices:
                fact = value['witnesses'][i]
                facts.append(dict(id=f'{spec["id"]}.fact-{len(facts)+1}', identity=value['identity'],
                    essential=True, statement=fact['statement'], source=fact['source']))
    elif kind in {'exact', 'ambiguous'}:
        for value in selected.values():
            facts.append(dict(id=f'{spec["id"]}.fact-{len(facts)+1}', identity=value['identity'],
                essential=kind == 'exact', statement='Declaration identity at the frozen source anchor.',
                source=value['declaration']))
    for source, target, needle in spec['edges']:
        src, dst = selected[source], selected[target]
        lines = content[key][src['identity']['file']]
        line = locate(lines, needle, start=src['identity']['startLine']-1)
        if line >= src['endLine'] or line - src['identity']['startLine'] > 300:
            raise ValueError('call witness too far from caller')
        witness = dict(file=src['identity']['file'], line=line+1, text=lines[line].strip())
        # Preserve all same-target call-name occurrences in this reviewed caller.
        # Reviewers must still verify receiver/import binding independently.
        call_pattern = re.compile(r'(?<![A-Za-z_0-9])' + re.escape(dst['identity']['symbol']) + r'(?:<[^>]*>)?\s*\(')
        python_calls = None
        if src['identity']['file'].endswith(('.py', '.pyi')):
            python_calls = {node.lineno for node in ast.walk(ast.parse('\n'.join(lines)))
                if isinstance(node, ast.Call) and
                ((isinstance(node.func, ast.Name) and node.func.id == dst['identity']['symbol']) or
                 (isinstance(node.func, ast.Attribute) and node.func.attr == dst['identity']['symbol']))}
        occurrences = [dict(file=src['identity']['file'], line=i+1, text=lines[i].strip())
            for i in range(src['identity']['startLine'], src['endLine'])
            if call_pattern.search(lines[i]) and not lines[i].lstrip().startswith(('//', '/*', '*', '#'))
            and (python_calls is None or i+1 in python_calls)]
        if not occurrences or len(occurrences) > 64 or witness not in occurrences:
            raise ValueError('relation occurrence selector mismatch/limit')
        facts.append(dict(id=f'{spec["id"]}.relation-{len(facts)+1}', identity=src['identity'], essential=True, requireOccurrences=False,
            statement=f'{src["identity"]["symbol"]} calls {dst["identity"]["symbol"]} at this source occurrence.',
            source=witness, relation=dict(source=src['identity'], target=dst['identity'], kind='calls',
                direction='source_to_target', occurrences=occurrences, occurrenceScope='line_witnesses')))
    task['facts'] = facts
    if spec['rationale']:
        task['rationale'] = spec['rationale']
        task['absenceReview'] = 'Source-scoped capability judgment; independent inventory/body review pending. A grep miss is not proof of absence.'
    return task


def merge_overlapping_clusters(tasks: list[dict]) -> list[list[str]]:
    """Conservative uncertainty units; repeated declarations are never independent."""
    parent = {task['cluster']: task['cluster'] for task in tasks}
    def find(key):
        while parent[key] != key:
            key = parent[key]
        return key
    seen, overlaps = {}, []
    for task in tasks:
        for relevant in task['relevant']:
            identity = relevant['identity']
            key = (task['repository'], identity['file'], identity['startLine'], identity['symbol'], identity['owner'])
            if key in seen:
                other = seen[key]
                left, right = find(task['cluster']), find(other['cluster'])
                parent[max(left, right)] = min(left, right)
                overlaps.append([other['id'], task['id']])
            else:
                seen[key] = task
    for task in tasks:
        task['authoredCluster'] = task['cluster']
        task['cluster'] = find(task['cluster'])
    return [list(pair) for pair in sorted({tuple(pair) for pair in overlaps})]


def freeze(source_root: Path, artifacts: Path, frozen_at: str) -> dict:
    from datetime import datetime
    stamp = datetime.fromisoformat(frozen_at.replace('Z', '+00:00'))
    if stamp.utcoffset() is None or stamp.utcoffset().total_seconds() != 0:
        raise ValueError('frozen-at must be a UTC timestamp')
    pins = json.loads(read_bounded(HERE / 'broad_query_sources.json', MAX_FILE))
    if set(pins) != set(REPOSITORIES):
        raise ValueError('source pins differ from authored corpus')
    reader = SourceReader(source_root, artifacts)
    repositories, content = [], {}
    for key, pin in pins.items():
        if (pin['language'], pin['slug']) != REPOSITORIES[key]:
            raise ValueError('source pin language/slug mismatch')
        record, lines = reader.repository(key, pin)
        repositories.append(record)
        content[key] = lines
    resolved, errors = {}, []
    for (key, name), value in ANCHORS.items():
        try:
            resolved[(key, name)] = anchor(value, content[key])
        except (KeyError, ValueError) as error:
            errors.append(f'{key}.{name}: {error}')
    if errors:
        raise ValueError('\n'.join(errors))
    tasks = [make_task(spec, resolved, content) for spec in TASKS]
    overlap_pairs = merge_overlapping_clusters(tasks)
    counts = {key: Counter(t['kind'] for t in tasks if t['repository'] == key) for key in pins}
    for key, count in counts.items():
        if count != Counter(broad=10, exact=2, directed=2, multihop=2, ambiguous=2, unsupported=2):
            raise ValueError(f'{key}: unbalanced task panel: {count}')
    if len(tasks) != 160 or len({t['id'] for t in tasks}) != 160:
        raise ValueError('expected 160 uniquely named tasks')
    for pin in pins.values():
        root = reader.contained_repository(pin['slug'])
        if reader.git(root, 'status', '--porcelain', '--untracked-files=all').strip():
            raise ValueError('source changed during freeze')
        if reader.git(root, 'rev-parse', 'HEAD').strip() != pin['commit']:
            raise ValueError('source commit changed during freeze')
    support = {name: sha(read_bounded(HERE / name, MAX_FILE)) for name in
        ['broad_query_sources.json', 'broad_query_corpus.py', 'broad_query_oracle.py',
         'broad_query_oracle_tests.py']}
    return dict(schema=SCHEMA, designation='heldout', frozenAt=frozen_at,
        sourceReview=dict(status='source_checked', author='Codex source author', reviewerType='ai',
            blindToOutputs=True, method='Manual source-body judgments with reproducible declarations/call witnesses; no query-tool graph or result was consulted.'),
        independentReview=dict(status='pending', reviewer=None, reviewerType=None, sourceAware=False, independentOfAuthor=False, toolLabelsBlinded=True,
            outputExposure='none', note='No independent or human review has yet occurred.'),
        qualificationEligible=False,
        qualificationGaps=['independent source review pending', 'experiment arms, versions, budgets and sampling not frozen',
            'no query/build outputs collected', 'no 2000-judgment accepted graph precision panel',
            'no graph recall review', 'no paired uncertainty/power or non-inferiority analysis'],
        sampling=dict(design='two previously unused small public libraries in each of four languages',
            independence='Distinct questions are not all independent observations; shared behavior/chain tasks use a common cluster.',
            limitations=['convenience sample of small libraries', 'no Java corpus in this four-language panel',
                'independent review may discover missing relevant identities or facts',
                'does not establish broad-product superiority or sufficient statistical power']),
        sourceSupportSha256=support, taskOverlapPairs=overlap_pairs,
        clusterCount=len({t['cluster'] for t in tasks}), repositories=repositories, tasks=tasks)


def publish_exclusive(path: Path, data: bytes) -> None:
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(prefix='.' + path.name + '.', dir=path.parent, delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temporary, path)  # Atomic publication; fails if a prior freeze exists.
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--artifacts', type=Path, required=True, help='External directory for bounded git captures')
    parser.add_argument('--frozen-at', required=True, help='Explicit UTC timestamp; retained on verification')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--verify', type=Path)
    args = parser.parse_args()
    if bool(args.output) == bool(args.verify):
        parser.error('choose exactly one of --output or --verify')
    args.artifacts.mkdir(parents=True, exist_ok=True)
    # Retain bounded before/after git captures as external audit evidence.
    capture_directory = Path(tempfile.mkdtemp(prefix='source-check-', dir=args.artifacts))
    result = freeze(args.source_root, capture_directory, args.frozen_at)
    data = (json.dumps(result, indent=2, ensure_ascii=False) + '\n').encode()
    if args.verify:
        if read_bounded(args.verify, 16 * 1024 * 1024) != data:
            raise ValueError('registration differs from deterministic source regeneration')
        print(json.dumps(dict(verified=True, sha256=sha(data), repositories=8, tasks=160,
            qualificationEligible=False, captureDirectory=str(capture_directory))))
    else:
        # Never overwrite a prior freeze silently. A new reviewed revision needs
        # a new output artifact, timestamp, and explicit provenance linkage.
        publish_exclusive(args.output, data)
        print(json.dumps(dict(frozen=True, sha256=sha(data), repositories=8, tasks=160,
            qualificationEligible=False, captureDirectory=str(capture_directory))))


if __name__ == '__main__':
    main()
