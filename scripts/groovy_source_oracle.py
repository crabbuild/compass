#!/usr/bin/env python3
"""Qualification-only Groovy compiler CompilationUnit source oracle."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from independent_language_oracle import canonical_bytes, run_oracle_with_provider  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--include", action="append", default=[])
    parser.add_argument("--exclude", action="append", default=[])
    args = parser.parse_args()
    try:
        payload = run_oracle_with_provider(
            args.root,
            language="groovy",
            provider="groovy-compilation-unit-source-oracle",
            toolchain="Apache Groovy 4.0.27 CompilationUnit conversion phase (qualification contract)",
            implementation="bounded_lexical_scanner; Groovy CompilationUnit provider unavailable",
            suffixes=(".groovy", ".gradle"),
            include_globs=tuple(args.include),
            exclude_globs=tuple(args.exclude),
        )
        encoded = canonical_bytes(payload)
        if args.output:
            args.output.write_bytes(encoded)
        else:
            sys.stdout.buffer.write(encoded)
        return 0
    except (OSError, RuntimeError) as error:
        print(f"groovy source oracle failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
