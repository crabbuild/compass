"""Combine hash-bound source panels before capture; never executes a query tool.

Review records are explicit author-maintained attestations, not automatic semantic
review. This utility checks their hashes and coverage and retains ineligibility.
"""
import argparse
from collections import Counter
from copy import deepcopy
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re

from benchmarks.agent_query.broad_query_oracle import (
    merge_overlapping_clusters, publish_exclusive, read_bounded)
from benchmarks.agent_query.broad_query_review import validate_registration

HERE = Path(__file__).resolve().parent


def digest(data):
    return hashlib.sha256(data).hexdigest()


def read_artifact(root, record):
    name = record['artifact']
    if not isinstance(name, str) or len(name)>255 or '/' in name or '\\' in name or Path(name).name != name or name in {'.', '..'}:
        raise ValueError('artifact must be a contained single filename')
    path = (root / name).resolve(strict=True)
    if not path.is_relative_to(root.resolve(strict=True)) or not path.is_file():
        raise ValueError('artifact containment failure')
    expected = record['sha256']
    if not isinstance(expected, str) or not re.fullmatch('[0-9a-f]{64}', expected):
        raise ValueError('artifact SHA256 required')
    data = read_bounded(path, 16 * 1024 * 1024)
    if digest(data) != expected:
        raise ValueError('artifact digest mismatch: ' + name)
    return json.loads(data)


def accepted_review_tasks(report):
    """Read actual review dispositions, without inferring acceptance from counts."""
    if report.get('schema') == 'compass.broad-query-independent-source-review-binding/1':
        if report.get('status') != 'accepted_for_listed_8_tasks_only':
            raise ValueError('unknown or unaccepted review binding')
        return set(report['taskIds'])
    rows = report.get('tasks', report.get('records', []))
    accepted = set()
    for row in rows:
        if row.get('verdict', row.get('status', row.get('judgment'))) == 'accepted':
            accepted.add(row.get('taskId', row.get('task')))
    if None in accepted or not accepted:
        raise ValueError('explicit accepted source task dispositions required')
    return accepted


