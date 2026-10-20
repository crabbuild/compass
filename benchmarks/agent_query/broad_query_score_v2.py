"""Protocol2 task scoring with whole-output and every-occurrence validation.

This is an internal scoring layer, not a file-backed assessment entry point.
Registration/source/graph pins, independent reviewer authentication, finite
lineage, actor hashes, full matrix, panels and formal statistics must be bound
by the collector/auditor before this layer can contribute qualifying evidence.
"""
from collections import Counter
import re

from benchmarks.agent_query import broad_query_review as legacy
from benchmarks.agent_query.broad_query_coverage import validate_printed_partition
from benchmarks.agent_query.broad_query_protocol import (
    ProtocolIntegrityError, precision_view,
)


class ExpressivenessCollision(ProtocolIntegrityError):
    """Distinct reviewed assertions collide under the frozen typed identity."""
    code = "EXPRESSIVENESS_COLLISION"


class _Witnesses:
    """Index only requested witness lines, scanning each pinned file once.

    Match the historical str.splitlines convention without materializing every
    source line or decoding a whole file again for each repeated assertion.
    """
    _line_end = re.compile(r"\r\n|[\n\r\v\f\x1c-\x1e\x85\u2028\u2029]")

    def __init__(self, sources, labels):
        self.sources = sources
        self.lines = {}
        wanted = {}
        for label in labels:
            if label.get("factId") is not None:
                continue
            for item in (label, *label["printedOccurrences"]):
                if item["verdict"] == "supported":
                    file, line, _ = legacy._site(item.get("source"))
                    wanted.setdefault(file, set()).add(line)
        if sources is None:
            return  # Isolated synthetic contracts only; no pin is established.
        total = 0
        for file, requested in wanted.items():
            data = sources.get(file)
            if type(data) is not bytes or len(data) > legacy.MAX_SOURCE_BYTES:
                raise ProtocolIntegrityError("supported assertion needs a bounded pinned source witness")
            total += len(data)
            if total > 256 * 1024**2:
                raise ProtocolIntegrityError("aggregate pinned witness source bytes exceed256MiB")
            text = data.decode("utf-8", errors="strict")
            start, number = 0, 1
            for ending in self._line_end.finditer(text):
                if number in requested:
                    self.lines[file, number] = text[start:ending.start()].strip()
                start, number = ending.end(), number + 1
            if start < len(text) and number in requested:
                self.lines[file, number] = text[start:].strip()

    def validate(self, source):
        file, line, text = legacy._site(source)
        if self.sources is not None and self.lines.get((file, line)) != text.strip():
            raise ProtocolIntegrityError("supported assertion differs from its pinned source witness")


def _source_verdict(verdict, fact_id, identity, relation, source, facts, witnesses):
    if verdict != "supported":
        return verdict
    if fact_id is None:
        witnesses.validate(source)
        return verdict
    if fact_id not in facts:
        raise ProtocolIntegrityError("review references an unknown frozen source fact")
    fact = facts[fact_id]
    if (identity != legacy.Identity.parse(fact["identity"])
            or not legacy._relation_matches(fact, relation)):
        return "unsupported"
    return verdict


def _score(task, observation, judgment, graph, output, budget, sources):
    if type(output) is not bytes or budget._raw != output:
        raise ProtocolIntegrityError("protocol2 scoring needs the budget's original immutable stdout bytes")
    partition = validate_printed_partition(observation, judgment, budget)
    if type(observation.get("normalizationComplete")) is not bool:
        raise ProtocolIntegrityError("normalization completeness must be explicit")
    if type(judgment.get("emptyConfirmed")) is not bool:
        raise ProtocolIntegrityError("confirmed-empty review must be explicit")
    claims, labels = partition["claims"], partition["labels"]
    for label in labels:
        if label.get("factId") is not None:
            # Match the unchanged frozen registration's fact-ID text contract
            # before witness selection, tuple hashing or dictionary membership.
            legacy._text(label["factId"], "reviewed fact id", 128)
    witnesses = _Witnesses(sources, labels)
    facts = {fact["id"]: fact for fact in task["facts"]}
    semantic_keys, effective_labels = set(), []
    occurrence_counts = Counter(dict.fromkeys(("supported", "unsupported", "unjudged"), 0))
    for claim, label in zip(claims, labels):
        identity = legacy.Identity.parse(claim.get("identity"))
        relation = legacy._relation(claim.get("relation"), observed=True)
        fact_id = label.get("factId")
        extra_site = legacy._site(label["source"]) if fact_id is None and "source" in label else None
        key = identity, relation, fact_id, extra_site
        if key in semantic_keys:
            # Arbitrary claim IDs, assertionKind, batches or printed repetitions
            # may not create another precision vote or bypass typed collisions.
            raise ExpressivenessCollision("reviewed assertions collide under the frozen typed equivalence key")
        semantic_keys.add(key)
        verdicts = []
        for occurrence in label["printedOccurrences"]:
            verdict = _source_verdict(occurrence["verdict"], fact_id, identity, relation,
                occurrence.get("source"), facts, witnesses)
            verdicts.append(verdict)
            occurrence_counts[verdict] += 1
        verdict = "unsupported" if "unsupported" in verdicts else "unjudged" if "unjudged" in verdicts else label["verdict"]
        # The shared engine retains original identity/relation/fact/group and
        # source-site matching. Each supported extra occurrence additionally
        # requires its own source witness above.
        if verdict == "supported" and fact_id is None:
            witnesses.validate(label.get("source"))
        effective_labels.append(dict(label, verdict=verdict))

    row = legacy._score_task_impl(task, observation, judgment, graph, output,
        claims_override=claims, labels_override=effective_labels,
        span_validator=lambda span, _: budget.validate(span))
    semantic_complete = (partition["semanticCoverageComplete"]
                         and observation["normalizationComplete"])
    judgment_complete = row["judgmentComplete"] and not occurrence_counts["unjudged"]
    view = precision_view(row["supportedClaims"], row["unsupportedClaims"], row["unjudgedClaims"],
        semantic_complete=semantic_complete, judgment_complete=judgment_complete,
        empty_confirmed=judgment["emptyConfirmed"])
    row.update(semanticCoverageComplete=semantic_complete, judgmentComplete=judgment_complete,
        normalizationComplete=observation["normalizationComplete"],
        printedOccurrenceCount=partition["printedOccurrenceCount"],
        occurrenceVerdicts=dict(occurrence_counts), unresolvedLineCount=partition["unresolvedLineCount"],
        precisionView=view, claimPrecision=view["fullMeasuredPrecision"],
        precisionWithEmptyZero=view["formalPrecisionEmptyZero"],
        falseEmpty=row["falseEmpty"] if semantic_complete else None)
    row["success"] = row["success"] and semantic_complete and judgment_complete
    return row


def score_task_v2(task, observation, judgment, graph, output, budget, *, sources=None):
    """Explicit version2 path; the public version1 scorer stays128-bounded.

    sources=None is for isolated synthetic contracts only. Full assessment must
    provide verified pinned source bytes and graphs; no qualification is implied
    by calling this arithmetic layer with a fixture.
    """
    try:
        return _score(task, observation, judgment, graph, output, budget, sources)
    except (ValueError, UnicodeError) as error:
        budget._failed = error
        raise
