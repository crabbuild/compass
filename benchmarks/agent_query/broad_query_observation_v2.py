"""Connect retained native execution, finite printed coverage and task scoring.

Internal only: the file-backed assessor still must verify source pins, graph
inputs, actors, independence, exposure, the full matrix and formal statistics.
No public protocol2 collection or qualification is enabled by this module.
"""
from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_coverage import validate_printed_partition
from benchmarks.agent_query.broad_query_execution_v2 import validate_execution
from benchmarks.agent_query.broad_query_lineage_v2 import _rejection
from benchmarks.agent_query.broad_query_protocol import (
    BundleWork, PrintedSpanBudget, ProtocolLimit, flatten_claim_batches, pack_claim_batches,
    precision_view,
)
from benchmarks.agent_query.broad_query_score_v2 import score_task_v2, ExpressivenessCollision


def _pending_judgment(initial):
    """Mechanical initial-partition check; never a source/completeness review."""
    claims = flatten_claim_batches(initial)
    count = 0
    for claim in claims:
        occurrences = old._array(claim.get("printedOccurrences"), "initial occurrences", 8192, 1)
        requested = count + len(occurrences)
        if requested > 8192:
            raise ProtocolLimit("OCCURRENCE_LIMIT", "registered_occurrence_records", 8192, count, requested)
        count = requested
    lines = old._array(initial.get("lineLedger"), "initial line ledger", 65536)
    labels = []
    for claim in claims:
        occurrences = old._array(claim.get("printedOccurrences"), "initial occurrences", 8192, 1)
        labels.append(dict(id=claim["id"], verdict="unjudged", reason="Initial suggestion only.",
            printedOccurrences=[dict(id=old._object(item, "initial occurrence").get("id"), verdict="unjudged",
                reason="Initial suggestion only.") for item in occurrences]))
    return dict(schema="compass.broad-query-judgments/2", semanticCoverageComplete=False,
        emptyConfirmed=False, **pack_claim_batches(labels),
        lineCoverage=[dict(index=old._object(line, "initial line").get("index"),
            classification="unresolved", claimIds=line.get("claimIds"),
            reason="Initial suggestion only.") for line in lines])


def _rejected_row(task, graph, failure):
    # Reuse source-fact/group diagnostics without treating a zero-record
    # rejection as an empty answer. Every precision/empty/completeness result
    # below is explicitly replaced with the canonical unavailable state.
    row = old._score_task_impl(task,
        dict(disposition="error", rankingKind="unavailable", ranked=[], admission=None),
        dict(candidates=[]), graph, claims_override=[], labels_override=[])
    view = precision_view(0, 0, 0, semantic_complete=False, judgment_complete=False)
    row.update(success=False, judgmentComplete=False, normalizationComplete=False,
        semanticCoverageComplete=False, claimPrecision=None, precisionWithEmptyZero=None,
        falseEmpty=None, printedAssertionCount=None, printedOccurrenceCount=None,
        occurrenceVerdicts=None, unresolvedLineCount=None, precisionView=view,
        evaluationRejected=True, evaluationFailure=failure)
    return row


def score_bound_observation(observation, task, plan, graph_entry, root, graph, sources, bundle_work):
    """Score one streamed body with one shared initial/reviewed work budget.

    graph and sources must come from the auditor's verified source/graph inputs.
    Row limits/collisions retain unknown precision and costs; malformed bindings
    or source witnesses raise and prevent publication of a successful prefix.
    Caller must retain every returned row, discard this body's large objects,
    and complete the lineage's final recheck before publishing anything.
    """
    if (not isinstance(graph, old.GraphIndex) or type(sources) is not dict
            or not isinstance(bundle_work, BundleWork)):
        raise ValueError("observation scoring requires pinned graph and source inputs")
    execution = validate_execution(observation, task, plan, graph_entry, root)
    before_hash, before_links = bundle_work.hash_bytes, bundle_work.line_links
    reviewed = observation.reviewed
    budget = None
    initial_rejected = observation.initial.get("normalizationStatus") == "rejected"
    reviewed_rejected = reviewed.get("normalizationStatus") == "rejected"
    if initial_rejected:
        _rejection(observation.initial)
    if reviewed_rejected:
        _rejection(reviewed)
        judgment = observation.judgment
        if (judgment.get("emptyConfirmed") is not False
                or judgment.get("semanticCoverageComplete") is not False
                or flatten_claim_batches(judgment)
                or judgment.get("candidates", []) != []
                or judgment.get("lineCoverage", []) != []):
            raise ValueError("rejected normalization cannot have successful review or an empty assertion attestation")
    try:
        if not initial_rejected or not reviewed_rejected:
            budget = PrintedSpanBudget(observation.stdout, bundle_work)
        if not initial_rejected:
            validate_printed_partition(observation.initial,
                _pending_judgment(observation.initial), budget, phase="initial")
        if reviewed_rejected:
            row = _rejected_row(task, graph, reviewed["normalizationFailure"])
        else:
            row = score_task_v2(task, reviewed, observation.judgment, graph,
                observation.stdout, budget, sources=sources)
            row["evaluationRejected"] = False
    except ProtocolLimit as error:
        row = _rejected_row(task, graph, dict(code=error.code, resource=error.resource))
    except ExpressivenessCollision as error:
        row = _rejected_row(task, graph, dict(code=error.code, resource=None))
    except UnicodeError:
        if budget is not None:
            raise  # Invalid pinned source text is an integrity failure.
        row = _rejected_row(task, graph, dict(code="INVALID_UTF8", resource=None))
    if initial_rejected:
        # Reviewed material may supply checked diagnostics, but the frozen
        # initial parser/limit failure remains in the original denominator.
        view = precision_view(row["supportedClaims"], row["unsupportedClaims"], row["unjudgedClaims"],
            semantic_complete=False, judgment_complete=row["judgmentComplete"])
        row.update(success=False, normalizationComplete=False, semanticCoverageComplete=False,
            precisionView=view, claimPrecision=None, precisionWithEmptyZero=None,
            falseEmpty=None, printedAssertionCount=None, evaluationRejected=True)
        row.setdefault("evaluationFailure", observation.initial["normalizationFailure"])
    row.update(executionFailed=execution["executionFailed"], nativeOutcome=execution["nativeOutcome"],
        initialNormalizationRejected=initial_rejected, reviewedNormalizationRejected=reviewed_rejected,
        initialNormalizationFailure=observation.initial.get("normalizationFailure"),
        reviewedNormalizationFailure=reviewed.get("normalizationFailure"))
    row["success"] = row["success"] and not execution["executionFailed"]
    return dict(row=row, cost=execution["cost"], measurementComplete=execution["measurementComplete"],
        work=dict(completedSpanChecks=None if budget is None else budget.checks,
            completedHashBytes=bundle_work.hash_bytes - before_hash,
            completedLineLinks=bundle_work.line_links - before_links))
