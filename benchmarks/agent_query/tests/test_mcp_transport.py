from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.agent_query.mcp_transport import StdioMcp


class McpTransportTests(unittest.TestCase):
    def test_unsupported_platform_fails_before_starting_a_process(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            client = StdioMcp([sys.executable], root, root/'capture')
            with patch('benchmarks.agent_query.mcp_transport.os.name', 'nt'):
                with self.assertRaisesRegex(OSError, 'POSIX'):
                    client.__enter__()
            self.assertFalse((root/'capture').exists())

    def invoke(self, body, *, timeout=1, max_bytes=4096):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            script = root / 'server.py'
            script.write_text('import sys,json,time\nr=json.loads(sys.stdin.readline())\n'+body)
            with StdioMcp([sys.executable, str(script)], root, root/'capture', timeout, max_bytes) as client:
                return client.send('tools/list', {})

    def test_response(self):
        r = self.invoke('print(json.dumps({"jsonrpc":"2.0","id":r["id"],"result":{"tools":[]}}),flush=True)')
        self.assertEqual(r['result']['tools'], [])

    def test_wrong_id(self):
        with self.assertRaisesRegex(ValueError, 'ID'):
            self.invoke('print(json.dumps({"jsonrpc":"2.0","id":42,"result":{}}),flush=True)')

    def test_eof(self):
        with self.assertRaisesRegex(RuntimeError, 'closed stdout'):
            self.invoke('pass')

    def test_deadline(self):
        with self.assertRaises(TimeoutError):
            self.invoke('time.sleep(5)', timeout=.05)

    def test_output_limit(self):
        with self.assertRaisesRegex(ValueError, 'byte bound'):
            self.invoke('print("x"*5000,flush=True)', max_bytes=100)

    def test_stderr_limit(self):
        with self.assertRaisesRegex(ValueError, 'stderr bound'):
            self.invoke('print("x"*5000,file=sys.stderr,flush=True);time.sleep(.2)', max_bytes=100)

    def test_notification_then_response(self):
        r = self.invoke('print(json.dumps({"jsonrpc":"2.0","method":"notice"}),flush=True)\nprint(json.dumps({"jsonrpc":"2.0","id":r["id"],"result":{}}),flush=True)')
        self.assertEqual(r['result'], {})
