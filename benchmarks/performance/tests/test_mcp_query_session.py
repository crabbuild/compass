from __future__ import annotations

import json
import os
import selectors
import signal
import subprocess
import threading
import time
import tempfile
from pathlib import Path
import unittest
from unittest import mock

from benchmarks.performance.compass import mcp_query_session


class _Process:
    def __init__(self, stdout, return_code: int | None = None):
        self.stdout = stdout
        self.return_code = return_code
        self.stdin = mock.Mock()

    def poll(self):
        return self.return_code


class McpSessionFramingTests(unittest.TestCase):
    def test_close_escalates_from_sigterm_to_sigkill(self) -> None:
        session = object.__new__(mcp_query_session.McpSession)
        session.process = mock.Mock()
        session.process.pid = 42
        session.process.wait.side_effect = [
            subprocess.TimeoutExpired("server", 5),
            subprocess.TimeoutExpired("server", 5),
            0,
        ]
        session.selector = mock.Mock()
        session.stderr_buffer = bytearray()
        with tempfile.TemporaryDirectory() as directory:
            session.stderr_path = Path(directory) / "server.err"
            with mock.patch.object(os, "killpg") as killpg, mock.patch.object(
                mcp_query_session.resource, "getrusage"
            ) as usage:
                usage.return_value.ru_maxrss = 1024
                self.assertTrue(session.close())
        self.assertEqual(
            killpg.call_args_list,
            [mock.call(42, signal.SIGTERM), mock.call(42, signal.SIGKILL)],
        )
        self.assertGreaterEqual(session.peak_rss_kib, 0)

    def session(self):
        read_fd, write_fd = os.pipe()
        stdout = os.fdopen(read_fd, "rb", buffering=0)
        os.set_blocking(read_fd, False)
        session = object.__new__(mcp_query_session.McpSession)
        session.timeout = 0.05
        session.next_id = 1
        session.buffer = bytearray()
        session.selector = selectors.DefaultSelector()
        session.selector.register(stdout, selectors.EVENT_READ)
        session.process = _Process(stdout)
        return session, write_fd

    def cleanup(self, session, write_fd):
        try:
            os.close(write_fd)
        except OSError:
            pass
        session.selector.close()
        session.process.stdout.close()

    def test_partial_line_times_out(self) -> None:
        session, write_fd = self.session()
        try:
            os.write(write_fd, b'{"jsonrpc":"2.0"')
            with self.assertRaises(TimeoutError):
                session._read_line(time.monotonic() + 0.02, 1)
        finally:
            self.cleanup(session, write_fd)

    def test_oversized_line_is_rejected(self) -> None:
        session, write_fd = self.session()
        original = mcp_query_session.MAX_RESPONSE_BYTES
        mcp_query_session.MAX_RESPONSE_BYTES = 128
        writer = threading.Thread(target=os.write, args=(write_fd, b"x" * 129 + b"\n"))
        writer.start()
        try:
            with self.assertRaisesRegex(RuntimeError, "20 MiB"):
                session._read_line(time.monotonic() + 1, 1)
        finally:
            writer.join()
            mcp_query_session.MAX_RESPONSE_BYTES = original
            self.cleanup(session, write_fd)

    def test_malformed_json_is_rejected(self) -> None:
        session, write_fd = self.session()
        session._write = mock.Mock()
        os.write(write_fd, b"not-json\n")
        try:
            with self.assertRaises(json.JSONDecodeError):
                session.request("test", {})
        finally:
            self.cleanup(session, write_fd)

    def test_wrong_id_noise_is_ignored_until_matching_response(self) -> None:
        session, write_fd = self.session()
        session._write = mock.Mock()
        os.write(
            write_fd,
            b'{"jsonrpc":"2.0","id":99,"result":{}}\n'
            b'{"jsonrpc":"2.0","id":1,"result":{"ok":true}}\n',
        )
        try:
            self.assertEqual(session.request("test", {}), {"ok": True})
        finally:
            self.cleanup(session, write_fd)

    def test_early_exit_before_response_is_rejected(self) -> None:
        session, write_fd = self.session()
        session.process.return_code = 7
        os.close(write_fd)
        try:
            with self.assertRaisesRegex(RuntimeError, "exited before response"):
                session._read_line(time.monotonic() + 1, 1)
        finally:
            self.cleanup(session, write_fd)


if __name__ == "__main__":
    unittest.main()
