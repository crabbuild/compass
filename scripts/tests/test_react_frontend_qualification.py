import importlib.util
import json
from pathlib import Path
import tempfile
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

    def test_graph_anchor_paths_are_bounded_and_portable(self) -> None:
        for path in ("src/app/page.tsx", "routes\\index.tsx"):
            self.assertTrue(MODULE.graph_path_is_safe(path))
        for path in (
            "/tmp/page.tsx",
            "../outside.tsx",
            "src/../outside.tsx",
            "C:/workspace/page.tsx",
            "https://example.test/page.tsx",
            "src/page\x00.tsx",
        ):
            self.assertFalse(MODULE.graph_path_is_safe(path))

    def test_source_anchor_matching_requires_containment(self) -> None:
        self.assertTrue(MODULE.spans_contain_either(0, 20, 4, 12))
        self.assertTrue(MODULE.spans_contain_either(4, 12, 0, 20))
        self.assertFalse(MODULE.spans_contain_either(0, 10, 5, 15))

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

    def test_pinned_projection_closes_over_root_typescript_base_config(self) -> None:
        with tempfile.TemporaryDirectory() as checkout_name, tempfile.TemporaryDirectory() as destination_name:
            checkout = Path(checkout_name).resolve()
            destination = Path(destination_name).resolve() / "projection"
            source = checkout / "playground" / "framework" / "app.tsx"
            source.parent.mkdir(parents=True)
            source.write_text("export function App() { return <main />; }\n", encoding="utf-8")
            (source.parent / "tsconfig.json").write_text(
                '{"extends":"../../tsconfig.base.json"}\n', encoding="utf-8"
            )
            (source.parent / "package.json").write_text(
                '{"private":true}\n', encoding="utf-8"
            )
            (checkout / "tsconfig.base.json").write_text(
                '{"compilerOptions":{"jsx":"react-jsx"}}\n', encoding="utf-8"
            )
            (checkout / "tsconfig.json").write_text(
                '{"include":["scripts"]}\n', encoding="utf-8"
            )
            repository = {
                "id": "projection-config-closure",
                "sourceRoot": ".",
                "sourceGlobs": ["playground/**/*"],
                "excludeGlobs": [],
            }

            files, source_files, _ = PINNED.project_repository(
                repository, checkout, destination
            )

            self.assertEqual(source_files, 1)
            self.assertEqual(files, 4)
            self.assertTrue((destination / "tsconfig.base.json").is_file())
            self.assertFalse((destination / "tsconfig.json").exists())

    def test_jsonc_extends_parser_is_comment_and_trailing_comma_aware(self) -> None:
        source = '''{
          // The word extends in a comment is not evidence.
          "extends": "../tsconfig.base",
          "compilerOptions": {"jsx": "react-jsx",},
        }'''
        self.assertEqual(
            PINNED.json.loads(PINNED.strip_jsonc(source))["extends"],
            "../tsconfig.base",
        )

    def test_expectation_policy_is_manifest_bound_and_reviewed(self) -> None:
        manifest_path = PINNED.ROOT / "tests/qualification/react-frontend-repositories.toml"
        manifest = PINNED.load_manifest(manifest_path)
        policy_path = PINNED.ROOT / manifest["expectationPolicy"]
        policy = PINNED.load_expectation_policy(
            policy_path,
            manifest,
            PINNED.digest_file(manifest_path),
        )
        self.assertEqual(policy["schema"], PINNED.EXPECTATION_POLICY_SCHEMA)
        self.assertEqual(
            {item["id"] for item in policy["repositories"]},
            {item["id"] for item in manifest["repository"]},
        )

        tampered = json.loads(policy_path.read_text(encoding="utf-8"))
        tampered["oracle"]["reviewStatus"] = "generated"
        with tempfile.NamedTemporaryFile("w", suffix=".json", encoding="utf-8") as stream:
            json.dump(tampered, stream)
            stream.flush()
            with self.assertRaises(PINNED.QualificationError):
                PINNED.load_expectation_policy(
                    Path(stream.name),
                    manifest,
                    PINNED.digest_file(manifest_path),
                )

    def test_capability_report_enforces_the_per_capability_floor(self) -> None:
        repository = {"id": "floor", "capabilities": ["react.hooks"]}
        scorecard = {"capabilities": {"react.hooks": {"expected": 99}}}

        with self.assertRaises(PINNED.QualificationError):
            PINNED.capability_report(repository, scorecard, 100)

        scorecard["capabilities"]["react.hooks"]["expected"] = 100
        report = PINNED.capability_report(repository, scorecard, 100)
        self.assertEqual(report["react.hooks"]["expected"], 100)


if __name__ == "__main__":
    unittest.main()
