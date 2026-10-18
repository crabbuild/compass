"""Synthetic command/cost bindings, not native measurements or qualification."""
import copy
from dataclasses import replace
from pathlib import Path
import tempfile
import unittest

from benchmarks.agent_query.broad_query_capture import command, launch, ENV_PATH_KEYS
from benchmarks.agent_query.broad_query_execution_v2 import validate_execution
from benchmarks.agent_query.broad_query_lineage_v2 import BoundObservation, SCHEMAS
from benchmarks.agent_query.broad_query_review import ARMS, TOOLS, QUERY_EXIT_SCHEMA


class Fixture:
    def __init__(self, root, *, tool="compass", arm="normalized", raw=b"answer\n", style="darwin-l"):
        self.root = Path(root)
        self.task = dict(id="save", repository="sample", question="Where are save failures handled?")
        self.plan = dict(schema=SCHEMAS["capture-plan"], timeoutSeconds=5,
            tools=[dict(id=name, executable=dict(path="/synthetic/" + name), argvPrefix=[]) for name in TOOLS],
            arms=[dict(id="default"), dict(id="normalized", measure="utf8_bytes", budgetBytes=128, overflow="fail")],
            childEnvironment=dict(inherit=False, PATH="/usr/bin:/bin", GRAPHIFY_NO_AUTO_REFRESH="1",
                GRAPHIFY_QUERY_LOG_DISABLE="1", **{key: "{artifactRoot}/isolated/" + key.lower() for key in ENV_PATH_KEYS}),
            environmentExecutable=dict(path="/synthetic/env"),
            measurement=dict(style=style, executable=dict(path="/synthetic/time")))
        self.graph = dict(repository="sample", tool=tool, artifact=dict(path="graphs/" + tool + ".json"))
        stderr = b"2048 maximum resident set size\n" if style == "darwin-l" else b"Maximum resident set size (kbytes): 2\n"
        argv = launch(self.plan, command(self.plan, self.plan["tools"][TOOLS.index(tool)], arm,
            self.task["question"], self.root / self.graph["artifact"]["path"]), self.root)
        capture = dict(argv=list(argv), exit_code=0, wall_ms=5, timed_out=False,
            output_limited=False, stdout_bytes=len(raw), stderr_bytes=len(stderr))
        self.observation = BoundObservation(("save", tool, arm),
            dict(condition="fresh_cli", peakRssBytes=2048), capture, raw, stderr,
            {}, dict(disposition="answered", normalizationStatus="pending_review"), {}, {})

    def check(self):
        return validate_execution(self.observation, self.task, self.plan, self.graph, self.root)


