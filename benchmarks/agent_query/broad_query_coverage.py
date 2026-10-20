"""Protocol2 printed partition validation, prior to typed source scoring.

This layer validates the whole partition once across all batches. Its complete
flag records an explicit review attestation plus mechanical closure, never an
inference of semantic completeness from line overlap. Source facts, identity,
typed semantic collisions and reviewer independence remain scorer obligations.
"""
from benchmarks.agent_query.broad_query_protocol import (
    ProtocolIntegrityError, ProtocolLimit, _claim_id, validate_review_partition,
)

NORMALIZED_SCHEMA = "compass.broad-query-normalized/2"
JUDGMENTS_SCHEMA = "compass.broad-query-judgments/2"
MAX_OCCURRENCES = 8192
MAX_REASON_BYTES = 4096
VERDICTS = ("supported", "unsupported", "unjudged")


def _reason(value):
    if (type(value) is not str or not value.strip()
            or len(value.encode("utf-8")) > MAX_REASON_BYTES):
        raise ProtocolIntegrityError("missing or oversized output review rationale")


def _span_key(span, identifier):
    return span["startByte"], span["endByte"], identifier


def _validate(normalized, judgment, budget, phase):
    budget._guard()
    if type(phase) is not str or phase not in ("initial", "reviewed"):
        raise ProtocolIntegrityError("only the finite selected initial/reviewed phases are allowed")
    if (type(normalized) is not dict or type(judgment) is not dict
            or normalized.get("schema") != NORMALIZED_SCHEMA
            or judgment.get("schema") != JUDGMENTS_SCHEMA):
        raise ProtocolIntegrityError("printed partitions require homogeneous protocol2 schemas")
    for document in (normalized, judgment):
        if type(document.get("semanticCoverageComplete")) is not bool:
            raise ProtocolIntegrityError("semantic completeness requires an explicit boolean attestation")
    claims, labels = validate_review_partition(normalized, judgment)
    # Bound the complete occurrence shape before making any span joins. A late
    # malformed batch cannot yield an accepted prefix or reset a global budget.
    count = 0
    for claim, label in zip(claims, labels):
        occurrences = claim.get("printedOccurrences")
        reviews = label.get("printedOccurrences")
        if (type(occurrences) is not list or not occurrences
                or type(reviews) is not list or len(reviews) != len(occurrences)):
            raise ProtocolIntegrityError("every claim needs its exact nonempty occurrence review partition")
        requested = count + len(occurrences)
        if requested > MAX_OCCURRENCES:
            raise ProtocolLimit("OCCURRENCE_LIMIT", "registered_occurrence_records",
                                MAX_OCCURRENCES, count, requested)
        count = requested

    ledger = budget.ledger()
    normalized_lines = normalized.get("lineLedger")
    reviewed_lines = judgment.get("lineCoverage")
    if (type(normalized_lines) is not list or len(normalized_lines) != len(ledger)
            or type(reviewed_lines) is not list or len(reviewed_lines) != len(ledger)):
        raise ProtocolIntegrityError("output line review must retain the complete exact nonempty LF partition")
    line_claims = [[] for _ in ledger]
    global_occurrences = set()
    claim_order = []
    effective = []
    occurrence_counts = dict.fromkeys(VERDICTS, 0)
    for claim, label in zip(claims, labels):
        claim_id = _claim_id(claim)
        occurrences = claim["printedOccurrences"]
        reviews = label["printedOccurrences"]
        occurrence_order, claim_spans, verdicts = [], set(), []
        for occurrence, review in zip(occurrences, reviews):
            if (type(occurrence) is not dict or not {"id", "span"} <= occurrence.keys()
                    or not occurrence.keys() <= {"id", "span", "printedSite"}
                    or type(review) is not dict):
                raise ProtocolIntegrityError("invalid printed occurrence or label shape")
            occurrence_id = _claim_id(occurrence)
            if occurrence_id in global_occurrences:
                raise ProtocolIntegrityError("printed occurrence IDs must be globally unique")
            global_occurrences.add(occurrence_id)
            if _claim_id(review) != occurrence_id:
                raise ProtocolIntegrityError("occurrence reviews must match the exact global partition order")
            # Labels bind the reviewed occurrence, never another arbitrary span.
            if "span" in review or "printedOccurrences" in review:
                raise ProtocolIntegrityError("occurrence labels cannot introduce alternative printed spans")
            span = occurrence["span"]
            indexes = budget.join(phase, claim_id, occurrence_id, span)
            key = _span_key(span, occurrence_id)
            if key[:2] in claim_spans:
                raise ProtocolIntegrityError("duplicate same-claim printed span")
            claim_spans.add(key[:2])
            occurrence_order.append(key)
            for index in indexes:
                # Claims arrive in canonical global order; multiple occurrences
                # of the same claim may touch a line without duplicating its ID.
                if not line_claims[index] or line_claims[index][-1] != claim_id:
                    line_claims[index].append(claim_id)
            verdict = review.get("verdict")
            if type(verdict) is not str or verdict not in VERDICTS:
                raise ProtocolIntegrityError("invalid printed occurrence verdict")
            _reason(review.get("reason"))
            occurrence_counts[verdict] += 1
            verdicts.append(verdict)
        if occurrence_order != sorted(occurrence_order):
            raise ProtocolIntegrityError("printed occurrences are not in canonical byte order")
        budget.validate(claim.get("span"))
        if claim["span"] != occurrences[0]["span"]:
            raise ProtocolIntegrityError("claim span must exactly equal its first printed occurrence")
        claim_order.append(_span_key(claim["span"], claim_id))
        verdict = label.get("verdict")
        if type(verdict) is not str or verdict not in VERDICTS:
            raise ProtocolIntegrityError("invalid claim verdict")
        _reason(label.get("reason"))
        # Occurrence completeness takes precedence over the independent
        # claim-level label. A pending occurrence remains pending unless an
        # occurrence itself is unsupported; otherwise retain the claim verdict.
        effective.append(dict(id=claim_id, verdict="unsupported" if "unsupported" in verdicts
            else "unjudged" if "unjudged" in verdicts else verdict))
    if claim_order != sorted(claim_order):
        raise ProtocolIntegrityError("claims must be globally ordered by first span then ID")

    unresolved = 0
    for index, (actual, observed, reviewed) in enumerate(zip(ledger, normalized_lines, reviewed_lines)):
        if type(observed) is dict:
            budget.validate(observed.get("span"))
        if (type(observed) is not dict or observed.keys() != {*actual, "claimIds"}
                or any(type(observed[k]) is not type(v) or observed[k] != v
                       for k, v in actual.items())
                or observed.get("claimIds") != line_claims[index]):
            raise ProtocolIntegrityError("normalized line ledger or occurrence-to-line links differ from raw output")
        if (type(reviewed) is not dict
                or reviewed.keys() != {"index", "classification", "claimIds", "reason"}
                or type(reviewed.get("index")) is not int or reviewed["index"] != index
                or reviewed.get("claimIds") != line_claims[index]):
            raise ProtocolIntegrityError("review line-to-claim links must match all charged occurrences")
        _reason(reviewed.get("reason"))
        classification = reviewed.get("classification")
        if type(classification) is not str or classification not in ("assertions", "non_assertion", "unresolved"):
            raise ProtocolIntegrityError("invalid printed line classification")
        if (classification == "assertions" and not line_claims[index]
                or classification == "non_assertion" and line_claims[index]):
            raise ProtocolIntegrityError("printed line classification contradicts its assertion links")
        unresolved += classification == "unresolved"
    attested = normalized["semanticCoverageComplete"] and judgment["semanticCoverageComplete"]
    if attested and unresolved:
        raise ProtocolIntegrityError("unresolved lines cannot have complete semantic coverage")
    return dict(claims=claims, labels=labels, effectiveClaimVerdicts=effective,
        printedOccurrenceCount=count, occurrenceVerdicts=occurrence_counts,
        unresolvedLineCount=unresolved, semanticCoverageComplete=attested and not unresolved,
        note="Mechanical partition closure plus explicit attestation; typed source scoring and independent review still required.")


def validate_printed_partition(normalized, judgment, budget, *, phase="reviewed"):
    """Reject the whole observation on a malformed or incomplete partition.

    Valid unresolved lines remain explicitly incomplete rather than rejected.
    One budget is retained across initial/reviewed processing; do not construct
    another budget to retry a failed row or reset its completed work charges.
    """
    try:
        return _validate(normalized, judgment, budget, phase)
    except (ValueError, UnicodeError) as error:
        budget._failed = error
        raise
