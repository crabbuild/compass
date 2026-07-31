from __future__ import annotations

import hashlib
from pathlib import Path
import sys
import tempfile
import unittest

from benchmarks.performance.compass.process import ProcessSpec, run_measured


FIXTURE = Path(__file__).parent / "helpers" / "process_fixture.py"


class ProcessTests(unittest.TestCase):
    def run_fixture(self, directory: Path, name: str, *arguments: str, timeout: float = 10):
        return run_measured(
            ProcessSpec(
                command=(sys.executable, str(FIXTURE), *arguments),
                cwd=directory,
                stdout_path=directory / f"{name}.out",
                stderr_path=directory / f"{name}.err",
                timeout_seconds=timeout,
            )
        )

    def test_success_records_output_and_digest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.run_fixture(root, "success", "--stdout", "measured")
            self.assertEqual(result.return_code, 0)
            self.assertFalse(result.timed_out)
            expected = hashlib.sha256(b"measured\n").hexdigest()
            self.assertEqual(result.stdout_sha256, expected)
            self.assertGreater(result.wall_seconds, 0)
            self.assertGreater(result.peak_rss_kib, 0)

    def test_nonzero_exit_is_evidence_not_worker_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_fixture(Path(directory), "failure", "--exit", "7")
            self.assertEqual(result.return_code, 7)
            self.assertFalse(result.timed_out)

    def test_peak_rss_is_sample_local(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            large = self.run_fixture(root, "large", "--allocate-mib", "48")
            small = self.run_fixture(root, "small", "--allocate-mib", "1")
            self.assertGreater(large.peak_rss_kib, small.peak_rss_kib + 20 * 1024)

    def test_timeout_terminates_process_group(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_fixture(
                Path(directory),
                "timeout",
                "--spawn-child",
                "--sleep",
                "60",
                timeout=0.2,
            )
            self.assertTrue(result.timed_out)
            self.assertIsNotNone(result.signal)


if __name__ == "__main__":
    unittest.main()
