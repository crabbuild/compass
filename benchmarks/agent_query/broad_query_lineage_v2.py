"""Finite immutable protocol2 review lineage, before source/cost assessment.

This validates artifact and partition bindings only. It does not authenticate
reviewers, judge assertions, run queries or compute qualification statistics.
Public version2 capture/assessment remains disabled until full integration.
"""
from dataclasses import dataclass

from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_actors_v2 import verify_retained_protocol
from benchmarks.agent_query.broad_query_protocol import (
    _claim_id, flatten_claim_batches, validate_review_partition,
    MAX_RAW_BYTES, MAX_DISTINCT_HASH_BYTES, MAX_DISTINCT_SPANS,
    MAX_SPAN_CHECKS, MAX_LINE_LINKS, MAX_BUNDLE_HASH_BYTES, MAX_BUNDLE_LINE_LINKS,
)

SCHEMAS = {name: f"compass.broad-query-{name}/2" for name in (
    "capture-plan", "capture-input", "capture", "normalized", "judgments",
    "review-input", "review", "panel-review-input", "panel-review")}
SELECTION_KEYS = {"task", "tool", "arm", "initialNormalizedRef", "reviewedNormalizedRef",
    "originalRunSha256", "rawStdoutSha256", "sourceRegistrationSha256", "actorSha256"}
NORMALIZED_KEYS = {"schema", "task", "tool", "arm", "rawStdoutSha256", "sourceRegistrationSha256",
    "actorSha256", "normalizationStatus", "normalizationComplete", "semanticCoverageComplete",
    "disposition", "rankingKind", "admission", "ranked", "claimCount", "claimBatches",
    "lineLedger", "reviewQueue", "normalizationCaveats", "printedAssertionCount",
    "fullMeasuredPrecision", "falseEmpty", "success", "normalizationFailure"}
LIMITS = {
    "CLAIM_LIMIT": ("registered_claim_records", 2048),
    "OCCURRENCE_LIMIT": ("registered_occurrence_records", 8192),
    "PHYSICAL_LINE_LIMIT": ("physical_lines", 65536),
    "LINE_BYTE_LIMIT": ("physical_line_bytes", 65536),
    "JSON_BYTE_LIMIT": ("encoded_json_bytes", old.MAX_JSON_BYTES),
    "SPAN_BYTE_WORK_LIMIT": ("completed_hash_bytes", MAX_DISTINCT_HASH_BYTES),
    "SPAN_CACHE_LIMIT": ("cached_intervals", MAX_DISTINCT_SPANS),
    "SPAN_CHECK_LIMIT": ("completed_span_checks", MAX_SPAN_CHECKS),
    "COVERAGE_LINK_LIMIT": ("completed_line_links", MAX_LINE_LINKS),
    "BUNDLE_HASH_WORK_LIMIT": ("completed_hash_bytes", MAX_BUNDLE_HASH_BYTES),
    "BUNDLE_COVERAGE_LINK_LIMIT": ("completed_line_links", MAX_BUNDLE_LINE_LINKS),
}
REJECTION_CODES = (*LIMITS, "INVALID_UTF8", "PARSER_REJECTION", "UNSUPPORTED_ASSERTION_SHAPE", "EXPRESSIVENESS_COLLISION")


def require_schema(document, kind):
    if type(document) is not dict or document.get("schema") != SCHEMAS[kind]:
        raise ValueError("unsupported or mixed protocol2 " + kind + " schema")
    return document


def _reference(value):
    if type(value) is not dict or value.keys() != {"path", "sha256"}:
        raise ValueError("artifact reference needs exactly path and sha256")
    old._relative(value["path"])
    old._digest(value["sha256"])
    return value


def _key(row):
    if type(row) is not dict:
        raise ValueError("observation must be an object")
    return (_claim_id(dict(id=row.get("task"))),
        old._choice(row.get("tool"), old.TOOLS, "native tool"),
        old._choice(row.get("arm"), old.ARMS, "cost arm"))


