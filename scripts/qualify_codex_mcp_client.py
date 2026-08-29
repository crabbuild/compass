#!/usr/bin/env python3
"""Exercise Compass through the installed Codex app-server MCP client."""

from __future__ import annotations

import argparse
import json
import os
import queue
import socket
import subprocess
import sys
import tempfile
import threading
import time
from collections import deque
from pathlib import Path
from typing import Any


class AppServerError(RuntimeError):
    pass


class AppServerClient:
    def __init__(self, codex: Path, env: dict[str, str], cwd: Path) -> None:
        self._next_id = 1
        self._messages: queue.Queue[dict[str, Any] | None] = queue.Queue()
        self._stderr: deque[str] = deque(maxlen=100)
        self._process = subprocess.Popen(
            [str(codex), "app-server"],
            cwd=cwd,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self._stdout_thread = threading.Thread(target=self._read_stdout, daemon=True)
        self._stderr_thread = threading.Thread(target=self._read_stderr, daemon=True)
        self._stdout_thread.start()
        self._stderr_thread.start()

    def _read_stdout(self) -> None:
        stdout = self._process.stdout
        if stdout is None:
            self._stderr.append("Codex app-server stdout pipe is unavailable")
            self._messages.put(None)
            return
        for line in stdout:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(message, dict):
                self._messages.put(message)
        self._messages.put(None)

    def _read_stderr(self) -> None:
        stderr = self._process.stderr
        if stderr is None:
            self._stderr.append("Codex app-server stderr pipe is unavailable")
            return
        for line in stderr:
            self._stderr.append(line.rstrip())

    def _send(self, message: dict[str, Any]) -> None:
        if self._process.poll() is not None:
            raise AppServerError(self.diagnostics("Codex app-server exited"))
        stdin = self._process.stdin
        if stdin is None:
            raise AppServerError(self.diagnostics("Codex app-server stdin pipe is unavailable"))
        stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        stdin.flush()

    def notify(self, method: str) -> None:
        self._send({"method": method})

    def request(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        request_id = self._next_id
        self._next_id += 1
        self._send({"id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + 30
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AppServerError(self.diagnostics(f"timeout waiting for {method}"))
            try:
                message = self._messages.get(timeout=remaining)
            except queue.Empty as error:
                raise AppServerError(self.diagnostics(f"timeout waiting for {method}")) from error
            if message is None:
                raise AppServerError(self.diagnostics(f"app-server closed during {method}"))
            if message.get("id") == request_id and "method" not in message:
                return message
            if "id" in message and isinstance(message.get("method"), str):
                self._send(
                    {
                        "id": message["id"],
                        "error": {"code": -32601, "message": "unsupported harness callback"},
                    }
                )

    def diagnostics(self, prefix: str) -> str:
        detail = "\n".join(self._stderr)
        return f"{prefix}: {detail[-4000:]}" if detail else prefix

    def close(self) -> None:
        if self._process.poll() is None:
            self._process.terminate()
            try:
                self._process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=3)


def object_result(response: dict[str, Any], method: str) -> dict[str, Any]:
    error = response.get("error")
    if error is not None:
        raise AppServerError(f"{method} failed: {json.dumps(error, sort_keys=True)}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise AppServerError(f"{method} returned no object result")
    return result


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def wait_for_server(process: subprocess.Popen[str], port: int) -> None:
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AppServerError("Compass HTTP server exited before becoming ready")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.25):
                return
        except OSError:
            time.sleep(0.1)
    raise AppServerError("Compass HTTP server did not become ready")


def run_checked(command: list[str], env: dict[str, str], cwd: Path) -> str:
    process = subprocess.Popen(
        command,
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    output = bytearray()

    def drain_output() -> None:
        stdout = process.stdout
        if stdout is None:
            return
        while chunk := stdout.read(8192):
            output.extend(chunk)
            if len(output) > 4000:
                del output[:-4000]

    reader = threading.Thread(target=drain_output, daemon=True)
    reader.start()
    try:
        returncode = process.wait(timeout=30)
    except subprocess.TimeoutExpired as error:
        process.kill()
        process.wait(timeout=3)
        reader.join(timeout=1)
        detail = output.decode(errors="replace").strip()
        suffix = f": {detail}" if detail else ""
        raise AppServerError(f"command timed out after 30 seconds{suffix}") from error
    reader.join(timeout=1)
    if returncode != 0:
        detail = output.decode(errors="replace").strip()
        raise AppServerError(f"command failed ({returncode}): {detail}")
    return output.decode(errors="replace").strip()


def run(args: argparse.Namespace) -> dict[str, str]:
    with tempfile.TemporaryDirectory(prefix="compass-codex-mcp-") as temp_value:
        temp = Path(temp_value)
        env = dict(os.environ)
        env["CODEX_HOME"] = str(temp / "codex-home")
        Path(env["CODEX_HOME"]).mkdir()
        version_output = run_checked([str(args.codex), "--version"], env, temp)
        expected_output = f"codex-cli {args.expected_version}"
        if version_output != expected_output:
            raise AppServerError(
                f"expected {expected_output!r}, got {version_output!r}"
            )
        run_checked(
            [str(args.codex), "features", "enable", "mcp_2026_07_28"],
            env,
            temp,
        )

        server: subprocess.Popen[str] | None = None
        server_log = None
        try:
            if args.transport == "stdio":
                registration = [
                    str(args.codex),
                    "mcp",
                    "add",
                    "compass",
                    "--",
                    str(args.compass),
                    "serve",
                    str(args.graph),
                    "--transport",
                    "stdio",
                ]
            else:
                port = free_port()
                server_log = (temp / "compass-http.log").open("w", encoding="utf-8")
                server = subprocess.Popen(
                    [
                        str(args.compass),
                        "serve",
                        str(args.graph),
                        "--transport",
                        "http",
                        "--host",
                        "127.0.0.1",
                        "--port",
                        str(port),
                        "--json-response",
                    ],
                    cwd=temp,
                    env=env,
                    stdout=subprocess.DEVNULL,
                    stderr=server_log,
                    text=True,
                )
                wait_for_server(server, port)
                registration = [
                    str(args.codex),
                    "mcp",
                    "add",
                    "compass",
                    "--url",
                    f"http://127.0.0.1:{port}/mcp",
                ]

            run_checked(registration, env, temp)
            client = AppServerClient(args.codex, env, temp)
            try:
                object_result(
                    client.request(
                        "initialize",
                        {
                            "clientInfo": {
                                "name": "compass-interop",
                                "title": "Compass interoperability harness",
                                "version": "1.0.0",
                            },
                            "capabilities": {"experimentalApi": True},
                        },
                    ),
                    "initialize",
                )
                client.notify("initialized")
                object_result(
                    client.request(
                        "experimentalFeature/enablement/set",
                        {"enablement": {"mcp_2026_07_28": True}},
                    ),
                    "experimentalFeature/enablement/set",
                )
                deadline = time.monotonic() + 15
                entry: dict[str, Any] | None = None
                tools: Any = None
                while time.monotonic() < deadline:
                    inventory = object_result(
                        client.request("mcpServerStatus/list", {}),
                        "mcpServerStatus/list",
                    )
                    entries = inventory.get("data")
                    entry = next(
                        (
                            value
                            for value in entries
                            if isinstance(value, dict) and value.get("name") == "compass"
                        ),
                        None,
                    ) if isinstance(entries, list) else None
                    tools = entry.get("tools") if isinstance(entry, dict) else None
                    if isinstance(tools, dict) and "graph_stats" in tools:
                        break
                    time.sleep(0.25)
                if not isinstance(tools, dict) or "graph_stats" not in tools:
                    raise AppServerError(
                        f"Codex did not discover Compass graph_stats: {entry!r}"
                    )

                thread = object_result(
                    client.request("thread/start", {"cwd": str(temp), "ephemeral": True}),
                    "thread/start",
                )
                thread_value = thread.get("thread")
                thread_id = thread_value.get("id") if isinstance(thread_value, dict) else None
                if not isinstance(thread_id, str):
                    raise AppServerError("Codex did not create an ephemeral thread")
                call = object_result(
                    client.request(
                        "mcpServer/tool/call",
                        {
                            "threadId": thread_id,
                            "server": "compass",
                            "tool": "graph_stats",
                            "arguments": {},
                        },
                    ),
                    "mcpServer/tool/call",
                )
                content = call.get("content")
                text = content[0].get("text") if isinstance(content, list) and content else None
                if not isinstance(text, str) or not text.startswith("Nodes: "):
                    raise AppServerError(f"unexpected graph_stats result: {call!r}")
                return {
                    "client": "codex-cli",
                    "version": args.expected_version,
                    "transport": args.transport,
                    "discovery": "PASS",
                    "invocation": "PASS",
                    "status": "PASS",
                    "tool": "graph_stats",
                    "evidence": text.splitlines()[0],
                }
            finally:
                client.close()
        finally:
            if server is not None and server.poll() is None:
                server.terminate()
                try:
                    server.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait(timeout=3)
            if server_log is not None:
                server_log.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex", type=Path, required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--compass", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--transport", choices=("stdio", "http"), required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(run(args), sort_keys=True))
    except (AppServerError, OSError, subprocess.SubprocessError) as error:
        print(f"Codex MCP interop failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
