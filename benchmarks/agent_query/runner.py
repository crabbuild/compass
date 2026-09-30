#!/usr/bin/env python3
"""Agent-facing query evaluation for Compass against Graphify.

The runner builds one graph per tool for each pinned repository in
``suite.toml``, executes the suite's exact per-tool argument vectors, judges
the bounded stdout against the suite's source-reviewed anchors, and reports
correctness, latency, and token cost. Token cost is the number of UTF-8 bytes
divided by ``TOKEN_BYTES``; both CLIs budget text with the same four-bytes-per-
token approximation, so the estimate is comparable across tools and is
documented in the report.

This is developer-side tooling. It is never a Compass runtime dependency and
it never installs Graphify itself: pass the binary to use with
``--graphify-binary``.
"""

from __future__ import annotations

import argparse
import hashlib
from dataclasses import dataclass
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time
import tomllib

SUITE_SCHEMA = "compass.agent-query-suite/1"
RUN_SCHEMA = "compass.agent-query-run/1"
TOKEN_BYTES = 4
MAX_OUTPUT_BYTES = 16 * 1024 * 1024
DEFAULT_TIMEOUT_SECONDS = 60.0

_COMMIT = re.compile(r"^[0-9a-f]{40}$")
_SHA256 = re.compile(r"sha256:[0-9a-f]{64}")
_CURSOR = re.compile(r"next=([^\s]+)")
_GRAPHIFY_NODE = re.compile(r"^NODE (.+?) \[src=(\S+) loc=L(\d+)", re.MULTILINE)
_GRAPHIFY_CANDIDATE = re.compile(r"^\s+id: (\S+)", re.MULTILINE)
_COMPASS_ENTITY = re.compile(r"^- (\S+) \[[a-z_]+\] \S+:\d", re.MULTILINE)

KINDS = {
    "explain",
    "explain_source",
    "brief",
    "callers",
    "callees",
    "brief_callers",
    "paged_callers",
    "impact",
    "path",
    "file_path",
    "ambiguity",
    "negative",
    "broad",
}
EXPECTATIONS = {"answer", "pick_list", "no_match"}

_REPOSITORY_KEYS = {"name", "language", "url", "commit", "question", "anchor"}
_QUESTION_KEYS = {
    "id",
    "kind",
    "subject",
    "compass",
    "graphify",
    "expect",
    "required",
    "required_one_of",
    "min_one_of",
    "min_candidates",
    "forbidden",
    "budget_tokens",
    "max_follow_ups",
    "judgment",
}
_ANCHOR_KEYS = {"file", "line", "symbol", "judgment"}


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def estimate_tokens(byte_count: int) -> int:
    """Approximate tokens for a UTF-8 byte count with the shared 4-byte rule."""
    return math.ceil(byte_count / TOKEN_BYTES)


def _unknown(record: dict, allowed: set, where: str) -> None:
    unknown = set(record) - allowed
    if unknown:
        raise ValueError(f"{where} has unknown keys: {sorted(unknown)}")


def _strings(value: object, where: str, *, allow_empty: bool = True) -> tuple[str, ...]:
    if not isinstance(value, list):
        raise ValueError(f"{where} must be a list of strings")
    result = tuple(value)
    if not all(isinstance(item, str) and item for item in result):
        raise ValueError(f"{where} must be a list of non-empty strings")
    if not allow_empty and not result:
        raise ValueError(f"{where} must not be empty")
    return result


@dataclass(frozen=True)
class Anchor:
    file: str
    line: int
    symbol: str
    judgment: str


@dataclass(frozen=True)
class Question:
    identifier: str
    kind: str
    subject: str
    compass: tuple[str, ...]
    graphify: tuple[str, ...]
    expect: str
    required: tuple[str, ...]
    required_one_of: tuple[str, ...]
    min_one_of: int
    min_candidates: int
    forbidden: tuple[str, ...]
    budget_tokens: int
    max_follow_ups: int
    judgment: str


@dataclass(frozen=True)
class Repository:
    name: str
    language: str
    url: str
    commit: str
    questions: tuple[Question, ...]
    anchors: tuple[Anchor, ...]


@dataclass(frozen=True)
class Suite:
    path: Path
    digest: str
    repositories: tuple[Repository, ...]

    def repository(self, name: str) -> Repository:
        for repository in self.repositories:
            if repository.name == name:
                return repository
        raise KeyError(name)


