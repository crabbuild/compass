"""Protocol2 bounds and printed-assertion contracts for collection/assessment.

Structural checks and byte bindings are not semantic/source completeness or
independent review. Version1 behavior is owned by its unchanged legacy path.
"""
from __future__ import annotations
from bisect import bisect_right
from dataclasses import dataclass
import hashlib
import re

MAX_RAW_BYTES = 64 * 1024 * 1024
MAX_DISTINCT_HASH_BYTES = 256 * 1024 * 1024
MAX_DISTINCT_SPANS = 65536 + 2 * 8192 + 2 * 512
MAX_SPAN_CHECKS = 262144
MAX_LINE_LINKS = 262144
MAX_BUNDLE_HASH_BYTES = 8 * 1024**3
MAX_BUNDLE_LINE_LINKS = 16777216
SHA = re.compile(r"[0-9a-f]{64}\Z")


class ProtocolIntegrityError(ValueError):
    """Invalid evidence/lineage is not a successful prefix or semantic empty."""


class ProtocolLimit(ValueError):
    def __init__(self, code, resource, bound, completed, requested):
        super().__init__(f"{code}: {resource} limit exceeded")
        self.code = code
        self.resource = dict(name=resource, bound=bound, countAtLeast=completed,
                             exactCountKnown=True, attemptedValue=requested)


@dataclass
class BundleWork:
    hash_bytes: int = 0
    line_links: int = 0

    def __post_init__(self):
        if (type(self.hash_bytes) is not int or not 0 <= self.hash_bytes <= MAX_BUNDLE_HASH_BYTES
                or type(self.line_links) is not int or not 0 <= self.line_links <= MAX_BUNDLE_LINE_LINKS):
            raise ProtocolIntegrityError("invalid bounded bundle work counters")

    def require_hash_capacity(self, increment):
        if type(increment) is not int or increment < 0:
            raise ProtocolIntegrityError("invalid hash increment")
        if self.hash_bytes + increment > MAX_BUNDLE_HASH_BYTES:
            raise ProtocolLimit("BUNDLE_HASH_WORK_LIMIT", "completed_hash_bytes",
                                MAX_BUNDLE_HASH_BYTES, self.hash_bytes, self.hash_bytes + increment)

    def require_link_capacity(self):
        if self.line_links + 1 > MAX_BUNDLE_LINE_LINKS:
            raise ProtocolLimit("BUNDLE_COVERAGE_LINK_LIMIT", "completed_line_links",
                                MAX_BUNDLE_LINE_LINKS, self.line_links, self.line_links + 1)


@dataclass(frozen=True)
class _Line:
    start: int
    end: int
    physical: int
    digest: str


