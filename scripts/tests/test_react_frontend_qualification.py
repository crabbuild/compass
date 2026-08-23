import importlib.util
from pathlib import Path
import unittest


MODULE_PATH = Path(__file__).resolve().parents[1] / "qualify_react_frontend_graph.py"
SPEC = importlib.util.spec_from_file_location("qualify_react_frontend_graph", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

PINNED_PATH = Path(__file__).resolve().parents[1] / "qualify_react_frontend_pinned.py"
PINNED_SPEC = importlib.util.spec_from_file_location("qualify_react_frontend_pinned", PINNED_PATH)
assert PINNED_SPEC is not None and PINNED_SPEC.loader is not None
PINNED = importlib.util.module_from_spec(PINNED_SPEC)
PINNED_SPEC.loader.exec_module(PINNED)


class ReactFrontendQualificationTests(unittest.TestCase):
    def test_capability_matching_preserves_duplicate_occurrences(self) -> None:
        graph = {
            "nodes": [
                {"id": "source", "kind": "file", "source": {"file": "src/a.tsx", "startByte": 0, "endByte": 20}},
                {"id": "target", "kind": "component", "source": {"file": "src/Card.tsx", "startByte": 0, "endByte": 20}},
            ],
            "links": [
                {
                    "id": "render-once",
                    "kind": "renders",
                    "source": "source",
                    "target": "target",
                    "relationshipSite": {"file": "src/a.tsx", "startByte": 4, "endByte": 10},
                    "details": {"data": {"renderKind": "jsx"}},
                }
            ],
        }
        oracle = {
            "facts": [
                {
                    "id": "render-1",
                    "factType": "relationship",
                    "capability": "react.render.jsx",
                    "relation": "renders",
                    "sourceFile": "src/a.tsx",
                    "startByte": 4,
                    "endByte": 10,
                },
                {
                    "id": "render-2",
                    "factType": "relationship",
                    "capability": "react.render.jsx",
                    "relation": "renders",
                    "sourceFile": "src/a.tsx",
                    "startByte": 4,
                    "endByte": 10,
                },
            ]
        }

        scorecard = MODULE.match_source_facts(graph, oracle)

        capability = scorecard["capabilities"]["react.render.jsx"]
        self.assertEqual(capability["expected"], 2)
        self.assertEqual(capability["candidates"], 1)
        self.assertEqual(capability["matched"], 1)
        self.assertEqual(capability["falseNegatives"], 1)
        self.assertEqual(capability["recall"], 0.5)

    def test_pinned_runner_normalizes_https_and_ssh_remotes(self) -> None:
        self.assertEqual(
            PINNED.normalized_remote_url("git@github.com:TanStack/router.git"),
            "https://github.com/tanstack/router",
        )
        self.assertEqual(
            PINNED.normalized_remote_url("https://github.com/TanStack/router"),
            "https://github.com/tanstack/router",
        )

    def test_pinned_runner_enforces_command_output_bound_before_completion(self) -> None:
        original_limit = PINNED.MAX_COMMAND_OUTPUT_BYTES
        PINNED.MAX_COMMAND_OUTPUT_BYTES = 4
        try:
            with self.assertRaises(PINNED.QualificationError):
                PINNED.run_checked(["python3", "-c", "print('too much output')"], cwd=PINNED.ROOT)
        finally:
            PINNED.MAX_COMMAND_OUTPUT_BYTES = original_limit


if __name__ == "__main__":
    unittest.main()
