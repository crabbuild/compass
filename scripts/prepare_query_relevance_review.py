#!/usr/bin/env python3
"""Prepare bounded, redacted query-log candidates for human relevance review."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import sys
import tempfile
from pathlib import Path
from typing import Any

SCHEMA = "compass.query-review-candidates/1"
MAX_INPUT_BYTES = 16 * 1024 * 1024
MAX_LINES = 10_000
MAX_RECORD_BYTES = 64 * 1024
MAX_QUESTION_BYTES = 4_096
MAX_CANDIDATES = 256

URL = re.compile(r"(?i)\b(?:https?|ssh)://[^\s<>'\"]+")
EMAIL = re.compile(r"(?i)(?<![\w.+-])[\w.+-]+@[\w.-]+\.[a-z]{2,}")
WINDOWS_PATH = re.compile(r"(?i)(?<![\w])(?:[a-z]:\\|\\\\)[^\s<>'\"]+")
UNIX_PATH = re.compile(r"(?<![\w])/(?:[^\s<>'\"]+/)*[^\s<>'\"]+")
SECRET_ASSIGNMENT = re.compile(
    r"(?i)\b(api[_-]?key|access[_-]?token|token|authorization|password|secret)"
    r"\s*[:=]\s*[^\s,;]+"
)
JWT = re.compile(r"\beyJ[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b")
LONG_TOKEN = re.compile(r"\b(?:[A-Fa-f0-9]{32,}|[A-Za-z0-9_+/=-]{48,})\b")
CONTROL = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]")
WHITESPACE = re.compile(r"\s+")


class ReviewInputError(ValueError):
    """Raised when an untrusted query log violates the review contract."""


def redact_question(question: str) -> str:
    """Apply deterministic best-effort redaction without inventing a judgment."""
    redacted = CONTROL.sub(" ", question)
    redacted = URL.sub("<url>", redacted)
    redacted = EMAIL.sub("<email>", redacted)
    redacted = WINDOWS_PATH.sub("<path>", redacted)
    redacted = UNIX_PATH.sub("<path>", redacted)
    redacted = SECRET_ASSIGNMENT.sub(lambda match: f"{match.group(1)}=<redacted>", redacted)
    redacted = JWT.sub("<token>", redacted)
    redacted = LONG_TOKEN.sub("<token>", redacted)
    return WHITESPACE.sub(" ", redacted).strip()


def normalized_key(question: str) -> str:
    return WHITESPACE.sub(" ", question).strip().casefold()


def load_candidates(path: Path) -> dict[str, Any]:
    try:
        metadata = path.stat()
    except OSError as error:
        raise ReviewInputError(f"cannot inspect input log: {error}") from error
    if not stat.S_ISREG(metadata.st_mode):
        raise ReviewInputError("input log must be a regular file")
    if metadata.st_size > MAX_INPUT_BYTES:
        raise ReviewInputError(f"input log exceeds {MAX_INPUT_BYTES} bytes")

    digest = hashlib.sha256()
    candidates: dict[str, dict[str, Any]] = {}
    records_read = 0
    bytes_read = 0
    try:
        with path.open("rb") as source:
            for line_number, raw_line in enumerate(source, start=1):
                bytes_read += len(raw_line)
                if bytes_read > MAX_INPUT_BYTES:
                    raise ReviewInputError(f"input log exceeds {MAX_INPUT_BYTES} bytes")
                if line_number > MAX_LINES:
                    raise ReviewInputError(f"input log exceeds {MAX_LINES} lines")
                digest.update(raw_line)
                if len(raw_line) > MAX_RECORD_BYTES:
                    raise ReviewInputError(
                        f"line {line_number} exceeds {MAX_RECORD_BYTES} bytes"
                    )
                if not raw_line.strip():
                    continue
                records_read += 1
                try:
                    record = json.loads(raw_line)
                except (UnicodeDecodeError, json.JSONDecodeError) as error:
                    raise ReviewInputError(f"line {line_number} is not valid JSON") from error
                if not isinstance(record, dict):
                    raise ReviewInputError(f"line {line_number} must be a JSON object")
                question = record.get("question")
                if not isinstance(question, str):
                    raise ReviewInputError(
                        f"line {line_number} must contain a string question"
                    )
                if len(question.encode("utf-8")) > MAX_QUESTION_BYTES:
                    raise ReviewInputError(
                        f"line {line_number} question exceeds {MAX_QUESTION_BYTES} bytes"
                    )
                redacted = redact_question(question)
                if not redacted:
                    continue
                key = normalized_key(redacted)
                candidate = candidates.get(key)
                if candidate is None:
                    candidate_id = hashlib.sha256(key.encode("utf-8")).hexdigest()[:20]
                    candidates[key] = {
                        "id": f"review-{candidate_id}",
                        "question": redacted,
                        "occurrences": 1,
                        "reviewStatus": "needs_judgment",
                    }
                else:
                    candidate["occurrences"] += 1
                    candidate["question"] = min(candidate["question"], redacted)
    except OSError as error:
        raise ReviewInputError(f"cannot read input log: {error}") from error

    ordered = sorted(
        candidates.values(),
        key=lambda item: (-item["occurrences"], item["id"], item["question"]),
    )
    truncated = len(ordered) > MAX_CANDIDATES
    retained = ordered[:MAX_CANDIDATES]
    return {
        "schema": SCHEMA,
        "sourceDigest": f"sha256:{digest.hexdigest()}",
        "recordsRead": records_read,
        "distinctQuestions": len(ordered),
        "truncated": truncated,
        "candidates": retained,
    }


def write_atomic(path: Path, document: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = (json.dumps(document, indent=2, sort_keys=True) + "\n").encode("utf-8")
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def self_test() -> None:
    with tempfile.TemporaryDirectory() as directory:
        source = Path(directory) / "queries.jsonl"
        source.write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "question": "Find /Users/alice/private.rs token=super-secret-value"
                        }
                    ),
                    json.dumps(
                        {
                            "question": "  find /Users/bob/private.rs TOKEN=another-secret  "
                        }
                    ),
                    json.dumps(
                        {
                            "question": "Who calls billing@example.com via https://internal/x?"
                        }
                    ),
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        first = load_candidates(source)
        second = load_candidates(source)
        if first != second:
            raise AssertionError("review candidate output is not deterministic")
        serialized = json.dumps(first)
        for sensitive in ["/Users/", "super-secret", "another-secret", "billing@example.com"]:
            if sensitive in serialized:
                raise AssertionError(f"redaction retained sensitive fixture text: {sensitive}")
        if first["recordsRead"] != 3 or first["distinctQuestions"] != 2:
            raise AssertionError("review candidate deduplication changed")
        reordered = Path(directory) / "queries-reordered.jsonl"
        reordered.write_text(
            "\n".join(reversed(source.read_text(encoding="utf-8").splitlines())) + "\n",
            encoding="utf-8",
        )
        if first["candidates"] != load_candidates(reordered)["candidates"]:
            raise AssertionError("candidate selection depends on input record order")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, help="local JSONL query log")
    parser.add_argument("--output", type=Path, help="redacted review-candidate JSON")
    parser.add_argument("--self-test", action="store_true", help="run deterministic unit checks")
    return parser.parse_args()


def main() -> int:
    arguments = parse_args()
    try:
        if arguments.self_test:
            if arguments.input is not None or arguments.output is not None:
                raise ReviewInputError("--self-test cannot be combined with --input or --output")
            self_test()
            return 0
        if arguments.input is None or arguments.output is None:
            raise ReviewInputError("--input and --output are required")
        if arguments.input.resolve() == arguments.output.resolve():
            raise ReviewInputError("input and output paths must differ")
        write_atomic(arguments.output, load_candidates(arguments.input))
        return 0
    except ReviewInputError as error:
        print(f"query review preparation failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