class PrintedSpanBudget:
    """One bounded raw page, finite initial/reviewed references, indexed joins."""
    def __init__(self, output, bundle=None):
        if type(output) is not bytes or len(output) > MAX_RAW_BYTES:
            raise ProtocolIntegrityError("invalid or oversized immutable raw stdout")
        self._text = output.decode("utf-8", errors="strict")
        self._raw = output
        self.raw_sha256 = hashlib.sha256(output).hexdigest()
        self.bundle = bundle if bundle is not None else BundleWork()
        self.checks = 0
        self.hashed_bytes = 0
        self.line_links = 0
        self._digests = {}
        self._registry = {"initial": {}, "reviewed": {}}
        self._joins = {}
        self._failed = None
        physical_count = output.count(b"\n") + int(bool(output) and not output.endswith(b"\n"))
        if physical_count > 65536:
            raise ProtocolLimit("PHYSICAL_LINE_LIMIT", "physical_lines", 65536,
                                physical_count, physical_count)
        lines, start, physical = [], 0, 0
        while start < len(output):
            lf = output.find(b"\n", start)
            end = lf + 1 if lf >= 0 else len(output)
            physical += 1
            if physical > 65536:
                raise ProtocolLimit("PHYSICAL_LINE_LIMIT", "physical_lines", 65536,
                                    physical - 1, physical)
            if end - start > 65536:
                raise ProtocolLimit("LINE_BYTE_LIMIT", "physical_line_bytes", 65536,
                                    end - start, end - start)
            content_end = lf if lf >= 0 else end
            if lf > start and output[lf - 1] == 13:
                content_end -= 1
            if any(byte not in (32, 9) for byte in memoryview(output)[start:content_end]):
                digest = self._digest(start, content_end)
                lines.append(_Line(start, content_end, physical, digest))
            start = end
        self.lines = tuple(lines)
        self._starts = tuple(line.start for line in lines)
        self._ends = tuple(line.end for line in lines)

    def _guard(self):
        if self._failed is not None:
            raise self._failed

    def _coordinates(self, start, end):
        if (type(start) is not int or type(end) is not int
                or not 0 <= start < end <= len(self._raw)):
            raise ProtocolIntegrityError("invalid printed span coordinates")
        if (self._raw[start] & 0xc0 == 0x80
                or end < len(self._raw) and self._raw[end] & 0xc0 == 0x80):
            raise ProtocolIntegrityError("printed span splits a UTF8 code point")

    def _digest(self, start, end):
        self._guard()
        if self.checks + 1 > MAX_SPAN_CHECKS:
            raise ProtocolLimit("SPAN_CHECK_LIMIT", "completed_span_checks", MAX_SPAN_CHECKS,
                                self.checks, self.checks + 1)
        self.checks += 1
        self._coordinates(start, end)
        key = self.raw_sha256, start, end
        if key not in self._digests:
            if len(self._digests) + 1 > MAX_DISTINCT_SPANS:
                raise ProtocolLimit("SPAN_CACHE_LIMIT", "cached_intervals", MAX_DISTINCT_SPANS,
                                    len(self._digests), len(self._digests) + 1)
            increment = end - start
            if self.hashed_bytes + increment > MAX_DISTINCT_HASH_BYTES:
                raise ProtocolLimit("SPAN_BYTE_WORK_LIMIT", "completed_hash_bytes", MAX_DISTINCT_HASH_BYTES,
                                    self.hashed_bytes, self.hashed_bytes + increment)
            self.bundle.require_hash_capacity(increment)
            actual = hashlib.sha256(memoryview(self._raw)[start:end]).hexdigest()
            self.hashed_bytes += increment
            self.bundle.hash_bytes += increment
            self._digests[key] = actual
        return self._digests[key]

    def validate(self, span):
        try:
            self._guard()
            if (not isinstance(span, dict) or set(span) != {"startByte", "endByte", "sha256"}
                    or not isinstance(span["sha256"], str) or not SHA.fullmatch(span["sha256"])):
                raise ProtocolIntegrityError("invalid printed span shape or SHA")
            actual = self._digest(span["startByte"], span["endByte"])
            if actual != span["sha256"]:
                raise ProtocolIntegrityError("printed span differs from original stdout")
        except (ValueError, UnicodeError) as error:
            self._failed = error
            raise

    def ledger(self):
        self._guard()
        return [dict(index=i, physicalLine=line.physical,
            span=dict(startByte=line.start, endByte=line.end, sha256=line.digest))
            for i, line in enumerate(self.lines)]

    def join(self, phase, claim_id, occurrence_id, span):
        try:
            self._guard()
            if not isinstance(phase, str) or phase not in self._registry:
                raise ProtocolIntegrityError("only selected initial/reviewed phases are allowed")
            _claim_id(dict(id=claim_id))
            _claim_id(dict(id=occurrence_id))
            self.validate(span)
            start, end = span["startByte"], span["endByte"]
            previous = self._registry[phase].get(occurrence_id)
            identity = claim_id, start, end
            if previous is not None and previous != identity:
                raise ProtocolIntegrityError("occurrence ID changed its claim/span")
            key = phase, occurrence_id
            if key in self._joins:
                return self._joins[key]
            if previous is None and len(self._registry[phase]) >= 8192:
                raise ProtocolLimit("OCCURRENCE_LIMIT", "registered_occurrence_records", 8192,
                                    len(self._registry[phase]), len(self._registry[phase]) + 1)
            self._registry[phase][occurrence_id] = identity
            result, index = [], bisect_right(self._ends, start)
            while index < len(self.lines) and self._starts[index] < end:
                if self.line_links + 1 > MAX_LINE_LINKS:
                    raise ProtocolLimit("COVERAGE_LINK_LIMIT", "completed_line_links", MAX_LINE_LINKS,
                                        self.line_links, self.line_links + 1)
                self.bundle.require_link_capacity()
                result.append(index)
                self.line_links += 1
                self.bundle.line_links += 1
                index += 1
            if not result:
                raise ProtocolIntegrityError("assertion span has no nonempty printed line")
            self._joins[key] = tuple(result)
            return self._joins[key]
        except (ValueError, UnicodeError) as error:
            self._failed = error
            raise

