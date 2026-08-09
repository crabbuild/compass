#!/usr/bin/env python3
"""Run bounded discovery requests through one Compass MCP stdio session."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import selectors
import signal
import subprocess
import sys
import time

MAX_RESPONSE_BYTES = 20 * 1024 * 1024
MAX_SERVER_STDERR_BYTES = 1024 * 1024
MAX_QUESTIONS_BYTES = 1024 * 1024
RECORD_SCHEMA = "compass.performance.mcp-query-session-record/1"
SESSION_SCHEMA = "compass.performance.mcp-query-session/1"


class McpSession:
    def __init__(self, binary: Path, graph: Path, stderr_path: Path, timeout: float):
        self.timeout = timeout
        self.next_id = 1
        self.stderr_path = stderr_path
        self.stderr_buffer = bytearray()
        self.process = subprocess.Popen(
            [str(binary), "serve", str(graph), "--transport", "stdio"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        self.selector = selectors.DefaultSelector()
        self.buffer = bytearray()
        self.peak_rss_kib = 0
        if self.process.stdout is None:
            raise RuntimeError("MCP server has no stdout")
        os.set_blocking(self.process.stdout.fileno(), False)
        self.selector.register(self.process.stdout, selectors.EVENT_READ, "stdout")
        if self.process.stderr is None:
            raise RuntimeError("MCP server has no stderr")
        os.set_blocking(self.process.stderr.fileno(), False)
        self.selector.register(self.process.stderr, selectors.EVENT_READ, "stderr")
        try:
            self.request(
                "initialize",
                {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {"name": "compass-performance", "version": "1"},
                },
            )
            self.notify("notifications/initialized", {})
        except BaseException:
            self.close()
            raise

    def _write(self, value: dict[str, object]) -> None:
        if self.process.stdin is None:
            raise RuntimeError("MCP server has no stdin")
        self.process.stdin.write(
            json.dumps(value, separators=(",", ":")).encode("utf-8") + b"\n"
        )
        self.process.stdin.flush()

    def notify(self, method: str, params: dict[str, object]) -> None:
        self._write({"jsonrpc": "2.0", "method": method, "params": params})

    def request(self, method: str, params: dict[str, object]) -> dict[str, object]:
        request_id = self.next_id
        self.next_id += 1
        self._write(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        )
        deadline = time.monotonic() + self.timeout
        while True:
            line = self._read_line(deadline, request_id)
            response = json.loads(line)
            if not isinstance(response, dict):
                raise RuntimeError("MCP response must be a JSON object")
            if response.get("id") != request_id:
                continue
            if "error" in response:
                raise RuntimeError(f"MCP request failed: {response['error']}")
            result = response.get("result")
            if not isinstance(result, dict):
                raise RuntimeError("MCP response has no result object")
            return result

    def _read_line(self, deadline: float, request_id: int) -> bytes:
        while True:
            newline = self.buffer.find(b"\n")
            if newline >= 0:
                if newline > MAX_RESPONSE_BYTES:
                    raise RuntimeError("MCP response exceeded the 20 MiB harness bound")
                line = bytes(self.buffer[:newline])
                del self.buffer[: newline + 1]
                return line
            if len(self.buffer) > MAX_RESPONSE_BYTES:
                raise RuntimeError("MCP response exceeded the 20 MiB harness bound")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"MCP request {request_id} exceeded {self.timeout:g}s")
            events = self.selector.select(remaining)
            if not events:
                raise TimeoutError(f"MCP request {request_id} exceeded {self.timeout:g}s")
            for key, _mask in events:
                try:
                    chunk = os.read(key.fd, 64 * 1024)
                except BlockingIOError:
                    continue
                if key.data == "stderr":
                    if not chunk:
                        self.selector.unregister(key.fileobj)
                        continue
                    self.stderr_buffer.extend(chunk)
                    if len(self.stderr_buffer) > MAX_SERVER_STDERR_BYTES:
                        raise RuntimeError("MCP server stderr exceeded the 1 MiB harness bound")
                    continue
                if not chunk:
                    raise RuntimeError(
                        f"MCP server exited before response {request_id}: {self.process.poll()}"
                    )
                self.buffer.extend(chunk)

    def close(self) -> bool:
        forced = False
        try:
            if self.process.stdin is not None:
                self.process.stdin.close()
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            forced = True
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
                self.process.wait(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.process.wait()
        finally:
            self.selector.close()
            for stream in (self.process.stdout, self.process.stderr):
                if stream is not None:
                    stream.close()
            self.stderr_path.write_bytes(self.stderr_buffer)
            rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
            self.peak_rss_kib = int(rss / 1024) if sys.platform == "darwin" else int(rss)
        return forced


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--questions", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--server-stderr", type=Path, required=True)
    parser.add_argument("--batches", type=int, required=True)
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument("--allow-legacy-digest", action="store_true")
    args = parser.parse_args()
    question_bytes = args.questions.read_bytes()
    if len(question_bytes) > MAX_QUESTIONS_BYTES:
        raise ValueError("questions exceed the 1 MiB harness bound")
    questions = json.loads(question_bytes)
    if not isinstance(questions, list) or not all(
        isinstance(question, str) and question for question in questions
    ):
        raise ValueError("questions must be a nonempty-string JSON array")
    args.output.mkdir(parents=True, exist_ok=True)
    args.server_stderr.parent.mkdir(parents=True, exist_ok=True)
    session = McpSession(args.binary, args.graph, args.server_stderr, args.timeout_seconds)
    records: list[dict[str, object]] = []
    forced_termination = False
    try:
        for query_index, question in enumerate(questions, 1):
            for iteration in range(args.batches + 1):
                started = time.perf_counter()
                result = session.request(
                    "tools/call",
                    {"name": "query_graph", "arguments": {"question": question}},
                )
                elapsed = time.perf_counter() - started
                structured = result.get("structuredContent")
                if not isinstance(structured, dict):
                    raise RuntimeError("MCP tool response has no structuredContent")
                payload = structured.get("result")
                digest = structured.get("semanticResultDigest")
                if not isinstance(payload, dict):
                    raise RuntimeError("MCP discovery response omitted its typed result")
                if not isinstance(digest, str) and args.allow_legacy_digest:
                    canonical = json.dumps(
                        payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False
                    ).encode("utf-8")
                    digest = (
                        "legacy-python-full-payload:sha256:"
                        + hashlib.sha256(canonical).hexdigest()
                    )
                if not isinstance(digest, str):
                    raise RuntimeError("MCP discovery response omitted semanticResultDigest")
                payload["__semanticResultDigest"] = digest
                destination = args.output / f"query-{query_index}-{iteration}.json"
                destination.write_text(
                    json.dumps(payload, separators=(",", ":"), ensure_ascii=False) + "\n",
                    encoding="utf-8",
                )
                records.append(
                    {
                        "schema": RECORD_SCHEMA,
                        "query_index": query_index,
                        "iteration": iteration,
                        "wall_seconds": elapsed,
                        "output": str(destination.resolve()),
                    }
                )
    finally:
        forced_termination = session.close()
    if forced_termination:
        raise RuntimeError("MCP server required forced termination after stdin closed")
    for record in records:
        record["peak_rss_kib"] = session.peak_rss_kib
    print(
        json.dumps(
            {"schema": SESSION_SCHEMA, "records": records},
            sort_keys=True,
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
