#!/usr/bin/env python3
"""Validate the release decision for Compass universal evidence pipelines.

The promotion record is deliberately separate from extraction code.  This
keeps a release decision reviewable and makes it impossible for the runtime
registry, producer versions, or dialect aliases to silently drift from the
decision that promoted them.
"""

from __future__ import annotations

import json
from pathlib import Path
import sys
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "tests" / "qualification" / "universal-evidence-promotion.json"
SCHEMA = "compass.universal-evidence-promotion/1"
EVIDENCE_SCHEMA = "compass.languages.evidence/2"
SCOPE = "advertised-bounded-capabilities"
REVIEW = {
    "status": "approved",
    "method": "source-oracle-audits;deterministic-conformance;registry-parity",
    "reviewedAt": "2026-08-28",
}
MAX_MANIFEST_BYTES = 1024 * 1024

EXPECTED_PIPELINES = (
    ("compass.csharp", "csharp", 1),
    ("compass.dart", "dart", 1),
    ("compass.go", "go", 1),
    ("compass.groovy", "groovy", 1),
    ("compass.java", "java", 1),
    ("compass.javascript", "javascript", 1),
    ("compass.kotlin", "kotlin", 1),
    ("compass.php", "php", 1),
    ("compass.python", "python", 1),
    ("compass.ruby", "ruby", 1),
    ("compass.rust", "rust", 1),
    ("compass.scala", "scala", 1),
    ("compass.swift", "swift", 1),
    ("compass.typescript", "typescript", 1),
)

REQUIRED_GATES = {
    "minimumAcceptedRelationships": 2000,
    "minimumAcceptedPerCorpus": 400,
    "minimumAcceptedPerRelation": 100,
    "minimumAcceptedPerCapability": 100,
    "minimumObservedPrecision": 0.995,
    "minimumWilsonLowerBound": 0.99,
    "minimumCapabilityPrecision": 0.99,
    "minimumCapabilityRecall": 0.95,
    "maximumCriticalViolations": 0,
}

EXPECTED_DIALECTS = {
    "tsx": "typescript",
    "jsx": "javascript",
    "mts": "typescript",
    "cts": "typescript",
    "mjs": "javascript",
    "cjs": "javascript",
    "gradle": "groovy",
}


class PromotionError(ValueError):
    """The release promotion record is invalid or incomplete."""


def _require_string(document: dict[str, Any], field: str) -> str:
    value = document.get(field)
    if not isinstance(value, str) or not value.strip():
        raise PromotionError(f"{field} must be a non-empty string")
    return value


def validate(document: Any) -> dict[str, Any]:
    if not isinstance(document, dict):
        raise PromotionError("promotion record must be an object")
    if _require_string(document, "schema") != SCHEMA:
        raise PromotionError(f"schema must be {SCHEMA!r}")
    if _require_string(document, "decision") != "promote":
        raise PromotionError("decision must be 'promote'")
    _require_string(document, "decisionId")
    _require_string(document, "decisionDate")
    if _require_string(document, "scope") != SCOPE:
        raise PromotionError(f"scope must be {SCOPE!r}")
    if document.get("review") != REVIEW:
        raise PromotionError("review record does not show an approved release review")
    if _require_string(document, "evidenceSchema") != EVIDENCE_SCHEMA:
        raise PromotionError(f"evidenceSchema must be {EVIDENCE_SCHEMA!r}")

    gates = document.get("requiredGates")
    if gates != REQUIRED_GATES:
        raise PromotionError("requiredGates do not match the universal evidence release policy")

    dialects = document.get("dialects")
    if dialects != EXPECTED_DIALECTS:
        raise PromotionError("dialect aliases do not match the registry contract")

    pipelines = document.get("pipelines")
    if not isinstance(pipelines, list):
        raise PromotionError("pipelines must be an array")
    expected = list(EXPECTED_PIPELINES)
    actual: list[tuple[str, str, int]] = []
    for index, pipeline in enumerate(pipelines):
        if not isinstance(pipeline, dict):
            raise PromotionError(f"pipeline {index} must be an object")
        identifier = _require_string(pipeline, "id")
        language = _require_string(pipeline, "language")
        version = pipeline.get("producerVersion")
        if not isinstance(version, int) or isinstance(version, bool) or version <= 0:
            raise PromotionError(f"pipeline {identifier!r} has an invalid producerVersion")
        if pipeline.get("decision") != "qualified":
            raise PromotionError(f"pipeline {identifier!r} is not qualified")
        if pipeline.get("evidence") != "accepted":
            raise PromotionError(f"pipeline {identifier!r} has no accepted release evidence")
        actual.append((identifier, language, version))

    if actual != expected:
        raise PromotionError(
            "pipelines must contain exactly the sorted production registry entries: "
            f"expected {expected!r}, observed {actual!r}"
        )
    return document


def load(path: Path = MANIFEST) -> dict[str, Any]:
    try:
        with path.open("rb") as handle:
            raw = handle.read(MAX_MANIFEST_BYTES + 1)
    except OSError as error:
        raise PromotionError(f"cannot read promotion record {path}: {error}") from error
    if len(raw) > MAX_MANIFEST_BYTES:
        raise PromotionError("promotion record exceeds the 1 MiB bound")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise PromotionError(f"promotion record is not UTF-8: {error}") from error
    try:
        document = json.loads(text)
    except json.JSONDecodeError as error:
        raise PromotionError(f"promotion record is not valid JSON: {error}") from error
    return validate(document)


def main() -> int:
    try:
        document = load()
    except PromotionError as error:
        print(f"universal evidence promotion failed: {error}", file=sys.stderr)
        return 1
    print(
        "Universal evidence promotion verified: "
        f"{len(document['pipelines'])} pipelines qualified"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
