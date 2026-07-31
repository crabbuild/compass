from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ANALYZER = Path(__file__).parents[1] / "analyze.py"
FIXTURES = Path(__file__).parent / "fixtures"


class AnalyzeTests(unittest.TestCase):
    def test_help_runs_outside_repository_and_describes_portable_inputs(self) -> None:
        completed = subprocess.run(
            [sys.executable, str(ANALYZER), "--help"],
            cwd="/tmp",
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        self.assertEqual(0, completed.returncode, completed.stderr)
        self.assertIn("--workspace", completed.stdout)
        self.assertIn("--corpora", completed.stdout)

    def test_analyzes_a_manifest_with_relative_source_paths(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "run"
            source = root / "sources" / "sample"
            source.mkdir(parents=True)
            subprocess.run(["git", "init", "-q", source], check=True)
            subprocess.run(
                ["git", "-C", source, "remote", "add", "origin", "https://example.test/sample.git"],
                check=True,
            )
            (source / "sample.py").write_text("def sample():\n    pass\n", encoding="utf-8")
            subprocess.run(["git", "-C", source, "add", "sample.py"], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    source,
                    "-c",
                    "user.name=Benchmark Test",
                    "-c",
                    "user.email=benchmark@example.test",
                    "commit",
                    "-qm",
                    "fixture",
                ],
                check=True,
            )

            compass_out = workspace / "outputs" / "compass" / "sample" / "compass-out"
            generation = compass_out / ".compass-generations" / "generation-1"
            generation.mkdir(parents=True)
            (compass_out / ".compass-active-generation").write_text(
                "generation-1\n", encoding="utf-8"
            )
            shutil.copyfile(FIXTURES / "compass_graph.json", generation / "graph.json")

            graphify_out = (
                workspace / "outputs" / "graphify" / "sample" / "graphify-out"
            )
            graphify_out.mkdir(parents=True)
            shutil.copyfile(
                FIXTURES / "graphify_graph.json", graphify_out / "graph.json"
            )

            logs = workspace / "logs"
            logs.mkdir(parents=True)
            (logs / "compass-sample.log").write_text(
                "0.50 real\n1024 maximum resident set size\n"
                "publication: omitting 2 nodes and 3 edges; 1 identity collisions\n",
                encoding="utf-8",
            )
            (logs / "graphify-sample.log").write_text(
                "1.25 real\n2048 maximum resident set size\n", encoding="utf-8"
            )

            manifest = root / "corpora.json"
            manifest.write_text(
                json.dumps(
                    {
                        "corpora": [
                            {
                                "name": "sample",
                                "language": "Python",
                                "framework": "Example",
                                "source": "sources/sample",
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )
            compass = root / "compass"
            compass.write_text("#!/bin/sh\necho 'compass 1.2.3'\n", encoding="utf-8")
            graphify = root / "graphify"
            graphify.write_text("#!/bin/sh\necho 'graphify 4.5.6'\n", encoding="utf-8")
            os.chmod(compass, 0o755)
            os.chmod(graphify, 0o755)

            completed = subprocess.run(
                [
                    sys.executable,
                    str(ANALYZER),
                    "--workspace",
                    str(workspace),
                    "--corpora",
                    str(manifest),
                    "--compass-binary",
                    str(compass),
                    "--compass-source",
                    str(source),
                    "--graphify-binary",
                    str(graphify),
                ],
                cwd="/tmp",
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            self.assertEqual(0, completed.returncode, completed.stderr)
            results = json.loads(
                (workspace / "metrics" / "results.json").read_text(encoding="utf-8")
            )
            self.assertEqual("compass-graphify-real-world-evaluation/1", results["schema"])
            self.assertEqual("compass 1.2.3", results["tools"]["compass"]["version"])
            self.assertEqual(
                subprocess.run(
                    ["git", "-C", source, "rev-parse", "HEAD"],
                    check=True,
                    stdout=subprocess.PIPE,
                    text=True,
                ).stdout.strip(),
                results["tools"]["compass"]["commit"],
            )
            corpus = results["corpora"][0]
            self.assertEqual(str(source.resolve()), corpus["source"])
            self.assertEqual(3, corpus["compass"]["nodes"])
            self.assertEqual(2, corpus["compass"]["edges"])
            self.assertEqual(2, corpus["graphify"]["nodes"])
            self.assertEqual(1, corpus["graphify"]["edges"])
            self.assertEqual(
                {"edges": 3, "identity_collisions": 1, "nodes": 2},
                corpus["compass_omissions"],
            )
            self.assertTrue(corpus["comparison"]["passed"])
            report = (workspace / "REPORT.md").read_text(encoding="utf-8")
            self.assertIn("| sample | Python / Example | 3 | 2 | 2 | 1 |", report)


if __name__ == "__main__":
    unittest.main()
