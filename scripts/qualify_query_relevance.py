#!/usr/bin/env python3
"""Run the deterministic native query-relevance qualification gate."""

from __future__ import annotations

import os
import subprocess
import sys


def main() -> int:
    target = os.environ.get("CARGO_TARGET_DIR")
    if not target:
        print("CARGO_TARGET_DIR must name this checkout's external target directory", file=sys.stderr)
        return 2
    corpus_check = subprocess.run(
        [sys.executable, "scripts/generate_query_relevance_corpus.py", "--check"],
        check=False,
    )
    if corpus_check.returncode != 0:
        return corpus_check.returncode
    review_self_test = subprocess.run(
        [sys.executable, "scripts/prepare_query_relevance_review.py", "--self-test"],
        check=False,
    )
    if review_self_test.returncode != 0:
        return review_self_test.returncode
    command = [
        "cargo",
        "test",
        "-p",
        "compass-query",
        "--test",
        "relevance_qualification",
        "--locked",
        "--quiet",
    ]
    return subprocess.run(command, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