MAX_CLAIMS_PER_BATCH = 128
MAX_CLAIM_BATCHES = 16
MAX_BATCHED_CLAIMS = MAX_CLAIMS_PER_BATCH * MAX_CLAIM_BATCHES


def _integer(value, where, maximum):
    if type(value) is not int or not 0 <= value <= maximum:
        raise ValueError(f"{where} must be an integer in0..{maximum}")
    return value


def _claim_id(record):
    if not isinstance(record, dict):
        raise ValueError("claim/label must be an object")
    identifier = record.get("id")
    if (not isinstance(identifier, str) or not identifier
            or len(identifier.encode("utf-8")) > 128
            or any(ord(c) < 32 for c in identifier)):
        raise ValueError("invalid claim/label ID")
    return identifier


def pack_claim_batches(claims):
    """Canonical128-record packing; never truncate an oversized successful prefix."""
    if not isinstance(claims, list) or len(claims) > MAX_BATCHED_CLAIMS:
        raise ValueError("claim aggregate exceeds2048 or is not a list")
    identifiers = [_claim_id(c) for c in claims]
    if len(set(identifiers)) != len(identifiers):
        raise ValueError("duplicate claim/label ID across batches")
    return dict(claimCount=len(claims), claimBatches=[
        dict(index=index, claims=claims[start:start + MAX_CLAIMS_PER_BATCH])
        for index, start in enumerate(range(0, len(claims), MAX_CLAIMS_PER_BATCH))])


def flatten_claim_batches(record):
    """Validate every partition before returning one global scoring sequence.

    This validates only transport. It does not establish semantic uniqueness,
    source truth, printed-assertion coverage, provenance, or reviewer independence.
    """
    if not isinstance(record, dict) or "claims" in record:
        raise ValueError("batched rows require objects without a flat claims field")
    count = _integer(record.get("claimCount"), "claim count", MAX_BATCHED_CLAIMS)
    batches = record.get("claimBatches")
    expected = (count + MAX_CLAIMS_PER_BATCH - 1) // MAX_CLAIMS_PER_BATCH
    if not isinstance(batches, list) or len(batches) != expected or len(batches) > MAX_CLAIM_BATCHES:
        raise ValueError("batch count does not match canonical claim count")
    result, identifiers = [], set()
    for index, batch in enumerate(batches):
        if not isinstance(batch, dict) or set(batch) != {"index", "claims"}:
            raise ValueError("invalid batch object")
        if type(batch["index"]) is not int or batch["index"] != index:
            raise ValueError("batch indexes must be contiguous and ordered")
        claims = batch["claims"]
        required = min(MAX_CLAIMS_PER_BATCH, count - len(result))
        if not isinstance(claims, list) or len(claims) != required:
            raise ValueError("noncanonical or empty claim batch")
        for claim in claims:
            identifier = _claim_id(claim)
            if identifier in identifiers:
                raise ValueError("duplicate claim/label ID across batches")
            identifiers.add(identifier)
            result.append(claim)
    if len(result) != count:
        raise ValueError("claim count mismatch")
    return result


def validate_review_partition(normalized, judgment):
    """A later batch cannot silently omit, reorder, duplicate or add labels."""
    claims = flatten_claim_batches(normalized)
    labels = flatten_claim_batches(judgment)
    if [c["id"] for c in claims] != [c["id"] for c in labels]:
        raise ValueError("claim review does not match exact global batch/ID partition")
    return claims, labels


MAX_CLAIMS = 2048
MAX_ROWS = 8192


