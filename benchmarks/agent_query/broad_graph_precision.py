"""Reproducible emitted-edge sampling and retained-denominator diagnostics.

Sampling does not authenticate source judgments or establish independent units.
No population interval, precision NI decision or joint qualification is issued.
"""
from collections import Counter
import hashlib
from pathlib import Path

from benchmarks.agent_query import broad_graph_evidence as evidence
from benchmarks.agent_query import broad_graph_statistics as statistics
from benchmarks.agent_query import broad_query_review as query
from benchmarks.agent_query import runner, path_audit


INPUT_SCHEMA = "compass.broad-graph-precision-input/1"
DESIGN_SCHEMA = "compass.broad-graph-precision-design/1"
REVIEW_SCHEMA = "compass.broad-graph-precision-reviews/1"
RESULT_SCHEMA = "compass.broad-graph-precision-diagnostics/1"
SAMPLER = "sha256-counter-floyd/1"
MAX_SAMPLED_EDGES = 65_536
MAX_POPULATION_EDGES = 8_000_000
MAX_RANDOM_DRAWS = 4 * MAX_SAMPLED_EDGES
MAX_REJECTIONS = 16
MAX_RECORDED_RELATIONS = 128
MAX_WITNESS_BYTES = 64 * 1024 * 1024
ISSUES = ("owner_identity", "target_identity", "relation_kind", "direction",
          "occurrence", "provenance", "fabricated_target")


def support_hashes():
    return {"coverage." + key: value for key, value in evidence.support_hashes().items()} | {
        name: query.sha(query.read_bounded(Path(path), query.MAX_JSON_BYTES))
        for name, path in [("precision", __file__), ("runner", runner.__file__),
                           ("pathAudit", path_audit.__file__)]}


