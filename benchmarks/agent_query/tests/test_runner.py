from __future__ import annotations

import json
from pathlib import Path
import tempfile
import sys
import subprocess
import unittest
from unittest.mock import patch

from benchmarks.agent_query.runner import (
    Anchor,
    CommandResult,
    _compass_graph,
    _verify_source,
    prepare_repository,
    run_question,
    run_bounded,
    Question,
    Repository,
    _paired_summary,
    estimate_tokens,
    graph_metrics,
    judge,
    load_suite,
    render_report,
)

ROOT = Path(__file__).resolve().parents[1]


def question(**overrides) -> Question:
    values = {
        "identifier": "sample",
        "kind": "explain",
        "subject": "sample",
        "compass": ("explain", "sample"),
        "graphify": ("explain", "sample"),
        "expect": "answer",
        "required": ("sample.go",),
        "required_one_of": (),
        "min_one_of": 0,
        "min_candidates": 0,
        "forbidden": (),
        "budget_tokens": 0,
        "max_follow_ups": 0,
        "judgment": "reviewed",
    }
    values.update(overrides)
    return Question(**values)


class SuiteTests(unittest.TestCase):
    def test_ask_suite_uses_identical_questions_and_budgets(self) -> None:
        suite = load_suite(ROOT / "suite_ask.toml")
        self.assertEqual({r.language for r in suite.repositories},
                         {"Go", "Python", "Java", "TypeScript", "Rust"})
        for repository in suite.repositories:
            self.assertEqual({q.kind for q in repository.questions}, {"callers", "callees"})
            self.assertEqual(len(repository.questions), 2)
            for question in repository.questions:
                self.assertEqual(question.compass, ("ask", question.subject, "--text-budget", "2000"))
                self.assertEqual(question.graphify, ("query", question.subject, "--budget", "2000"))
                self.assertEqual(question.max_follow_ups, 0)
                self.assertIn("Same source fact", question.judgment)

    def test_fd_source_first_inputs_remain_distinct_and_pinned(self) -> None:
        suite = load_suite(ROOT / "suite_fd.toml")
        self.assertEqual(len(suite.repositories), 1)
        repository = suite.repository("fd")
        self.assertEqual(len(repository.questions), 12)
        self.assertEqual(len(repository.anchors), 5)
        manifest = json.loads((ROOT / "edge_witnesses_fd.json").read_text())
        self.assertEqual(manifest["schema"], "compass.agent-edge-witnesses/1")
        self.assertEqual(manifest["repository"], repository.name)
        self.assertEqual(manifest["commit"], repository.commit)
        witnesses = manifest["witnesses"]
        self.assertEqual(len({row["id"] for row in witnesses}), 12)
        self.assertEqual(sum(row["expected"] == "present" for row in witnesses), 10)
        self.assertEqual(sum(len(row["occurrences"]) for row in witnesses), 16)
        for row in witnesses:
            self.assertEqual(bool(row["occurrences"]), row["expected"] == "present")
            self.assertTrue(row["judgment"])
        for question in repository.questions:
            self.assertNotIn("--brief", question.compass)
            if question.kind in {"path", "file_path"}:
                self.assertIn("--undirected", question.graphify)

    def test_report_describes_the_actual_repository_count(self) -> None:
        run = {"runId": "test", "suiteDigest": "digest", "startedAt": "date",
               "tools": [], "graphMetrics": [], "observations": [], "questions": [],
               "summaries": {}, "paired": {}, "repositories": [{"repository": "fd"}],
               "verdict": {key: "unmeasured" for key in
                           ("correctness", "tokens", "pairedTokens", "split", "graphQuality")}}
        self.assertIn("sample of 1 repository", render_report(run))
        run["repositories"].append({"repository": "second"})
        self.assertIn("sample of 2 repositories", render_report(run))

    def test_checked_in_suite_covers_five_languages(self) -> None:
        suite = load_suite(ROOT / "suite.toml")
        self.assertEqual(len(suite.digest), 64)
        self.assertEqual(len(suite.repositories), 5)
        self.assertEqual(
            {repository.language for repository in suite.repositories},
            {"Go", "Python", "Java", "TypeScript", "Rust"},
        )
        for repository in suite.repositories:
            self.assertEqual(len(repository.commit), 40)
            self.assertGreaterEqual(len(repository.questions), 6)
            self.assertGreaterEqual(len(repository.anchors), 3)
            for anchor in repository.anchors:
                self.assertTrue(anchor.judgment)
        kinds = {
            question.kind
            for repository in suite.repositories
            for question in repository.questions
        }
        self.assertEqual(
            kinds,
            {
                "explain",
                "explain_source",
                "brief",
                "callers",
                "brief_callers",
                "paged_callers",
                "path",
                "file_path",
                "ambiguity",
                "negative",
                "broad",
            },
        )

    def test_blackbox_suite_asks_fifty_symmetric_questions(self) -> None:
        suite = load_suite(ROOT / "suite_v2.toml")
        self.assertEqual(
            {repository.name for repository in suite.repositories},
            {"cobra", "flask", "gson", "zod", "axum"},
        )
        questions = [question for repository in suite.repositories for question in repository.questions]
        self.assertEqual(len(questions), 50)
        for repository in suite.repositories:
            self.assertEqual(len(repository.questions), 10)
            self.assertGreaterEqual(len(repository.anchors), 3)
        self.assertEqual(
            {question.kind for question in questions},
            {
                "explain",
                "explain_source",
                "callers",
                "callees",
                "impact",
                "path",
                "file_path",
                "ambiguity",
                "negative",
                "broad",
            },
        )
        for question in questions:
            # The blackbox suite never prices a Compass-only projection.
            self.assertNotIn("--brief", question.compass)
            self.assertNotIn("--format", question.compass)
            if question.kind == "explain_source":
                self.assertIn("--source", question.compass)
            if question.kind == "path" or question.kind == "file_path":
                # Compass `path` searches relationships in both directions.
                self.assertIn("--undirected", question.graphify)
            if question.kind == "impact":
                # Compass pages the bounded traversal with its cursor ledger.
                self.assertGreater(question.max_follow_ups, 0)
            if question.kind == "broad":
                self.assertGreater(question.budget_tokens, 0)