class ExecutionTests(unittest.TestCase):
    def test_both_tools_and_arms_keep_one_command_and_original_costs(self):
        with tempfile.TemporaryDirectory() as directory:
            for tool in TOOLS:
                for arm in ARMS:
                    for style in ("darwin-l", "gnu-v"):
                        with self.subTest(tool=tool, arm=arm, style=style):
                            fixture = Fixture(directory, tool=tool, arm=arm, style=style)
                            result = fixture.check()
                            self.assertEqual(result["nativeOutcome"], "success")
                            self.assertFalse(result["executionFailed"])
                            self.assertEqual(result["cost"], dict(task="save", tool=tool, arm=arm,
                                firstPageBytes=7, workflowBytes=7, workflowWallMs=5, followUps=0,
                                overBudget=False, failed=False, peakRssBytes=2048, condition="fresh_cli"))

    def test_failed_timeout_limit_and_overbudget_output_remain_measured(self):
        with tempfile.TemporaryDirectory() as directory:
            for field, value in (("exit_code", 1), ("timed_out", True), ("output_limited", True)):
                fixture = Fixture(directory)
                fixture.observation.capture[field] = value
                result = fixture.check()
                self.assertEqual(result["nativeOutcome"], "failure")
                self.assertTrue(result["executionFailed"])
                self.assertEqual(result["cost"]["workflowWallMs"], 5)
            for arm in ARMS:
                for size in (128, 129):
                    fixture = Fixture(directory, arm=arm, raw=b"x" * size)
                    result = fixture.check()
                    expected = arm == "normalized" and size == 129
                    self.assertEqual(result["cost"]["overBudget"], expected)
                    self.assertEqual(result["executionFailed"], expected)
                    self.assertEqual(result["cost"]["firstPageBytes"], size)

    def test_exact_no_match_contract_preserves_exit_and_cannot_establish_answered(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory, tool="graphify", raw=b"No matching nodes found.\n")
            fixture.plan["tools"][1]["queryExitContract"] = dict(schema=QUERY_EXIT_SCHEMA,
                noMatchExitCode=4, noMatchStdoutLine="No matching nodes found.")
            fixture.observation.capture["exit_code"] = 4
            self.assertTrue(fixture.check()["executionFailed"])
            fixture.observation.reviewed["disposition"] = "abstain"
            self.assertFalse(fixture.check()["executionFailed"])
            self.assertEqual(fixture.check()["nativeOutcome"], "no_match")
            fixture.observation.capture["timed_out"] = True
            self.assertEqual(fixture.check()["nativeOutcome"], "failure")

    def test_rejected_normalization_cannot_erase_original_native_costs(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory, raw=b"\xff")
            fixture.observation.reviewed.update(normalizationStatus="rejected", disposition="error")
            result = fixture.check()
            self.assertTrue(result["normalizationRejected"])
            self.assertEqual(result["nativeOutcome"], "success")
            self.assertEqual(result["cost"]["firstPageBytes"], 1)
            fixture.observation.reviewed["normalizationStatus"] = "reviewed"
            fixture.observation.initial["normalizationStatus"] = "rejected"
            self.assertTrue(fixture.check()["normalizationRejected"])

    def test_missing_or_duplicate_rss_remains_unavailable_and_cannot_be_invented(self):
        with tempfile.TemporaryDirectory() as directory:
            for stderr in (b"no measurement", b"1 maximum resident set size\n2 maximum resident set size\n"):
                fixture = Fixture(directory)
                fixture.observation = replace(fixture.observation, stderr=stderr)
                fixture.observation.capture["stderr_bytes"] = len(stderr)
                fixture.observation.captured["peakRssBytes"] = None
                self.assertFalse(fixture.check()["measurementComplete"])
                fixture.observation.captured["peakRssBytes"] = 2048
                with self.assertRaisesRegex(ValueError, "RSS differs"):
                    fixture.check()

    def test_wrong_question_wrapper_graph_and_free_followup_arguments_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            for change in ("question", "wrapper", "graph", "followup", "environment"):
                fixture = Fixture(directory)
                argv = fixture.observation.capture["argv"]
                if change == "question": argv[argv.index(fixture.task["question"])] = "different question"
                elif change == "wrapper": argv[0] = "/different/env"
                elif change == "graph": argv[argv.index("--graph") + 1] = "/different/graph.json"
                elif change == "environment": argv.insert(2, "MODEL_API_KEY=unfrozen")
                else: argv.extend(["--cursor", "free-page"])
                with self.assertRaisesRegex(ValueError, "exact frozen native command"):
                    fixture.check()

    def test_raw_counts_rss_conditions_and_numeric_types_must_match(self):
        with tempfile.TemporaryDirectory() as directory:
            for field, value in (("stdout_bytes", 0), ("stderr_bytes", 0), ("wall_ms", True),
                    ("timed_out", 1), ("output_limited", 0), ("exit_code", False)):
                fixture = Fixture(directory)
                fixture.observation.capture[field] = value
                with self.assertRaises(ValueError): fixture.check()
            for field, value in (("peakRssBytes", 4096), ("peakRssBytes", True), ("condition", "cold_cli")):
                fixture = Fixture(directory)
                fixture.observation.captured[field] = value
                with self.assertRaises(ValueError): fixture.check()

    def test_wrong_task_or_graph_identity_and_duplicate_plan_entries_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            for change in ("task", "graph", "tools", "arms", "schema"):
                fixture = Fixture(directory)
                if change == "task": fixture.task["id"] = "foreign"
                elif change == "graph": fixture.graph["tool"] = "graphify"
                elif change == "schema": fixture.plan["schema"] = "compass.broad-query-capture-plan/1"
                else: fixture.plan[change][-1] = copy.deepcopy(fixture.plan[change][0])
                with self.assertRaises(ValueError): fixture.check()

    def test_unconfigured_ranked_view_and_changed_default_or_budget_policy_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            for change in ("ranked", "default", "overflow", "budget", "configured"):
                fixture = Fixture(directory)
                normalized = fixture.plan["arms"][1]
                if change == "ranked": normalized["compassView"] = "ranked"
                elif change == "default": fixture.plan["arms"][0]["compassView"] = "ranked"
                elif change == "overflow": normalized["overflow"] = "clip"
                elif change == "budget": normalized["budgetBytes"] = True
                else: normalized["configuredComparison"] = "true"
                with self.assertRaises(ValueError): fixture.check()


if __name__ == "__main__": unittest.main()
