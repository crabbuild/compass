from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import unittest

from benchmarks.performance.compass.model import RepositorySpec
from benchmarks.performance.harness import (
    _existing_ancestor,
    build_parser,
    requested_repository_commits,
)


HARNESS = Path(__file__).parents[1] / "harness.py"


class HarnessTests(unittest.TestCase):
    def test_commands_are_exposed(self) -> None:
        parser = build_parser()
        self.assertEqual("doctor", parser.parse_args(["doctor"]).command)
        self.assertEqual("prepare", parser.parse_args(["prepare"]).command)
        self.assertEqual("run", parser.parse_args(["run"]).command)
        self.assertEqual("compare", parser.parse_args(["compare"]).command)
        self.assertEqual("report", parser.parse_args(["report", "run.json"]).command)
        self.assertEqual("promote", parser.parse_args(["promote", "run.json"]).command)
        self.assertEqual(
            "audit",
            parser.parse_args(
                [
                    "audit",
                    "--manifest",
                    "audit.json",
                    "--graph",
                    "graph.json",
                    "--corpus",
                    "repository",
                ]
            ).command,
        )

    def test_comparison_is_explicit(self) -> None:
        parser = build_parser()
        self.assertEqual("run", parser.parse_args(["run"]).command)
        self.assertEqual("compare", parser.parse_args(["compare"]).command)

    def test_exact_revision_options_are_exposed(self) -> None:
        parser = build_parser()
        args = parser.parse_args(
            [
                "compare",
                "--repository",
                "django",
                "--repository-commit",
                f"django={'a' * 40}",
                "--graphify-commit",
                "b" * 40,
            ]
        )
        self.assertEqual([f"django={'a' * 40}"], args.repository_commit)
        self.assertEqual("b" * 40, args.graphify_commit)

    def test_repository_commit_overrides_fail_closed(self) -> None:
        selected = (
            RepositorySpec("django", "https://example.invalid/django", ".py", ()),
        )
        self.assertEqual(
            {"django": "a" * 40},
            requested_repository_commits([f"django={'A' * 40}"], selected),
        )
        with self.assertRaisesRegex(ValueError, "invalid"):
            requested_repository_commits(["django=short"], selected)
        with self.assertRaisesRegex(ValueError, "not selected"):
            requested_repository_commits([f"entire={'b' * 40}"], selected)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            requested_repository_commits(
                [f"django={'a' * 40}", f"django={'b' * 40}"],
                selected,
            )

    def test_help_runs_from_outside_repository(self) -> None:
        completed = subprocess.run(
            [sys.executable, str(HARNESS), "--help"],
            cwd="/tmp",
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.assertEqual(0, completed.returncode, completed.stderr)
        self.assertIn("correctness-first", completed.stdout.lower())

    def test_qualification_minimums_are_rejected(self) -> None:
        completed = subprocess.run(
            [sys.executable, str(HARNESS), "run", "--build-repeats", "2"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.assertNotEqual(0, completed.returncode)
        self.assertIn("at least 3", completed.stderr)

    def test_disk_check_accepts_a_not_yet_created_workspace(self) -> None:
        missing = Path("/tmp") / "compass-does-not-exist" / "workspace"
        self.assertEqual(Path("/tmp").resolve(), _existing_ancestor(missing))


if __name__ == "__main__":
    unittest.main()