def load_suite(path: Path) -> Suite:
    raw = path.read_bytes()
    document = tomllib.loads(raw.decode("utf-8"))
    _unknown(document, {"schema", "repository"}, str(path))
    if document.get("schema") != SUITE_SCHEMA:
        raise ValueError(f"{path} has unsupported schema {document.get('schema')!r}")
    repository_records = document.get("repository")
    if not isinstance(repository_records, list) or not repository_records:
        raise ValueError(f"{path} must declare at least one repository")
    repositories: list[Repository] = []
    seen_repositories: set[str] = set()
    seen_questions: set[str] = set()
    for repository_index, record in enumerate(repository_records):
        where = f"repository[{repository_index}]"
        if not isinstance(record, dict):
            raise ValueError(f"{where} must be a table")
        _unknown(record, _REPOSITORY_KEYS, where)
        name = record.get("name")
        if not isinstance(name, str) or not name:
            raise ValueError(f"{where} needs a name")
        if name in seen_repositories:
            raise ValueError(f"duplicate repository {name!r}")
        seen_repositories.add(name)
        language = record.get("language")
        if not isinstance(language, str) or not language:
            raise ValueError(f"{where} needs a language")
        url = record.get("url")
        if not isinstance(url, str) or not url:
            raise ValueError(f"{where} needs a url")
        commit = record.get("commit")
        if not isinstance(commit, str) or _COMMIT.fullmatch(commit) is None:
            raise ValueError(f"{where} needs a 40-character lowercase commit")
        question_records = record.get("question")
        if not isinstance(question_records, list) or not question_records:
            raise ValueError(f"{where} must declare at least one question")
        questions: list[Question] = []
        for question_index, question in enumerate(question_records):
            question_where = f"{where} question[{question_index}]"
            if not isinstance(question, dict):
                raise ValueError(f"{question_where} must be a table")
            _unknown(question, _QUESTION_KEYS, question_where)
            identifier = question.get("id")
            if not isinstance(identifier, str) or not identifier:
                raise ValueError(f"{question_where} needs an id")
            if identifier in seen_questions:
                raise ValueError(f"duplicate question id {identifier!r}")
            seen_questions.add(identifier)
            kind = question.get("kind")
            if kind not in KINDS:
                raise ValueError(f"{question_where} has invalid kind {kind!r}")
            subject = question.get("subject")
            if not isinstance(subject, str) or not subject:
                raise ValueError(f"{question_where} needs a subject")
            compass = _strings(question.get("compass"), f"{question_where} compass", allow_empty=False)
            graphify = _strings(
                question.get("graphify"), f"{question_where} graphify", allow_empty=False
            )
            expect = question.get("expect")
            if expect not in EXPECTATIONS:
                raise ValueError(f"{question_where} has invalid expect {expect!r}")
            required = _strings(question.get("required", []), f"{question_where} required")
            required_one_of = _strings(
                question.get("required_one_of", []), f"{question_where} required_one_of"
            )
            min_one_of = question.get("min_one_of", 0)
            if not isinstance(min_one_of, int) or min_one_of < 0:
                raise ValueError(f"{question_where} has invalid min_one_of")
            min_candidates = question.get("min_candidates", 0)
            if not isinstance(min_candidates, int) or min_candidates < 0:
                raise ValueError(f"{question_where} has invalid min_candidates")
            forbidden = _strings(question.get("forbidden", []), f"{question_where} forbidden")
            budget_tokens = question.get("budget_tokens", 0)
            if not isinstance(budget_tokens, int) or budget_tokens < 0:
                raise ValueError(f"{question_where} has invalid budget_tokens")
            max_follow_ups = question.get("max_follow_ups", 0)
            if not isinstance(max_follow_ups, int) or max_follow_ups < 0:
                raise ValueError(f"{question_where} has invalid max_follow_ups")
            judgment = question.get("judgment")
            if not isinstance(judgment, str) or not judgment:
                raise ValueError(f"{question_where} needs a judgment reason")
            if expect == "answer" and not required and not required_one_of:
                raise ValueError(f"{question_where} needs required anchors for an answer oracle")
            if expect == "pick_list" and min_one_of <= 0:
                raise ValueError(f"{question_where} needs min_one_of for a pick list oracle")
            if kind == "broad" and budget_tokens <= 0:
                raise ValueError(f"{question_where} needs budget_tokens for a broad question")
            questions.append(
                Question(
                    identifier=identifier,
                    kind=kind,
                    subject=subject,
                    compass=compass,
                    graphify=graphify,
                    expect=expect,
                    required=required,
                    required_one_of=required_one_of,
                    min_one_of=min_one_of,
                    min_candidates=min_candidates,
                    forbidden=forbidden,
                    budget_tokens=budget_tokens,
                    max_follow_ups=max_follow_ups,
                    judgment=judgment,
                )
            )
        anchor_records = record.get("anchor", [])
        if not isinstance(anchor_records, list) or not anchor_records:
            raise ValueError(f"{where} must declare at least one graph anchor")
        anchors: list[Anchor] = []
        for anchor_index, anchor in enumerate(anchor_records):
            anchor_where = f"{where} anchor[{anchor_index}]"
            if not isinstance(anchor, dict):
                raise ValueError(f"{anchor_where} must be a table")
            _unknown(anchor, _ANCHOR_KEYS, anchor_where)
            file = anchor.get("file")
            if not isinstance(file, str) or not file:
                raise ValueError(f"{anchor_where} needs a file")
            line = anchor.get("line")
            if not isinstance(line, int) or line <= 0:
                raise ValueError(f"{anchor_where} needs a positive line")
            symbol = anchor.get("symbol")
            if not isinstance(symbol, str) or not symbol:
                raise ValueError(f"{anchor_where} needs a symbol")
            anchor_judgment = anchor.get("judgment")
            if not isinstance(anchor_judgment, str) or not anchor_judgment:
                raise ValueError(f"{anchor_where} needs a judgment reason")
            anchors.append(Anchor(file=file, line=line, symbol=symbol, judgment=anchor_judgment))
        repositories.append(
            Repository(
                name=name,
                language=language,
                url=url,
                commit=commit,
                questions=tuple(questions),
                anchors=tuple(anchors),
            )
        )
    return Suite(
        path=path,
        digest=hashlib.sha256(raw).hexdigest(),
        repositories=tuple(repositories),
    )


@dataclass(frozen=True)
class CommandResult:
    argv: tuple[str, ...]
    exit_code: int
    timed_out: bool
    wall_ms: int
    stdout_bytes: int
    stderr_bytes: int
    stdout: str
    stderr: str


