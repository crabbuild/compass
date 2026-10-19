"""Bounded paired source-fact coverage arithmetic, not graph qualification.

Rows must come from a separately frozen, source-authored panel and reviewed
graph observations. This module cannot establish their semantics, provenance,
independence or precision. A positive interval never authorizes a quality claim.
"""
from collections import defaultdict

from benchmarks.agent_query import broad_query_review as query


STATISTICS_SCHEMA = "compass.broad-graph-coverage-statistics/1"
MAX_SOURCE_FACTS = 65_536
MAX_DECLARED_CLUSTERS = 8_192
MAX_REPOSITORIES = 32
MAX_STRATUM_VALUES = 128
MAX_IDENTIFIER_BYTES = 16 * 1024 * 1024
MAX_RESAMPLING_WORK = 8_192_000
TOOLS = ("compass", "graphify")
TEXT_FIELDS = ("id", "repository", "cluster", "language", "relation")
ROW_FIELDS = frozenset((*TEXT_FIELDS, "sourceFactSha256", *TOOLS))


def _counts(rows):
    result = {}
    for tool in TOOLS:
        recovered = sum(row[tool] is True for row in rows)
        missing = sum(row[tool] is False for row in rows)
        unknown = len(rows) - recovered - missing
        result[tool] = dict(sourceFacts=len(rows), recovered=recovered,
            missing=missing, unknown=unknown,
            microRecall=recovered / len(rows) if rows and not unknown else None)
    return result


def paired_graph_coverage(rows, *, resamples=2000, seed=29):
    """Keep unknown pairs and distinguish micro recall from the cluster estimand.

    Each row identifies one canonical source fact, not one emitted graph edge.
    Repeated occurrences need distinct source-fact digests. Aliases/paraphrases
    of one fact must not be submitted as additional units. Digest correctness
    and appropriate cross-fact clustering require upstream source review.
    """
    query._array(rows, "paired graph source facts", MAX_SOURCE_FACTS)
    query._integer(resamples, "graph bootstrap resamples", 1000, 10_000)
    query._integer(seed, "graph bootstrap seed", 0, 2**32 - 1)
    validated, identifiers, facts, clusters, repositories = [], set(), set(), set(), set()
    strata_keys = {field: set() for field in ("language", "relation")}
    text_bytes = 0
    for raw in rows:
        row = query._object(raw, "paired graph source fact")
        if set(row) != ROW_FIELDS:
            raise ValueError("paired graph source fact fields differ from the version1 contract")
        for field in TEXT_FIELDS:
            value = query._text(row[field], f"graph {field}", 128)
            text_bytes += len(value.encode())
        digest = query._digest(row["sourceFactSha256"], "canonical source-fact SHA-256")
        text_bytes += len(digest)
        if text_bytes > MAX_IDENTIFIER_BYTES:
            raise ValueError("paired graph identifier byte limit exceeded")
        for tool in TOOLS:
            if row[tool] is not None:
                query._boolean(row[tool], f"{tool} recovered source fact")
        key = (row["repository"], row["id"])
        fact = (row["repository"], digest)
        if key in identifiers or fact in facts:
            raise ValueError("duplicate graph source-fact ID or canonical digest")
        identifiers.add(key)
        facts.add(fact)
        clusters.add((row["repository"], row["cluster"]))
        repositories.add(row["repository"])
        if len(clusters) > MAX_DECLARED_CLUSTERS or len(repositories) > MAX_REPOSITORIES:
            raise ValueError("paired graph cluster or repository limit exceeded")
        for field, keys in strata_keys.items():
            keys.add(row[field])
            if len(keys) > MAX_STRATUM_VALUES:
                raise ValueError("paired graph language or relation stratum limit exceeded")
        validated.append(dict(row))
    # Bound the inherited bootstrap's repository and cluster draws before it
    # allocates/resamples. Source-fact rows are grouped by that implementation.
    cluster_counts = defaultdict(int)
    for repository, _cluster in clusters:
        cluster_counts[repository] += 1
    # A bootstrap can choose the largest repository at every outer draw. Sum
    # of cluster counts alone underestimates work for an unbalanced panel.
    maximum_draws = resamples * len(repositories) * (max(cluster_counts.values(), default=0) + 1)
    if maximum_draws > MAX_RESAMPLING_WORK:
        raise ValueError("paired graph resampling work limit exceeded")
    validated.sort(key=lambda row: (row["repository"], row["cluster"], row["id"]))
    missing_pairs = sum(any(row[tool] is None for tool in TOOLS) for row in validated)
    complete = bool(validated) and missing_pairs == 0
    if complete:
        paired = query.paired_interval([
            (row["repository"], row["cluster"], float(row["compass"]), float(row["graphify"]))
            for row in validated
        ], samples=resamples, seed=seed)
    else:
        sizes = defaultdict(set)
        for row in validated:
            sizes[row["repository"]].add(row["cluster"])
        paired = dict(estimate=None, lower=None, upper=None,
            repositories=len(repositories), clusters=len(clusters),
            balancedIndependentClusters=bool(sizes) and len({len(v) for v in sizes.values()}) == 1,
            method="paired_repository_then_task_cluster_bootstrap/1",
            reason="Empty source panel or unknown graph observations; no imputation or row exclusion.")
    strata = {}
    for field in ("language", "relation"):
        groups = defaultdict(list)
        for row in validated:
            groups[row[field]].append(row)
        strata[field] = {key: _counts(group) for key, group in sorted(groups.items())}
    return dict(schema=STATISTICS_SCHEMA, rowsRetained=len(validated),
        missingPairRows=missing_pairs, completePairs=complete,
        declaredRepositories=len(repositories), declaredClusters=len(clusters),
        coverage=_counts(validated), strata=strata,
        pairedClusterWeightedRecallDifference=paired,
        qualificationEstablished=False,
        interpretation="Diagnostic arithmetic only. Micro recall and equal-repository/cluster-weighted paired recall are different estimands. Declared clusters do not prove independence.",
        missingQualificationEvidence=[
            "independent source/graph judgments and source-fact digest verification",
            "frozen source, graph, tool, configuration and capture bindings",
            "symmetric emitted-edge precision and existing production graph floors",
            "prospective power, sampling, strata and stopping-rule acceptance",
            "paired graph decision rule and original/lookup/union query gates",
        ])
