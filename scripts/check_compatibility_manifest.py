#!/usr/bin/env python3
"""Validate Compass's machine-readable Graphify compatibility contract."""

from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys
import tomllib
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "compatibility.toml"
WORKFLOWS = (
    ROOT / ".github" / "workflows" / "compass-ci.yml",
    ROOT / ".github" / "workflows" / "compass-hardening.yml",
)
SHA = re.compile(r"^[0-9a-f]{40}$")
ALLOWED_DISPOSITIONS = {
    "compatible",
    "superseded",
    "intentional-divergence",
    "not-supported",
}


def validate_manifest(data: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if data.get("schema_version") != 1:
        errors.append("schema_version must be 1")

    oracle = data.get("oracle")
    if not isinstance(oracle, dict):
        errors.append("oracle must be a table")
        oracle = {}
    _validate_sha(errors, oracle.get("commit"), "oracle.commit")
    _validate_sha(errors, oracle.get("release_commit"), "oracle.release_commit")
    if oracle.get("repository") != "Graphify-Labs/graphify":
        errors.append("oracle.repository must be Graphify-Labs/graphify")
    if not oracle.get("release") or not oracle.get("lineage"):
        errors.append("oracle.release and oracle.lineage are required")
    if not oracle.get("python"):
        errors.append("oracle.python is required")

    upstream = data.get("upstream")
    main = upstream.get("main") if isinstance(upstream, dict) else None
    if not isinstance(main, dict):
        errors.append("upstream.main must be a table")
        main = {}
    _validate_sha(errors, main.get("commit"), "upstream.main.commit")
    _validate_sha(errors, main.get("merge_base"), "upstream.main.merge_base")
    if main.get("repository") != "Graphify-Labs/graphify":
        errors.append("upstream.main.repository must be Graphify-Labs/graphify")
    if main.get("role") != "capability-audit":
        errors.append("upstream.main.role must be capability-audit")
    for field in ("package_version", "lineage"):
        if not main.get(field):
            errors.append(f"upstream.main.{field} is required")

    _validate_unique_tables(errors, data, "normalization", "id")
    _validate_unique_tables(errors, data, "command_family", "name")
    _validate_unique_tables(errors, data, "capability_audit", "key")
    _validate_table_arrays(errors, data, "command_family", ("entry_points", "evidence"))

    normalizations = data.get("normalization", [])
    for index, normalization in enumerate(normalizations):
        if not isinstance(normalization, dict) or not normalization.get("description"):
            errors.append(f"normalization[{index}].description is required")

    audits = data.get("capability_audit", [])
    if not isinstance(audits, list) or not audits:
        errors.append("capability_audit must contain at least one entry")
    else:
        for index, audit in enumerate(audits):
            status = audit.get("status") if isinstance(audit, dict) else None
            if status not in ALLOWED_DISPOSITIONS:
                errors.append(
                    f"capability_audit[{index}].status has unknown disposition {status!r}"
                )
            if not isinstance(audit, dict) or not audit.get("evidence"):
                errors.append(f"capability_audit[{index}].evidence is required")

    required = data.get("required_evidence")
    if not isinstance(required, dict):
        errors.append("required_evidence must be a table")
    else:
        for field in ("test_targets", "benchmark_profiles"):
            values = required.get(field)
            if not isinstance(values, list) or not values:
                errors.append(f"required_evidence.{field} must be a non-empty array")

    environments = data.get("environment")
    if not isinstance(environments, list) or not environments:
        errors.append("environment must contain at least one entry")
    else:
        _validate_unique_tables(errors, data, "environment", "name")
        for index, environment in enumerate(environments):
            extras = environment.get("extras") if isinstance(environment, dict) else None
            if not isinstance(extras, list):
                errors.append(f"environment[{index}].extras must be an array")
    return errors


def validate_checkout_refs(text: str, oracle_commit: str, source: str) -> list[str]:
    errors: list[str] = []
    lines = text.splitlines()
    found = 0
    for index, line in enumerate(lines):
        if "repository: Graphify-Labs/graphify" not in line:
            continue
        found += 1
        ref = None
        for candidate in lines[index + 1 : index + 8]:
            match = re.match(r"\s*ref:\s*([^\s#]+)", candidate)
            if match:
                ref = match.group(1)
                break
        if ref is None:
            errors.append(f"{source}:{index + 1}: Graphify checkout has no nearby ref")
        elif ref != oracle_commit:
            errors.append(
                f"{source}:{index + 1}: Graphify checkout ref {ref!r} "
                f"does not match oracle.commit {oracle_commit}"
            )
    if found == 0:
        errors.append(f"{source}: no Graphify compatibility checkout found")
    return errors


def validate_repository(root: Path, data: dict[str, Any]) -> list[str]:
    errors = validate_manifest(data)
    oracle = data.get("oracle", {})
    oracle_commit = oracle.get("commit", "") if isinstance(oracle, dict) else ""
    release_commit = (
        oracle.get("release_commit", "") if isinstance(oracle, dict) else ""
    )
    main = data.get("upstream", {}).get("main", {})
    main_commit = main.get("commit", "") if isinstance(main, dict) else ""

    for workflow in (
        root / ".github" / "workflows" / "compass-ci.yml",
        root / ".github" / "workflows" / "compass-hardening.yml",
    ):
        if not workflow.is_file():
            errors.append(f"missing workflow: {workflow.relative_to(root)}")
            continue
        errors.extend(
            validate_checkout_refs(
                workflow.read_text(encoding="utf-8"),
                oracle_commit,
                str(workflow.relative_to(root)),
            )
        )

    ledger_path = root / "COMPATIBILITY.md"
    if not ledger_path.is_file():
        errors.append("missing COMPATIBILITY.md")
    else:
        ledger = ledger_path.read_text(encoding="utf-8")
        for token, label in (
            (oracle_commit, "oracle commit"),
            (release_commit, "oracle release commit"),
            (main_commit, "upstream main commit"),
            ("compatibility.toml", "authoritative manifest"),
        ):
            if token not in ledger:
                errors.append(f"COMPATIBILITY.md is missing {label}: {token!r}")
    return errors


def load_manifest(path: Path = MANIFEST) -> dict[str, Any]:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def _validate_sha(errors: list[str], value: Any, field: str) -> None:
    if not isinstance(value, str) or SHA.fullmatch(value) is None:
        errors.append(f"{field} must be a full lowercase 40-character Git SHA")


def _validate_unique_tables(
    errors: list[str], data: dict[str, Any], table: str, key: str
) -> None:
    values = data.get(table)
    if not isinstance(values, list) or not values:
        errors.append(f"{table} must contain at least one entry")
        return
    keys = [entry.get(key) for entry in values if isinstance(entry, dict)]
    if len(keys) != len(values) or any(not value for value in keys):
        errors.append(f"{table}.{key} is required for every entry")
    elif len(keys) != len(set(keys)):
        errors.append(f"{table}.{key} contains duplicates")


def _validate_table_arrays(
    errors: list[str],
    data: dict[str, Any],
    table: str,
    fields: tuple[str, ...],
) -> None:
    values = data.get(table, [])
    if not isinstance(values, list):
        return
    for index, entry in enumerate(values):
        for field in fields:
            items = entry.get(field) if isinstance(entry, dict) else None
            if not isinstance(items, list) or not items:
                errors.append(f"{table}[{index}].{field} must be a non-empty array")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="validate the Compass compatibility manifest and consumers"
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="validate without modifying files (the only supported mode)",
    )
    args = parser.parse_args()
    if not args.check:
        parser.error("--check is required")

    try:
        data = load_manifest()
    except (OSError, tomllib.TOMLDecodeError) as error:
        print(f"error: could not load {MANIFEST.relative_to(ROOT)}: {error}", file=sys.stderr)
        raise SystemExit(1) from error

    errors = validate_repository(ROOT, data)
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)

    oracle = data["oracle"]
    main_line = data["upstream"]["main"]
    print(
        "Compatibility manifest verified: "
        f"oracle {oracle['release']}@{oracle['commit']}; "
        f"main capability audit @{main_line['commit']}"
    )


if __name__ == "__main__":
    main()