def _ordered(rows, expected, where):
    if type(rows) is not list or len(rows) != len(expected):
        raise ValueError(where + " must retain the complete frozen observation matrix")
    if [_key(row) for row in rows] != expected:
        raise ValueError(where + " has missing, duplicate, reordered or foreign observations")


def _rejection(document):
    if (document.get("normalizationComplete") is not False
            or document.get("semanticCoverageComplete") is not False
            or type(document.get("claimCount")) is not int or document["claimCount"] != 0
            or type(document.get("claimBatches")) is not list or document["claimBatches"]
            or any(key not in document or document[key] is not None for key in
                   ("printedAssertionCount", "fullMeasuredPrecision", "falseEmpty"))
            or document.get("success") is not False
            or document.get("rankingKind") != "unavailable"
            or document.get("admission") is not None
            or type(document.get("ranked")) is not list or document["ranked"]
            or any(key in document and (type(document[key]) is not list or document[key])
                   for key in ("lineLedger", "reviewQueue"))):
        raise ValueError("rejection cannot publish a successful prefix or assert semantic empty")
    failure = document.get("normalizationFailure")
    if (type(failure) is not dict or not {"code", "resource"} <= failure.keys()
            or not failure.keys() <= {"code", "resource", "message"}):
        raise ValueError("invalid closed normalization rejection")
    code = old._choice(failure["code"], REJECTION_CODES, "normalization rejection code")
    if document.get("disposition") not in ("error", "limit"):
        raise ValueError("rejection cannot publish a successful native disposition")
    if "message" in failure:
        old._text(failure["message"], "failure message", 1024)
        if len(failure["message"].encode()) > 1024:
            raise ValueError("failure message exceeds1024 UTF8 bytes")
    resource = failure["resource"]
    if code not in LIMITS:
        if resource is not None:
            raise ValueError("non-limit rejection must not invent resource counts")
        return
    if (type(resource) is not dict
            or not {"name", "bound", "countAtLeast", "exactCountKnown"} <= resource.keys()
            or not resource.keys() <= {"name", "bound", "countAtLeast", "exactCountKnown", "attemptedValue"}):
        raise ValueError("invalid normalization resource record")
    name, bound = LIMITS[code]
    if resource["name"] != name or type(resource["bound"]) is not int or resource["bound"] != bound:
        raise ValueError("normalization resource differs from frozen bound or unit")
    count = old._integer(resource["countAtLeast"], "completed/scanned work", 0, 2**63 - 1)
    old._boolean(resource["exactCountKnown"], "resource count completeness")
    if "attemptedValue" in resource:
        attempted = old._integer(resource["attemptedValue"], "attempted work", 0, 2**63 - 1)
        if attempted <= bound or attempted < count:
            raise ValueError("attempted work does not explain the frozen limit")


def _normalization(document, key, raw_sha, registration_sha, actors, *, initial):
    require_schema(document, "normalized")
    if not document.keys() <= NORMALIZED_KEYS or _key(document) != key:
        raise ValueError("normalization has foreign identity or unsupported reference-chain fields")
    if (document.get("rawStdoutSha256") != raw_sha
            or document.get("sourceRegistrationSha256") != registration_sha
            or document.get("actorSha256") != actors):
        raise ValueError("normalization differs from original stdout, source registration or actors")
    status = old._choice(document.get("normalizationStatus"),
        ("pending_review", "reviewed", "rejected"), "normalization status")
    complete = old._boolean(document.get("normalizationComplete"), "normalization completeness")
    semantic = old._boolean(document.get("semanticCoverageComplete"), "semantic coverage")
    if initial and (status == "reviewed" or complete or semantic):
        raise ValueError("initial normalization cannot claim independent review or bind a future run")
    if (complete or semantic) and status != "reviewed":
        raise ValueError("completed normalization must explicitly be reviewed")
    if status == "rejected":
        _rejection(document)
    else:
        if document.keys() & {"normalizationFailure", "printedAssertionCount",
                "fullMeasuredPrecision", "falseEmpty", "success"}:
            raise ValueError("normalization transport cannot publish scoring or rejection fields")
        flatten_claim_batches(document)