def precision_view(supported, unsupported, unjudged, *, semantic_complete,
                   judgment_complete, empty_confirmed=False):
    counts = supported, unsupported, unjudged
    if (any(type(n) is not int or not 0 <= n <= MAX_CLAIMS for n in counts)
            or sum(counts) > MAX_CLAIMS
            or any(type(b) is not bool for b in (semantic_complete, judgment_complete, empty_confirmed))):
        raise ValueError("invalid bounded precision counts/completeness flags")
    total = sum(counts)
    if judgment_complete and unjudged:
        raise ValueError("unjudged assertions cannot have complete judgments")
    complete = semantic_complete and judgment_complete
    if empty_confirmed and (not complete or total):
        raise ValueError("confirmed empty requires complete coverage/judgment and no assertions")
    if complete and not total and not empty_confirmed:
        raise ValueError("zero emitted records do not establish empty printed assertions")
    measured = supported / total if complete and total else None
    formal = measured if total else 0.0 if empty_confirmed else None
    return dict(knownSupported=supported, knownUnsupported=unsupported,
        knownUnjudged=unjudged, knownClaimCount=total,
        semanticCoverageComplete=semantic_complete, judgmentComplete=judgment_complete,
        emptyConfirmed=empty_confirmed,
        fullMeasuredPrecision=measured,
        formalPrecisionEmptyZero=formal if complete else None,
        judgedOnlyPrecision=supported/(supported+unsupported) if supported+unsupported else None,
        knownSupportFraction=supported/total if semantic_complete and total else None,
        completionPenalizedPrecision=(measured if total else 0.0) if complete else 0.0,
        formalPrecisionAvailable=complete,
        note="Penalty and partial-count fractions are diagnostics, never paired NI evidence.")


def _canonical_precision_row(row):
    """Recompute every derived value; mutable/serialized flags have no authority.

    Source completeness still requires independent source review. This validates
    only that the supplied precision record is a canonical mechanical view of
    its bounded primitive counts and completeness flags.
    """
    primitives = ("knownSupported", "knownUnsupported", "knownUnjudged",
                  "semanticCoverageComplete", "judgmentComplete", "emptyConfirmed")
    if type(row) is not dict or any(key not in row for key in primitives):
        raise ValueError("invalid canonical precision row")
    canonical = precision_view(row["knownSupported"], row["knownUnsupported"],
        row["knownUnjudged"], semantic_complete=row["semanticCoverageComplete"],
        judgment_complete=row["judgmentComplete"], empty_confirmed=row["emptyConfirmed"])
    if row.keys() != canonical.keys() or any(
            type(row[key]) is not type(value) or row[key] != value
            for key, value in canonical.items()):
        raise ValueError("precision row contradicts its canonical primitive counts/flags")
    return canonical


def aggregate_precision(rows):
    if not isinstance(rows, list) or not 1 <= len(rows) <= MAX_ROWS:
        raise ValueError("invalid complete precision row collection")
    # Validate all rows before pooling; never skip unknown or malformed rows.
    # This preview does not authenticate a source judgment or its completeness.
    rows = [_canonical_precision_row(row) for row in rows]
    available = all(r["formalPrecisionAvailable"] for r in rows)
    supported = sum(r["knownSupported"] for r in rows)
    unsupported = sum(r["knownUnsupported"] for r in rows)
    return dict(rowsRetained=len(rows),
        unknownRows=sum(not r["formalPrecisionAvailable"] for r in rows),
        formalPrecisionAvailable=available,
        fullMeasuredPooledPrecision=supported/(supported+unsupported)
            if available and supported+unsupported else None,
        completeRowMacroEmptyZero=sum(r["formalPrecisionEmptyZero"] for r in rows)/len(rows)
            if available else None,
        diagnosticCompletionPenalizedMacro=sum(r["completionPenalizedPrecision"] for r in rows)/len(rows))


def paired_precision_status(candidate, comparator):
    if (not isinstance(candidate, dict) or not isinstance(comparator, dict)
            or not candidate or len(candidate) > MAX_ROWS or candidate.keys() != comparator.keys()):
        raise ValueError("paired precision requires identical nonempty frozen row keys")
    a = aggregate_precision(list(candidate.values()))
    b = aggregate_precision(list(comparator.values()))
    available = a["formalPrecisionAvailable"] and b["formalPrecisionAvailable"]
    return dict(rowsRetained=len(candidate), formalPrecisionComparisonAvailable=available,
        diagnosticCompletionPenalizedDifference=a["diagnosticCompletionPenalizedMacro"]-b["diagnosticCompletionPenalizedMacro"],
        precisionInputsComplete=available,
        note="No formal effect/NI is computed here; fixed cohort/cluster weights and all canonical gates remain required.")
