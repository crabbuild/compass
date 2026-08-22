#!/usr/bin/env python3
"""Run Swift universal-evidence fixture, pinned, audit, or performance qualification."""

from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from qualify_universal_language import run_cli  # noqa: E402


ROOT = Path(__file__).resolve().parents[1]


if __name__ == "__main__":
    raise SystemExit(
        run_cli(
            sys.argv[1:],
            language="swift",
            manifest_path=ROOT / "tests/qualification/swift-universal-repositories.toml",
            oracle_path=ROOT / "scripts/swift_source_oracle.py",
            fixture_root=ROOT / "tests/qualification/language-wave/swift",
        )
    )