class PairedSummaryTests(unittest.TestCase):
    def observation(self, question: str, tool: str, passed: bool, tokens: int) -> dict:
        return {
            "repository": "cobra",
            "question": question,
            "kind": "callers",
            "tool": tool,
            "passed": passed,
            "totalTokens": tokens,
            "wallMs": tokens,
        }

    def test_paired_tokens_use_only_questions_both_tools_passed(self) -> None:
        observations = [
            # Both answered: this pair sets the reported medians.
            self.observation("shared", "compass", True, 400),
            self.observation("shared", "graphify", True, 100),
            # Only Compass answered: its cheap row must not enter the medians.
            self.observation("compass-only", "compass", True, 10),
            self.observation("compass-only", "graphify", False, 0),
            # Only Graphify answered.
            self.observation("graphify-only", "compass", False, 0),
            self.observation("graphify-only", "graphify", True, 20),
            # Neither answered.
            self.observation("neither", "compass", False, 0),
            self.observation("neither", "graphify", False, 0),
        ]
        summary = _paired_summary(observations)
        self.assertEqual(summary["questions"], 4)
        self.assertEqual(summary["bothPassed"], 1)
        self.assertEqual(summary["compassOnly"], 1)
        self.assertEqual(summary["graphifyOnly"], 1)
        self.assertEqual(summary["neither"], 1)
        self.assertEqual(summary["medianCompassTokens"], 400)
        self.assertEqual(summary["medianGraphifyTokens"], 100)

    def test_paired_tokens_are_zero_without_a_shared_answer(self) -> None:
        observations = [
            self.observation("a", "compass", True, 50),
            self.observation("a", "graphify", False, 5),
        ]
        summary = _paired_summary(observations)
        self.assertEqual(summary["bothPassed"], 0)
        self.assertEqual(summary["medianCompassTokens"], 0)
        self.assertEqual(summary["medianGraphifyTokens"], 0)


