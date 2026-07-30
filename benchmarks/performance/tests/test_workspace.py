from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from benchmarks.performance.compass_perf.model import RepositorySpec
from benchmarks.performance.compass_perf.workspace import (
    QualificationWorkspace,
    guarded_remove,
    prepare_checkout,
    resolve_remote_head,
)


def git(cwd: Path, *arguments: str) -> str:
    return subprocess.check_output(
        ["git", *arguments],
        cwd=cwd,
        text=True,
        encoding="utf-8",
    ).strip()


class WorkspaceTests(unittest.TestCase):
    def test_guarded_remove_requires_narrow_owned_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            workspace = QualificationWorkspace.create(Path(directory) / "workspace")
            target = workspace.root / "runs" / "one"
            target.mkdir(parents=True)
            (target / "sample").write_text("data", encoding="utf-8")
            guarded_remove(target)
            self.assertFalse(target.exists())
            with self.assertRaisesRegex(ValueError, "broad"):
                guarded_remove(workspace.root / "runs")

    def test_guarded_remove_rejects_symlink_escape(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            workspace = QualificationWorkspace.create(base / "workspace")
            outside = base / "outside"
            outside.mkdir()
            link = workspace.root / "runs" / "escape"
            link.parent.mkdir()
            os.symlink(outside, link)
            with self.assertRaisesRegex(ValueError, "escapes"):
                guarded_remove(link / "victim")

    def test_lock_contention_is_observable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            workspace = QualificationWorkspace.create(Path(directory) / "workspace")
            with workspace.acquire():
                with self.assertRaisesRegex(RuntimeError, "locked"):
                    with workspace.acquire():
                        self.fail("second lock unexpectedly acquired")
            self.assertFalse(workspace.lock.exists())

    def test_local_remote_head_and_checkout_are_exact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source = base / "source"
            remote = base / "remote.git"
            source.mkdir()
            git(source, "init", "-q", "-b", "main")
            git(source, "config", "user.name", "Compass")
            git(source, "config", "user.email", "compass@example.invalid")
            (source / "main.py").write_text("def run():\n    return 1\n", encoding="utf-8")
            git(source, "add", "main.py")
            git(source, "commit", "-q", "-m", "fixture")
            commit = git(source, "rev-parse", "HEAD")
            git(base, "clone", "-q", "--bare", str(source), str(remote))

            branch, resolved = resolve_remote_head(str(remote))
            self.assertEqual((branch, resolved), ("main", commit))

            workspace = QualificationWorkspace.create(base / "workspace")
            spec = RepositorySpec(
                name="fixture",
                url=str(remote),
                mutation_suffix=".py",
                queries=(),
            )
            destination = workspace.root / "corpora" / "fixture"
            identity = prepare_checkout(spec, commit, destination)
            self.assertEqual(identity.commit, commit)
            self.assertEqual(git(destination, "status", "--porcelain"), "")
            self.assertTrue((workspace.root / "identities" / "fixture.json").is_file())


if __name__ == "__main__":
    unittest.main()