def run_bounded(
    argv: tuple[str, ...],
    *,
    cwd: Path,
    timeout_seconds: float,
    stdout_path: Path,
    stderr_path: Path,
) -> CommandResult:
    """Run one command with a hard timeout and on-disk bounded capture."""
    stdout_path.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    timed_out = False
    with stdout_path.open("wb") as stdout_stream, stderr_path.open("wb") as stderr_stream:
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            stdout=stdout_stream,
            stderr=stderr_stream,
            start_new_session=True,
        )
        try:
            process.wait(timeout=timeout_seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(process.pid, subprocess.signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
    wall_ms = int((time.monotonic() - started) * 1000)
    stdout_bytes = stdout_path.stat().st_size
    stderr_bytes = stderr_path.stat().st_size
    stdout = stdout_path.read_bytes()[:MAX_OUTPUT_BYTES].decode("utf-8", errors="replace")
    stderr = stderr_path.read_bytes()[:MAX_OUTPUT_BYTES].decode("utf-8", errors="replace")
    return CommandResult(
        argv=argv,
        exit_code=process.returncode,
        timed_out=timed_out,
        wall_ms=wall_ms,
        stdout_bytes=stdout_bytes,
        stderr_bytes=stderr_bytes,
        stdout=stdout,
        stderr=stderr,
    )


def _candidate_count(tool: str, text: str) -> int:
    if tool == "compass":
        # A pick list is the rendered entity lines: the stable identifier is
        # printed beside each candidate, and a page that lists candidates
        # without identifiers is still a pick list.
        identities = set(_SHA256.findall(text))
        if identities:
            return len(identities)
        return len(set(_COMPASS_ENTITY.findall(text)))
    labels = {match.group(1) for match in _GRAPHIFY_NODE.finditer(text)}
    if labels:
        return len(labels)
    # `graphify explain` answers a name that matches several nodes with an
    # ambiguity list, one `id:` line per candidate.
    candidates = set(_GRAPHIFY_CANDIDATE.findall(text))
    if candidates:
        return len(candidates)
    # `graphify affected` prints one "- <label> [relation] file:Lline" per node.
    return len({line.strip() for line in text.splitlines() if line.startswith("- ")})


def judge(question: Question, tool: str, text: str) -> tuple[bool, tuple[str, ...]]:
    """Return the oracle verdict and the unmet requirements for one response."""
    if question.expect == "no_match":
        if tool == "compass":
            passed = "no_match" in text
        else:
            # `graphify query` reports "No matching nodes found"; the documented
            # name-resolution command `explain` reports "No node matching ...".
            passed = "No matching nodes found" in text or "No node matching" in text
        return passed, () if passed else ("no-match signal",)
    if question.expect == "pick_list":
        hits = [value for value in question.required_one_of if value in text]
        failures: list[str] = []
        if len(hits) < question.min_one_of:
            failures.append(
                f"pick list {len(hits)}/{question.min_one_of} of {list(question.required_one_of)}"
            )
        if question.min_candidates:
            candidates = _candidate_count(tool, text)
            if candidates < question.min_candidates:
                failures.append(f"candidates {candidates}/{question.min_candidates}")
        return not failures, tuple(failures)
    missing = [value for value in question.required if value not in text]
    forbidden = [value for value in question.forbidden if value in text]
    failures = [f"missing {value!r}" for value in missing]
    if question.required_one_of and question.min_one_of > 0:
        hits = [value for value in question.required_one_of if value in text]
        if len(hits) < question.min_one_of:
            failures.append(
                f"at least {question.min_one_of} of {list(question.required_one_of)}"
            )
    failures.extend(f"forbidden {value!r}" for value in forbidden)
    failures = tuple(failures)
    return not failures, failures


@dataclass(frozen=True)
class Observation:
    repository: str
    question: str
    kind: str
    tool: str
    argv: tuple[str, ...]
    passed: bool
    failures: tuple[str, ...]
    first_page_pass: bool
    exit_code: int
    timed_out: bool
    wall_ms: int
    first_page_tokens: int
    total_tokens: int
    follow_ups: int
    stdout_bytes: int

    def as_record(self) -> dict:
        return {
            "repository": self.repository,
            "question": self.question,
            "kind": self.kind,
            "tool": self.tool,
            "argv": list(self.argv),
            "passed": self.passed,
            "failures": list(self.failures),
            "firstPagePass": self.first_page_pass,
            "exitCode": self.exit_code,
            "timedOut": self.timed_out,
            "wallMs": self.wall_ms,
            "firstPageTokens": self.first_page_tokens,
            "totalTokens": self.total_tokens,
            "followUps": self.follow_ups,
            "stdoutBytes": self.stdout_bytes,
        }


def _tool_argv(
    tool: str,
    question: Question,
    graph: Path,
    *,
    budget: int | None = None,
    cursor: str | None = None,
) -> tuple[str, ...]:
    base = list(question.compass if tool == "compass" else question.graphify)
    if tool == "compass":
        if question.kind in {"broad", "paged_callers"}:
            base.extend(["--text-budget", str(budget or question.budget_tokens)])
        if cursor is not None:
            base.extend(["--cursor", cursor])
    else:
        if question.kind == "broad":
            base.extend(["--budget", str(budget or question.budget_tokens)])
    if "--graph" not in base:
        base.extend(["--graph", str(graph)])
    return tuple(base)


def run_question(
    repository: Repository,
    question: Question,
    *,
    tool: str,
    binary: Path,
    graph: Path,
    cwd: Path,
    raw_dir: Path,
    timeout_seconds: float,
) -> Observation:
    budget = question.budget_tokens
    cursor: str | None = None
    first_page_pass = False
    first_page_tokens = 0
    total_tokens = 0
    follow_ups = 0
    argv: tuple[str, ...] = ()
    exit_code = 0
    timed_out = False
    wall_ms = 0
    stdout_bytes = 0
    failures: tuple[str, ...] = ("not executed",)
    passed = False
    pages: list[str] = []
    for attempt in range(question.max_follow_ups + 1):
        argv = (str(binary), *_tool_argv(tool, question, graph, budget=budget, cursor=cursor))
        stem = f"{question.identifier}.{tool}.{attempt}"
        result = run_bounded(
            argv,
            cwd=cwd,
            timeout_seconds=timeout_seconds,
            stdout_path=raw_dir / f"{stem}.stdout",
            stderr_path=raw_dir / f"{stem}.stderr",
        )
        exit_code = result.exit_code
        timed_out = timed_out or result.timed_out
        wall_ms += result.wall_ms
        stdout_bytes += result.stdout_bytes
        tokens = estimate_tokens(result.stdout_bytes)
        total_tokens += tokens
        pages.append(result.stdout)
        aggregate = "\n".join(pages)
        attempt_pass, attempt_failures = judge(question, tool, aggregate)
        if attempt == 0:
            first_page_tokens = tokens
            first_page_pass = attempt_pass
        if attempt_pass:
            passed = True
            failures = ()
            break
        failures = attempt_failures
        if attempt == question.max_follow_ups:
            break
        if result.timed_out or result.exit_code != 0:
            break
        if tool == "compass":
            match = _CURSOR.search(result.stdout)
            if match is None:
                break
            cursor = match.group(1)
        else:
            if question.kind != "broad":
                # Graphify documents a continuation for `query` only: it re-runs
                # with a larger `--budget`. `affected`, `explain` and `path` have
                # no continuation, so one response is its complete answer.
                break
            budget *= 4
        follow_ups += 1
    return Observation(
        repository=repository.name,
        question=question.identifier,
        kind=question.kind,
        tool=tool,
        argv=argv,
        passed=passed,
        failures=failures,
        first_page_pass=first_page_pass,
        exit_code=exit_code,
        timed_out=timed_out,
        wall_ms=wall_ms,
        first_page_tokens=first_page_tokens,
        total_tokens=total_tokens,
        follow_ups=follow_ups,
        stdout_bytes=stdout_bytes,
    )


def _compass_graph(artifact_root: Path) -> Path:
    output = artifact_root / "compass-out"
    pointer = output / "current-snapshot"
    snapshot = pointer.read_text(encoding="utf-8").strip()
    if snapshot.startswith("snapshot-") and Path(snapshot).name == snapshot:
        graph = output / "snapshots" / snapshot / "graph.json"
        if graph.is_file():
            return graph
    candidates = sorted(output.glob("snapshots/*/graph.json"))
    if not candidates:
        raise RuntimeError(f"no Compass graph under {output}")
    return candidates[-1]


def _graphify_graph(artifact_root: Path) -> Path:
    graph = artifact_root / "graphify-out" / "graph.json"
    if not graph.is_file():
        raise RuntimeError(f"no Graphify graph at {graph}")
    return graph


def prepare_repository(
    repository: Repository,
    source: Path,
    artifacts: Path,
    *,
    compass_binary: Path,
    graphify_binary: Path,
    force: bool,
    timeout_seconds: float,
) -> dict:
    root = artifacts / repository.name
    compass_root = root / "compass"
    graphify_root = root / "graphify"
    record: dict[str, object] = {"repository": repository.name, "source": str(source)}
    if force:
        for path in (compass_root, graphify_root):
            if path.exists():
                shutil.rmtree(path)
    if not (compass_root / "compass-out" / "current-snapshot").is_file():
        started = time.monotonic()
        result = run_bounded(
            (
                str(compass_binary),
                "extract",
                str(source),
                "--code-only",
                "--no-viz",
                "--store",
                "sqlite",
                "--out",
                str(compass_root),
            ),
            cwd=source,
            timeout_seconds=timeout_seconds,
            stdout_path=root / "compass-build.stdout",
            stderr_path=root / "compass-build.stderr",
        )
        record["compassBuildMs"] = int((time.monotonic() - started) * 1000)
        record["compassBuildExit"] = result.exit_code
        record["compassBuildReport"] = result.stdout.strip().splitlines()[-3:]
        if result.exit_code != 0:
            raise RuntimeError(f"Compass extract failed for {repository.name}: {result.stderr[-2000:]}")
    if not (graphify_root / "graphify-out" / "graph.json").is_file():
        started = time.monotonic()
        result = run_bounded(
            (
                str(graphify_binary),
                "extract",
                str(source),
                "--code-only",
                "--out",
                str(graphify_root),
            ),
            cwd=source,
            timeout_seconds=timeout_seconds,
            stdout_path=root / "graphify-build.stdout",
            stderr_path=root / "graphify-build.stderr",
        )
        record["graphifyBuildMs"] = int((time.monotonic() - started) * 1000)
        record["graphifyBuildExit"] = result.exit_code
        record["graphifyBuildReport"] = result.stdout.strip().splitlines()[-3:]
        if result.exit_code != 0:
            raise RuntimeError(f"Graphify extract failed for {repository.name}: {result.stderr[-2000:]}")
    compass_graph = _compass_graph(compass_root)
    graphify_graph = _graphify_graph(graphify_root)
    record["compassGraph"] = str(compass_graph)
    record["graphifyGraph"] = str(graphify_graph)
    record["compassGraphSha256"] = _sha256_file(compass_graph)
    record["graphifyGraphSha256"] = _sha256_file(graphify_graph)
    return record


def graph_metrics(repository: Repository, tool: str, graph: Path) -> dict:
    document = json.loads(graph.read_text(encoding="utf-8"))
    nodes = document.get("nodes", [])
    edges = document.get("edges") or document.get("links") or []
    if tool == "compass":
        identifiers = [node.get("id") for node in nodes]
        sourced = sum(1 for node in nodes if (node.get("source") or {}).get("file"))
        locations = [
            (
                (node.get("source") or {}).get("file"),
                (node.get("source") or {}).get("startLine"),
                (node.get("source") or {}).get("endLine"),
            )
            for node in nodes
        ]
    else:
        identifiers = [node.get("id") for node in nodes]
        sourced = sum(1 for node in nodes if node.get("source_file"))
        locations = []
        for node in nodes:
            file = node.get("source_file")
            location = node.get("source_location")
            line = None
            if isinstance(location, str) and location.startswith("L"):
                try:
                    line = int(location[1:])
                except ValueError:
                    line = None
            locations.append((file, line, line))
    known = {identifier for identifier in identifiers if isinstance(identifier, str)}
    dangling = 0
    for edge in edges:
        source = edge.get("source")
        target = edge.get("target")
        if source not in known or target not in known:
            dangling += 1
    duplicates = len(identifiers) - len(known)
    anchor_hits = 0
    for anchor in repository.anchors:
        for file, start, end in locations:
            if file != anchor.file or start is None:
                continue
            stop = end if isinstance(end, int) else start
            if isinstance(start, int) and start <= anchor.line <= stop:
                anchor_hits += 1
                break
    return {
        "tool": tool,
        "graph": str(graph),
        "nodes": len(nodes),
        "edges": len(edges),
        "sourceBackedNodes": sourced,
        "sourceBackedRatio": round(sourced / len(nodes), 4) if nodes else 0.0,
        "danglingEdges": dangling,
        "duplicateIds": duplicates,
        "anchorHits": anchor_hits,
        "anchorTotal": len(repository.anchors),
    }


def _median(values: list[int]) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return float(ordered[middle])
    return (ordered[middle - 1] + ordered[middle]) / 2


def render_report(run: dict) -> str:
    lines: list[str] = []
    lines.append("# Compass vs. Graphify agent query evaluation")
    lines.append("")
    lines.append(f"Run: `{run['runId']}`  ")
    lines.append(f"Suite: `{run['suiteDigest']}`  ")
    lines.append(f"Started: {run['startedAt']}")
    lines.append("")
    lines.append("## Tool revisions")
    lines.append("")
    for tool in run["tools"]:
        lines.append(
            f"- **{tool['name']}**: {tool['version']} (`{tool['binary']}`, sha256 `{tool['binarySha256'][:16]}`)"
        )
    lines.append("")
    lines.append("Token counts use the shared four-bytes-per-token approximation both CLIs")
    lines.append("document for their text budgets; `answer tokens` sums every response the")
    lines.append("reviewed workflow needed, including documented follow-up pages.")
    lines.append("")
    lines.append("## Graph quality")
    lines.append("")
    lines.append("| Repository | Language | Tool | Nodes | Edges | Source-backed | Dangling | Duplicate IDs | Reviewed anchors |")
    lines.append("| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |")
    for metrics in run["graphMetrics"]:
        lines.append(
            "| {repository} | {language} | {tool} | {nodes} | {edges} | {ratio:.1%} | {dangling} | {duplicates} | {hits}/{total} |".format(
                repository=metrics["repository"],
                language=metrics["language"],
                tool=metrics["tool"],
                nodes=metrics["nodes"],
                edges=metrics["edges"],
                ratio=metrics["sourceBackedRatio"],
                dangling=metrics["danglingEdges"],
                duplicates=metrics["duplicateIds"],
                hits=metrics["anchorHits"],
                total=metrics["anchorTotal"],
            )
        )
    lines.append("")
    lines.append("## Query results")
    lines.append("")
    lines.append("| Repository | Question | Kind | Compass | C tokens | G tokens | Compass ms | Graphify ms |")
    lines.append("| --- | --- | --- | --- | ---: | ---: | ---: | ---: |")
    index = {(o["repository"], o["question"], o["tool"]): o for o in run["observations"]}
    for question in run["questions"]:
        key = (question["repository"], question["question"])
        compass = index.get((*key, "compass"))
        graphify = index.get((*key, "graphify"))
        lines.append(
            "| {repo} | {question} | {kind} | {verdict} | {ct} | {gt} | {cms} | {gms} |".format(
                repo=key[0],
                question=key[1],
                kind=question["kind"],
                verdict=_verdict_pair(compass, graphify),
                ct=compass["totalTokens"] if compass else 0,
                gt=graphify["totalTokens"] if graphify else 0,
                cms=compass["wallMs"] if compass else 0,
                gms=graphify["wallMs"] if graphify else 0,
            )
        )
    lines.append("")
    lines.append("`P` passed the source-reviewed oracle, `F` failed, `T` timed out.")
    lines.append("")
    lines.append("## Questions both tools answered")
    lines.append("")
    lines.append(
        "Token medians above cover each tool's own passing rows, which prices different"
    )
    lines.append(
        "questions. The table below restricts the comparison to rows where the same"
    )
    lines.append("source-reviewed oracle passed for both tools.")
    lines.append("")
    lines.append("| Scope | Both answered | Compass only | Graphify only | Neither | Median Compass tokens | Median Graphify tokens |")
    lines.append("| --- | ---: | ---: | ---: | ---: | ---: | ---: |")
    for scope, entry in run["paired"].items():
        lines.append(
            f"| {scope} | {entry['bothPassed']} | {entry['compassOnly']} | "
            f"{entry['graphifyOnly']} | {entry['neither']} | "
            f"{entry['medianCompassTokens']:.0f} | {entry['medianGraphifyTokens']:.0f} |"
        )
    lines.append("")
    lines.append("## Aggregate")
    lines.append("")
    lines.append("| Scope | Tool | Passed | Pass rate | Median first-page tokens | Median answer tokens | Median wall ms |")
    lines.append("| --- | --- | ---: | ---: | ---: | ---: | ---: |")
    for scope, summary in run["summaries"].items():
        for tool in ("compass", "graphify"):
            entry = summary[tool]
            lines.append(
                f"| {scope} | {tool} | {entry['passed']}/{entry['questions']} | {entry['passRate']:.1%} | "
                f"{entry['medianFirstPageTokens']:.0f} | {entry['medianAnswerTokens']:.0f} | {entry['medianWallMs']:.0f} |"
            )
    lines.append("")
    lines.append("## Verdict")
    lines.append("")
    verdict = run["verdict"]
    lines.append(f"- Correctness (suite pass rate): {verdict['correctness']}")
    lines.append(f"- Token efficiency (median answer tokens): {verdict['tokens']}")
    lines.append(f"- Token efficiency (paired answers only): {verdict['pairedTokens']}")
    lines.append(f"- Oracle split: {verdict['split']}")
    lines.append(f"- Graph quality (source-backed ratio and reviewed anchors): {verdict['graphQuality']}")
    lines.append("")
    lines.append("## Limits")
    lines.append("")
    lines.append("- Anchor matching is a deterministic text-recall proxy over bounded stdout,")
    lines.append("  not an independent precision oracle.")
    lines.append("- `graphify` prints an installation warning on stderr; the report counts stdout")
    lines.append("  only and records stderr separately in `run.json`.")
    lines.append("- The suite is a focused sample of five repositories and does not estimate")
    lines.append("  population-wide accuracy.")
    lines.append("")
    return "\n".join(lines)


def _verdict_pair(compass: dict | None, graphify: dict | None) -> str:
    def mark(entry: dict | None) -> str:
        if entry is None:
            return "?"
        if entry["timedOut"]:
            return "T"
        return "P" if entry["passed"] else "F"

    return f"{mark(compass)}/{mark(graphify)}"


def _aggregate(run: dict) -> None:
    observations = run["observations"]
    summaries: dict[str, dict] = {}
    paired: dict[str, dict] = {}
    for scope in ("all", *sorted(KINDS)):
        subset = [
            observation
            for observation in observations
            if scope == "all" or observation["kind"] == scope
        ]
        if not subset:
            continue
        summaries[scope] = {
            tool: _summary_objects(subset, tool) for tool in ("compass", "graphify")
        }
        paired[scope] = _paired_summary(subset)
    run["summaries"] = summaries
    run["paired"] = paired
    verdict: dict[str, str] = {}
    all_scope = summaries["all"]
    compass, graphify = all_scope["compass"], all_scope["graphify"]
    if compass["passed"] > graphify["passed"]:
        verdict["correctness"] = (
            f"Compass leads ({compass['passed']}/{compass['questions']} vs "
            f"{graphify['passed']}/{graphify['questions']})"
        )
    elif compass["passed"] == graphify["passed"]:
        verdict["correctness"] = (
            f"Tie ({compass['passed']}/{compass['questions']} vs "
            f"{graphify['passed']}/{graphify['questions']})"
        )
    else:
        verdict["correctness"] = (
            f"Graphify leads ({graphify['passed']}/{graphify['questions']} vs "
            f"{compass['passed']}/{compass['questions']})"
        )
    if graphify["medianAnswerTokens"] and compass["medianAnswerTokens"]:
        ratio = graphify["medianAnswerTokens"] / compass["medianAnswerTokens"]
        if compass["medianAnswerTokens"] <= graphify["medianAnswerTokens"]:
            verdict["tokens"] = (
                f"Compass is cheaper per answered question "
                f"({compass['medianAnswerTokens']:.0f} vs {graphify['medianAnswerTokens']:.0f} tokens, "
                f"{ratio:.2f}x)"
            )
        else:
            verdict["tokens"] = (
                f"Graphify is cheaper per answered question "
                f"({graphify['medianAnswerTokens']:.0f} vs {compass['medianAnswerTokens']:.0f} tokens, "
                f"{1 / ratio:.2f}x)"
            )
    else:
        verdict["tokens"] = "Not comparable: one tool has no passing answers"
    both = paired["all"]
    if both["bothPassed"]:
        verdict["pairedTokens"] = (
            f"On the {both['bothPassed']} questions both tools answered, Compass costs "
            f"{both['medianCompassTokens']:.0f} tokens and Graphify "
            f"{both['medianGraphifyTokens']:.0f} tokens (median)"
        )
    else:
        verdict["pairedTokens"] = "No question was answered by both tools"
    verdict["split"] = (
        f"Both {both['bothPassed']}, Compass only {both['compassOnly']}, "
        f"Graphify only {both['graphifyOnly']}, neither {both['neither']}"
    )
    metrics = run["graphMetrics"]
    compass_anchors = sum(m["anchorHits"] for m in metrics if m["tool"] == "compass")
    graphify_anchors = sum(m["anchorHits"] for m in metrics if m["tool"] == "graphify")
    compass_sourced = sum(m["sourceBackedNodes"] for m in metrics if m["tool"] == "compass")
    compass_nodes = sum(m["nodes"] for m in metrics if m["tool"] == "compass")
    graphify_sourced = sum(m["sourceBackedNodes"] for m in metrics if m["tool"] == "graphify")
    graphify_nodes = sum(m["nodes"] for m in metrics if m["tool"] == "graphify")
    compass_ratio = compass_sourced / compass_nodes if compass_nodes else 0.0
    graphify_ratio = graphify_sourced / graphify_nodes if graphify_nodes else 0.0
    verdict["graphQuality"] = (
        f"Compass {compass_ratio:.1%} source-backed and {compass_anchors} reviewed anchors vs "
        f"Graphify {graphify_ratio:.1%} and {graphify_anchors}"
    )
    run["verdict"] = verdict


def _summary_objects(observations: list[dict], tool: str) -> dict:
    subset = [observation for observation in observations if observation["tool"] == tool]
    passed = [observation for observation in subset if observation["passed"]]
    return {
        "questions": len(subset),
        "passed": len(passed),
        "passRate": round(len(passed) / len(subset), 4) if subset else 0.0,
        "medianFirstPageTokens": _median([o["firstPageTokens"] for o in subset]),
        "medianAnswerTokens": _median([o["totalTokens"] for o in passed]),
        "medianWallMs": _median([o["wallMs"] for o in subset]),
    }


def _paired_summary(observations: list[dict]) -> dict:
    """Compare token cost only on questions the same tool-independent oracle passed.

    A per-tool median over each tool's own passing subset prices different
    questions, so a tool that passes cheap rows and fails expensive ones - or the
    reverse - is credited or charged for work the other tool never did. Pairing
    the observations keeps the comparison on identical, source-reviewed answers.
    """
    by_question: dict[tuple[str, str], dict[str, dict]] = {}
    for observation in observations:
        by_question.setdefault(
            (observation["repository"], observation["question"]), {}
        )[observation["tool"]] = observation
    both_passed = [
        entry for entry in by_question.values() if all(t in entry for t in ("compass", "graphify"))
    ]
    passed = [entry for entry in both_passed if entry["compass"]["passed"] and entry["graphify"]["passed"]]
    compass_only = 0
    graphify_only = 0
    neither = 0
    for entry in by_question.values():
        compass = entry.get("compass")
        graphify = entry.get("graphify")
        if compass is None or graphify is None:
            continue
        if compass["passed"] and not graphify["passed"]:
            compass_only += 1
        elif graphify["passed"] and not compass["passed"]:
            graphify_only += 1
        elif not compass["passed"] and not graphify["passed"]:
            neither += 1
    return {
        "questions": len(by_question),
        "bothPassed": len(passed),
        "compassOnly": compass_only,
        "graphifyOnly": graphify_only,
        "neither": neither,
        "medianCompassTokens": _median([entry["compass"]["totalTokens"] for entry in passed]),
        "medianGraphifyTokens": _median([entry["graphify"]["totalTokens"] for entry in passed]),
        "medianCompassWallMs": _median([entry["compass"]["wallMs"] for entry in passed]),
        "medianGraphifyWallMs": _median([entry["graphify"]["wallMs"] for entry in passed]),
    }


def _tool_identity(name: str, binary: Path) -> dict:
    try:
        result = subprocess.run(
            (str(binary), "--version"),
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=30,
        )
        lines = [line.strip() for line in (result.stdout or "").splitlines() if line.strip()]
        version_text = next(
            (line for line in lines if line.startswith(name)), lines[0] if lines else "unknown"
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        version_text = f"unavailable: {error}"
    return {
        "name": name,
        "binary": str(binary),
        "binarySha256": _sha256_file(binary),
        "version": version_text,
    }


def _parse_sources(values: list[str] | None) -> dict[str, Path]:
    sources: dict[str, Path] = {}
    for value in values or []:
        name, separator, path = value.partition("=")
        if not separator or not name or not path:
            raise SystemExit(f"--source must be NAME=PATH, got {value!r}")
        sources[name] = Path(path)
    return sources


def execute(args: argparse.Namespace) -> int:
    suite = load_suite(args.suite)
    repositories = [
        repository
        for repository in suite.repositories
        if not args.repository or repository.name in args.repository
    ]
    if not repositories:
        raise SystemExit("no repositories selected")
    sources = _parse_sources(args.source)
    workspace = args.workspace.resolve()
    artifacts = workspace / "artifacts"
    stamp = args.run_id or datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_root = workspace / "runs" / stamp
    raw_root = run_root / "raw"
    run_root.mkdir(parents=True, exist_ok=True)
    started = datetime.now(timezone.utc).isoformat()
    prepared: list[dict] = []
    metrics: list[dict] = []
    observations: list[Observation] = []
    for repository in repositories:
        source = sources.get(repository.name)
        if source is None and args.corpus_root is not None:
            source = args.corpus_root / repository.name
        if source is None or not source.is_dir():
            raise SystemExit(f"{repository.name}: pass --source {repository.name}=PATH or --corpus-root")
        head = subprocess.run(
            ("git", "rev-parse", "HEAD"),
            cwd=source,
            check=True,
            stdout=subprocess.PIPE,
            text=True,
        ).stdout.strip()
        if head != repository.commit:
            raise SystemExit(
                f"{repository.name}: checkout HEAD {head} does not match pinned {repository.commit}"
            )
        record = prepare_repository(
            repository,
            source,
            artifacts,
            compass_binary=args.compass_binary,
            graphify_binary=args.graphify_binary,
            force=args.force,
            timeout_seconds=args.build_timeout,
        )
        prepared.append(record)
        compass_graph = Path(record["compassGraph"])
        graphify_graph = Path(record["graphifyGraph"])
        for tool, graph in (("compass", compass_graph), ("graphify", graphify_graph)):
            entry = graph_metrics(repository, tool, graph)
            entry["repository"] = repository.name
            entry["language"] = repository.language
            metrics.append(entry)
        for question in repository.questions:
            for tool, binary, graph in (
                ("compass", args.compass_binary, compass_graph),
                ("graphify", args.graphify_binary, graphify_graph),
            ):
                observation = run_question(
                    repository,
                    question,
                    tool=tool,
                    binary=binary,
                    graph=graph,
                    cwd=source,
                    raw_dir=raw_root / repository.name,
                    timeout_seconds=args.query_timeout,
                )
                observations.append(observation)
                print(
                    f"{repository.name:8} {question.identifier:34} {tool:8} "
                    f"{'PASS' if observation.passed else 'FAIL':4} "
                    f"tokens={observation.total_tokens:6} ms={observation.wall_ms:6}"
                    + (f" {list(observation.failures)}" if not observation.passed else ""),
                    flush=True,
                )
    run = {
        "schema": RUN_SCHEMA,
        "runId": stamp,
        "startedAt": started,
        "completedAt": datetime.now(timezone.utc).isoformat(),
        "suiteDigest": suite.digest,
        "tools": [
            _tool_identity("compass", args.compass_binary),
            _tool_identity("graphify", args.graphify_binary),
        ],
        "repositories": prepared,
        "graphMetrics": metrics,
        "questions": [
            {
                "repository": repository.name,
                "question": question.identifier,
                "kind": question.kind,
                "subject": question.subject,
                "judgment": question.judgment,
            }
            for repository in repositories
            for question in repository.questions
        ],
        "observations": [observation.as_record() for observation in observations],
    }
    _aggregate(run)
    (run_root / "run.json").write_text(json.dumps(run, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (run_root / "REPORT.md").write_text(render_report(run), encoding="utf-8")
    print(f"wrote {run_root / 'run.json'}")
    print(f"wrote {run_root / 'REPORT.md'}")
    return 0


def doctor(args: argparse.Namespace) -> int:
    suite = load_suite(args.suite)
    print(f"suite: {suite.path} ({len(suite.repositories)} repositories, digest {suite.digest[:16]})")
    for name in ("compass", "graphify"):
        identity = _tool_identity(name, getattr(args, f"{name}_binary"))
        print(f"{name}: {identity['version']} ({identity['binary']})")
    sources = _parse_sources(args.source)
    failures = 0
    for repository in suite.repositories:
        source = sources.get(repository.name)
        if source is None and args.corpus_root is not None:
            source = args.corpus_root / repository.name
        if source is None or not source.is_dir():
            print(f"{repository.name}: missing source (pass --source {repository.name}=PATH)")
            failures += 1
            continue
        head = subprocess.run(
            ("git", "rev-parse", "HEAD"), cwd=source, check=True, stdout=subprocess.PIPE, text=True
        ).stdout.strip()
        status = "pinned" if head == repository.commit else f"HEAD {head} != {repository.commit}"
        print(f"{repository.name}: {source} ({status})")
        if head != repository.commit:
            failures += 1
    return 1 if failures else 0


def _common_parser() -> argparse.ArgumentParser:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument(
        "--suite",
        type=Path,
        default=Path(__file__).with_name("suite.toml"),
        help="evaluation suite (default: suite.toml next to this script)",
    )
    common.add_argument("--compass-binary", type=Path, default=Path("compass"))
    common.add_argument("--graphify-binary", type=Path, default=Path("graphify"))
    common.add_argument("--corpus-root", type=Path, help="root containing one directory per repository")
    common.add_argument(
        "--source",
        action="append",
        metavar="NAME=PATH",
        help="explicit checkout for one repository; repeatable",
    )
    return common


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    common = _common_parser()
    doctor_parser = subparsers.add_parser(
        "doctor", parents=[common], help="verify binaries and pinned checkouts"
    )
    doctor_parser.set_defaults(handler=doctor)
    run_parser = subparsers.add_parser(
        "run", parents=[common], help="build graphs and execute the suite"
    )
    run_parser.add_argument("--workspace", type=Path, required=True)
    run_parser.add_argument("--run-id")
    run_parser.add_argument("--repository", action="append")
    run_parser.add_argument("--force", action="store_true", help="rebuild both graphs")
    run_parser.add_argument("--build-timeout", type=float, default=1800.0)
    run_parser.add_argument("--query-timeout", type=float, default=DEFAULT_TIMEOUT_SECONDS)
    run_parser.set_defaults(handler=execute)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.handler(args)


if __name__ == "__main__":
    sys.exit(main())
