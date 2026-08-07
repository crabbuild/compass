"""Machine-checkable TypeScript/JavaScript target-quality scorecards.

The scorecard is a qualification input, not a Compass runtime dependency. It
consumes explicitly adjudicated records produced from the independent
TypeScript checker oracle and the candidate evidence report. A record without
an explicit judgment is rejected; an automatic checker result is never
silently promoted to accepted precision evidence.
"""

from __future__ import annotations

from collections import Counter, defaultdict
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
from typing import Any

from .audit import (
    CAPABILITY_MINIMUM,
    CAPABILITY_PRECISION_GATE,
    CAPABILITY_RECALL_GATE,
    CORPUS_MINIMUM,
    CORRECT_ACCEPTED_JUDGMENTS,
    CRITICAL_JUDGMENTS,
    PRECISION_GATE,
    PRECISION_WILSON_LOWER_GATE,
    QUALIFICATION_MINIMUM,
    RECALL_RECOVERED_JUDGMENTS,
    RECALL_TRUTH_JUDGMENTS,
    RELATION_MINIMUM,
    TARGET_CLUSTER_MAXIMUM_FRACTION,
    wilson_interval,
)


SCORECARD_SCHEMA = "compass.typescript-target-scorecard/1"
RESULT_SCHEMA = "compass.typescript-target-scorecard-result/1"
PROVIDER = "typescript_checker_api_5_9_3"
MIN_RELEASE_CORPORA = 4
LEADERSHIP_PRECISION_GATE = 0.997
LEADERSHIP_WILSON_LOWER_GATE = 0.992
LEADERSHIP_CAPABILITY_PRECISION_GATE = 0.995
LEADERSHIP_RECALL_GATE = 0.97
HEX_40 = re.compile(r"^[0-9a-f]{40}$")
HEX_64 = re.compile(r"^[0-9a-f]{64}$")
IDENTITY = re.compile(r"^[a-z0-9][a-z0-9_.:/-]*$")
POOLS = frozenset(("accepted", "source_oracle"))
MODES = frozenset(("diagnostic", "qualification", "leadership"))
JUDGMENTS = frozenset(
    (
        "correct",
        "invalid",
        "ambiguous",
        "external",
        "represented_elsewhere",
        "missing",
        "fabricated_occurrence",
        "cross_language_match",
        "unsafe_local_substitution",
    )
)
ACCEPTED_JUDGMENTS = CORRECT_ACCEPTED_JUDGMENTS | frozenset(
    ("invalid", *CRITICAL_JUDGMENTS)
)
SOURCE_JUDGMENTS = RECALL_TRUTH_JUDGMENTS | frozenset(("ambiguous",))


class TypeScriptScorecardError(ValueError):
    """A scorecard is stale, incomplete, unsafe, or structurally invalid."""


@dataclass(frozen=True)
class ScorecardRecord:
    record_id: str
    corpus: str
    adapter: str
    framework_pack: str | None
    language: str
    capability: str
    relation: str
    pool: str
    target_cluster: str
    source_file: str
    start_byte: int
    end_byte: int
    judgment: str
    judgment_source: str


