#!/usr/bin/env python3
"""Deterministic Python framework qualification harness.

Fixture mode freezes the established, explicitly unqualified baseline. Pinned
mode validates read-only corpus identity and source inventories but cannot make
a production claim until a reviewed ``compass.quality-audit/2`` ledger meets
the repository thresholds.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any

import python_framework_oracle


SCHEMA = "compass.python-framework-qualification/1"
ROOT = Path(__file__).resolve().parents[1]
FIXTURE_ROOT = ROOT / "fixtures" / "code-graph" / "routes" / "python"
EXPECTATIONS = ROOT / "tests" / "qualification" / "python-framework-expectations.json"
EXPECTED_GAPS = ROOT / "tests" / "qualification" / "python-framework-expected-gaps.json"
BASELINE = ROOT / "tests" / "qualification" / "python-framework-baseline.json"
REPOSITORIES = ROOT / "tests" / "qualification" / "python-framework-repositories.toml"
FRAMEWORK_EXPECTATIONS_SCHEMA = "compass.framework-evidence/1"
GAPS_SCHEMA = "compass.python-framework-expected-gaps/1"
BASELINE_SCHEMA = "compass.python-framework-baseline/1"
MOUNTED_CHECKOUT_ROOT = Path("/Volumes/Workspace/Github")
CHECKOUT_ROOT_ENV = "COMPASS_PYTHON_FRAMEWORK_CHECKOUT_ROOT"


class QualificationError(RuntimeError):
    """A deterministic qualification failure."""


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise QualificationError(f"cannot load {path}: {error}") from error
    if not isinstance(value, dict):
        raise QualificationError(f"{path} must contain one JSON object")
    return value


def validate_expectations(document: dict[str, Any], fixture_root: Path) -> dict[str, int]:
    if set(document) != {"schema", "corpusId", "records"}:
        raise QualificationError("framework expectation set has unknown or missing fields")
    if document["schema"] != FRAMEWORK_EXPECTATIONS_SCHEMA:
        raise QualificationError(f"unexpected framework expectation schema {document['schema']!r}")
    if not isinstance(document["corpusId"], str) or not document["corpusId"]:
        raise QualificationError("framework expectation corpusId must be non-empty")
    records = document["records"]
    if not isinstance(records, list) or not records:
        raise QualificationError("framework expectations must be a non-empty array")
    ids: set[str] = set()
    frameworks: dict[str, int] = {}
    for record in records:
        if not isinstance(record, dict):
            raise QualificationError("framework expectation must be an object")
        identity = record.get("id")
        if not isinstance(identity, str) or not identity or identity in ids:
            raise QualificationError(f"invalid or duplicate framework expectation ID {identity!r}")
        ids.add(identity)
        source_file = record.get("sourceFile")
        if not isinstance(source_file, str) or Path(source_file).is_absolute() or ".." in Path(source_file).parts:
            raise QualificationError(f"{identity}: sourceFile escapes the fixture root")
        path = fixture_root / source_file
        try:
            source = path.read_bytes()
        except OSError as error:
            raise QualificationError(f"{identity}: cannot read {source_file}: {error}") from error
        start = record.get("startByte")
        end = record.get("endByte")
        if not isinstance(start, int) or not isinstance(end, int) or not 0 <= start < end <= len(source):
            raise QualificationError(f"{identity}: invalid source byte range {start!r}:{end!r}")
        if not source[start:end].strip():
            raise QualificationError(f"{identity}: source byte range is empty whitespace")
        framework = record.get("framework")
        if not isinstance(framework, str) or not framework:
            raise QualificationError(f"{identity}: framework must be non-empty")
        frameworks[framework] = frameworks.get(framework, 0) + 1
    return dict(sorted(frameworks.items()))


def validate_gaps(document: dict[str, Any]) -> list[str]:
    if set(document) != {"schema", "status", "records"} or document["schema"] != GAPS_SCHEMA:
        raise QualificationError("unexpected Python framework gap-ledger contract")
    if document["status"] != "established-unqualified":
        raise QualificationError("Phase 0 gap ledger must remain established-unqualified")
    records = document["records"]
    if not isinstance(records, list) or not records:
        raise QualificationError("expected-gap ledger must be non-empty")
    ids = [record.get("id") for record in records if isinstance(record, dict)]
    if len(ids) != len(records) or any(not isinstance(item, str) or not item for item in ids):
        raise QualificationError("every expected gap must have one non-empty ID")
    if len(set(ids)) != len(ids):
        raise QualificationError("expected-gap IDs must be unique")
    return sorted(ids)


def validate_baseline(document: dict[str, Any], expectations: int, gaps: int) -> None:
    if document.get("schema") != BASELINE_SCHEMA:
        raise QualificationError(f"unexpected Python framework baseline schema {document.get('schema')!r}")
    if document.get("status") != "established-unqualified":
        raise QualificationError("Phase 0 baseline must remain established-unqualified")
    producer = document.get("pythonProducer", {})
    if producer != {"id": "compass.python", "version": 13, "qualification": "qualifying"}:
        raise QualificationError("Python producer baseline drifted")
    fixture = document.get("fixtureEvidence", {})
    if fixture.get("expectations") != expectations or fixture.get("expectedGaps") != gaps:
        raise QualificationError("baseline fixture counts disagree with checked-in ledgers")
    qualification = document.get("qualification", {})
    if qualification.get("eligibleForProductionClaim") is not False:
        raise QualificationError("the Phase 0 baseline cannot claim production qualification")


def fixture_report() -> dict[str, Any]:
    first_oracle = python_framework_oracle.build_inventory(FIXTURE_ROOT)
    second_oracle = python_framework_oracle.build_inventory(FIXTURE_ROOT)
    first_bytes = canonical_bytes(first_oracle)
    if first_bytes != canonical_bytes(second_oracle):
        raise QualificationError("Python framework source oracle is not byte deterministic")
    expectations = load_json(EXPECTATIONS)
    gaps = load_json(EXPECTED_GAPS)
    baseline = load_json(BASELINE)
    frameworks = validate_expectations(expectations, FIXTURE_ROOT)
    gap_ids = validate_gaps(gaps)
    validate_baseline(baseline, len(expectations["records"]), len(gap_ids))
    return {
        "schema": SCHEMA,
        "mode": "fixtures",
        "status": "established-unqualified",
        "productionQualified": False,
        "pythonProducer": baseline["pythonProducer"],
        "frameworks": frameworks,
        "expectations": len(expectations["records"]),
        "expectedGaps": gap_ids,
        "oracle": {
            "schema": first_oracle["schema"],
            "parser": first_oracle["parser"],
            "summary": first_oracle["summary"],
            "inventorySha256": first_oracle["inventorySha256"],
            "reportSha256": sha256(first_bytes),
        },
        "contracts": {
            "expectationsSha256": sha256(canonical_bytes(expectations)),
            "gapsSha256": sha256(canonical_bytes(gaps)),
            "baselineSha256": sha256(canonical_bytes(baseline)),
        },
    }


def checkout_root(environment: dict[str, str] | None = None) -> Path:
    values = os.environ if environment is None else environment
    configured = values.get(CHECKOUT_ROOT_ENV)
    candidate = MOUNTED_CHECKOUT_ROOT if configured is None else Path(configured)
    if not candidate.is_absolute():
        raise QualificationError(f"{CHECKOUT_ROOT_ENV} must be an absolute path")
    mounted_root = MOUNTED_CHECKOUT_ROOT.resolve()
    resolved = candidate.resolve()
    if resolved != mounted_root and mounted_root not in resolved.parents:
        raise QualificationError(
            f"{CHECKOUT_ROOT_ENV} must remain under {MOUNTED_CHECKOUT_ROOT}"
        )
    return resolved


def checkout_for(url: str, root: Path) -> Path:
    pieces = url.rstrip("/").split("/")
    if len(pieces) < 2:
        raise QualificationError(f"cannot infer mounted checkout for {url!r}")
    return root / pieces[-2] / pieces[-1].removesuffix(".git")


def verify_checkout(repository: dict[str, Any], root: Path) -> tuple[Path, str]:
    checkout = checkout_for(repository["url"], root)
    if not checkout.is_dir():
        raise QualificationError(f"missing pinned checkout {checkout}")
    revision = subprocess.run(
        ["git", "-C", str(checkout), "rev-parse", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
    )
    if revision.returncode or revision.stdout.strip() != repository["commit"]:
        raise QualificationError(f"{repository['name']} is not at pinned commit {repository['commit']}")
    status = subprocess.run(
        ["git", "-C", str(checkout), "status", "--porcelain=v1", "--untracked-files=all"],
        check=False,
        capture_output=True,
        text=True,
    )
    if status.returncode or status.stdout:
        raise QualificationError(f"{repository['name']} checkout is not clean")
    licenses = repository.get("license_files")
    if not isinstance(licenses, list) or not licenses or not any((checkout / item).is_file() for item in licenses):
        raise QualificationError(f"{repository['name']} has no reviewable declared license")
    return checkout, revision.stdout.strip()


def pinned_report(baseline_path: Path) -> dict[str, Any]:
    manifest = tomllib.loads(REPOSITORIES.read_text(encoding="utf-8"))
    if manifest.get("schema") != SCHEMA:
        raise QualificationError(f"unexpected repository manifest schema {manifest.get('schema')!r}")
    repositories = manifest.get("repository")
    if not isinstance(repositories, list) or not repositories:
        raise QualificationError("pinned repository manifest is empty")
    root = checkout_root()
    reports = []
    for repository in repositories:
        checkout, revision = verify_checkout(repository, root)
        inventory = python_framework_oracle.build_inventory(checkout)
        reports.append(
            {
                "name": repository["name"],
                "commit": revision,
                "inventorySha256": inventory["inventorySha256"],
                "summary": inventory["summary"],
            }
        )
    baseline = load_json(baseline_path)
    if baseline.get("schema") != BASELINE_SCHEMA:
        raise QualificationError("pinned baseline has an unsupported schema")
    return {
        "schema": SCHEMA,
        "mode": "pinned",
        "status": "source-inventory-only",
        "productionQualified": False,
        "repositories": reports,
        "reason": "Phase 0 has no independently adjudicated compass.quality-audit/2 ledger",
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--fixtures-only", action="store_true")
    mode.add_argument("--pinned", action="store_true")
    parser.add_argument("--baseline", type=Path, default=BASELINE)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args(argv)
    try:
        report = fixture_report() if arguments.fixtures_only else pinned_report(arguments.baseline)
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_bytes(canonical_bytes(report))
    except (OSError, QualificationError, python_framework_oracle.OracleError) as error:
        print(f"python framework qualification: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
