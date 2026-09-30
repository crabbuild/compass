"""Derive review cohorts from the published MLCQ CSV, without tool outputs.

Ratings are annotator evidence, not certain design truth. Repeated ratings from
one reviewer never create an independent vote. Preserve every excluded group.
"""
from collections import Counter, defaultdict
import csv
import hashlib
import io
from pathlib import PurePosixPath
import re


MAX_BYTES = 16 * 1024 * 1024
MAX_ROWS = 100000
SEVERITIES = {'none', 'minor', 'major', 'critical'}
FIELDS = ['id', 'reviewer_id', 'sample_id', 'smell', 'severity', 'review_timestamp',
          'type', 'code_name', 'repository', 'commit_hash', 'path', 'start_line',
          'end_line', 'link', 'is_from_industry_relevant_project']


def positive_int(value):
    if not isinstance(value, str) or re.fullmatch(r'[1-9][0-9]{0,11}', value) is None:
        raise ValueError('expected a positive bounded integer')
    return int(value)


def source_identity(row):
    match = re.fullmatch(r'git@github.com:([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)\.git', row['repository'])
    if match is None or any(part in {'.', '..'} for part in match.groups()):
        raise ValueError('unsupported repository address')
    if re.fullmatch(r'[0-9a-f]{40}', row['commit_hash']) is None:
        raise ValueError('source revision is not a full commit')
    raw = row['path']
    path = raw[1:] if raw.startswith('/') else raw
    if (not path or len(path) > 4096 or '\\' in path
            or any(part in {'', '.', '..'} for part in path.split('/'))
            or PurePosixPath(path).is_absolute() or not path.endswith('.java')):
        raise ValueError('invalid source path')
    start, end = positive_int(row['start_line']), positive_int(row['end_line'])
    if end < start or end-start > 1000000:
        raise ValueError('invalid source span')
    name = row['code_name']
    if not name or len(name) > 2048 or any(ord(c) < 32 for c in name):
        raise ValueError('invalid code name')
    return dict(repository='/'.join(match.groups()), commit=row['commit_hash'],
                file=path, startLine=start, endLine=end, codeName=name)


def census(data):
    if len(data) > MAX_BYTES:
        raise ValueError('dataset byte limit exceeded')
    reader = csv.DictReader(io.StringIO(data.decode('utf-8-sig'), newline=''), delimiter=';')
    if reader.fieldnames != FIELDS:
        raise ValueError('unexpected dataset columns')
    groups, row_ids, total = defaultdict(list), set(), 0
    for row in reader:
        total += 1
        if total > MAX_ROWS or None in row or any(v is None for v in row.values()):
            raise ValueError('invalid or excessive dataset rows')
        identifier = positive_int(row['id'])
        if identifier in row_ids:
            raise ValueError('duplicate review identity')
        row_ids.add(identifier)
        if row['smell'] != 'blob':
            continue
        if row['type'] != 'class' or row['severity'] not in SEVERITIES:
            raise ValueError('invalid Blob review')
        sample, reviewer = positive_int(row['sample_id']), positive_int(row['reviewer_id'])
        groups[sample].append(dict(reviewId=identifier, reviewerId=reviewer,
                                  severity=row['severity'], source=source_identity(row)))
    samples = []
    for sample, rows in sorted(groups.items()):
        source = rows[0]['source']
        if any(row['source'] != source for row in rows):
            raise ValueError('sample identity has inconsistent source coordinates')
        votes = defaultdict(set)
        for row in rows:
            votes[row['reviewerId']].add(row['severity'])
        levels = {row['severity'] for row in rows}
        if any(len(v) != 1 for v in votes.values()):
            classification = 'conflicting-same-reviewer'
        elif len(votes) < 2:
            classification = 'single-reviewer'
        elif levels == {'none'}:
            classification = 'unanimous-none'
        elif levels <= {'major', 'critical'}:
            classification = 'unanimous-major-critical'
        else:
            classification = 'minor-or-disagreement'
        samples.append(dict(sampleId=sample, **source, classification=classification,
                            uniqueReviewers=len(votes), reviewRows=len(rows),
                            duplicateReviewerRows=len(rows)-len(votes),
                            ratings=[{k: row[k] for k in ['reviewId', 'reviewerId', 'severity']}
                                     for row in sorted(rows, key=lambda r: r['reviewId'])]))
    cohorts = defaultdict(list)
    for sample in samples:
        cohorts[sample['repository'], sample['commit']].append(sample)
    projects = []
    for (repository, commit), records in sorted(cohorts.items()):
        counts = dict(sorted(Counter(r['classification'] for r in records).items()))
        eligible = min(counts.get('unanimous-none', 0), counts.get('unanimous-major-critical', 0)) >= 2
        projects.append(dict(repository=repository, commit=commit, classifications=counts,
                             eligible=eligible, sampleIds=[r['sampleId'] for r in records]))
    return dict(schema='compass.mlcq-blob-census/1', datasetSha256=hashlib.sha256(data).hexdigest(),
                totalReviews=total, blobReviews=sum(r['reviewRows'] for r in samples),
                blobSamples=len(samples), classifications=dict(sorted(Counter(r['classification'] for r in samples).items())),
                projects=projects, samples=samples)


def select_cohorts(report):
    """All revisions with >=2 multi-reviewer positive and >=2 negative samples."""
    eligible = {(r['repository'], r['commit']) for r in report['projects'] if r['eligible']}
    return [s for s in report['samples'] if (s['repository'], s['commit']) in eligible]
