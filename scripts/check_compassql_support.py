#!/usr/bin/env python3
"""Verify the pinned CompassQL conformance corpus and documentation contract."""

from __future__ import annotations

import hashlib
from pathlib import Path
import re
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "tests" / "opencypher-tck"


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    manifest = tomllib.loads((CORPUS / "manifest.toml").read_text(encoding="utf-8"))
    if manifest.get("commit") != "677cbafabb8c3c5eed458fd3b1ec0daec8d67d23":
        fail("openCypher corpus commit is not the reviewed 2024.3 snapshot")
    for feature in manifest.get("feature", []):
        path = CORPUS / feature["path"]
        if not path.is_file():
            fail(f"missing vendored feature: {feature['path']}")
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != feature["sha256"]:
            fail(f"hash mismatch for {feature['path']}: {actual}")
        source = path.read_text(encoding="utf-8")
        scenario_ids = {
            int(value)
            for value in re.findall(r"^\s*Scenario(?: Outline)?: \[(\d+)\]", source, re.MULTILINE)
        }
        selected = set(feature.get("supported", []))
        if not selected or not selected <= scenario_ids:
            fail(f"invalid supported scenarios for {feature['path']}")
        rejected = {entry["id"] for entry in feature.get("rejected", [])}
        if not rejected <= scenario_ids or selected & rejected:
            fail(f"invalid rejected scenarios for {feature['path']}")
        if not feature.get("support"):
            fail(f"missing feature IDs for {feature['path']}")
    support = (ROOT / "docs" / "COMPASSQL_SUPPORT.md").read_text(encoding="utf-8")
    for token in ("read-only", "MATCH", "OPTIONAL MATCH", "UNION", "EXISTS"):
        if token not in support:
            fail(f"support matrix is missing {token!r}")
    for required in (CORPUS / "LICENSE", CORPUS / "NOTICE", ROOT / "THIRD_PARTY_NOTICES.md"):
        if not required.is_file():
            fail(f"missing license artifact: {required.relative_to(ROOT)}")
    print(f"CompassQL support corpus verified: {len(manifest['feature'])} feature files")


if __name__ == "__main__":
    main()