def _occurrence_partition(normalized, judgment):
    claims, labels = validate_review_partition(normalized, judgment)
    identifiers, count = set(), 0
    for claim, label in zip(claims, labels):
        occurrences, reviews = claim.get("printedOccurrences"), label.get("printedOccurrences")
        if (type(occurrences) is not list or not occurrences or type(reviews) is not list
                or len(reviews) != len(occurrences) or count + len(occurrences) > 8192):
            raise ValueError("judgments must retain the entire bounded occurrence partition")
        count += len(occurrences)
        for occurrence, review in zip(occurrences, reviews):
            identifier = _claim_id(occurrence)
            if identifier in identifiers or _claim_id(review) != identifier:
                raise ValueError("duplicate or foreign occurrence label")
            identifiers.add(identifier)


@dataclass(frozen=True)
class BoundObservation:
    key: tuple
    captured: dict
    capture: dict
    stdout: bytes
    stderr: bytes
    initial: dict
    reviewed: dict
    judgment: dict
    selection: dict


class ReviewLineage:
    """One bounded traversal; incomplete/failed traversals cannot be published.

    Large raw/normalization/judgment bodies are loaded one observation at a
    time. Global JSON contains only the complete finite matrix and references.
    """
    def __init__(self, registration, run, reviews, rows, artifacts, code_root,
                 registration_sha, run_sha, actors):
        self.registration, self.run, self.reviews = registration, run, reviews
        self.keys = tuple(row[0] for row in rows)
        self._rows, self.artifacts, self.code_root = rows, artifacts, code_root
        self._registration_sha, self._run_sha, self._actors = registration_sha, run_sha, actors
        self._started, self._complete, self._failed = False, False, False

    def iter_observations(self):
        if self._started or self._failed:
            raise ValueError("lineage permits only one observation traversal")
        self._started = True
        return self._iterate()

    def _iterate(self):
        paths = set()
        try:
            for key, observed, selection, index in self._rows:
                yield _load_observation(key, observed, selection, index, self.artifacts,
                    self._registration_sha, self._run_sha, self._actors, paths)
            self._complete = True
        finally:
            if not self._complete:
                self._failed = True

    def recheck(self):
        if self._failed or not self._complete:
            raise ValueError("the entire lineage must finish successfully before publication")
        try:
            verify_retained_protocol(self.run["protocolBinding"], self.artifacts, self.code_root)
            self.artifacts.recheck()
        except (ValueError, OSError):
            self._failed = True
            raise


def _load_observation(key, observed, selection, index, artifacts, registration_sha, run_sha, actors, capture_paths):
    capture_ref = _reference(observed["captures"][0])
    capture = artifacts.document(capture_ref)
    stdout_ref, stderr_ref = (_reference(capture.get(stream)) for stream in ("stdout", "stderr"))
    for ref in (capture_ref, stdout_ref, stderr_ref):
        if ref["path"] in capture_paths:
            raise ValueError("separate frozen executions cannot reuse captured stream/record paths")
        capture_paths.add(ref["path"])
    stdout, stderr = artifacts.read(stdout_ref, MAX_RAW_BYTES), artifacts.read(stderr_ref, MAX_RAW_BYTES)
    for name, data in (("stdout", stdout), ("stderr", stderr)):
        if old._integer(capture.get(name + "_bytes"), "captured stream bytes", 0, MAX_RAW_BYTES) != len(data):
            raise ValueError("native stream byte count differs from original artifact")
    if selection["rawStdoutSha256"] != stdout_ref["sha256"]:
        raise ValueError("selection differs from original raw stdout")
    initial_ref, reviewed_ref = selection["initialNormalizedRef"], selection["reviewedNormalizedRef"]
    initial = artifacts.document(initial_ref)
    reviewed = initial if reviewed_ref == initial_ref else artifacts.document(reviewed_ref)
    _normalization(initial, key, stdout_ref["sha256"], registration_sha, actors, initial=True)
    _normalization(reviewed, key, stdout_ref["sha256"], registration_sha, actors, initial=False)
    judgment = require_schema(artifacts.document(index["judgmentRef"]), "judgments")
    if (_key(judgment) != key or judgment.get("runSha256") != run_sha
            or judgment.get("reviewedNormalizedSha256") != reviewed_ref["sha256"]):
        raise ValueError("judgments differ from original run or exact reviewed normalization bytes")
    _occurrence_partition(reviewed, judgment)
    return BoundObservation(key, observed, capture, stdout, stderr, initial, reviewed, judgment, selection)


