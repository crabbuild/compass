"""Internal file-backed protocol2 assessment; public dispatch stays disabled.

No graph query, provider or network is run. Bounded Git metadata checks verify
current source pins/cleanliness. Supplied review attestations remain explicit
inputs, not authenticated human/source judgments.
"""
from pathlib import Path
from tempfile import TemporaryDirectory

from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_inputs_v2 import BoundedArtifacts, verify_native_inputs
from benchmarks.agent_query.broad_query_lineage_v2 import load_review_lineage
from benchmarks.agent_query.broad_query_observation_v2 import score_bound_observation
from benchmarks.agent_query.broad_query_oracle import SourceReader
from benchmarks.agent_query.broad_query_protocol import BundleWork


def _source_state(registration, roots, artifacts, spool):
    reader = SourceReader(artifacts.root, spool)
    for repository in registration["repositories"]:
        root = Path(roots[repository["id"]]).resolve(strict=True)
        options = ("--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false")
        if reader.git(root, *options, "rev-parse", "HEAD").strip() != repository["commit"]:
            raise ValueError("source HEAD differs from the frozen source pin")
        if reader.git(root, *options, "status", "--porcelain", "--untracked-files=all").strip():
            raise ValueError("source checkout is not clean")


def assess_v2(manifest, root):
    """Assess the full frozen matrix and recheck every input before returning.

    All row payloads use one reader and are discarded after scoring. Every
    required row remains in summaries/comparisons, including failed executions
    and normalization rejections. Calling this internal function does not
    authorize a comparison or prove reviewer independence.
    """
    artifacts = BoundedArtifacts(Path(root))
    code_root = Path(__file__).resolve().parents[2]
    lineage = load_review_lineage(manifest, artifacts, code_root)
    registration, run = lineage.registration, lineage.run
    roots = old._object(manifest.get("sourceRoots"), "source roots")
    if roots.keys() != {repo["id"] for repo in registration["repositories"]}:
        raise ValueError("source roots differ from the frozen source matrix")
    for value in roots.values():
        if not Path(old._text(value, "absolute source root")).is_absolute():
            raise ValueError("source roots must be absolute")
    # The evidence root must be outside the product and every source checkout;
    # the temporary bounded Git captures live alongside it and are removed.
    protected = [code_root, *(Path(value).resolve() for value in roots.values())]
    if any(artifacts.root.is_relative_to(path) or path.is_relative_to(artifacts.root) for path in protected):
        raise ValueError("assessment artifacts must be external to product and source checkouts")
    with TemporaryDirectory(prefix="compass-review-v2-", dir=artifacts.root.parent) as directory:
        spool = Path(directory)
        if any(spool.is_relative_to(path) for path in protected):
            raise ValueError("temporary audit captures must be outside source checkouts")
        _source_state(registration, roots, artifacts, spool)
        originals = verify_native_inputs(manifest, registration, run, artifacts)
        started = old._timestamp(run["startedAt"])
        state = old._prepare_assessment(manifest, registration, run, artifacts, started,
            actors=run["protocolBinding"]["actorSha256"])
        tasks, failures = state["tasks"], state["failures"]
        graphs = {(entry["repository"], entry["tool"]): entry for entry in state["graph_records"]}
        work = BundleWork()
        scored, costs, reviewers, row_work = {}, [], set(), []
        for observation in lineage.iter_observations():
            key = observation.key
            task = tasks[key[0]]
            review = observation.judgment.get("independentReview")
            failures += old._review_failures(review, f"{key}: output normalization/facts")
            reviewers.add(str(review.get("reviewerType")) if type(review) is dict else "None")
            graph_key = task["repository"], key[1]
            result = score_bound_observation(observation, task, registration["capturePlan"],
                graphs[graph_key], artifacts.root, state["graph_indexes"][graph_key],
                state["sources"][task["repository"]], work)
            row = result["row"]
            if not row["normalizationComplete"] or not row["semanticCoverageComplete"]:
                failures.append(f"{key}: output normalization/semantic coverage incomplete")
            if not row["judgmentComplete"]:
                failures.append(f"{key}: unjudged output or post-output source-oracle correction")
            if row["evaluationRejected"]:
                failures.append(f"{key}: evaluation rejected: {row['evaluationFailure']['code']}")
            if not result["measurementComplete"]:
                failures.append(f"{key}: peak RSS measurement unavailable")
            scored[key] = row
            costs.append(result["cost"])
            row_work.append(dict(task=key[0], tool=key[1], arm=key[2], **result["work"]))
            del observation
        expected = {(task, tool, arm) for task in tasks for tool in old.TOOLS for arm in old.ARMS}
        if scored.keys() != expected:
            raise ValueError("assessment did not retain the full frozen matrix")
        report = old._finish_assessment(manifest, registration, artifacts, started, state,
            scored, costs, sorted(reviewers), protocol2=True)
        _source_state(registration, roots, artifacts, spool)
        originals.recheck()
        lineage.recheck()
    report.update(protocol="compass.broad-query-review/2", actorSha256=run["protocolBinding"]["actorSha256"],
        printedWork=dict(completedHashBytes=work.hash_bytes, completedLineLinks=work.line_links, rows=row_work),
        sourceStateVerified=True)
    return report


def assess_file_v2(path):
    """Bind the actual input file's bytes before and after full assessment."""
    path = Path(path).absolute()
    data = old.read_bounded(path, old.MAX_JSON_BYTES)
    report = assess_v2(old._json(data), path.parent)
    if old.read_bounded(path, old.MAX_JSON_BYTES) != data:
        raise ValueError("review-input manifest changed during assessment")
    return report
