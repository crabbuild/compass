"""Bounded, local stdio MCP client for developer-side comparisons."""
from __future__ import annotations

import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import time

MAX_BYTES = 16 * 1024 * 1024
MAX_SESSION_BYTES = 64 * 1024 * 1024


class StdioMcp:
    def __init__(self, argv: list[str], cwd: Path, directory: Path,
                 timeout: float = 60, max_bytes: int = MAX_BYTES):
        self.argv, self.cwd, self.directory = argv, cwd, directory
        self.timeout, self.max_bytes = timeout, max_bytes
        self.sequence, self.total, self.errors = 0, 0, 0
        self.buffer = b""

    def __enter__(self):
        self.directory.mkdir(parents=True, exist_ok=False)
        self.process = subprocess.Popen(self.argv, cwd=self.cwd, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                        start_new_session=True)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ, "stdout")
        self.selector.register(self.process.stderr, selectors.EVENT_READ, "stderr")
        self.error_file = (self.directory / "stderr").open("wb")
        return self

    def __exit__(self, *_):
        # Bound cleanup even if a server fails its handshake or ignores EOF.
        if self.process.stdin:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=2)
        self.selector.close()
        self.error_file.close()
        self.process.stdout.close()
        self.process.stderr.close()

    def send(self, method: str, params: dict, *, notification: bool = False):
        self.sequence += 1
        request = {"jsonrpc": "2.0", "method": method, "params": params}
        if not notification:
            request["id"] = self.sequence
        data = json.dumps(request, ensure_ascii=False).encode() + b"\n"
        if len(data) > 4096:
            raise ValueError("MCP evaluation request exceeds 4096 bytes")
        (self.directory / f"{self.sequence:02}.request.json").write_bytes(data)
        self.process.stdin.write(data)
        self.process.stdin.flush()
        if notification:
            return None
        deadline = time.monotonic() + self.timeout
        received = 0
        notifications = 0
        with (self.directory / f"{self.sequence:02}.response.jsonl").open("wb") as capture:
            while True:
                if b"\n" in self.buffer:
                    line, self.buffer = self.buffer.split(b"\n", 1)
                    packet = json.loads(line)
                    if not isinstance(packet, dict) or packet.get("jsonrpc") != "2.0":
                        raise ValueError("invalid JSON-RPC response")
                    if "id" not in packet:
                        notifications += 1
                        if notifications > 128:
                            raise ValueError("MCP notification bound exceeded")
                        continue
                    if packet["id"] != self.sequence:
                        raise ValueError("MCP response ID does not match request")
                    return packet
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("MCP request deadline exceeded")
                for key, _ in self.selector.select(remaining):
                    chunk = os.read(key.fd, 65536)
                    if not chunk:
                        self.selector.unregister(key.fileobj)
                        if key.data == "stdout":
                            raise RuntimeError("MCP server closed stdout before response")
                        continue
                    self.total += len(chunk)
                    if self.total > MAX_SESSION_BYTES:
                        raise ValueError("MCP session output bound exceeded")
                    if key.data == "stderr":
                        self.errors += len(chunk)
                        self.error_file.write(chunk[:max(0, self.max_bytes - self.errors + len(chunk))])
                        if self.errors > self.max_bytes:
                            raise ValueError("MCP stderr bound exceeded")
                    else:
                        received += len(chunk)
                        capture.write(chunk[:max(0, self.max_bytes - received + len(chunk))])
                        if received > self.max_bytes:
                            raise ValueError("MCP response byte bound exceeded")
                        self.buffer += chunk

    def initialize(self):
        result = self.send("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                           "clientInfo": {"name": "compass-paired-evaluation", "version": "1"}})
        if "error" in result:
            raise ValueError(f"MCP initialization failed: {result['error']}")
        self.send("notifications/initialized", {}, notification=True)
        return result