def assemble(initial, batches, reviewed_records, frozen_at):
    stamp = datetime.fromisoformat(frozen_at.replace('Z', '+00:00'))
    if stamp.utcoffset() is None or stamp.utcoffset().total_seconds() != 0 or stamp > datetime.now(timezone.utc):
        raise ValueError('freeze must be a nonfuture UTC timestamp')
    if len(initial['tasks']) != 160 or initial['independentReview']['status'] != 'confirmed':
        raise ValueError('the independently reviewed original160 is required')
    if len(batches) != 5:
        raise ValueError('one broad supplement and four language control batches required')
    tasks = deepcopy(initial['tasks'])
    repositories = {r['id']: deepcopy(r) for r in initial['repositories']}
    original_ids = {t['id'] for t in tasks}
    expanded_ids = set()
    covered = set()
    source_lineage = []
    for record, batch in batches:
        validate_registration(batch)
        source_lineage.append(record)
        selected = {t['id'] for t in batch['tasks']}
        if selected & (original_ids | expanded_ids):
            raise ValueError('source task ID reused')
        expanded_ids |= selected
        tasks.extend(deepcopy(batch['tasks']))
        for repo in batch['repositories']:
            if repo['id'] in repositories and repositories[repo['id']] != repo:
                raise ValueError('inconsistent source pins or inventories')
            repositories[repo['id']] = deepcopy(repo)
    for record in reviewed_records:
        selected = set(record['taskIds'])
        if not selected or len(selected) != len(record['taskIds']) or selected & covered or not selected <= expanded_ids:
            raise ValueError('review coverage must be explicit, unique and disjoint')
        if record.get('status') != 'accepted' or record.get('reviewerType') not in {'ai', 'human'}:
            raise ValueError('actual independently accepted review required')
        if any(record.get(k) is not True for k in ['sourceAware', 'independentOfAuthor', 'toolLabelsBlinded']):
            raise ValueError('source-aware independent output-blind review required')
        if record.get('outputExposure') != 'none':
            raise ValueError('pre-output source review required')
        candidate = next((b for r, b in batches if r['sha256'] == record['registrationSha256']), None)
        if candidate is None or not selected <= {t['id'] for t in candidate['tasks']}:
            raise ValueError('review is not bound to the matching source candidate')
        covered |= selected
    if covered != expanded_ids:
        raise ValueError('expanded source review is incomplete')
    if len(tasks) != 366 or len(repositories) != 24:
        raise ValueError('complete366 task /24 repository source panel required')
    old_clusters = {t['id']: t['cluster'] for t in tasks}
    authored = {t['id']: t.get('authoredCluster', t['cluster']) for t in tasks}
    overlaps = merge_overlapping_clusters(tasks)
    for task in tasks:
        task['authoredCluster'] = authored[task['id']]
    broad_ids = [t['id'] for t in tasks if t['kind'] == 'broad']
    controls = [t['id'] for t in tasks if t['id'] in expanded_ids and t['kind'] in {'exact', 'directed'}]
    broad_clusters = {r: {t['cluster'] for t in tasks if t['repository'] == r and t['kind'] == 'broad'} for r in repositories}
    if len(broad_ids) != 94 or Counter(len(v) for v in broad_clusters.values() if v) != Counter({10: 8}):
        raise ValueError('broad panel must have80 conservative clusters /10 per original repository')
    control_clusters = {r: {t['cluster'] for t in tasks if t['repository'] == r and t['id'] in controls} for r in repositories}
    if len(controls) != 192 or Counter(len(v) for v in control_clusters.values() if v) != Counter({12: 16}):
        raise ValueError('new controls must retain192 source clusters /12 per new repository')
    cohort = lambda ids, repos, clusters, why: dict(taskIds=ids, rationale=why,
        minimumRepositories=repos, minimumIndependentClusters=clusters)
    result = dict(schema=initial['schema'], designation='heldout', frozenAt=frozen_at,
        qualificationEligible=False,
        sourceReview=deepcopy(initial['sourceReview']),
        independentReview=dict(status='confirmed', reviewer='Codex query_evaluation and Codex cli_usability source reviewers',
            reviewerType='ai', sourceAware=True, independentOfAuthor=True, toolLabelsBlinded=True,
            outputExposure='none', tasksReviewed=366, accepted=366, uncertain=0,
            initialPanel=deepcopy(initial['independentReview']), records=reviewed_records,
            disclosure='Independent of source authorship and heldout outputs, not of Compass implementation knowledge. AI review, not human review. Source acceptance does not certify statistical independence, output correctness, graph precision or multiplicity.'),
        qualificationGaps=['experiment binaries/settings/arms/budgets not frozen by this source-only step',
            'no heldout query outputs or independent output-fact normalization',
            'no current digest-bound graph quality/recall qualification for original8',
            '400 distinct source-occurrence judgments per original repository are not established by task witnesses',
            '192 nominal control clusters have insufficient prospective power at adverse probability .005/no definite wins; source dependence may reduce effective units',
            'no measured paired effect, hierarchical uncertainty, non-inferiority or cost/latency acceptance'],
        samplingDesign='Preserve all original160 tasks. Add14 source-only broad responsibilities to reach94 labels /80 conservative declaration-overlap clusters, ten per original repository. Add192 distinct source-authored controls from16 untouched repositories (four/language, nine exact plus three directed each). Original32 controls remain diagnostic. No tool-driven filtering or optional stopping. Source-disjoint endpoints do not prove stochastic independence: shared APIs, library policy and retrieval failures remain dependencies. Use paired repository-aware analysis; keep unmet prospective power explicit. Graph-quality claims cover only original8 broad corpora; extra16 graphs are query-control diagnostics only. All366 belong to the complete capture matrix.',
        graphQualificationRepositories=sorted(r['id'] for r in initial['repositories']),
        metricCohorts=dict(broadSuccess=cohort(broad_ids,8,80,'Original80 plus14 source-only supplements; ten conservative broad clusters per original repository.'),
            exactDirected=cohort(controls,16,192,'New192 source controls only; original32 retained diagnostically; no unsupported zero-discordance/equality premise.'),
            claimPrecisionEmptyZero=cohort(broad_ids,8,80,'Same frozen broad cohort as broad success; empty output has zero claim precision.')),
        sourceFreezeLineage=deepcopy(initial.get('sourceFreezeLineage', [])),
        expansionSourceBatches=source_lineage,
        clusteringChanges=[dict(task=t['id'], before=old_clusters[t['id']], after=t['cluster']) for t in tasks if old_clusters[t['id']] != t['cluster']],
        taskOverlapPairs=overlaps, repositories=sorted(repositories.values(), key=lambda r:r['id']), tasks=tasks)
    return validate_registration(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(read_bounded(args.manifest, 4 * 1024 * 1024))
    if manifest.get('schema') != 'compass.broad-query-source-expansion-manifest/1':
        raise ValueError('unknown source expansion manifest schema')
    initial_record = manifest['initial']
    initial = read_artifact(HERE, initial_record)
    batches = [(r, read_artifact(args.artifacts, r)) for r in manifest['batches']]
    for record in manifest['reviews']:
        report = read_artifact(args.artifacts, record)
        if report.get('registrationSha256') != record['registrationSha256']:
            raise ValueError('independent review source digest differs')
        if set(record['taskIds']) != accepted_review_tasks(report):
            raise ValueError('manifest review coverage differs from actual accepted task dispositions')
    stamp = datetime.now(timezone.utc).isoformat().replace('+00:00','Z')
    result = assemble(initial, batches, manifest['reviews'], stamp)
    result['sourceFreezeLineage'].append(initial_record)
    result['powerDesign'] = manifest['powerDesign']
    read_artifact(HERE, manifest['powerDesign'])
    result['sourceSupportSha256'] = {name:digest(read_bounded(HERE/name,4*1024*1024)) for name in
        ['broad_query_combine.py','broad_query_expansion_corpus.py','broad_query_expansion_oracle.py','broad_query_power.py']}
    result['sourceManifestSha256'] = digest(read_bounded(args.manifest,4*1024*1024))
    result['sourceManifestArtifact'] = args.manifest.name
    data = (json.dumps(result,indent=2,ensure_ascii=False)+'\n').encode()
    publish_exclusive(args.output,data)
    print(json.dumps(dict(sha256=digest(data),tasks=366,repositories=24,qualificationEligible=False)))


if __name__ == '__main__':
    main()
