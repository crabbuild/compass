"""Canonical protocol2 summaries without dropping unknown precision rows.

Complete paired data uses the existing repository/cluster estimand and risk
bounds unchanged. Missing precision does not become zero or a smaller cohort.
"""
from collections import defaultdict

from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_protocol import _canonical_precision_row, aggregate_precision


def paired_interval_v2(rows, *, samples=2000, seed=29, unavailable=False):
    old._array(rows, "complete paired cohort", old.MAX_TASKS)
    old._integer(samples, "bootstrap resamples", 1000, 10000)
    old._integer(seed, "bootstrap seed", 0, 2**32 - 1)
    old._boolean(unavailable, "required precision unavailable")
    clusters, missing = defaultdict(set), 0
    for row in rows:
        if type(row) not in (tuple, list) or len(row) != 4:
            raise ValueError("paired rows need repository, cluster and both scores")
        repository, cluster, candidate, comparator = row
        old._text(repository, "paired repository", 128)
        old._text(cluster, "paired cluster", 128)
        clusters[repository].add(cluster)
        missing += candidate is None or comparator is None
        for value in (candidate, comparator):
            if value is not None:
                old._number(value, "paired score", 0, 1)
    if not missing and not unavailable:
        return old.paired_interval(rows, samples=samples, seed=seed)
    return dict(estimate=None, lower=None, upper=None,
        repositories=len(clusters), clusters=sum(len(values) for values in clusters.values()),
        balancedIndependentClusters=bool(clusters) and len({len(values) for values in clusters.values()}) == 1,
        method="paired_repository_then_task_cluster_bootstrap/1",
        rowsRetained=len(rows), missingPairRows=missing, formalPrecisionComparisonAvailable=False,
        reason="Required matrix precision is unavailable; no imputation, exclusions or paired interval computed.")


def row_precision(row):
    view = _canonical_precision_row(row.get("precisionView"))
    fields = dict(supportedClaims="knownSupported", unsupportedClaims="knownUnsupported",
        unjudgedClaims="knownUnjudged", semanticCoverageComplete="semanticCoverageComplete",
        judgmentComplete="judgmentComplete", claimPrecision="fullMeasuredPrecision",
        precisionWithEmptyZero="formalPrecisionEmptyZero")
    if any(type(row.get(key)) is not type(view[target]) or row[key] != view[target]
           for key, target in fields.items()):
        raise ValueError("scored precision fields differ from their canonical counts/completeness")
    return view


def summary_v2(rows):
    old._array(rows, "complete summary rows", old.MAX_TASKS * 4)
    views = [row_precision(row) for row in rows]
    result = old._summary(rows)
    result["diagnosticKnownClaimPrecision"] = result["claimPrecision"]
    result["diagnosticKnownUnsupportedClaimRate"] = result["unsupportedClaimRate"]
    precision = aggregate_precision(views) if views else None
    result["precision"] = precision
    result["claimPrecision"] = precision["fullMeasuredPooledPrecision"] if precision else None
    result["unsupportedClaimRate"] = (1 - result["claimPrecision"]
        if result["claimPrecision"] is not None else None)
    if any(row["falseEmpty"] is None for row in rows if row["answerable"]):
        result["falseEmptyRate"] = None
    return result