def _object(value: object, context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise TypeScriptScorecardError(f"{context} must be an object")
    return value


def _array(value: object, context: str) -> list[Any]:
    if not isinstance(value, list):
        raise TypeScriptScorecardError(f"{context} must be an array")
    return value


def _text(value: object, context: str, *, identity: bool = False) -> str:
    if not isinstance(value, str) or not value:
        raise TypeScriptScorecardError(f"{context} must be a non-empty string")
    if identity and IDENTITY.fullmatch(value) is None:
        raise TypeScriptScorecardError(f"{context} is not a stable identity: {value!r}")
    return value


def _hex(value: object, context: str, pattern: re.Pattern[str]) -> str:
    text = _text(value, context)
    if pattern.fullmatch(text) is None:
        raise TypeScriptScorecardError(f"{context} must be lowercase hexadecimal")
    return text


def _non_negative_int(value: object, context: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise TypeScriptScorecardError(f"{context} must be a non-negative integer")
    return value


def _safe_source_file(value: object, context: str) -> str:
    source_file = _text(value, context)
    relative = PurePosixPath(source_file.replace("\\", "/"))
    if relative.is_absolute() or ".." in relative.parts:
        raise TypeScriptScorecardError(f"{context} must be a safe relative path")
    return "/".join(relative.parts)


def _target_evidence(value: object, context: str) -> None:
    if value is None:
        return
    item = _object(value, context)
    allowed = {"kind", "file", "startByte", "endByte", "qualifiedName"}
    if set(item) - allowed:
        raise TypeScriptScorecardError(f"{context} has unknown fields")
    if "kind" in item:
        _text(item["kind"], f"{context}.kind", identity=True)
    if "file" in item:
        _safe_source_file(item["file"], f"{context}.file")
    if "startByte" in item:
        _non_negative_int(item["startByte"], f"{context}.startByte")
    if "endByte" in item:
        _non_negative_int(item["endByte"], f"{context}.endByte")
    if "qualifiedName" in item:
        _text(item["qualifiedName"], f"{context}.qualifiedName")


def _record(value: object, index: int) -> ScorecardRecord:
    context = f"records[{index}]"
    item = _object(value, context)
    required = {
        "id",
        "corpus",
        "adapter",
        "language",
        "capability",
        "relation",
        "pool",
        "targetCluster",
        "sourceFile",
        "startByte",
        "endByte",
        "judgment",
        "judgmentSource",
    }
    optional = {
        "automaticOutcome",
        "frameworkPack",
        "expectedTarget",
        "observedTargets",
        "oracleResolutionKind",
        "targetSpelling",
        "reason",
    }
    unknown = sorted(set(item) - required - optional)
    missing = sorted(required - set(item))
    if missing:
        raise TypeScriptScorecardError(
            f"{context} missing fields: {', '.join(missing)}"
        )
    if unknown:
        raise TypeScriptScorecardError(
            f"{context} has unknown fields: {', '.join(unknown)}"
        )
    if "expectedTarget" in item:
        _target_evidence(item["expectedTarget"], f"{context}.expectedTarget")
    if "observedTargets" in item:
        for target_index, target in enumerate(
            _array(item["observedTargets"], f"{context}.observedTargets")
        ):
            _target_evidence(target, f"{context}.observedTargets[{target_index}]")
    for field in ("oracleResolutionKind", "targetSpelling", "reason"):
        if field in item and not isinstance(item[field], str):
            raise TypeScriptScorecardError(f"{context}.{field} must be a string")
    if "automaticOutcome" in item:
        _text(item["automaticOutcome"], f"{context}.automaticOutcome", identity=True)
    pool = _text(item["pool"], f"{context}.pool", identity=True)
    if pool not in POOLS:
        raise TypeScriptScorecardError(f"{context}.pool is unknown: {pool!r}")
    judgment = _text(item["judgment"], f"{context}.judgment", identity=True)
    if judgment not in JUDGMENTS:
        raise TypeScriptScorecardError(f"{context}.judgment is unknown: {judgment!r}")
    allowed = ACCEPTED_JUDGMENTS if pool == "accepted" else SOURCE_JUDGMENTS
    if judgment not in allowed:
        raise TypeScriptScorecardError(
            f"{context}.judgment {judgment!r} is invalid for pool {pool!r}"
        )
    judgment_source = _text(
        item["judgmentSource"], f"{context}.judgmentSource", identity=True
    )
    if judgment_source != "manual":
        raise TypeScriptScorecardError(
            f"{context}.judgmentSource must be 'manual'; automatic outcomes "
            "cannot become scorecard judgments"
        )
    if judgment != "correct" and not item.get("reason"):
        raise TypeScriptScorecardError(
            f"{context}.reason is required for non-correct judgments"
        )
    start = _non_negative_int(item["startByte"], f"{context}.startByte")
    end = _non_negative_int(item["endByte"], f"{context}.endByte")
    if end <= start:
        raise TypeScriptScorecardError(f"{context} byte range must be non-empty")
    return ScorecardRecord(
        record_id=_text(item["id"], f"{context}.id", identity=True),
        corpus=_text(item["corpus"], f"{context}.corpus", identity=True),
        adapter=_text(item["adapter"], f"{context}.adapter", identity=True),
        framework_pack=(
            _text(item["frameworkPack"], f"{context}.frameworkPack", identity=True)
            if item.get("frameworkPack") is not None
            else None
        ),
        language=_text(item["language"], f"{context}.language", identity=True),
        capability=_text(item["capability"], f"{context}.capability", identity=True),
        relation=_text(item["relation"], f"{context}.relation", identity=True),
        pool=pool,
        target_cluster=_text(
            item["targetCluster"], f"{context}.targetCluster", identity=True
        ),
        source_file=_safe_source_file(item["sourceFile"], f"{context}.sourceFile"),
        start_byte=start,
        end_byte=end,
        judgment=judgment,
        judgment_source=judgment_source,
    )


def _corpus(value: object, index: int) -> tuple[str, str]:
    context = f"corpora[{index}]"
    item = _object(value, context)
    if set(item) != {"name", "commit"}:
        raise TypeScriptScorecardError(
            f"{context} must contain exactly name and commit"
        )
    return (
        _text(item["name"], f"{context}.name", identity=True),
        _hex(item["commit"], f"{context}.commit", HEX_40),
    )


def _capability(value: object, index: int) -> tuple[str, str, str | None]:
    context = f"advertisedCapabilities[{index}]"
    item = _object(value, context)
    allowed = {"adapter", "capability", "frameworkPack"}
    if set(item) - allowed or not {"adapter", "capability"} <= set(item):
        raise TypeScriptScorecardError(
            f"{context} must contain adapter, capability, and optional frameworkPack"
        )
    framework = item.get("frameworkPack")
    return (
        _text(item["adapter"], f"{context}.adapter", identity=True),
        _text(item["capability"], f"{context}.capability", identity=True),
        (
            _text(framework, f"{context}.frameworkPack", identity=True)
            if framework is not None
            else None
        ),
    )


def _comparator(value: object, index: int) -> tuple[str, bool, bool]:
    context = f"comparators[{index}]"
    item = _object(value, context)
    required = {"name", "version", "scopeDigest", "equivalentScope", "adjudicated"}
    if set(item) != required:
        raise TypeScriptScorecardError(
            f"{context} must contain name, version, scopeDigest, "
            "equivalentScope, and adjudicated"
        )
    name = _text(item["name"], f"{context}.name", identity=True)
    _text(item["version"], f"{context}.version")
    _hex(item["scopeDigest"], f"{context}.scopeDigest", HEX_64)
    if not isinstance(item["equivalentScope"], bool) or not isinstance(
        item["adjudicated"], bool
    ):
        raise TypeScriptScorecardError(
            f"{context}.equivalentScope and adjudicated must be booleans"
        )
    return name, item["equivalentScope"], item["adjudicated"]


def _load(path: Path) -> tuple[dict[str, Any], tuple[ScorecardRecord, ...]]:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise TypeScriptScorecardError(f"could not load scorecard {path}: {error}") from error
    item = _object(raw, "scorecard")
    required = {
        "schema",
        "mode",
        "provider",
        "oracleScriptSha256",
        "candidateAdapter",
        "corpora",
        "releaseGateCorpora",
        "advertisedCapabilities",
        "requiredRelations",
        "records",
        "comparators",
    }
    missing = sorted(required - set(item))
    unknown = sorted(set(item) - required)
    if missing:
        raise TypeScriptScorecardError(
            f"scorecard missing fields: {', '.join(missing)}"
        )
    if unknown:
        raise TypeScriptScorecardError(
            f"scorecard has unknown fields: {', '.join(unknown)}"
        )
    if item["schema"] != SCORECARD_SCHEMA:
        raise TypeScriptScorecardError(f"scorecard schema must be {SCORECARD_SCHEMA!r}")
    mode = _text(item["mode"], "scorecard.mode", identity=True)
    if mode not in MODES:
        raise TypeScriptScorecardError(f"scorecard.mode is unknown: {mode!r}")
    if item["provider"] != PROVIDER:
        raise TypeScriptScorecardError(
            f"scorecard.provider must be {PROVIDER!r}, got {item['provider']!r}"
        )
    _hex(item["oracleScriptSha256"], "scorecard.oracleScriptSha256", HEX_64)
    _text(item["candidateAdapter"], "scorecard.candidateAdapter", identity=True)
    corpora = tuple(
        _corpus(value, index)
        for index, value in enumerate(_array(item["corpora"], "scorecard.corpora"))
    )
    if not corpora or len({name for name, _ in corpora}) != len(corpora):
        raise TypeScriptScorecardError("scorecard.corpora must be non-empty and unique")
    corpus_names = {name for name, _ in corpora}
    release = tuple(
        _text(value, f"releaseGateCorpora[{index}]", identity=True)
        for index, value in enumerate(
            _array(item["releaseGateCorpora"], "scorecard.releaseGateCorpora")
        )
    )
    if len(set(release)) != len(release) or any(name not in corpus_names for name in release):
        raise TypeScriptScorecardError(
            "releaseGateCorpora must be unique and reference declared corpora"
        )
    capabilities = tuple(
        _capability(value, index)
        for index, value in enumerate(
            _array(item["advertisedCapabilities"], "scorecard.advertisedCapabilities")
        )
    )
    if not capabilities or len(set(capabilities)) != len(capabilities):
        raise TypeScriptScorecardError(
            "advertisedCapabilities must be non-empty and unique"
        )
    relations = tuple(
        _text(value, f"requiredRelations[{index}]", identity=True)
        for index, value in enumerate(
            _array(item["requiredRelations"], "scorecard.requiredRelations")
        )
    )
    if not relations or len(set(relations)) != len(relations):
        raise TypeScriptScorecardError(
            "requiredRelations must be non-empty and unique"
        )
    records = tuple(
        _record(value, index)
        for index, value in enumerate(_array(item["records"], "scorecard.records"))
    )
    record_ids = [record.record_id for record in records]
    if record_ids != sorted(record_ids) or len(record_ids) != len(set(record_ids)):
        raise TypeScriptScorecardError(
            "records must be sorted by unique id"
        )
    advertised = set(capabilities)
    for record in records:
        if record.corpus not in corpus_names:
            raise TypeScriptScorecardError(
                f"record {record.record_id!r} references unknown corpus"
            )
        if (record.adapter, record.capability, record.framework_pack) not in advertised:
            raise TypeScriptScorecardError(
                f"record {record.record_id!r} references unadvertised capability"
            )
        if record.relation not in relations:
            raise TypeScriptScorecardError(
                f"record {record.record_id!r} references undeclared relation"
            )
    comparators = tuple(
        _comparator(value, index)
        for index, value in enumerate(
            _array(item["comparators"], "scorecard.comparators")
        )
    )
    if len({name for name, _, _ in comparators}) != len(comparators):
        raise TypeScriptScorecardError("comparators must have unique names")
    return item, records


def _metric(numerator: int, denominator: int, *, interval: bool) -> dict[str, Any]:
    interval_value = wilson_interval(numerator, denominator) if interval else None
    return {
        "numerator": numerator,
        "denominator": denominator,
        "observed": numerator / denominator if denominator else None,
        "wilson95": (
            {"lower": interval_value.lower, "upper": interval_value.upper}
            if interval_value is not None
            else None
        ),
    }


def _contribution(record: ScorecardRecord) -> tuple[int, int, int, int]:
    accepted = record.pool == "accepted"
    source = record.pool == "source_oracle"
    return (
        int(accepted and record.judgment in CORRECT_ACCEPTED_JUDGMENTS),
        int(accepted),
        int(source and record.judgment in RECALL_RECOVERED_JUDGMENTS),
        int(source and record.judgment in RECALL_TRUTH_JUDGMENTS),
    )


def _strata(records: tuple[ScorecardRecord, ...]) -> dict[str, dict[str, dict[str, Any]]]:
    dimensions = {
        "corpus": lambda record: record.corpus,
        "adapter": lambda record: record.adapter,
        "frameworkPack": lambda record: record.framework_pack or "none",
        "language": lambda record: record.language,
        "relation": lambda record: record.relation,
        "capability": lambda record: record.capability,
        "targetCluster": lambda record: record.target_cluster,
    }
    result: dict[str, dict[str, dict[str, Any]]] = {}
    for dimension, key_for in dimensions.items():
        groups: dict[str, list[ScorecardRecord]] = defaultdict(list)
        for record in records:
            groups[key_for(record)].append(record)
        result[dimension] = {}
        for key in sorted(groups):
            contributions = [_contribution(record) for record in groups[key]]
            precision_numerator = sum(value[0] for value in contributions)
            precision_denominator = sum(value[1] for value in contributions)
            recall_numerator = sum(value[2] for value in contributions)
            recall_denominator = sum(value[3] for value in contributions)
            result[dimension][key] = {
                "records": len(groups[key]),
                "correctAccepted": precision_numerator,
                "auditedAccepted": precision_denominator,
                "precision": (
                    precision_numerator / precision_denominator
                    if precision_denominator
                    else None
                ),
                "recovered": recall_numerator,
                "recallCandidates": recall_denominator,
                "recall": (
                    recall_numerator / recall_denominator
                    if recall_denominator
                    else None
                ),
            }
    return result


def _failures(
    item: dict[str, Any],
    records: tuple[ScorecardRecord, ...],
    strata: dict[str, dict[str, dict[str, Any]]],
    precision: dict[str, Any],
) -> list[str]:
    diagnostic = item["mode"] == "diagnostic"
    leadership = item["mode"] == "leadership"
    failures: list[str] = []
    if not diagnostic and len(item["releaseGateCorpora"]) < MIN_RELEASE_CORPORA:
        failures.append(
            f"releaseGateCorpora has {len(item['releaseGateCorpora'])} entries; "
            f"{MIN_RELEASE_CORPORA} required"
        )
    if not diagnostic and precision["denominator"] < QUALIFICATION_MINIMUM:
        failures.append(
            f"accepted target sample has {precision['denominator']} records; "
            f"{QUALIFICATION_MINIMUM} required"
        )
    for corpus in item["releaseGateCorpora"] if not diagnostic else ():
        count = int(strata["corpus"].get(corpus, {}).get("auditedAccepted", 0))
        if count < CORPUS_MINIMUM:
            failures.append(
                f"corpus {corpus!r} has {count} accepted records; "
                f"{CORPUS_MINIMUM} required"
            )
    for relation in item["requiredRelations"] if not diagnostic else ():
        count = int(strata["relation"].get(relation, {}).get("auditedAccepted", 0))
        if count < RELATION_MINIMUM:
            failures.append(
                f"relation {relation!r} has {count} accepted records; "
                f"{RELATION_MINIMUM} required"
            )
    if not diagnostic:
        capability_precision_gate = (
            LEADERSHIP_CAPABILITY_PRECISION_GATE
            if leadership
            else CAPABILITY_PRECISION_GATE
        )
        recall_gate = LEADERSHIP_RECALL_GATE if leadership else CAPABILITY_RECALL_GATE
        for identity in item["advertisedCapabilities"]:
            adapter = identity["adapter"]
            capability = identity["capability"]
            framework = identity.get("frameworkPack")
            values = [
                record
                for record in records
                if record.adapter == adapter
                and record.framework_pack == framework
                and record.capability == capability
            ]
            accepted = sum(record.pool == "accepted" for record in values)
            if accepted < CAPABILITY_MINIMUM:
                failures.append(
                    f"capability {(adapter, framework, capability)!r} has {accepted} "
                    f"accepted records; {CAPABILITY_MINIMUM} required"
                )
            correct = sum(
                record.pool == "accepted"
                and record.judgment in CORRECT_ACCEPTED_JUDGMENTS
                for record in values
            )
            if accepted and correct / accepted < capability_precision_gate:
                failures.append(
                    f"capability {(adapter, framework, capability)!r} precision "
                    f"{correct / accepted:.6f} is below {capability_precision_gate:.3f}"
                )
            recall_values = [
                record for record in values if record.pool == "source_oracle"
            ]
            recall_denominator = sum(
                record.judgment in RECALL_TRUTH_JUDGMENTS for record in recall_values
            )
            recall_numerator = sum(
                record.judgment in RECALL_RECOVERED_JUDGMENTS
                for record in recall_values
            )
            if recall_denominator == 0:
                failures.append(
                    f"capability {(adapter, framework, capability)!r} has no "
                    "source-derived recall candidates"
                )
            elif recall_numerator / recall_denominator < recall_gate:
                failures.append(
                    f"capability {(adapter, framework, capability)!r} recall "
                    f"{recall_numerator / recall_denominator:.6f} is below {recall_gate:.3f}"
                )
        precision_gate = LEADERSHIP_PRECISION_GATE if leadership else PRECISION_GATE
        wilson_gate = (
            LEADERSHIP_WILSON_LOWER_GATE
            if leadership
            else PRECISION_WILSON_LOWER_GATE
        )
        if precision["observed"] is None or precision["observed"] < precision_gate:
            failures.append(
                f"overall precision {precision['observed']!r} is below {precision_gate:.3f}"
            )
        lower = (precision["wilson95"] or {}).get("lower")
        if lower is None or lower < wilson_gate:
            failures.append(
                f"Wilson 95% precision lower bound {lower!r} is below {wilson_gate:.3f}"
            )
        for dimension, groups in strata.items():
            if dimension not in {
                "corpus",
                "adapter",
                "frameworkPack",
                "language",
                "relation",
                "capability",
            }:
                continue
            for key in groups:
                accepted = [
                    record
                    for record in records
                    if record.pool == "accepted"
                    and {
                        "corpus": record.corpus,
                        "adapter": record.adapter,
                        "frameworkPack": record.framework_pack or "none",
                        "language": record.language,
                        "relation": record.relation,
                        "capability": record.capability,
                    }[dimension]
                    == key
                ]
                if not accepted:
                    continue
                clusters = Counter(record.target_cluster for record in accepted)
                cluster, count = max(
                    clusters.items(), key=lambda pair: (pair[1], pair[0])
                )
                if count / len(accepted) > TARGET_CLUSTER_MAXIMUM_FRACTION:
                    failures.append(
                        f"target cluster {cluster!r} supplies {count}/{len(accepted)} "
                        f"accepted records in {dimension} stratum {key!r}; maximum is 10%"
                    )
    if leadership:
        comparator_names = {
            comparator["name"]
            for comparator in item["comparators"]
            if comparator["equivalentScope"] and comparator["adjudicated"]
        }
        for required in ("graphify", "scip_typescript"):
            if required not in comparator_names:
                failures.append(
                    f"leadership comparator {required!r} lacks equivalent-scope "
                    "and adjudicated evidence"
                )
    critical = Counter(
        record.judgment for record in records if record.judgment in CRITICAL_JUDGMENTS
    )
    failures.extend(
        f"critical semantic violation {judgment!r}: {count}"
        for judgment, count in sorted(critical.items())
    )
    return sorted(set(failures))


def scorecard_result(path: Path) -> dict[str, Any]:
    """Validate a scorecard and return deterministic quality metrics."""

    item, records = _load(path)
    contributions = [_contribution(record) for record in records]
    precision = _metric(
        sum(value[0] for value in contributions),
        sum(value[1] for value in contributions),
        interval=True,
    )
    recall = _metric(
        sum(value[2] for value in contributions),
        sum(value[3] for value in contributions),
        interval=False,
    )
    strata = _strata(records)
    failures = _failures(item, records, strata, precision)
    judgments = Counter(record.judgment for record in records)
    canonical = json.dumps(item, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    scorecard_digest = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
    result = {
        "schema": RESULT_SCHEMA,
        "scorecardSchema": SCORECARD_SCHEMA,
        "scorecardSha256": scorecard_digest,
        "mode": item["mode"],
        "passed": not failures,
        "eligibleForQualityClaim": item["mode"] != "diagnostic" and not failures,
        "provider": item["provider"],
        "candidateAdapter": item["candidateAdapter"],
        "releaseGateCorpora": list(item["releaseGateCorpora"]),
        "auditedRecords": len(records),
        "precision": precision,
        "recall": recall,
        "judgments": {key: judgments[key] for key in sorted(judgments)},
        "strata": strata,
        "failures": failures,
    }
    return result


def write_scorecard_result(scorecard: Path, destination: Path) -> dict[str, Any]:
    """Evaluate and atomically write a JSON result."""

    result = scorecard_result(scorecard)
    destination.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(result, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    temporary = destination.with_name(f".{destination.name}.tmp")
    temporary.write_text(encoded + "\n", encoding="utf-8")
    temporary.replace(destination)
    return result


__all__ = [
    "RESULT_SCHEMA",
    "SCORECARD_SCHEMA",
    "TypeScriptScorecardError",
    "scorecard_result",
    "write_scorecard_result",
]