def sample_indices(population, count, namespace, *, draw_budget=None):
    """Floyd sample using a versioned, portable SHA-256 counter byte stream.

    Rejection removes modulo bias; a fixed resource limit rejects pathological
    draws instead of changing the sampling rule. The pseudorandom stream and
    source/sampling independence assumptions still require design review.
    """
    query._integer(population, "edge population", 0, 4_000_000)
    query._integer(count, "edge sample size", 0, min(population, 4096))
    if count == population:
        return list(range(population))
    key = hashlib.sha256(evidence._canonical_bytes(namespace)).digest()
    counter, selected = 0, set()
    budget = [0] if draw_budget is None else draw_budget
    for upper in range(population - count + 1, population + 1):
        limit = (2**64 // upper) * upper
        for _ in range(MAX_REJECTIONS):
            budget[0] += 1
            if budget[0] > MAX_RANDOM_DRAWS:
                raise ValueError("precision sampling random-draw limit exceeded")
            value = int.from_bytes(hashlib.sha256(key + counter.to_bytes(8, "big")).digest()[:8], "big")
            counter += 1
            if value < limit:
                selected.add(upper - 1 if value % upper in selected else value % upper)
                break
        else:
            raise ValueError("precision sampler rejection limit exceeded")
    return sorted(selected)


def _view(frames, selected, labels):
    population = sum(frame["population"] for frame in frames)
    counts = Counter(labels.get(key, "unreviewed") for key in selected)
    complete = bool(selected) and counts["unreviewed"] == 0 and all(f["buildSuccessful"] for f in frames)
    known_population = all(f["buildSuccessful"] for f in frames)
    correct, incorrect = counts["correct"], counts["incorrect"]
    weighted = None
    if population and known_population:
        contributions = []
        for frame in frames:
            if not frame["population"]:
                continue
            keys = frame["sampleKeys"]
            if not keys or any(labels.get(key, "unreviewed") == "unreviewed" for key in keys):
                break
            contributions.append(frame["population"] * sum(labels[key] == "correct" for key in keys) / len(keys))
        else:
            weighted = sum(contributions) / population
    return dict(capturedEdges=population, populationEdges=population if known_population else None,
        sampledEdges=len(selected), correct=correct, incorrect=incorrect,
        unreviewed=counts["unreviewed"], unsampledEdges=population - len(selected),
        samplePrecision=correct / len(selected) if complete else None,
        populationPrecision=correct / population if complete and len(selected) == population else None,
        populationWeightedPointEstimate=weighted,
        identifiedPopulationPrecisionBounds=[correct / population, 1 - incorrect / population]
            if population and known_population else None,
        populationConfidenceInterval=None, independenceEstablished=False)


def assess_graph_precision(manifest, root):
    evidence._fields(manifest, ("schema", "coverageInput", "design", "reviews"), "graph precision input")
    if manifest["schema"] != INPUT_SCHEMA:
        raise ValueError("unsupported graph precision input schema")
    artifacts = evidence._Artifacts(Path(root))
    coverage_input = artifacts.document(manifest["coverageInput"])
    coverage = evidence.assess_graph_coverage(coverage_input, root)
    registration = artifacts.document(coverage_input["registration"])
    capture = artifacts.document(coverage_input["capture"])
    design = artifacts.document(manifest["design"])
    evidence._fields(design, ("schema", "frozenAt", "coverageRegistrationSha256", "supportSha256", "sampler", "seed", "quotaPerGraph"), "precision design")
    if design["schema"] != DESIGN_SCHEMA or design["sampler"] != SAMPLER or design["supportSha256"] != support_hashes():
        raise ValueError("unsupported or changed graph precision sampling protocol")
    if design["coverageRegistrationSha256"] != coverage_input["registration"]["sha256"]:
        raise ValueError("precision design differs from frozen graph source registration")
    if not (query._timestamp(registration["frozenAt"]) <= query._timestamp(design["frozenAt"])
            < query._timestamp(capture["startedAt"])):
        raise ValueError("precision sampling must be frozen after source registration and before collection")
    seed = query._integer(design["seed"], "precision sample seed", 0, 2**32 - 1)
    quota = query._integer(design["quotaPerGraph"], "predeclared edge quota", 1, 4096)
    source_files, _, source_paths = evidence._sources(registration["repositories"], coverage_input["sourceRoots"])
    failed = {(entry["repository"], entry["tool"]) for entry in coverage["buildFailures"]}
    frames, plan, relations = {}, {}, set()
    draws, record_bytes, population_total = [0], 0, 0
    for entry in capture["graphs"]:
        key = (entry["repository"], entry["tool"])
        graph = artifacts.document(entry["artifact"], query.MAX_GRAPH_BYTES)
        records = query._array(graph.get("edges", graph.get("links", [])), "captured edge population", 4_000_000)
        population_total += len(records)
        if population_total > MAX_POPULATION_EDGES:
            raise ValueError("aggregate precision edge-population limit exceeded")
        indices = sample_indices(len(records), min(quota, len(records)),
            dict(seed=seed, repository=key[0], tool=key[1], graphSha256=entry["artifact"]["sha256"]), draw_budget=draws)
        if len(plan) + len(indices) > MAX_SAMPLED_EDGES:
            raise ValueError("aggregate precision sample limit exceeded")
        kind_counts = Counter()
        for record in records:
            kind = query._text(record.get("kind", record.get("relation")), "recorded edge relation", 128)
            kind_counts[kind] += 1
            relations.add(kind)
            if len(relations) > MAX_RECORDED_RELATIONS:
                raise ValueError("precision recorded-relation limit exceeded")
        keys = []
        for index in indices:
            data = evidence._canonical_bytes(records[index])
            record_bytes += len(data)
            if record_bytes > evidence.MAX_RECORD_BYTES_TOTAL:
                raise ValueError("precision canonical-record byte limit exceeded")
            sample_key = (*key, index)
            kind = records[index].get("kind", records[index].get("relation"))
            plan[sample_key] = dict(repository=key[0], tool=key[1], index=index,
                graphSha256=entry["artifact"]["sha256"], recordSha256=query.sha(data),
                recordedRelation=kind, directed=graph["directed"])
            keys.append(sample_key)
        frames[key] = dict(repository=key[0], tool=key[1], population=len(records),
            sampleKeys=keys, byRecordedRelation=kind_counts, buildSuccessful=key not in failed)
    reviews = artifacts.document(manifest["reviews"])
    evidence._fields(reviews, ("schema", "coverageCaptureSha256", "designSha256", "reviewedAt", "observations"), "precision reviews")
    if (reviews["schema"] != REVIEW_SCHEMA or reviews["coverageCaptureSha256"] != coverage_input["capture"]["sha256"]
            or reviews["designSha256"] != manifest["design"]["sha256"]):
        raise ValueError("precision review is not bound to its capture and sampling design")
    if query._timestamp(reviews["reviewedAt"]) < query._timestamp(capture["startedAt"]):
        raise ValueError("precision review predates collection")
    labels, witness_cache, witness_bytes = {}, set(), 0
    issues = {tool: Counter() for tool in statistics.TOOLS}
    for item in query._array(reviews["observations"], "sampled edge observations", MAX_SAMPLED_EDGES):
        evidence._fields(item, ("repository", "tool", "index", "recordSha256", "verdict", "issues", "reason", "witnesses"), "precision observation")
        query._text(item["repository"], "precision repository", 128)
        query._choice(item["tool"], statistics.TOOLS, "precision tool")
        query._integer(item["index"], "sampled edge index", 0, 3_999_999)
        key = (item["repository"], item["tool"], item["index"])
        if key in labels or key not in plan:
            raise ValueError("duplicate or non-sampled precision observation")
        if query._digest(item["recordSha256"]) != plan[key]["recordSha256"]:
            raise ValueError("precision observation differs from its captured raw record")
        verdict = query._choice(item["verdict"], ("correct", "incorrect", "unreviewed"), "edge verdict")
        query._text(item["reason"], "source precision rationale")
        problems = query._array(item["issues"], "edge semantic issues", len(ISSUES))
        for issue in problems:
            query._choice(issue, ISSUES, "edge semantic issue")
        query._unique(problems, "edge semantic issues")
        if (verdict == "incorrect") != bool(problems):
            raise ValueError("only an incorrect edge verdict may name semantic issues")
        if "direction" in problems and not plan[key]["directed"]:
            raise ValueError("undirected contract cannot be scored as an invented direction error")
        witnesses = query._array(item["witnesses"], "precision source witnesses", 16, 0 if verdict == "unreviewed" else 1)
        for witness in witnesses:
            evidence._fields(witness, ("file", "startByte", "endByte", "sha256"), "precision source witness")
            file = query._relative(witness["file"])
            data = source_files[key[0]].get(file)
            if data is None:
                raise ValueError("precision source witness is not inventoried")
            start = query._integer(witness["startByte"], "precision witness start", 0, len(data))
            end = query._integer(witness["endByte"], "precision witness end", start + 1, len(data))
            digest = query._digest(witness["sha256"])
            witness_key = (key[0], file, start, end, digest)
            if witness_key not in witness_cache:
                witness_bytes += end - start
                if witness_bytes > MAX_WITNESS_BYTES:
                    raise ValueError("aggregate precision witness-byte limit exceeded")
                evidence._span(witness, source_files[key[0]])
                witness_cache.add(witness_key)
        labels[key] = verdict
        issues[key[1]].update(problems)
    by_tool = {}
    for tool in statistics.TOOLS:
        selected_frames = [frame for key, frame in sorted(frames.items()) if key[1] == tool]
        selected_keys = [key for key in sorted(plan) if key[1] == tool]
        by_repo = {frame["repository"]: _view([frame], frame["sampleKeys"], labels) for frame in selected_frames}
        by_kind = {}
        for kind in sorted(relations):
            kind_frames = [dict(frame, population=frame["byRecordedRelation"].get(kind, 0),
                sampleKeys=[key for key in frame["sampleKeys"] if plan[key]["recordedRelation"] == kind]) for frame in selected_frames]
            kind_keys = [key for key in selected_keys if plan[key]["recordedRelation"] == kind]
            by_kind[kind] = _view(kind_frames, kind_keys, labels)
        by_tool[tool] = dict(overall=_view(selected_frames, selected_keys, labels),
            byRepository=by_repo, byRecordedRelation=by_kind,
            issueCounts=dict(sorted(issues[tool].items())))
    for source_root, file, digest in source_paths.values():
        path = (source_root / file).resolve()
        if source_root.resolve() != source_root or not path.is_relative_to(source_root):
            raise ValueError("precision source witness escapes registered root during recheck")
        evidence._regular(path)
        if query.sha(query.read_bounded(path, query.MAX_SOURCE_BYTES)) != digest:
            raise ValueError("precision source witness file changed during assessment")
    artifacts.recheck()
    if design["supportSha256"] != support_hashes():
        raise ValueError("precision support changed during assessment")
    return dict(schema=RESULT_SCHEMA, qualificationEstablished=False,
        coverage=coverage, precision=by_tool, samplePlan=[plan[key] for key in sorted(plan)],
        precisionComparisonInterval=None, sampling=dict(method=SAMPLER, quotaPerGraph=quota,
            seed=seed, selectedRecords=len(plan), randomDraws=draws[0]),
        artifacts=dict(sorted(artifacts.seen.items())),
        limitations=["Recorded labels and source witnesses do not authenticate semantic truth or review independence.",
            "Sample precision and a population-weighted point estimate are not population confidence bounds.",
            "Unselected and unreviewed records remain in population bounds; empty populations have undefined precision.",
            "Production qualification floors, prospective sampling/power acceptance and final joint eligibility remain required."])