def load_review_lineage(manifest, artifacts, code_root):
    """Bind one immutable run to finite initial/reviewed selections.

    code_root is the runtime's own product source root, never captured metadata.
    Consume iter_observations once using this same reader for graph/source work;
    discard each observation body after scoring and call recheck before publishing.
    The index must point to separate bounded per-observation judgment documents.
    This does not validate source truth, graph/cost claims or all task fields.
    """
    require_schema(manifest, "review-input")
    if not manifest.keys() <= {"schema", "registration", "run", "reviews", "normalizedSelections",
            "sourceRoots", "graphQualification", "exposureEvidence", "calibrationEvidence"}:
        raise ValueError("unsupported review-input fields or transitive amendment references")
    registration_ref, run_ref, reviews_ref = (_reference(manifest.get(name)) for name in ("registration", "run", "reviews"))
    registration = old.validate_registration(artifacts.document(registration_ref))
    run = require_schema(artifacts.document(run_ref), "capture")
    reviews = require_schema(artifacts.document(reviews_ref), "judgments")
    if reviews.keys() != {"schema", "runSha256", "observations"}:
        raise ValueError("judgment index needs exactly the frozen run and observation references")
    plan = require_schema(registration.get("capturePlan"), "capture-plan")
    if run.get("registrationSha256") != registration_ref["sha256"] or reviews.get("runSha256") != run_ref["sha256"]:
        raise ValueError("run or judgments differ from frozen source/run lineage")
    started, completed = old._timestamp(run.get("startedAt")), old._timestamp(run.get("completedAt"))
    if old._timestamp(registration["frozenAt"]) >= started or completed < started:
        raise ValueError("source freeze and capture times are inconsistent")
    _, actors = verify_retained_protocol(run.get("protocolBinding"), artifacts, code_root)
    if plan.get("actorSha256") != actors or run.get("arms") != registration.get("arms") or plan.get("arms") != run.get("arms"):
        raise ValueError("frozen plan actors or cost arms differ")
    expected = [(task["id"], tool, arm) for task in sorted(registration["tasks"], key=lambda task: task["id"])
                for arm in old.ARMS for tool in old.TOOLS]
    captured, selections, labels = run.get("observations"), manifest.get("normalizedSelections"), reviews.get("observations")
    for rows, where in ((captured, "run"), (selections, "normalization selections"), (labels, "judgment index")):
        _ordered(rows, expected, where)
    rows = []
    for key, observed, selection, index in zip(expected, captured, selections, labels):
        if selection.keys() != SELECTION_KEYS:
            raise ValueError("selection must have exactly one initial and one reviewed reference")
        initial_ref = _reference(selection["initialNormalizedRef"])
        _reference(selection["reviewedNormalizedRef"])
        if initial_ref != _reference(observed.get("normalized")):
            raise ValueError("initial normalization reference differs from immutable original run")
        captures = observed.get("captures")
        if type(captures) is not list or len(captures) != 1:
            raise ValueError("protocol2 requires one captured query per frozen observation")
        _reference(captures[0])
        if index.keys() != {"task", "tool", "arm", "judgmentRef"}:
            raise ValueError("judgment index must reference a single bounded observation document")
        _reference(index["judgmentRef"])
        bindings = dict(originalRunSha256=run_ref["sha256"],
                        sourceRegistrationSha256=registration_ref["sha256"], actorSha256=actors)
        old._digest(selection["rawStdoutSha256"])
        if any(selection[name] != value for name, value in bindings.items()):
            raise ValueError("selection differs from original run/source/actor bindings")
        rows.append((key, observed, selection, index))
    return ReviewLineage(registration, run, reviews, tuple(rows), artifacts, code_root,
                         registration_ref["sha256"], run_ref["sha256"], actors)
