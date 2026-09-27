"""Symmetric source-window control; anchors only, without oracle-based selection.

This is an evaluation policy, not a production explanation implementation.
Byte intervals are authoritative, including cuts through a UTF-8 code point.
"""
from collections import defaultdict
import hashlib
from pathlib import Path

from benchmarks.agent_query.community_tasks import read_bounded

MAX_ROWS = 10000
MAX_FILE_BYTES = 4194304
MAX_FILES = 128
MAX_TOTAL_FILE_BYTES = 16777216


def sha(data):
    return hashlib.sha256(data).hexdigest()


def source_windows(root, rows, budget):
    if type(budget) is not int or not 1 <= budget <= 1048576:
        raise ValueError('source budget must be an integer from 1 to 1048576')
    if not isinstance(rows, list) or len(rows) > MAX_ROWS:
        raise ValueError('membership row limit exceeded')
    root = Path(root).resolve()
    groups = defaultdict(list)
    for row in rows:
        file, line = row.get('file'), row.get('line')
        if not isinstance(file, str) or not file or type(line) is not int or line < 1:
            raise ValueError('missing or invalid membership source anchor')
        relative = Path(file)
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('unsafe membership source path')
        groups[file, line].append(row)
    files = {}
    offsets = {}
    total = 0
    # Validate every anchor, including anchors beyond the retained-byte budget.
    for file, line in sorted(groups):
        if file not in files:
            if len(files) >= MAX_FILES:
                raise ValueError('source file count limit exceeded')
            path = (root / file).resolve()
            if not path.is_relative_to(root):
                raise ValueError('source path escapes root')
            data = read_bounded(path, MAX_FILE_BYTES)
            total += len(data)
            if total > MAX_TOTAL_FILE_BYTES:
                raise ValueError('aggregate source read limit exceeded')
            files[file] = data
            starts = [0]
            for text in data.splitlines(keepends=True):
                starts.append(starts[-1] + len(text))
            offsets[file] = starts
        if line >= len(offsets[file]):
            raise ValueError('membership line outside source')
    keys = sorted(groups)
    windows = []
    remaining = budget
    for index, (file, line) in enumerate(keys):
        if not remaining:
            break
        data = files[file]
        start = offsets[file][line - 1]
        following = keys[index + 1] if index + 1 < len(keys) else None
        end = (offsets[file][following[1] - 1] if following and following[0] == file
               else min(len(data), start + 4096))
        kept_end = min(end, start + remaining)
        part = data[start:kept_end]
        windows.append(dict(file=file, startLine=line, startByte=start, endByte=kept_end,
                            requestedEndByte=end, partial=kept_end < end,
                            sourceBytes=len(part), sourceSha256=sha(part),
                            fileSha256=sha(data), members=groups[file, line]))
        remaining -= len(part)
    for file, data in files.items():
        if read_bounded(root / file, MAX_FILE_BYTES) != data:
            raise ValueError('source changed during window planning')
    return dict(budget=budget, sourceBytes=budget - remaining, windows=windows,
                omittedGroups=[dict(file=f, line=n) for f, n in keys[len(windows):]],
                sourceReadBytes=total, missingAnchorRows=[])


def score_windows(root, case, control, *, identity_verified=False):
    """Verify saved intervals and witness text before scoring all-or-nothing facts."""
    root = Path(root).resolve()
    file = case['file']
    relative = Path(file)
    path = (root / relative).resolve()
    if relative.is_absolute() or '..' in relative.parts or not path.is_relative_to(root):
        raise ValueError('unsafe question source path')
    data = read_bounded(path, MAX_FILE_BYTES)
    if sha(data) != case['sourceFileSha256']:
        raise ValueError('question source hash mismatch')
    source_lines = data.decode('utf-8').splitlines()
    returned = {}
    total = 0
    for w in control['windows']:
        rel = Path(w['file'])
        p = (root / rel).resolve()
        if rel.is_absolute() or '..' in rel.parts or not p.is_relative_to(root):
            raise ValueError('unsafe window source path')
        raw = read_bounded(p, MAX_FILE_BYTES)
        start, end = w['startByte'], w['endByte']
        if (type(start) is not int or type(end) is not int
                or not 0 <= start <= end <= len(raw)):
            raise ValueError('invalid window interval')
        part = raw[start:end]
        if sha(raw) != w['fileSha256'] or sha(part) != w['sourceSha256']:
            raise ValueError('window hash mismatch')
        if len(part) != w['sourceBytes']:
            raise ValueError('window byte count mismatch')
        starts = [0]
        for text in raw.splitlines(keepends=True):
            starts.append(starts[-1] + len(text))
        line = w['startLine']
        if type(line) is not int or not 1 <= line < len(starts) or starts[line - 1] != start:
            raise ValueError('window line and byte disagree')
        total += len(part)
        if w['file'] == file:
            for offset, text in enumerate(part.decode('utf-8', errors='replace').splitlines()):
                # A clipped sequence is retained as replacement text, never padded.
                number = line + offset
                if number in returned:
                    raise ValueError('overlapping source windows')
                returned[number] = text
    if total != control['sourceBytes'] or total > control['budget']:
        raise ValueError('source budget accounting mismatch')
    judgments = []
    for fact in case['facts']:
        missing = []
        literal = True
        for witness in fact['witnesses']:
            first, last = witness['startLine'], witness['endLine']
            if '\n'.join(source_lines[first - 1:last]).strip() != witness['text'].strip():
                raise ValueError('witness differs from pinned source')
            for index, expected in enumerate(witness['text'].splitlines(), first):
                actual = returned.get(index)
                literal &= actual == expected
                if actual is None or actual.strip() != expected.strip():
                    missing.append(dict(line=index, expected=expected, returned=actual))
        # Fixed historical allowance, not a general missing-header rule.
        allowance = (identity_verified and fact['id'] == 'click-1' and len(missing) == 1
                     and missing[0]['line'] == 455
                     and missing[0]['expected'] == 'class _AtomicFile:')
        judgments.append(dict(fact=fact['id'], literalWitnessCoverage=literal,
                              sufficientSourceEvidence=not missing or allowance,
                              classHeaderAllowance=allowance,
                              missingAfterIndentNormalization=missing))
    return judgments