class ExecutionEvidenceTests(unittest.TestCase):
    def test_source_root_must_contain_the_reviewed_anchor_files(self) -> None:
        repository = load_suite(ROOT / "suite.toml").repositories[0]
        with tempfile.TemporaryDirectory() as temporary:
            responses = [subprocess.CompletedProcess([], 0, stdout=value) for value in (
                "false\n", repository.commit + "\n", "",
            )]
            with patch("benchmarks.agent_query.runner.subprocess.run", side_effect=responses):
                with self.assertRaisesRegex(RuntimeError, "reviewed source file"):
                    _verify_source(repository, Path(temporary))

    def test_bare_repository_is_not_accepted_as_source(self) -> None:
        repository = load_suite(ROOT / "suite.toml").repositories[0]
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            subprocess.run(("git", "init", "--bare", "--quiet", str(source)), check=True)
            with self.assertRaisesRegex(RuntimeError, "working checkout"):
                _verify_source(repository, source)

    def test_capture_limits_both_streams_without_unbounded_disk_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for stream in ("stdout", "stderr"):
                with self.subTest(stream=stream):
                    with patch("benchmarks.agent_query.runner.MAX_OUTPUT_BYTES", 4096):
                        result = run_bounded(
                            (sys.executable, "-c", f"import sys; sys.{stream}.write('x' * 100000)"),
                            cwd=root, timeout_seconds=5,
                            stdout_path=root / "stdout", stderr_path=root / "stderr",
                        )
                    self.assertTrue(result.output_limited)
                    self.assertLessEqual((root / "stdout").stat().st_size, 4096)
                    self.assertLessEqual((root / "stderr").stat().st_size, 4096)

    def test_capture_preserves_success_and_reports_timeout(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            result = run_bounded(
                (sys.executable, "-c", "import sys; print('answer'); print('notice', file=sys.stderr)"),
                cwd=root, timeout_seconds=5,
                stdout_path=root / "stdout", stderr_path=root / "stderr",
            )
            self.assertEqual(result.stdout, "answer\n")
            self.assertEqual(result.stderr, "notice\n")
            self.assertEqual(result.exit_code, 0)
            self.assertFalse(result.timed_out or result.output_limited)
            result = run_bounded(
                (sys.executable, "-c", "import time; time.sleep(10)"),
                cwd=root, timeout_seconds=0.05,
                stdout_path=root / "stdout", stderr_path=root / "stderr",
            )
            self.assertTrue(result.timed_out)
            self.assertFalse(result.output_limited)
            self.assertNotEqual(result.exit_code, 0)

    def test_graphify_ambiguity_exit_one_is_a_valid_pick_list_only(self) -> None:
        repository = load_suite(ROOT / "suite.toml").repositories[0]
        output = "Ambiguous: 'sample' matches 2 nodes in different files.\n  sample.go\n    id: a\n  other.go\n    id: b\n"
        oracle = question(
            kind="ambiguity", expect="pick_list", required_one_of=("sample.go",),
            min_one_of=1, min_candidates=2,
        )
        result = CommandResult((), 1, False, 1, len(output), 0, output, "")
        with patch("benchmarks.agent_query.runner.run_bounded", return_value=result):
            observation = run_question(
                repository, oracle, tool="graphify", binary=Path("tool"),
                graph=Path("graph.json"), cwd=ROOT, raw_dir=ROOT, timeout_seconds=1,
            )
        self.assertTrue(observation.passed)
        self.assertEqual(observation.exit_code, 1)

    def test_graphify_exit_one_exception_does_not_accept_other_outcomes(self) -> None:
        repository = load_suite(ROOT / "suite.toml").repositories[0]
        output = "Ambiguous: 'sample' matches 2 nodes in different files.\n  sample.go\n    id: a\n  other.go\n    id: b\n"
        base = dict(kind="ambiguity", expect="pick_list", required_one_of=("sample.go",), min_one_of=1, min_candidates=2)
        cases = (
            (question(**base), output.replace("Ambiguous:", "Error:")),
            (question(**base), output.replace("    id: b", "")),
            (question(**base, graphify=("query", "sample")), output),
            (question(), output),
            (question(**{**base, "required_one_of": ("missing.go",)}), output),
        )
        for oracle, stdout in cases:
            with self.subTest(oracle=oracle, stdout=stdout):
                result = CommandResult((), 1, False, 1, len(stdout), 0, stdout, "")
                with patch("benchmarks.agent_query.runner.run_bounded", return_value=result):
                    observation = run_question(
                        repository, oracle, tool="graphify", binary=Path("tool"),
                        graph=Path("graph.json"), cwd=ROOT, raw_dir=ROOT, timeout_seconds=1,
                    )
                self.assertFalse(observation.passed)

    def test_failed_or_timed_out_output_cannot_pass(self) -> None:
        repository = load_suite(ROOT / "suite.toml").repositories[0]
        for tool in ("compass", "graphify"):
            for exit_code, timed_out, output_limited in ((1, False, False), (0, True, False), (0, False, True)):
                with self.subTest(tool=tool, exit_code=exit_code, timed_out=timed_out):
                    result = CommandResult((), exit_code, timed_out, 1, 9, 0, "sample.go", "", output_limited)
                    with patch("benchmarks.agent_query.runner.run_bounded", return_value=result):
                        observation = run_question(
                            repository, question(), tool=tool, binary=Path("tool"),
                            graph=Path("graph.json"), cwd=ROOT, raw_dir=ROOT,
                            timeout_seconds=1,
                        )
                    self.assertFalse(observation.passed)
                    self.assertFalse(observation.first_page_pass)
                    self.assertTrue(observation.failures)

    def test_compass_continuation_requires_one_real_footer_cursor(self) -> None:
        repository = load_suite(ROOT / "suite.toml").repositories[0]
        outputs = (
            "Pagination: range=1-1 of 1 next=none\n",
            "Source excerpt: next=source_text\n",
            "Bound: next= continues the response\n",
            "Pagination: page=1 range=1-2 of 4 next=first\nPagination: page=2 range=3-4 of 4 next=second\n",
        )
        for output in outputs:
            with self.subTest(output=output):
                result = CommandResult((), 0, False, 1, len(output), 0, output, "")
                with patch("benchmarks.agent_query.runner.run_bounded", return_value=result) as run:
                    observation = run_question(
                        repository, question(kind="broad", compass=("query", "sample"), max_follow_ups=2), tool="compass",
                        binary=Path("tool"), graph=Path("graph.json"), cwd=ROOT,
                        raw_dir=ROOT, timeout_seconds=1,
                    )
                self.assertEqual(run.call_count, 1)
                self.assertEqual(observation.follow_ups, 0)
                self.assertFalse(observation.passed)
                self.assertEqual(observation.exit_code, 0)

    def test_compass_follows_footer_token_and_stops_on_repeated_cursor(self) -> None:
        repository = load_suite(ROOT / "suite.toml").repositories[0]
        for footer in (
            "Pagination: range=1-1 of 2 next=opaque-token",
            "Pagination: page=1 range=1-1 of 2 next=opaque-token",
            "Pagination: page=1/2 items=1-1/2 next=opaque-token",
            "Pagination: range=1-1 of 2 next=opaque-token\r",
        ):
            for final in ("sample.go\n", footer + "\n"):
                with self.subTest(footer=footer, final=final):
                    first = "source next=unrelated\n" + footer + "\n"
                    replies = [CommandResult((), 0, False, 1, len(s), 0, s, "")
                               for s in (first, final)]
                    with patch("benchmarks.agent_query.runner.run_bounded", side_effect=replies) as run:
                        observation = run_question(
                            repository, question(kind="broad", compass=("query", "sample"), max_follow_ups=3), tool="compass",
                            binary=Path("tool"), graph=Path("graph.json"), cwd=ROOT,
                            raw_dir=ROOT, timeout_seconds=1,
                        )
                    self.assertEqual(run.call_count, 2)
                    argv = run.call_args_list[1].args[0]
                    self.assertEqual(argv[argv.index("--cursor") + 1], "opaque-token")
                    self.assertEqual(observation.follow_ups, 1)
                    self.assertEqual(observation.passed, final == "sample.go\n")

    def test_snapshot_pointer_cannot_fall_back_to_an_unpublished_graph(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = root / "compass-out" / "snapshots" / "snapshot-other"
            snapshot.mkdir(parents=True)
            (snapshot / "graph.json").write_text("{}")
            (root / "compass-out" / "current-snapshot").write_text("snapshot-missing")
            with self.assertRaisesRegex(RuntimeError, "published Compass snapshot"):
                _compass_graph(root)

    def test_each_new_run_builds_both_graphs_and_retains_build_arguments(self) -> None:
        repository = load_suite(ROOT / "suite.toml").repositories[0]
        def build(argv, **kwargs):
            output = Path(argv[-1])
            if argv[0] == "compass":
                snapshot = output / "compass-out" / "snapshots" / "snapshot-current"
                snapshot.mkdir(parents=True)
                (snapshot / "graph.json").write_text("{}")
                (output / "compass-out" / "current-snapshot").write_text("snapshot-current")
            else:
                (output / "graphify-out").mkdir(parents=True)
                (output / "graphify-out" / "graph.json").write_text("{}")
            return CommandResult(argv, 0, False, 1, 0, 0, "", "")
        with tempfile.TemporaryDirectory() as temporary:
            with patch("benchmarks.agent_query.runner.run_bounded", side_effect=build) as run:
                for name in ("first", "second"):
                    record = prepare_repository(
                        repository, ROOT, Path(temporary) / name,
                        compass_binary=Path("compass"), graphify_binary=Path("graphify"),
                        timeout_seconds=1,
                    )
                    self.assertEqual(record["commit"], repository.commit)
                    for tool in ("compass", "graphify"):
                        self.assertEqual(record[f"{tool}BuildArgv"][:2], [tool, "extract"])
                        self.assertEqual(len(record[f"{tool}GraphSha256"]), 64)
                self.assertEqual(run.call_count, 4)

    def test_existing_artifacts_are_never_silently_reused_or_removed(self) -> None:
        repository = load_suite(ROOT / "suite.toml").repositories[0]
        with tempfile.TemporaryDirectory() as temporary:
            artifacts = Path(temporary)
            root = artifacts / repository.name
            root.mkdir()
            marker = root / "prior-graph.json"
            marker.write_text("prior evidence")
            with patch("benchmarks.agent_query.runner.run_bounded") as run:
                with self.assertRaises(FileExistsError):
                    prepare_repository(
                        repository, ROOT, artifacts,
                        compass_binary=Path("compass"), graphify_binary=Path("graphify"),
                        timeout_seconds=1,
                    )
                run.assert_not_called()
            self.assertEqual(marker.read_text(), "prior evidence")


class EstimateTests(unittest.TestCase):
    def test_tokens_round_up_by_four_bytes(self) -> None:
        self.assertEqual(estimate_tokens(0), 0)
        self.assertEqual(estimate_tokens(1), 1)
        self.assertEqual(estimate_tokens(4), 1)
        self.assertEqual(estimate_tokens(5), 2)


class JudgeTests(unittest.TestCase):
    def test_answer_oracle(self) -> None:
        oracle = question(required=("ExecuteC", "command.go"))
        passed, failures = judge(oracle, "compass", "ExecuteC in command.go")
        self.assertTrue(passed)
        self.assertEqual(failures, ())
        passed, failures = judge(oracle, "compass", "ExecuteC only")
        self.assertFalse(passed)
        self.assertEqual(failures, ("missing 'command.go'",))

    def test_answer_oracle_accepts_one_of_alternatives(self) -> None:
        oracle = question(
            required=(),
            required_one_of=("safeParse", "validate"),
            min_one_of=1,
        )
        self.assertTrue(judge(oracle, "compass", "ZodType.validate")[0])
        passed, failures = judge(oracle, "compass", "unrelated output")
        self.assertFalse(passed)
        self.assertTrue(any("at least 1 of" in failure for failure in failures))

    def test_forbidden_anchor_fails(self) -> None:
        oracle = question(forbidden=("wrong.go",))
        passed, failures = judge(oracle, "graphify", "sample.go wrong.go")
        self.assertFalse(passed)
        self.assertEqual(failures, ("forbidden 'wrong.go'",))

    def test_pick_list_requires_enough_distinct_candidates(self) -> None:
        oracle = question(
            kind="ambiguity",
            expect="pick_list",
            required=("parse",),
            required_one_of=("core/parse.ts", "classic/parse.ts"),
            min_one_of=2,
        )
        self.assertTrue(judge(oracle, "compass", "core/parse.ts classic/parse.ts")[0])
        passed, failures = judge(oracle, "compass", "core/parse.ts only")
        self.assertFalse(passed)
        self.assertTrue(failures[0].startswith("pick list"))

    def test_pick_list_counts_compass_identifiers(self) -> None:
        oracle = question(
            kind="ambiguity",
            expect="pick_list",
            required=("fromJson",),
            required_one_of=("fromJson",),
            min_one_of=1,
            min_candidates=2,
        )
        payload = json.dumps(
            {
                "request": {"operation": "search", "operands": [{"role": "query", "value": "fromJson"}]},
                "primaryResults": [
                    {"id": "sha256:" + "a" * 64},
                    {"id": "sha256:" + "b" * 64},
                ]
            }
        )
        self.assertTrue(judge(oracle, "compass", payload)[0])
        single = json.dumps(
            {
                "request": {"operation": "search", "operands": [{"role": "query", "value": "fromJson"}]},
                "primaryResults": [{"id": "sha256:" + "a" * 64}],
            }
        )
        self.assertFalse(judge(oracle, "compass", single)[0])

    def test_negative_oracle_requires_an_explicit_no_match(self) -> None:
        oracle = question(kind="negative", expect="no_match", required=())
        self.assertTrue(judge(oracle, "compass", '{"resultState": "no_match"}')[0])
        self.assertTrue(judge(oracle, "graphify", "No matching nodes found.")[0])
        # `graphify explain` is the documented name-resolution command and
        # reports the same outcome with different wording.
        self.assertTrue(judge(oracle, "graphify", "No node matching 'Zed' found.")[0])
        self.assertFalse(judge(oracle, "graphify", "NODE Zebra [src=a.go loc=L1")[0])

    def test_pick_list_counts_graphify_ambiguity_candidates(self) -> None:
        oracle = question(
            kind="ambiguity",
            expect="pick_list",
            required=(),
            required_one_of=("command.go", "completions.go"),
            min_one_of=2,
            min_candidates=2,
        )
        payload = (
            "Ambiguous: 'Command' matches 2 nodes in different files.\n"
            "  command.go\n"
            "    id: command_go_cobra_command\n"
            "  completions.go\n"
            "    id: completions_go_cobra_command\n"
        )
        self.assertTrue(judge(oracle, "graphify", payload)[0])
        single = (
            "Ambiguous: 'Command' matches 1 nodes in different files.\n"
            "  command.go\n"
            "    id: command_go_cobra_command\n"
        )
        passed, failures = judge(oracle, "graphify", single)
        self.assertFalse(passed)
        self.assertIn("candidates 1/2", failures[-1])


class GraphMetricTests(unittest.TestCase):
    def metric_for_node(self, tool: str, node: dict, symbol: str = "parse") -> dict:
        repository = Repository(
            name="sample",
            language="TypeScript",
            url="https://example.invalid/sample.git",
            commit="0" * 40,
            questions=(),
            anchors=(Anchor(file="sample.ts", line=42, symbol=symbol, judgment="reviewed"),),
        )
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "graph.json"
            graph.write_text(json.dumps({"nodes": [node], "links": []}), encoding="utf-8")
            return graph_metrics(repository, tool, graph)

    def test_enclosing_module_does_not_prove_a_declaration_anchor(self) -> None:
        metrics = self.metric_for_node("compass", {
            "id": "module",
            "name": "sample",
            "kind": "module",
            "source": {"file": "sample.ts", "startLine": 1, "endLine": 100},
        })
        self.assertEqual(metrics["anchorHits"], 0)

    def test_anchor_requires_symbol_identity_for_both_tools(self) -> None:
        for tool in ("compass", "graphify"):
            for name in ("safeParse", "parseOther", "Parse", "", "unrelated"):
                with self.subTest(tool=tool, name=name):
                    node = {"id": "wrong", "name": name, "label": name}
                    if tool == "compass":
                        node["source"] = {"file": "sample.ts", "startLine": 42, "endLine": 50}
                    else:
                        node.update(source_file="sample.ts", source_location="L42")
                    self.assertEqual(self.metric_for_node(tool, node)["anchorHits"], 0)

    def test_anchor_uses_exact_declaration_start_not_an_overlapping_span(self) -> None:
        for start in (1, 41, 43, True):
            with self.subTest(start=start):
                metrics = self.metric_for_node("compass", {
                    "id": "wrong-overload",
                    "name": "parse",
                    "source": {"file": "sample.ts", "startLine": start, "endLine": 90},
                })
                self.assertEqual(metrics["anchorHits"], 0)

    def test_qualified_names_and_signature_labels_use_the_same_rule(self) -> None:
        for tool in ("compass", "graphify"):
            for name in ("parse", "Schema.parse", "Schema::parse", "parse(Input)"):
                with self.subTest(tool=tool, name=name):
                    node = {"id": "declaration", "name": name, "label": name}
                    if tool == "compass":
                        node["source"] = {"file": "sample.ts", "startLine": 42, "endLine": 50}
                    else:
                        node.update(source_file="sample.ts", source_location="L42")
                    self.assertEqual(self.metric_for_node(tool, node, "Schema.parse(Input)")["anchorHits"], 1)

    def test_compass_metrics_find_dangling_and_duplicate_records(self) -> None:
        repository = Repository(
            name="sample",
            language="Go",
            url="https://example.invalid/sample.git",
            commit="0" * 40,
            questions=(),
            anchors=(
                Anchor(file="a.go", line=3, symbol="A", judgment="reviewed"),
                Anchor(file="b.go", line=9, symbol="B", judgment="reviewed"),
            ),
        )
        document = {
            "nodes": [
                {
                    "id": "one",
                    "name": "A",
                    "source": {"file": "a.go", "startLine": 3, "endLine": 5},
                },
                {
                    "id": "one",
                    "name": "B",
                    "source": {"file": "b.go", "startLine": 9, "endLine": 12},
                },
            ],
            "links": [{"source": "one", "target": "missing"}],
        }
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "graph.json"
            graph.write_text(json.dumps(document), encoding="utf-8")
            metrics = graph_metrics(repository, "compass", graph)
        self.assertEqual(metrics["nodes"], 2)
        self.assertEqual(metrics["duplicateIds"], 1)
        self.assertEqual(metrics["danglingEdges"], 1)
        self.assertEqual(metrics["sourceBackedNodes"], 2)
        self.assertEqual(metrics["anchorHits"], 2)

    def test_graphify_metrics_match_declaration_lines(self) -> None:
        repository = Repository(
            name="sample",
            language="Go",
            url="https://example.invalid/sample.git",
            commit="0" * 40,
            questions=(),
            anchors=(Anchor(file="a.go", line=42, symbol="A", judgment="reviewed"),),
        )
        document = {
            "nodes": [
                {"id": "one", "label": "A", "source_file": "a.go", "source_location": "L42"},
                {"id": "two", "label": "A", "source_file": "a.go", "source_location": "L7"},
            ],
            "links": [],
        }
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "graph.json"
            graph.write_text(json.dumps(document), encoding="utf-8")
            metrics = graph_metrics(repository, "graphify", graph)
        self.assertEqual(metrics["anchorHits"], 1)
        self.assertEqual(metrics["sourceBackedRatio"], 1.0)


if __name__ == "__main__":
    unittest.main()
