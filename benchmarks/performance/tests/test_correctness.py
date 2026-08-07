from __future__ import annotations

from contextlib import closing
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest

from benchmarks.performance.compass.correctness import (
    canonical_graph_digest,
    compare_graphs,
    index_graph,
)
from benchmarks.performance.compass.occurrences import (
    _typescript_payload_from_jsonl,
    _typescript_inventory_from_payload,
    independent_source_constructs,
    independent_source_inventory,
    source_construct_inventory_sha256,
)


FIXTURES = Path(__file__).parent / "fixtures"
RESOLUTION_ORACLE = (
    Path(__file__).resolve().parents[1] / "oracles" / "typescript-resolution-oracle.mjs"
)
SOURCE_ORACLE = (
    Path(__file__).resolve().parents[1] / "oracles" / "typescript-source-oracle.mjs"
)


def compare_documents(
    compass_document: str,
    graphify_document: str,
    source_documents: dict[str, str] | None = None,
):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        compass = root / "compass.json"
        graphify = root / "graphify.json"
        compass.write_text(compass_document, encoding="utf-8")
        graphify.write_text(graphify_document, encoding="utf-8")
        if source_documents is not None:
            for relative_path, source in source_documents.items():
                source_path = root / relative_path
                source_path.parent.mkdir(parents=True, exist_ok=True)
                source_path.write_text(source, encoding="utf-8")
        database = sqlite3.connect(":memory:")
        try:
            index_graph("compass", compass, database)
            index_graph("graphify", graphify, database)
            return compare_graphs(database, root if source_documents is not None else None)
        finally:
            database.close()


class CorrectnessTests(unittest.TestCase):
    def database(self) -> sqlite3.Connection:
        database = sqlite3.connect(":memory:")
        self.addCleanup(database.close)
        return database

    def test_typescript_oracle_payload_preserves_unicode_byte_ranges(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "src" / "main.ts"
            source.parent.mkdir()
            contents = "const café = 1;\nrun(café);\n"
            source.write_text(contents, encoding="utf-8")
            start = len("const ".encode("utf-8"))
            end = start + len("café".encode("utf-8"))
            payload = {
                "schema": "compass.typescript-source-oracle/1",
                "provider": "typescript_compiler_api_5_9_3",
                "metadata": {
                    "compilerVersion": "5.9.3",
                    "scriptSha256": "a" * 64,
                    "nodeVersion": "v22.0.0",
                },
                "scannedFiles": 1,
                "parsedFiles": 1,
                "rejectedFiles": [],
                "constructs": [
                    {
                        "sourceFile": "src/main.ts",
                        "relation": "references",
                        "capability": "references",
                        "ownerQualifiedName": "src.main",
                        "targetSpelling": "café",
                        "qualifier": None,
                        "startByte": start,
                        "endByte": end,
                        "startLine": 1,
                    }
                ],
            }
            inventory = _typescript_inventory_from_payload(payload, root)
            source_bytes = source.read_bytes()

        self.assertEqual(inventory.scanned_files, 1)
        self.assertEqual(inventory.parsed_files, 1)
        self.assertEqual(inventory.provider_metadata[0], ("compilerVersion", "5.9.3"))
        construct = inventory.constructs[0]
        self.assertEqual(
            source_bytes[construct.start_byte : construct.end_byte],
            "café".encode(),
        )
        self.assertEqual(
            source_construct_inventory_sha256("typescript", inventory),
            source_construct_inventory_sha256("typescript", inventory),
        )

    def test_typescript_oracle_payload_rejects_incomplete_coverage(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(RuntimeError, "coverage does not account"):
                _typescript_inventory_from_payload(
                    {
                        "schema": "compass.typescript-source-oracle/1",
                        "provider": "typescript_compiler_api_5_9_3",
                        "metadata": {
                            "compilerVersion": "5.9.3",
                            "scriptSha256": "a" * 64,
                        },
                        "scannedFiles": 2,
                        "parsedFiles": 1,
                        "rejectedFiles": [],
                        "constructs": [],
                    },
                    root,
                )

    def test_typescript_resolution_oracle_is_pinned_and_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            (root / "node_modules" / "@example" / "pkg").mkdir(parents=True)
            (root / "tsconfig.json").write_text(
                '{"compilerOptions":{"module":"NodeNext",'
                '"moduleResolution":"NodeNext"},"include":["src/**/*"]}',
                encoding="utf-8",
            )
            (root / "node_modules" / "@example" / "pkg" / "package.json").write_text(
                '{"name":"@example/pkg","exports":{".":{'
                '"import":"./import.d.ts","require":"./require.d.cts"}}}',
                encoding="utf-8",
            )
            (root / "node_modules" / "@example" / "pkg" / "import.d.ts").write_text(
                "export declare const value: string;\n",
                encoding="utf-8",
            )
            (root / "node_modules" / "@example" / "pkg" / "require.d.cts").write_text(
                "export declare const value: string;\n",
                encoding="utf-8",
            )
            (root / "src" / "importer.mts").write_text(
                'const café = "🙂";\n'
                'import { value } from "@example/pkg";\n'
                'export const imported = value + café.length;\n',
                encoding="utf-8",
            )
            (root / "src" / "consumer.cts").write_text(
                'import packageValue = require("@example/pkg");\n'
                'const { value } = require("@example/pkg");\n'
                'export = packageValue || value;\n',
                encoding="utf-8",
            )
            importer_source = (root / "src" / "importer.mts").read_bytes()
            command = ("node", str(RESOLUTION_ORACLE), "--root", str(root))
            first = subprocess.run(
                command,
                cwd=RESOLUTION_ORACLE.parents[3],
                check=False,
                capture_output=True,
                text=True,
            )
            second = subprocess.run(
                command,
                cwd=RESOLUTION_ORACLE.parents[3],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(first.returncode, 0, first.stderr)
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertEqual(first.stdout, second.stdout)
            payload = json.loads(first.stdout)
            trace = subprocess.run(
                (
                    str(RESOLUTION_ORACLE.parents[3] / "node_modules" / ".bin" / "tsc"),
                    "--project",
                    str(root / "tsconfig.json"),
                    "--traceResolution",
                    "--noEmit",
                ),
                cwd=RESOLUTION_ORACLE.parents[3],
                check=False,
                capture_output=True,
                text=True,
            )
            trace_output = f"{trace.stdout}\n{trace.stderr}"
            self.assertIn("Resolving module '@example/pkg'", trace_output)
            self.assertIn("import.d.ts", trace_output)
            self.assertIn("require.d.cts", trace_output)

        self.assertEqual(payload["schema"], "compass.typescript-resolution-oracle/1")
        self.assertEqual(payload["provider"], "typescript_compiler_api_5_9_3")
        self.assertEqual(payload["scannedFiles"], 2)
        self.assertEqual(payload["parsedFiles"], 2)
        self.assertEqual(payload["rejectedFiles"], [])
        resolutions = {
            (item["sourceFile"], item["context"]): item for item in payload["resolutions"]
        }
        self.assertEqual(
            resolutions[("src/importer.mts", "import")]["targetFile"],
            "node_modules/@example/pkg/import.d.ts",
        )
        importer_resolution = resolutions[("src/importer.mts", "import")]
        self.assertEqual(
            importer_source[
                importer_resolution["startByte"] : importer_resolution["endByte"]
            ],
            b'"@example/pkg"',
        )
        self.assertEqual(importer_resolution["startLine"], 2)
        self.assertEqual(
            resolutions[("src/consumer.cts", "require")]["targetFile"],
            "node_modules/@example/pkg/require.d.cts",
        )
        self.assertTrue(payload["metadata"]["configDigest"])
        self.assertRegex(payload["metadata"]["sourceDigest"], r"^[0-9a-f]{64}$")

    def test_typescript_source_oracle_honors_project_boundaries_and_references(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            (root / "packages" / "lib" / "src").mkdir(parents=True)
            (root / "tsconfig.base.json").write_text(
                json.dumps({"compilerOptions": {"strict": True}}),
                encoding="utf-8",
            )
            (root / "tsconfig.json").write_text(
                json.dumps(
                    {
                        "compilerOptions": {
                            "composite": True,
                            "module": "NodeNext",
                            "moduleResolution": "NodeNext",
                            "target": "ES2022",
                        },
                        "include": ["src/**/*"],
                        "exclude": ["src/excluded.ts"],
                        "references": [{"path": "packages/lib"}],
                    }
                ),
                encoding="utf-8",
            )
            (root / "packages" / "lib" / "tsconfig.json").write_text(
                json.dumps(
                    {
                        "compilerOptions": {"composite": True, "target": "ES2022"},
                        "include": ["src/**/*"],
                    }
                ),
                encoding="utf-8",
            )
            (root / "src" / "main.ts").write_text(
                "const café = '🙂';\nrun(café);\n",
                encoding="utf-8",
            )
            (root / "src" / "bad.ts").write_text("const = ;\n", encoding="utf-8")
            (root / "src" / "excluded.ts").write_text("ignored();\n", encoding="utf-8")
            (root / "outside.ts").write_text("notInAProject();\n", encoding="utf-8")
            (root / "packages" / "lib" / "src" / "lib.ts").write_text(
                "export const library = 1;\n",
                encoding="utf-8",
            )
            command = ("node", str(SOURCE_ORACLE), "--root", str(root))
            first = subprocess.run(
                command,
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
                text=True,
            )
            second = subprocess.run(
                command,
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(first.returncode, 0, first.stderr)
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertEqual(first.stdout, second.stdout)
            payload = json.loads(first.stdout)
            source_bytes = (root / "src" / "main.ts").read_bytes()

        self.assertEqual(payload["scannedFiles"], 3)
        self.assertEqual(payload["parsedFiles"], 2)
        self.assertEqual(payload["rejectedFiles"], ["src/bad.ts"])
        self.assertEqual(payload["metadata"]["projectMode"], "project")
        self.assertRegex(payload["metadata"]["configDigest"], r"^[0-9a-f]{64}$")
        self.assertRegex(payload["metadata"]["sourceDigest"], r"^[0-9a-f]{64}$")
        self.assertEqual(
            [project["configFile"] for project in payload["projects"]],
            ["packages/lib/tsconfig.json", "tsconfig.json"],
        )
        self.assertEqual(
            payload["projects"][1]["references"], ["packages/lib/tsconfig.json"]
        )
        self.assertEqual(
            sorted(file_name for project in payload["projects"] for file_name in project["files"]),
            ["packages/lib/src/lib.ts", "src/bad.ts", "src/main.ts"],
        )
        self.assertTrue(
            any(diagnostic["file"] == "src/bad.ts" for diagnostic in payload["diagnostics"])
        )
        self.assertFalse(any(construct["sourceFile"] == "outside.ts" for construct in payload["constructs"]))
        cafe_construct = next(
            construct
            for construct in payload["constructs"]
            if construct["sourceFile"] == "src/main.ts"
            and construct["targetSpelling"] == "café"
        )
        self.assertEqual(
            source_bytes[cafe_construct["startByte"] : cafe_construct["endByte"]],
            "café".encode(),
        )

    def test_typescript_source_oracle_jsonl_is_complete_and_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            (root / "src" / "main.ts").write_text(
                'export function run(café: string): string { return café; }\n'
                'const result = run(café);\n',
                encoding="utf-8",
            )
            (root / "src" / "bad.ts").write_text("const = ;\n", encoding="utf-8")
            command = ("node", str(SOURCE_ORACLE), "--root", str(root), "--jsonl")
            first = subprocess.run(
                command,
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
            )
            second = subprocess.run(
                command,
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
            )
            self.assertEqual(first.returncode, 0, first.stderr.decode())
            self.assertEqual(second.returncode, 0, second.stderr.decode())
            self.assertEqual(first.stdout, second.stdout)
            lines = first.stdout.splitlines()
            self.assertGreaterEqual(len(lines), 2)
            self.assertEqual(json.loads(lines[0])["recordType"], "header")
            self.assertEqual(json.loads(lines[-1])["recordType"], "footer")
            payload = _typescript_payload_from_jsonl(first.stdout)
            inventory = _typescript_inventory_from_payload(payload, root)
            provider_inventory = independent_source_inventory(root, "typescript")
            source_bytes = (root / "src" / "main.ts").read_bytes()
            tampered_records = [json.loads(line) for line in lines]
            tampered_records[-1]["callCount"] += 1
            with self.assertRaisesRegex(RuntimeError, "footer count callCount"):
                _typescript_payload_from_jsonl(
                    "\n".join(json.dumps(record) for record in tampered_records).encode()
                )
            payload["scopes"][1]["parentScopeId"] = "missing-scope"
            with self.assertRaisesRegex(RuntimeError, "scope parent"):
                _typescript_inventory_from_payload(payload, root)

        self.assertEqual(inventory.scanned_files, 2)
        self.assertEqual(inventory.parsed_files, 1)
        self.assertEqual(inventory.rejected_files, ("src/bad.ts",))
        self.assertEqual(provider_inventory, inventory)
        header = json.loads(lines[0])
        footer = json.loads(lines[-1])
        self.assertEqual(footer["constructCount"], len(inventory.constructs))
        self.assertEqual(footer["scannedFiles"], inventory.scanned_files)
        self.assertEqual(header["scopeCount"], len(payload["scopes"]))
        self.assertEqual(header["declarationCount"], len(payload["declarations"]))
        self.assertEqual(header["callCount"], len(payload["calls"]))
        self.assertGreaterEqual(len(payload["scopes"]), 2)
        self.assertTrue(any(scope["kind"] == "module" for scope in payload["scopes"]))
        run = next(
            declaration
            for declaration in payload["declarations"]
            if declaration["name"] == "run"
        )
        self.assertEqual(run["kind"], "function")
        self.assertEqual(run["parameterCount"], 1)
        cafe = next(
            declaration
            for declaration in payload["declarations"]
            if declaration["name"] == "café"
        )
        self.assertEqual(source_bytes[cafe["startByte"] : cafe["endByte"]], "café".encode())
        call = next(call for call in payload["calls"] if call["targetSpelling"] == "run")
        self.assertEqual(source_bytes[call["startByte"] : call["endByte"]], b"run")
        self.assertEqual(
            source_bytes[call["callStartByte"] : call["callEndByte"]],
            "run(café)".encode(),
        )

    def test_typescript_source_oracle_jsonl_exposes_typed_relationship_records(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            (root / "src" / "base.ts").write_text(
                "export default class Default { base(): void {} }\n"
                "export const imported = () => 1;\n"
                "export interface Runnable { run(): void }\n",
                encoding="utf-8",
            )
            (root / "src" / "main.tsx").write_text(
                'import Default, { imported, type Runnable as RunType } from "./base";\n'
                'export { imported as exported };\n'
                'export * from "./base";\n'
                'class Child extends Default implements RunType {\n'
                '  field = imported;\n'
                '  run(): void {\n'
                '    const obj = { literal: imported };\n'
                '    obj.literal = imported;\n'
                '    obj["literal"];\n'
                '    new Default();\n'
                '    imported();\n'
                '    return <Default />;\n'
                '  }\n'
                '}\n'
                'export default Child;\n',
                encoding="utf-8",
            )
            command = ("node", str(SOURCE_ORACLE), "--root", str(root), "--jsonl")
            result = subprocess.run(
                command,
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            payload = _typescript_payload_from_jsonl(result.stdout)
            _typescript_inventory_from_payload(payload, root)
            source_sizes = {
                file_name: (root / file_name).stat().st_size
                for file_name in {record["sourceFile"] for field in ("imports", "reexports", "constructions", "bases", "members", "references") for record in payload[field]}
            }
        self.assertGreaterEqual(len(payload["imports"]), 3)
        self.assertIn("named", {item["kind"] for item in payload["imports"]})
        self.assertIn("default", {item["kind"] for item in payload["imports"]})
        self.assertTrue(any(item["isTypeOnly"] for item in payload["imports"]))
        self.assertGreaterEqual(len(payload["reexports"]), 3)
        self.assertIn("star", {item["kind"] for item in payload["reexports"]})
        self.assertIn("default", {item["kind"] for item in payload["reexports"]})
        self.assertTrue(payload["constructions"])
        self.assertTrue(all(item["relation"] == "instantiates" for item in payload["constructions"]))
        self.assertIn("extends", {item["relation"] for item in payload["bases"]})
        self.assertIn("implements", {item["relation"] for item in payload["bases"]})
        self.assertIn("property", {item["kind"] for item in payload["members"]})
        self.assertIn("computed_literal", {item["kind"] for item in payload["members"]})
        self.assertIn("write", {item["accessKind"] for item in payload["members"]})
        self.assertIn("jsx", {item["kind"] for item in payload["references"]})
        for field in (
            "imports",
            "reexports",
            "constructions",
            "bases",
            "members",
            "references",
        ):
            for record in payload[field]:
                source_size = source_sizes[record["sourceFile"]]
                self.assertLessEqual(record["endByte"], source_size)
                if "statementEndByte" in record:
                    self.assertLessEqual(record["statementEndByte"], source_size)
                if "callEndByte" in record:
                    self.assertLessEqual(record["callEndByte"], source_size)

    def test_typescript_source_oracle_records_decorator_occurrences_without_factory_calls(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            source = (
                'function nested(): string { return "ok"; }\n'
                '@Controller({ path: nested() })\n'
                'class ControllerHost {}\n'
                '@unknownFactory()\n'
                'class DynamicHost {}\n'
            )
            (root / "src" / "decorators.ts").write_text(source, encoding="utf-8")
            result = subprocess.run(
                ("node", str(SOURCE_ORACLE), "--root", str(root), "--jsonl"),
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            payload = _typescript_payload_from_jsonl(result.stdout)

        decorators = [
            construct
            for construct in payload["constructs"]
            if construct["relation"] == "decorates"
        ]
        self.assertEqual(len(decorators), 2)
        self.assertEqual({item["capability"] for item in decorators}, {"decorators"})
        self.assertEqual(
            {item["targetSpelling"] for item in decorators},
            {"Controller", "unknownFactory"},
        )
        source_bytes = source.encode("utf-8")
        controller = next(item for item in decorators if item["targetSpelling"] == "Controller")
        self.assertEqual(
            source_bytes[controller["startByte"] : controller["endByte"]],
            b"Controller",
        )
        self.assertFalse(
            any(
                call["targetSpelling"] in {"Controller", "unknownFactory"}
                for call in payload["calls"]
            )
        )
        self.assertTrue(any(call["targetSpelling"] == "nested" for call in payload["calls"]))

    def test_typescript_source_oracle_records_jsx_value_and_spread_references(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = (
                'const title = "Hello";\n'
                'const props = { title };\n'
                'function handle(label: string): void {}\n'
                'export function View(label: string) {\n'
                '  return <Button title={title} onClick={() => handle(label)} '
                'data={user.name} {...props}>{title}</Button>;\n'
                '}\n'
            )
            (root / "view.tsx").write_text(source, encoding="utf-8")
            result = subprocess.run(
                ("node", str(SOURCE_ORACLE), "--root", str(root), "--jsonl"),
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            payload = _typescript_payload_from_jsonl(result.stdout)

        jsx_values = [
            construct
            for construct in payload["constructs"]
            if construct["relation"] == "references"
            and construct["capability"] == "jsx_values"
        ]
        self.assertGreaterEqual(len(jsx_values), 5)
        self.assertEqual(
            {construct["targetSpelling"] for construct in jsx_values},
            {"title", "label", "props", "user"},
        )
        self.assertEqual(
            {reference["kind"] for reference in payload["references"] if reference["kind"] in {
                "jsx_value", "jsx_spread", "jsx_child"
            }},
            {"jsx_value", "jsx_spread", "jsx_child"},
        )
        source_bytes = source.encode("utf-8")
        for construct in jsx_values:
            self.assertEqual(
                source_bytes[construct["startByte"] : construct["endByte"]],
                construct["targetSpelling"].encode("utf-8"),
            )
        self.assertFalse(any(construct["targetSpelling"] == "onClick" for construct in jsx_values))

    def test_typescript_source_oracle_preserves_using_resource_declarations(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = (
                "declare function acquire(): Disposable;\n"
                "declare function acquireAsync(): Promise<Disposable>;\n"
                "function run() {\n"
                "  using resource = acquire();\n"
                "  await using asyncResource = acquireAsync();\n"
                "  resource.close();\n"
                "  asyncResource.close();\n"
                "}\n"
            )
            (root / "resources.ts").write_text(source, encoding="utf-8")
            result = subprocess.run(
                ("node", str(SOURCE_ORACLE), "--root", str(root), "--jsonl"),
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            payload = _typescript_payload_from_jsonl(result.stdout)

        resources = {
            declaration["name"]
            for declaration in payload["declarations"]
            if declaration["name"] in {"resource", "asyncResource"}
        }
        self.assertEqual(resources, {"resource", "asyncResource"})
        self.assertTrue(
            all(
                call["targetSpelling"] in {"acquire", "acquireAsync", "close"}
                for call in payload["calls"]
            )
        )
        self.assertIn("acquire", {call["targetSpelling"] for call in payload["calls"]})
        self.assertIn("acquireAsync", {call["targetSpelling"] for call in payload["calls"]})

    def test_typescript_source_oracle_reports_invalid_config_and_follows_cycles_once(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            (root / "tsconfig.json").write_text("{ invalid", encoding="utf-8")
            (root / "src" / "main.ts").write_text("run();\n", encoding="utf-8")
            fallback = subprocess.run(
                ("node", str(SOURCE_ORACLE), "--root", str(root)),
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(fallback.returncode, 0, fallback.stderr)
            fallback_payload = json.loads(fallback.stdout)
            self.assertEqual(fallback_payload["metadata"]["projectMode"], "fallback")
            self.assertEqual(fallback_payload["scannedFiles"], 1)
            self.assertEqual(fallback_payload["parsedFiles"], 1)
            self.assertEqual(
                [diagnostic["file"] for diagnostic in fallback_payload["diagnostics"]],
                ["tsconfig.json"],
            )

            (root / "tsconfig.json").unlink()
            (root / "a").mkdir()
            (root / "b").mkdir()
            (root / "a" / "tsconfig.json").write_text(
                json.dumps(
                    {
                        "compilerOptions": {"composite": True},
                        "include": ["src/**/*"],
                        "references": [{"path": "../b"}],
                    }
                ),
                encoding="utf-8",
            )
            (root / "b" / "tsconfig.json").write_text(
                json.dumps(
                    {
                        "compilerOptions": {"composite": True},
                        "include": ["src/**/*"],
                        "references": [{"path": "../a"}],
                    }
                ),
                encoding="utf-8",
            )
            (root / "a" / "src").mkdir()
            (root / "b" / "src").mkdir()
            (root / "a" / "src" / "a.ts").write_text("export const a = 1;\n", encoding="utf-8")
            (root / "b" / "src" / "b.ts").write_text("export const b = 1;\n", encoding="utf-8")
            cycled = subprocess.run(
                ("node", str(SOURCE_ORACLE), "--root", str(root)),
                cwd=SOURCE_ORACLE.parents[3],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(cycled.returncode, 0, cycled.stderr)
            cycled_payload = json.loads(cycled.stdout)

        self.assertEqual(cycled_payload["metadata"]["projectMode"], "project")
        self.assertEqual(
            [project["configFile"] for project in cycled_payload["projects"]],
            ["a/tsconfig.json", "b/tsconfig.json"],
        )
        self.assertEqual(cycled_payload["scannedFiles"], 2)
        self.assertEqual(cycled_payload["parsedFiles"], 2)

    def test_compass_superset_passes_shared_fact_comparison(self) -> None:
        database = self.database()
        compass = index_graph("compass", FIXTURES / "compass_graph.json", database)
        graphify = index_graph("graphify", FIXTURES / "graphify_graph.json", database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)
        self.assertGreater(compass.nodes, graphify.nodes)
        self.assertEqual(compass.digest, canonical_graph_digest(database, "compass"))
        self.assertEqual(result.digest, compare_graphs(database).digest)

    def test_storage_order_does_not_change_digest(self) -> None:
        database = self.database()
        first = index_graph("compass", FIXTURES / "compass_graph.json", database)
        with tempfile.TemporaryDirectory() as directory:
            reordered = Path(directory) / "graph.json"
            reordered.write_text(
                """
                {"links":[
                  {"relation":"routes_to","target":"a","source":"c","confidence":"EXTRACTED"},
                  {"relation":"calls","target":"b","source":"a","confidence":"EXTRACTED"}
                ],"nodes":[
                  {"source_location":"L3","source_file":"src/c.py","kind":"route","label":"CompassOnly","id":"c"},
                  {"source_location":"L2","source_file":"src/b.py","kind":"function","label":"Beta","id":"b"},
                  {"source_location":"L1","source_file":"src/a.py","kind":"function","label":"Alpha","id":"a"}
                ],"graph":{"schema":"compass.graph/1","diagnostics":[]}}
                """,
                encoding="utf-8",
            )
            second = index_graph("compass", reordered, database)
        self.assertEqual(first.digest, second.digest)

    def test_anchor_distinct_same_line_edges_are_retained(self) -> None:
        database = self.database()
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "graph.json"
            graph.write_text(
                '{"graph":{"diagnostics":[]},"nodes":['
                '{"id":"source","label":"run()","kind":"function",'
                '"source_file":"main.py","source_location":"L1"},'
                '{"id":"target","label":"target()","kind":"function",'
                '"source_file":"main.py","source_location":"L1"}],"links":['
                '{"source":"source","target":"target","relation":"calls",'
                '"relationshipSite":{"file":"main.py","startLine":1,'
                '"startByte":0,"endByte":8}},'
                '{"source":"source","target":"target","relation":"calls",'
                '"relationshipSite":{"file":"main.py","startLine":1,'
                '"startByte":10,"endByte":18}}]}',
                encoding="utf-8",
            )
            summary = index_graph("compass", graph, database)

        self.assertEqual(summary.edges, 2)
        self.assertEqual(
            database.execute(
                "SELECT occurrence_start_byte,occurrence_end_byte FROM edges "
                "WHERE tool = 'compass' ORDER BY occurrence_start_byte"
            ).fetchall(),
            [(0, 8), (10, 18)],
        )

    def test_source_oracle_rejects_escaped_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "corpus"
            outside = Path(directory) / "outside.py"
            root.mkdir()
            outside.write_text("target()\n", encoding="utf-8")
            escaped = root / "escaped.py"
            try:
                escaped.symlink_to(outside)
            except (OSError, NotImplementedError) as error:
                self.skipTest(f"symlinks unavailable: {error}")

            inventory = independent_source_inventory(root, "python")

        self.assertEqual(inventory.scanned_files, 0)
        self.assertEqual(inventory.parsed_files, 0)
        self.assertEqual(inventory.rejected_files, ("escaped.py",))

    def test_missing_shared_node_fails(self) -> None:
        database = self.database()
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "compass.json"
            graph.write_text(
                '{"graph":{},"nodes":[{"id":"a","label":"Alpha","kind":"function",'
                '"source_file":"src/a.py","source_location":"L1"}],"links":[]}',
                encoding="utf-8",
            )
            index_graph("compass", graph, database)
        index_graph("graphify", FIXTURES / "graphify_graph.json", database)
        result = compare_graphs(database)
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 1)

    def test_v1_compass_nodes_match_graphify_by_source_fact_not_internal_id(self) -> None:
        database = self.database()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compass = root / "compass.json"
            graphify = root / "graphify.json"
            compass.write_text(
                """
                {
                  "graph":{"schema":"compass.code-graph/1","diagnostics":[]},
                  "nodes":[
                    {"id":"sha256:file","kind":"file","name":"base.py",
                     "source":{"file":"src/base.py","startLine":1}},
                    {"id":"sha256:function","kind":"function","name":"run",
                     "source":{"file":"src/base.py","startLine":12}}
                  ],
                  "edges":[
                    {"source":"sha256:file","target":"sha256:function","kind":"contains"}
                  ]
                }
                """,
                encoding="utf-8",
            )
            graphify.write_text(
                """
                {
                  "nodes":[
                    {"id":"src_base","label":"src/base.py","source_file":"src/base.py",
                     "source_location":"L1"},
                    {"id":"src_base_run","label":"run()","source_file":"src/base.py",
                     "source_location":"L12"}
                  ],
                  "links":[
                    {"source":"src_base","target":"src_base_run","relation":"contains"}
                  ]
                }
                """,
                encoding="utf-8",
            )
            index_graph("compass", compass, database)
            index_graph("graphify", graphify, database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 0)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)

    def test_shared_relation_projection_accepts_more_precise_compass_edges(self) -> None:
        database = self.database()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compass = root / "compass.json"
            graphify = root / "graphify.json"
            nodes = (
                '{"id":"source","label":"Source","source_file":"src/a.py","source_location":"L1"},'
                '{"id":"target","label":"Target","source_file":"src/b.py","source_location":"L2"}'
            )
            compass.write_text(
                '{"graph":{"diagnostics":[]},"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target","relation":"instantiates"}]}',
                encoding="utf-8",
            )
            graphify.write_text(
                '{"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target","relation":"calls"}]}',
                encoding="utf-8",
            )
            index_graph("compass", compass, database)
            index_graph("graphify", graphify, database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)

    def test_graphify_case_of_matches_canonical_compass_containment(self) -> None:
        nodes = (
            '{"id":"enum","label":"Policy","kind":"enum",'
            '"source_file":"Policy.java","source_location":"L1"},'
            '{"id":"member","label":"ALLOW","kind":"enum_member",'
            '"source_file":"Policy.java","source_location":"L2"}'
        )
        result = compare_documents(
            '{"graph":{"diagnostics":[]},"nodes":['
            + nodes
            + '],"links":[{"source":"enum","target":"member",'
            '"relation":"contains","source_file":"Policy.java",'
            '"source_location":"L2"}]}',
            '{"nodes":['
            + nodes
            + '],"links":[{"source":"enum","target":"member",'
            '"relation":"case_of","source_file":"Policy.java",'
            '"source_location":"L2"}]}',
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["exact_graphify_edges"], 1)

    def test_multiline_python_imports_match_only_with_same_statement_evidence(self) -> None:
        nodes = """
          {"id":"source","label":"module.py","kind":"file",
           "source_file":"pkg/module.py","source_location":"L1","language":"python"},
          {"id":"target","label":"Target","kind":"type_alias","language":"python"}
        """
        compass = (
            '{"graph":{"diagnostics":[]},"nodes":['
            + nodes
            + '],"links":[{"source":"source","target":"target","relation":"imports",'
            '"source_file":"pkg/module.py","source_location":"L2"}]}'
        )
        graphify = (
            '{"nodes":['
            + nodes
            + '],"links":[{"source":"source","target":"target","relation":"imports",'
            '"source_file":"pkg/module.py","source_location":"L1"}]}'
        )
        source = {"pkg/module.py": "from package import (\n    Target,\n)\n"}

        strict = compare_documents(compass, graphify)
        self.assertEqual(strict.metrics["missing_graphify_edges"], 1)

        proven = compare_documents(compass, graphify, source)
        self.assertTrue(proven.passed, proven.failures)
        self.assertEqual(proven.metrics["exact_graphify_edges"], 0)
        self.assertEqual(proven.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            '"dominated:source_statement_occurrence":1',
            proven.metrics["graphify_edges_coverage_reasons"],
        )

    def test_occurrence_oracle_rejects_different_statements_and_invalid_sources(self) -> None:
        def result_for(source_file: str, compass_line: int, source: str):
            nodes = (
                f'{{"id":"source","label":"module","kind":"file",'
                f'"source_file":"{source_file}","source_location":"L1",'
                '"language":"python"},'
                '{"id":"target","label":"Target","kind":"type_alias",'
                '"language":"python"}'
            )
            compass = (
                '{"graph":{"diagnostics":[]},"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target",'
                f'"relation":"imports","source_file":"{source_file}",'
                f'"source_location":"L{compass_line}"}}]}}'
            )
            graphify = (
                '{"nodes":['
                + nodes
                + '],"links":[{"source":"source","target":"target",'
                f'"relation":"imports","source_file":"{source_file}",'
                '"source_location":"L1"}]}'
            )
            return compare_documents(compass, graphify, {source_file: source})

        different = result_for(
            "pkg/module.py",
            2,
            "from package import Target\nfrom package import Target\n",
        )
        self.assertEqual(different.metrics["missing_graphify_edges"], 1)

        malformed = result_for(
            "pkg/module.py",
            2,
            "from package import (\n    Target,\n",
        )
        self.assertEqual(malformed.metrics["missing_graphify_edges"], 1)

        unsupported = result_for(
            "pkg/module.java",
            2,
            "import package.\n    Target;\n",
        )
        self.assertEqual(unsupported.metrics["missing_graphify_edges"], 1)

    def test_occurrence_oracle_does_not_merge_nested_calls(self) -> None:
        nodes = """
          {"id":"source","label":"run","kind":"function",
           "source_file":"pkg/module.py","source_location":"L1","language":"python"},
          {"id":"target","label":"target","kind":"function",
           "source_file":"pkg/target.py","source_location":"L1","language":"python"}
        """
        compass = (
            '{"graph":{"diagnostics":[]},"nodes":['
            + nodes
            + '],"links":[{"source":"source","target":"target","relation":"calls",'
            '"source_file":"pkg/module.py","source_location":"L2"}]}'
        )
        graphify = (
            '{"nodes":['
            + nodes
            + '],"links":[{"source":"source","target":"target","relation":"calls",'
            '"source_file":"pkg/module.py","source_location":"L1"}]}'
        )
        result = compare_documents(
            compass,
            graphify,
            {"pkg/module.py": "target(\n    target()\n)\n"},
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 1)

    def test_python_source_inventory_includes_definition_time_calls(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "sample.py"
            source.write_text(
                """
@class_decorator(class_option())
class Widget(base_factory(), metaclass=meta_factory()):
    @method_decorator(method_option())
    def run(
        self,
        value: annotation_factory() = default_factory(),
    ) -> return_factory():
        body_call()
""".lstrip(),
                encoding="utf-8",
            )

            constructs = independent_source_constructs(root, "python")

        calls = {
            (construct.owner_qualified_name, construct.target_spelling)
            for construct in constructs
            if construct.relation == "calls"
        }
        self.assertEqual(
            calls,
            {
                ("sample", "base_factory"),
                ("sample", "class_decorator"),
                ("sample", "class_option"),
                ("sample", "meta_factory"),
                ("sample.Widget", "annotation_factory"),
                ("sample.Widget", "default_factory"),
                ("sample.Widget", "method_decorator"),
                ("sample.Widget", "method_option"),
                ("sample.Widget", "return_factory"),
                ("sample.Widget.run", "body_call"),
            },
        )

    def test_rationale_facts_match_by_source_anchor_across_schema_names(self) -> None:
        database = self.database()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compass = root / "compass.json"
            graphify = root / "graphify.json"
            compass.write_text(
                """
                {"graph":{"diagnostics":[]},"nodes":[
                  {"id":"r","kind":"resource","name":"Long rationale without ellipsis",
                   "source":{"file":"src/a.py","startLine":9},
                   "details":{"type":"resource","data":{"resourceKind":"rationale"}}},
                  {"id":"f","kind":"function","name":"run",
                   "source":{"file":"src/a.py","startLine":10}}
                ],"edges":[{"source":"r","target":"f","kind":"documents"}]}
                """,
                encoding="utf-8",
            )
            graphify.write_text(
                """
                {"nodes":[
                  {"id":"legacy_r","label":"Long rationale…","file_type":"rationale",
                   "source_file":"src/a.py","source_location":"L9"},
                  {"id":"legacy_f","label":"run()","source_file":"src/a.py","source_location":"L10"}
                ],"links":[{"source":"legacy_r","target":"legacy_f",
                            "relation":"rationale_for"}]}
                """,
                encoding="utf-8",
            )
            index_graph("compass", compass, database)
            index_graph("graphify", graphify, database)
        result = compare_graphs(database)
        self.assertTrue(result.passed, result.failures)

    def test_dangling_edge_is_rejected(self) -> None:
        database = self.database()
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "graph.json"
            graph.write_text(
                '{"graph":{},"nodes":[{"id":"a"}],'
                '"links":[{"source":"a","target":"missing","relation":"calls"}]}',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "dangling"):
                index_graph("compass", graph, database)

    def test_conflicting_duplicate_id_is_rejected(self) -> None:
        database = self.database()
        with tempfile.TemporaryDirectory() as directory:
            graph = Path(directory) / "graph.json"
            graph.write_text(
                '{"graph":{},"nodes":[{"id":"a","label":"One"},{"id":"a","label":"Two"}],'
                '"links":[]}',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "conflicting"):
                index_graph("compass", graph, database)

    def test_unique_generated_receiver_and_occurrence_edge_are_dominated(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"caller","label":"run()","kind":"function",
               "source_file":"pkg/call.go","source_location":"L5","language":"go"},
              {"id":"type","label":"Widget","kind":"class",
               "source_file":"pkg/schema.go","source_location":"L1","language":"go"},
              {"id":"stub","label":"Widget","kind":"type_alias","language":"go"}
            ],"links":[
              {"source":"caller","target":"stub","relation":"references",
               "source_file":"pkg/call.go","source_location":"L9"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_caller","label":"run()",
               "source_file":"pkg/call.go","source_location":"L5"},
              {"id":"generated_receiver","label":"Widget",
               "source_file":"pkg/generated.go","source_location":"L20"}
            ],"links":[
              {"source":"legacy_caller","target":"generated_receiver","relation":"uses",
               "source_file":"pkg/call.go","source_location":"L9"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_nodes"], 1)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)

    def test_unqualified_placeholder_cannot_choose_a_cross_package_definition(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"left","label":"Agent","kind":"class",
               "source_file":"pkg/left/agent.go","source_location":"L1","language":"go"},
              {"id":"right","label":"Agent","kind":"class",
               "source_file":"pkg/right/agent.go","source_location":"L1","language":"go"}
            ],"links":[]}
            """,
            """
            {"nodes":[{"id":"receiver","label":"Agent"}],"links":[]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
        self.assertEqual(result.metrics["ambiguous_graphify_nodes"], 0)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 0)
        self.assertIn(
            "rejected:unverifiable_placeholder",
            result.metrics["graphify_nodes_coverage_reasons"],
        )

    def test_unqualified_placeholder_cannot_bind_to_a_value_or_module(self) -> None:
        for kind in ("field", "module"):
            with self.subTest(kind=kind):
                result = compare_documents(
                    f"""
                    {{"graph":{{"diagnostics":[]}},"nodes":[
                      {{"id":"result","label":"result","kind":"{kind}",
                       "source_file":"src/lib.rs","source_location":"L8",
                       "language":"rust"}}
                    ],"links":[]}}
                    """,
                    """
                    {"nodes":[
                      {"id":"src_build_rs_result","label":"Result"}
                    ],"links":[]}
                    """,
                )
                self.assertTrue(result.passed, result.failures)
                self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
                self.assertEqual(result.metrics["dominated_graphify_nodes"], 0)
                self.assertIn(
                    "rejected:unverifiable_placeholder",
                    result.metrics["graphify_nodes_coverage_reasons"],
                )

    def test_case_exact_generated_owner_disambiguates_case_distinct_types(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"exported","label":"EphemeralStore","kind":"interface",
               "source_file":"pkg/checkpoint/api.go","source_location":"L10",
               "language":"go"},
              {"id":"private","label":"ephemeralStore","kind":"struct",
               "source_file":"pkg/checkpoint/store.go","source_location":"L20",
               "language":"go"}
            ],"links":[]}
            """,
            """
            {"nodes":[
              {"id":"pkg_checkpoint_generated_ephemeralstore",
               "label":"ephemeralStore",
               "source_file":"pkg/checkpoint/write.go","source_location":"L30"}
            ],"links":[]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_nodes"], 1)
        self.assertIn(
            "dominated:case_exact_owner",
            result.metrics["graphify_nodes_coverage_reasons"],
        )

    def test_rust_generic_parameter_remains_missing_instead_of_binding_an_unrelated_alias(
        self,
    ) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"field","label":"i","kind":"field",
               "source_file":"src/iter/chunks.rs","source_location":"L12",
               "language":"rust"},
              {"id":"alias","label":"I","kind":"type_alias",
               "source_file":"src/iter/test.rs","source_location":"L1918",
               "language":"rust"}
            ],"links":[]}
            """,
            """
            {"nodes":[
              {"id":"src_iter_mod_i","label":"I",
               "source_file":"src/iter/mod.rs","source_location":"L290",
               "language":"rust"}
            ],"links":[]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 1)
        self.assertEqual(result.metrics["dominated_graphify_nodes"], 0)
        self.assertIn(
            "missing:no_compatible_anchored_definition",
            result.metrics["graphify_nodes_coverage_reasons"],
        )

    def test_rust_blanket_impl_uses_the_occurrence_scoped_parameter_owner(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"first_i","label":"I","kind":"parameter",
               "qualified_name":"<impl First for I>::<I>",
               "source_file":"src/iter.rs","source_location":"L10","language":"rust"},
              {"id":"second_i","label":"I","kind":"parameter",
               "qualified_name":"<impl Second for I>::<I>",
               "source_file":"src/iter.rs","source_location":"L20","language":"rust"},
              {"id":"first","label":"First","kind":"trait",
               "qualified_name":"crate::First",
               "source_file":"src/iter.rs","source_location":"L1","language":"rust"},
              {"id":"second","label":"Second","kind":"trait",
               "qualified_name":"crate::Second",
               "source_file":"src/iter.rs","source_location":"L2","language":"rust"}
            ],"links":[
              {"source":"first_i","target":"first","relation":"implements",
               "source_file":"src/iter.rs","source_location":"L10"},
              {"source":"second_i","target":"second","relation":"implements",
               "source_file":"src/iter.rs","source_location":"L20"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"merged_i","label":"I","kind":"parameter",
               "source_file":"src/iter.rs","source_location":"L10","language":"rust"},
              {"id":"first","label":"First","kind":"trait",
               "source_file":"src/iter.rs","source_location":"L1","language":"rust"},
              {"id":"second","label":"Second","kind":"trait",
               "source_file":"src/iter.rs","source_location":"L2","language":"rust"}
            ],"links":[
              {"source":"merged_i","target":"first","relation":"implements",
               "source_file":"src/iter.rs","source_location":"L10"},
              {"source":"merged_i","target":"second","relation":"implements",
               "source_file":"src/iter.rs","source_location":"L20"}
            ]}
            """,
        )
        self.assertEqual(result.metrics["exact_graphify_edges"], 1)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:precise_rust_blanket_impl_owner",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_embedding_relation_requires_first_class_compass_embedding(self) -> None:
        graphify = """
            {"nodes":[
              {"id":"owner","label":"Owner","source_file":"pkg/a.go","source_location":"L1"},
              {"id":"target","label":"Target","source_file":"pkg/a.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"embeds",
               "source_file":"pkg/a.go","source_location":"L3"}
            ]}
        """
        collapsed = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Owner","kind":"struct",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"target","label":"Target","kind":"interface",
               "source_file":"pkg/a.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"contains",
               "source_file":"pkg/a.go","source_location":"L3"}
            ]}
            """,
            graphify,
        )
        self.assertFalse(collapsed.passed)
        self.assertEqual(collapsed.metrics["missing_graphify_edges"], 1)

        preserved = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Owner","kind":"struct",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"target","label":"Target","kind":"interface",
               "source_file":"pkg/a.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"embeds",
               "source_file":"pkg/a.go","source_location":"L3"}
            ]}
            """,
            graphify,
        )
        self.assertTrue(preserved.passed, preserved.failures)

    def test_generated_receiver_id_disambiguates_an_exact_module(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"left","label":"Agent","kind":"class",
               "source_file":"pkg/left/agent.go","source_location":"L1","language":"go"},
              {"id":"right","label":"Agent","kind":"class",
               "source_file":"pkg/right/agent.go","source_location":"L1","language":"go"}
            ],"links":[]}
            """,
            """
            {"nodes":[
              {"id":"pkg_left_generated_go_agent","label":"Agent"}
            ],"links":[]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_nodes"], 1)
        self.assertIn(
            "dominated:qualified_generated_owner",
            result.metrics["graphify_nodes_coverage_reasons"],
        )

    def test_two_hop_containment_dominates_flat_graphify_ownership(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"a.go","kind":"file",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"owner","label":"Widget","kind":"class",
               "source_file":"pkg/a.go","source_location":"L2"},
              {"id":"method","label":"run()","kind":"method",
               "source_file":"pkg/a.go","source_location":"L3"}
            ],"links":[
              {"source":"file","target":"owner","relation":"contains"},
              {"source":"owner","target":"method","relation":"contains"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_file","label":"pkg/a.go",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"legacy_method","label":"run()",
               "source_file":"pkg/a.go","source_location":"L3"}
            ],"links":[
              {"source":"legacy_file","target":"legacy_method","relation":"contains"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:containment_path",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_containment_owner_rejects_cross_type_graphify_ownership(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"flakey.go","kind":"file",
               "source_file":"pkg/flakey.go","source_location":"L1","language":"go"},
              {"id":"interface","label":"Flakey","kind":"interface",
               "qualified_name":"pkg.Flakey",
               "source_file":"pkg/flakey.go","source_location":"L2","language":"go"},
              {"id":"implementation","label":"flakey","kind":"struct",
               "qualified_name":"pkg.flakey",
               "source_file":"pkg/flakey.go","source_location":"L8","language":"go"},
              {"id":"interface_method","label":".Close()","kind":"method",
               "qualified_name":"pkg.Flakey::Close",
               "source_file":"pkg/flakey.go","source_location":"L3","language":"go"},
              {"id":"implementation_method","label":".Close()","kind":"method",
               "qualified_name":"pkg.flakey::Close",
               "source_file":"pkg/flakey.go","source_location":"L9","language":"go"}
            ],"links":[
              {"source":"file","target":"interface","relation":"contains"},
              {"source":"file","target":"implementation","relation":"contains"},
              {"source":"file","target":"implementation_method","relation":"contains"},
              {"source":"interface","target":"interface_method","relation":"contains"},
              {"source":"implementation","target":"implementation_method","relation":"contains"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_interface","label":"Flakey","kind":"interface",
               "qualified_name":"pkg.Flakey",
               "source_file":"pkg/flakey.go","source_location":"L2","language":"go"},
              {"id":"legacy_method","label":".Close()","kind":"method",
               "qualified_name":"pkg.flakey::Close",
               "source_file":"pkg/flakey.go","source_location":"L9","language":"go"}
            ],"links":[
              {"source":"legacy_interface","target":"legacy_method","relation":"contains"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:exact_containment_owner_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_go_type_conversion_rejects_graphify_call_classification(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"convert","label":"convert()","kind":"function",
               "source_file":"pkg/convert.go","source_location":"L3","language":"go"},
              {"id":"pgid","label":"Pgid","kind":"type_alias",
               "qualified_name":"common.Pgid",
               "source_file":"common/types.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"convert","target":"pgid","relation":"references",
               "source_file":"pkg/convert.go","source_location":"L4"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_convert","label":"convert()","kind":"function",
               "source_file":"pkg/convert.go","source_location":"L3","language":"go"},
              {"id":"legacy_pgid","label":"Pgid","kind":"type_alias",
               "qualified_name":"common.Pgid",
               "source_file":"common/types.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"legacy_convert","target":"legacy_pgid","relation":"calls",
               "source_file":"pkg/convert.go","source_location":"L4"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:go_type_conversion_not_call",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_argument_reference_rejects_wrong_indirect_call_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"fp/add/index.ts","kind":"file",
               "source_file":"fp/add/index.ts","source_location":"L1","language":"typescript"},
              {"id":"correct","label":"add","kind":"function",
               "source_file":"add/index.ts","source_location":"L73","language":"typescript"},
              {"id":"wrong","label":"fn","kind":"function",
               "source_file":"convert/test.ts","source_location":"L11","language":"typescript"}
            ],"links":[
              {"source":"owner","target":"correct","relation":"references",
               "context":"argument","source_file":"fp/add/index.ts","source_location":"L6"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"fp/add/index.ts",
               "source_file":"fp/add/index.ts","source_location":"L1"},
              {"id":"wrong","label":"fn()",
               "source_file":"convert/test.ts","source_location":"L11"}
            ],"links":[
              {"source":"owner","target":"wrong","relation":"indirect_call",
               "context":"argument","source_file":"fp/add/index.ts","source_location":"L6"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:argument_reference_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_collection_reference_is_not_an_indirect_call(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"locale/index.ts","kind":"file",
               "source_file":"locale/index.ts","source_location":"L1","language":"typescript"},
              {"id":"format","label":"formatDistance","kind":"function",
               "source_file":"locale/format.ts","source_location":"L12","language":"typescript"}
            ],"links":[
              {"source":"owner","target":"format","relation":"references",
               "context":"collection","source_file":"locale/index.ts","source_location":"L17"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"locale/index.ts",
               "source_file":"locale/index.ts","source_location":"L1"},
              {"id":"format","label":"formatDistance()",
               "source_file":"locale/format.ts","source_location":"L12"}
            ],"links":[
              {"source":"owner","target":"format","relation":"indirect_call",
               "context":"collection","source_file":"locale/index.ts","source_location":"L17"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:value_reference_not_indirect_call",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_anchored_cross_language_type_reference_is_rejected(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"rust_owner","label":"build()","kind":"function",
               "source_file":"src/build.rs","source_location":"L2","language":"rust"},
              {"id":"python_result","label":"Result","kind":"class",
               "source_file":"tools/bench","source_location":"L10","language":"python"}
            ],"links":[]}
            """,
            """
            {"nodes":[
              {"id":"legacy_owner","label":"build()",
               "source_file":"src/build.rs","source_location":"L2"},
              {"id":"legacy_result","label":"Result",
               "source_file":"tools/bench","source_location":"L10"}
            ],"links":[
              {"source":"legacy_owner","target":"legacy_result","relation":"references",
               "context":"return_type","source_file":"src/build.rs","source_location":"L2"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:cross_language_target",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_associated_return_alias_dominates_its_concrete_realization(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"method","label":"iter","kind":"method",
               "qualified_name":"<crate::Collection as crate::Iterate>::iter",
               "source_file":"src/lib.rs","source_location":"L10","language":"rust"},
              {"id":"associated","label":"Iter","kind":"type_alias",
               "qualified_name":"<impl Iterate for Collection>::Iter",
               "source_file":"src/lib.rs","source_location":"L9","language":"rust"},
              {"id":"concrete","label":"Iter","kind":"struct",
               "qualified_name":"crate::Iter","source_file":"src/lib.rs",
               "source_location":"L3","language":"rust"}
            ],"links":[
              {"source":"method","target":"associated","relation":"returns",
               "source_file":"src/lib.rs","source_location":"L10"},
              {"source":"associated","target":"concrete","relation":"references",
               "source_file":"src/lib.rs","source_location":"L9"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"method","label":"iter()","source_file":"src/lib.rs",
               "source_location":"L10"},
              {"id":"concrete","label":"Iter","source_file":"src/lib.rs",
               "source_location":"L3"}
            ],"links":[
              {"source":"method","target":"concrete","relation":"references",
               "context":"return_type","source_file":"src/lib.rs",
               "source_location":"L10"}
            ]}
            """,
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:associated_return_realization",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_associated_return_rejects_a_terminal_name_trait_projection(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"method","label":"folder","kind":"method",
               "qualified_name":"<crate::Consumer as crate::Consume>::folder",
               "source_file":"src/lib.rs","source_location":"L10","language":"rust"},
              {"id":"associated","label":"Folder","kind":"type_alias",
               "qualified_name":"<impl Consume for Consumer>::Folder",
               "source_file":"src/lib.rs","source_location":"L9","language":"rust"},
              {"id":"concrete","label":"LocalFolder","kind":"struct",
               "qualified_name":"crate::LocalFolder","source_file":"src/lib.rs",
               "source_location":"L3","language":"rust"},
              {"id":"wrong","label":"Folder","kind":"trait",
               "qualified_name":"crate::Folder","source_file":"src/api.rs",
               "source_location":"L2","language":"rust"}
            ],"links":[
              {"source":"method","target":"associated","relation":"returns",
               "source_file":"src/lib.rs","source_location":"L10"},
              {"source":"associated","target":"concrete","relation":"references",
               "source_file":"src/lib.rs","source_location":"L9"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"method","label":"folder()","source_file":"src/lib.rs",
               "source_location":"L10"},
              {"id":"wrong","label":"Folder","source_file":"src/api.rs",
               "source_location":"L2"}
            ],"links":[
              {"source":"method","target":"wrong","relation":"references",
               "context":"return_type","source_file":"src/lib.rs",
               "source_location":"L10"}
            ]}
            """,
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:associated_return_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_rust_generic_impl_owner_is_dominated_by_exact_type_ownership(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"StandardImpl","kind":"struct",
               "source_file":"src/printer.rs","source_location":"L1","language":"rust"},
              {"id":"method","label":".write()","kind":"method",
               "source_file":"src/printer.rs","source_location":"L4","language":"rust"}
            ],"links":[
              {"source":"owner","target":"method","relation":"contains",
               "source_file":"src/printer.rs","source_location":"L4"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_impl","label":"StandardImpl<'a, M, W>",
               "source_file":"src/printer.rs","source_location":"L3"},
              {"id":"legacy_method","label":".write()",
               "source_file":"src/printer.rs","source_location":"L4"}
            ],"links":[
              {"source":"legacy_impl","target":"legacy_method","relation":"method",
               "source_file":"src/printer.rs","source_location":"L4"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:canonical_rust_generic_owner",
            result.metrics["graphify_nodes_coverage_reasons"],
        )

    def test_exact_field_type_dominates_flat_owner_reference(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Config","kind":"struct",
               "source_file":"src/config.rs","source_location":"L1","language":"rust"},
              {"id":"field","label":"matcher","kind":"field",
               "source_file":"src/config.rs","source_location":"L2","language":"rust"},
              {"id":"target","label":"Matcher","kind":"struct",
               "source_file":"src/matcher.rs","source_location":"L1","language":"rust"}
            ],"links":[
              {"source":"owner","target":"field","relation":"contains",
               "source_file":"src/config.rs","source_location":"L2"},
              {"source":"field","target":"target","relation":"type_of",
               "source_file":"src/config.rs","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_owner","label":"Config",
               "source_file":"src/config.rs","source_location":"L1"},
              {"id":"legacy_target","label":"Matcher",
               "source_file":"src/matcher.rs","source_location":"L1"}
            ],"links":[
              {"source":"legacy_owner","target":"legacy_target","relation":"references",
               "context":"field","source_file":"src/config.rs","source_location":"L2"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:precise_field_type",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_field_generic_argument_dominates_flat_owner_reference(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"JobFifo","kind":"struct",
               "source_file":"src/job.rs","source_location":"L1","language":"rust"},
              {"id":"field","label":"inner","kind":"field",
               "source_file":"src/job.rs","source_location":"L2","language":"rust"},
              {"id":"target","label":"JobRef","kind":"struct",
               "source_file":"src/job.rs","source_location":"L10","language":"rust"}
            ],"links":[
              {"source":"owner","target":"field","relation":"contains",
               "source_file":"src/job.rs","source_location":"L2"},
              {"source":"field","target":"target","relation":"type_of",
               "source_file":"src/job.rs","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_owner","label":"JobFifo",
               "source_file":"src/job.rs","source_location":"L1"},
              {"id":"legacy_target","label":"JobRef",
               "source_file":"src/job.rs","source_location":"L10"}
            ],"links":[
              {"source":"legacy_owner","target":"legacy_target","relation":"references",
               "context":"generic_arg","source_file":"src/job.rs","source_location":"L2"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:precise_field_type",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_scoped_field_type_rejects_wrong_same_named_parameter(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Chunks","kind":"struct",
               "source_file":"src/chunks.rs","source_location":"L10","language":"rust"},
              {"id":"field","label":"i","kind":"field",
               "source_file":"src/chunks.rs","source_location":"L12","language":"rust"},
              {"id":"exact","label":"I","kind":"parameter",
               "qualified_name":"crate::chunks::Chunks::<I>",
               "source_file":"src/chunks.rs","source_location":"L10","language":"rust"},
              {"id":"wrong","label":"I","kind":"parameter",
               "qualified_name":"crate::iter::I",
               "source_file":"src/iter.rs","source_location":"L290","language":"rust"}
            ],"links":[
              {"source":"owner","target":"field","relation":"contains",
               "source_file":"src/chunks.rs","source_location":"L12"},
              {"source":"field","target":"exact","relation":"type_of",
               "source_file":"src/chunks.rs","source_location":"L12"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_owner","label":"Chunks",
               "source_file":"src/chunks.rs","source_location":"L10"},
              {"id":"legacy_wrong","label":"I",
               "source_file":"src/iter.rs","source_location":"L290"}
            ],"links":[
              {"source":"legacy_owner","target":"legacy_wrong","relation":"references",
               "context":"field","source_file":"src/chunks.rs","source_location":"L12"}
            ]}
            """,
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:exact_typed_child_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_scoped_parameter_type_rejects_wrong_same_named_parameter(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"extend()","kind":"method",
               "source_file":"src/extend.rs","source_location":"L10","language":"rust"},
              {"id":"parameter","label":"values","kind":"parameter",
               "source_file":"src/extend.rs","source_location":"L12","language":"rust"},
              {"id":"exact","label":"I","kind":"parameter",
               "qualified_name":"crate::Extend::extend::<I>",
               "source_file":"src/extend.rs","source_location":"L10","language":"rust"},
              {"id":"wrong","label":"I","kind":"parameter",
               "qualified_name":"crate::iter::I",
               "source_file":"src/iter.rs","source_location":"L290","language":"rust"}
            ],"links":[
              {"source":"owner","target":"parameter","relation":"contains",
               "source_file":"src/extend.rs","source_location":"L12"},
              {"source":"parameter","target":"exact","relation":"type_of",
               "source_file":"src/extend.rs","source_location":"L12"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_owner","label":"extend()",
               "source_file":"src/extend.rs","source_location":"L10"},
              {"id":"legacy_wrong","label":"I",
               "source_file":"src/iter.rs","source_location":"L290"}
            ],"links":[
              {"source":"legacy_owner","target":"legacy_wrong","relation":"references",
               "context":"parameter_type","source_file":"src/extend.rs","source_location":"L12"}
            ]}
            """,
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:exact_typed_child_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_signature_occurrence_rejects_wrong_owner_and_parameter(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"first","label":"extend()","kind":"method",
               "source_file":"src/extend.rs","source_location":"L10","language":"rust"},
              {"id":"exact_owner","label":"extend()","kind":"method",
               "source_file":"src/extend.rs","source_location":"L12","language":"rust"},
              {"id":"exact_target","label":"I","kind":"parameter",
               "qualified_name":"crate::Second::extend::<I>",
               "source_file":"src/extend.rs","source_location":"L12","language":"rust"},
              {"id":"wrong_target","label":"I","kind":"parameter",
               "qualified_name":"crate::iter::I",
               "source_file":"src/iter.rs","source_location":"L290","language":"rust"}
            ],"links":[
              {"source":"exact_owner","target":"exact_target","relation":"references",
               "context":"type_reference","source_file":"src/extend.rs","source_location":"L12"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_first","label":"extend()",
               "source_file":"src/extend.rs","source_location":"L10"},
              {"id":"legacy_wrong","label":"I",
               "source_file":"src/iter.rs","source_location":"L290"}
            ],"links":[
              {"source":"legacy_first","target":"legacy_wrong","relation":"references",
               "context":"parameter_type","source_file":"src/extend.rs","source_location":"L12"}
            ]}
            """,
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:exact_typed_occurrence_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_multiple_signature_occurrences_remain_ambiguous(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"extend()","kind":"method",
               "source_file":"src/extend.rs","source_location":"L12","language":"rust"},
              {"id":"first","label":"I","kind":"parameter",
               "qualified_name":"crate::First::<I>",
               "source_file":"src/first.rs","source_location":"L1","language":"rust"},
              {"id":"second","label":"I","kind":"parameter",
               "qualified_name":"crate::Second::<I>",
               "source_file":"src/second.rs","source_location":"L1","language":"rust"},
              {"id":"wrong","label":"I","kind":"parameter",
               "qualified_name":"crate::Wrong::<I>",
               "source_file":"src/wrong.rs","source_location":"L1","language":"rust"}
            ],"links":[
              {"source":"owner","target":"first","relation":"references",
               "source_file":"src/extend.rs","source_location":"L12"},
              {"source":"owner","target":"second","relation":"references",
               "source_file":"src/extend.rs","source_location":"L12"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_owner","label":"extend()",
               "source_file":"src/extend.rs","source_location":"L12"},
              {"id":"legacy_wrong","label":"I",
               "source_file":"src/wrong.rs","source_location":"L1"}
            ],"links":[
              {"source":"legacy_owner","target":"legacy_wrong","relation":"references",
               "context":"parameter_type","source_file":"src/extend.rs","source_location":"L12"}
            ]}
            """,
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertEqual(result.metrics["ambiguous_graphify_edges"], 1)
        self.assertIn(
            "ambiguous:multiple_typed_occurrences",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_return_occurrence_rejects_wrong_overload_owner(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"first","label":"generate()","kind":"function",
               "source_file":"src/generate.rs","source_location":"L10","language":"rust"},
              {"id":"exact_owner","label":"generate()","kind":"function",
               "source_file":"src/generate.rs","source_location":"L20","language":"rust"},
              {"id":"return_target","label":"ParallelIterator","kind":"trait",
               "source_file":"src/iter.rs","source_location":"L100","language":"rust"}
            ],"links":[
              {"source":"exact_owner","target":"return_target","relation":"returns",
               "source_file":"src/generate.rs","source_location":"L20"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_first","label":"generate()",
               "source_file":"src/generate.rs","source_location":"L10"},
              {"id":"legacy_target","label":"ParallelIterator",
               "source_file":"src/iter.rs","source_location":"L100"}
            ],"links":[
              {"source":"legacy_first","target":"legacy_target","relation":"references",
               "context":"return_type","source_file":"src/generate.rs","source_location":"L20"}
            ]}
            """,
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:exact_return_occurrence_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_unique_three_hop_containment_dominates_flat_graphify_ownership(self) -> None:
        containment = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"a.go","kind":"file",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"outer","label":"Outer","kind":"class",
               "source_file":"pkg/a.go","source_location":"L2"},
              {"id":"inner","label":"Inner","kind":"class",
               "source_file":"pkg/a.go","source_location":"L3"},
              {"id":"method","label":"run()","kind":"method",
               "source_file":"pkg/a.go","source_location":"L4"}
            ],"links":[
              {"source":"file","target":"outer","relation":"contains"},
              {"source":"outer","target":"inner","relation":"contains"},
              {"source":"inner","target":"method","relation":"contains"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_file","label":"pkg/a.go",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"legacy_method","label":"run()",
               "source_file":"pkg/a.go","source_location":"L4"}
            ],"links":[
              {"source":"legacy_file","target":"legacy_method","relation":"contains"}
            ]}
            """,
        )
        self.assertTrue(containment.passed, containment.failures)
        self.assertEqual(containment.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:containment_path",
            containment.metrics["graphify_edges_coverage_reasons"],
        )

    def test_multiple_bounded_containment_paths_fail_closed(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"a.go","kind":"file",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"left","label":"Left","kind":"class",
               "source_file":"pkg/a.go","source_location":"L2"},
              {"id":"right","label":"Right","kind":"class",
               "source_file":"pkg/a.go","source_location":"L3"},
              {"id":"method","label":"run()","kind":"method",
               "source_file":"pkg/a.go","source_location":"L4"}
            ],"links":[
              {"source":"file","target":"left","relation":"contains"},
              {"source":"file","target":"right","relation":"contains"},
              {"source":"left","target":"method","relation":"contains"},
              {"source":"right","target":"method","relation":"contains"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_file","label":"pkg/a.go",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"legacy_method","label":"run()",
               "source_file":"pkg/a.go","source_location":"L4"}
            ],"links":[
              {"source":"legacy_file","target":"legacy_method","relation":"contains"}
            ]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["ambiguous_graphify_edges"], 1)
        self.assertIn(
            "ambiguous:multiple_containment_paths",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_different_call_sites_still_fail_closed(self) -> None:
        occurrence = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"caller","label":"run()","kind":"function",
               "source_file":"pkg/a.go","source_location":"L1","language":"go"},
              {"id":"type","label":"Widget","kind":"class",
               "source_file":"pkg/a.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"caller","target":"type","relation":"references",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"legacy_caller","label":"run()",
               "source_file":"pkg/a.go","source_location":"L1"},
              {"id":"receiver","label":"Widget",
               "source_file":"pkg/generated.go","source_location":"L5"}
            ],"links":[
              {"source":"legacy_caller","target":"receiver","relation":"uses",
               "source_file":"pkg/a.go","source_location":"L11"}
            ]}
            """,
        )
        self.assertFalse(occurrence.passed)
        self.assertEqual(occurrence.metrics["missing_graphify_edges"], 1)

    def test_module_import_projection_is_rejected_but_real_use_is_required(self) -> None:
        graphify = """
            {"nodes":[
              {"id":"module","label":"app.py",
               "source_file":"app.py","source_location":"L1"},
              {"id":"owner","label":"Owner",
               "source_file":"app.py","source_location":"L20"},
              {"id":"symbol","label":"Widget",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"symbol","relation":"uses",
               "source_file":"app.py","source_location":"L3"},
              {"source":"owner","target":"symbol","relation":"uses",
               "source_file":"app.py","source_location":"L21"}
            ]}
        """
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"module","label":"app.py","kind":"file",
               "source_file":"app.py","source_location":"L1"},
              {"id":"owner","label":"Owner","kind":"class",
               "source_file":"app.py","source_location":"L20"},
              {"id":"symbol","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"module","target":"symbol","relation":"imports",
               "source_file":"app.py","source_location":"L3"}
            ]}
            """,
            graphify,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 1)
        self.assertIn(
            "rejected:module_import_projected_to_symbol",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_occurrence_with_more_precise_owner_dominates_baseline(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"outer","label":"run","kind":"function",
               "source_file":"app.py","source_location":"L10"},
              {"id":"inner","label":"run_inner","kind":"function",
               "source_file":"app.py","source_location":"L20"},
              {"id":"target","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"inner","target":"target","relation":"calls",
               "source_file":"app.py","source_location":"L21"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"outer","label":"run()",
               "source_file":"app.py","source_location":"L10"},
              {"id":"target","label":"Widget",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"outer","target":"target","relation":"calls",
               "source_file":"app.py","source_location":"L21"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:precise_occurrence_owner",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_precise_reference_site_dominates_a_declaration_line_projection(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Run","kind":"function",
               "source_file":"app.go","source_location":"L10","language":"go"},
              {"id":"target","label":"Widget","kind":"struct",
               "source_file":"lib.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"owner","target":"target","relation":"references",
               "source_file":"app.go","source_location":"L12"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"Run()",
               "source_file":"app.go","source_location":"L10"},
              {"id":"target","label":"Widget",
               "source_file":"lib.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"uses",
               "source_file":"app.go","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:precise_declaration_reference_occurrence",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_precise_return_type_dominates_a_declaration_line_projection(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"build","kind":"function",
               "source_file":"src/lib.rs","source_location":"L10","language":"rust"},
              {"id":"target","label":"BuildError","kind":"struct",
               "source_file":"src/error.rs","source_location":"L2","language":"rust"}
            ],"links":[
              {"source":"owner","target":"target","relation":"returns",
               "source_file":"src/lib.rs","source_location":"L12"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"build()",
               "source_file":"src/lib.rs","source_location":"L10"},
              {"id":"target","label":"BuildError",
               "source_file":"src/error.rs","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"references",
               "context":"generic_arg","source_file":"src/lib.rs","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:precise_return_type_declaration_projection",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_return_projection_requires_exact_endpoint_and_type_context(self) -> None:
        compass = """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"build","kind":"function",
               "source_file":"src/lib.rs","source_location":"L10","language":"rust"},
              {"id":"returned","label":"BuildError","kind":"struct",
               "source_file":"src/error.rs","source_location":"L2","language":"rust"},
              {"id":"other","label":"OtherError","kind":"struct",
               "source_file":"src/other.rs","source_location":"L3","language":"rust"}
            ],"links":[
              {"source":"owner","target":"returned","relation":"returns",
               "source_file":"src/lib.rs","source_location":"L12"}
            ]}
        """
        wrong_endpoint = compare_documents(
            compass,
            """
            {"nodes":[
              {"id":"owner","label":"build()",
               "source_file":"src/lib.rs","source_location":"L10"},
              {"id":"other","label":"OtherError",
               "source_file":"src/other.rs","source_location":"L3"}
            ],"links":[
              {"source":"owner","target":"other","relation":"references",
               "context":"return_type","source_file":"src/lib.rs","source_location":"L10"}
            ]}
            """,
        )
        value_reference = compare_documents(
            compass,
            """
            {"nodes":[
              {"id":"owner","label":"build()",
               "source_file":"src/lib.rs","source_location":"L10"},
              {"id":"returned","label":"BuildError",
               "source_file":"src/error.rs","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"returned","relation":"references",
               "context":"value","source_file":"src/lib.rs","source_location":"L10"}
            ]}
            """,
        )
        wrong_occurrence = compare_documents(
            compass,
            """
            {"nodes":[
              {"id":"owner","label":"build()",
               "source_file":"src/lib.rs","source_location":"L10"},
              {"id":"returned","label":"BuildError",
               "source_file":"src/error.rs","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"returned","relation":"references",
               "context":"return_type","source_file":"src/lib.rs","source_location":"L11"}
            ]}
            """,
        )
        for result in (wrong_endpoint, value_reference, wrong_occurrence):
            self.assertFalse(result.passed)
            self.assertEqual(result.metrics["missing_graphify_edges"], 1)

    def test_multiple_projected_return_occurrences_remain_ambiguous(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"build","kind":"function",
               "source_file":"src/lib.rs","source_location":"L10","language":"rust"},
              {"id":"target","label":"BuildError","kind":"struct",
               "source_file":"src/error.rs","source_location":"L2","language":"rust"}
            ],"links":[
              {"source":"owner","target":"target","relation":"returns",
               "source_file":"src/lib.rs","source_location":"L12"},
              {"source":"owner","target":"target","relation":"returns",
               "source_file":"src/lib.rs","source_location":"L13"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"build()",
               "source_file":"src/lib.rs","source_location":"L10"},
              {"id":"target","label":"BuildError",
               "source_file":"src/error.rs","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"target","relation":"references",
               "context":"return_type","source_file":"src/lib.rs","source_location":"L10"}
            ]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["ambiguous_graphify_edges"], 1)
        self.assertIn(
            "ambiguous:multiple_projected_return_types",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_qualified_external_target_rejects_same_named_local_rebinding(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Run","kind":"function",
               "source_file":"app.go","source_location":"L10","language":"go"},
              {"id":"external","label":"Context","kind":"type_alias",
               "source_file":"","source_location":"","language":"go",
               "qualified_name":"context.context"},
              {"id":"local","label":"Context","kind":"struct",
               "source_file":"internal/contexts.go","source_location":"L2",
               "language":"go"}
            ],"links":[
              {"source":"owner","target":"external","relation":"references",
               "source_file":"app.go","source_location":"L11"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"Run()",
               "source_file":"app.go","source_location":"L10"},
              {"id":"local","label":"Context",
               "source_file":"internal/contexts.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"local","relation":"uses",
               "source_file":"app.go","source_location":"L11"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertIn(
            "rejected:qualified_external_target_rebound_to_local",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_occurrence_rejects_same_named_receiver_misbinding(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"MarshalJSON","kind":"method",
               "source_file":"pkg/a.go","source_location":"L8","language":"go"},
              {"id":"correct","label":"A::Encode","kind":"method",
               "source_file":"pkg/a.go","source_location":"L2","language":"go",
               "qualified_name":"pkg.A::Encode"},
              {"id":"wrong","label":".Encode()","kind":"method",
               "source_file":"pkg/b.go","source_location":"L2","language":"go",
               "qualified_name":"pkg.B::Encode"}
            ],"links":[
              {"source":"owner","target":"correct","relation":"calls",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"MarshalJSON()",
               "source_file":"pkg/a.go","source_location":"L8"},
              {"id":"wrong","label":".Encode()",
               "source_file":"pkg/b.go","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"wrong","relation":"calls",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:exact_occurrence_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_multiple_exact_occurrences_reject_absent_same_line_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"run","kind":"function",
               "source_file":"src/main.rs","source_location":"L8","language":"rust"},
              {"id":"first","label":"First::new","kind":"method",
               "source_file":"src/first.rs","source_location":"L2","language":"rust"},
              {"id":"second","label":"Second::new","kind":"method",
               "source_file":"src/second.rs","source_location":"L2","language":"rust"},
              {"id":"wrong","label":".new()","kind":"method",
               "source_file":"src/wrong.rs","source_location":"L2","language":"rust"}
            ],"links":[
              {"source":"owner","target":"first","relation":"calls",
               "source_file":"src/main.rs","source_location":"L10"},
              {"source":"owner","target":"second","relation":"calls",
               "source_file":"src/main.rs","source_location":"L10"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"run()",
               "source_file":"src/main.rs","source_location":"L8"},
              {"id":"wrong","label":".new()",
               "source_file":"src/wrong.rs","source_location":"L2"}
            ],"links":[
              {"source":"owner","target":"wrong","relation":"calls",
               "source_file":"src/main.rs","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:exact_occurrence_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_occurrence_resolves_an_ambiguous_sourceless_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"MarshalJSON","kind":"method",
               "source_file":"pkg/a.go","source_location":"L8","language":"go"},
              {"id":"correct","label":".Encode()","kind":"method",
               "source_file":"pkg/a.go","source_location":"L2","language":"go"},
              {"id":"other","label":".Encode()","kind":"method",
               "source_file":"pkg/b.go","source_location":"L2","language":"go"}
            ],"links":[
              {"source":"owner","target":"correct","relation":"calls",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"MarshalJSON()",
               "source_file":"pkg/a.go","source_location":"L8"},
              {"id":"generated_encode","label":".Encode()"}
            ],"links":[
              {"source":"owner","target":"generated_encode","relation":"calls",
               "source_file":"pkg/a.go","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
        self.assertEqual(result.metrics["missing_graphify_nodes"], 0)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:exact_occurrence_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_import_binding_dominates_a_sourceless_external_placeholder(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"owner","label":"Run","kind":"function",
               "source_file":"app.go","source_location":"L10","language":"go"},
              {"id":"external","label":"RawMessage","kind":"import",
               "source_file":"app.go","source_location":"L2","language":"go",
               "qualified_name":"encoding/json.rawmessage"}
            ],"links":[
              {"source":"owner","target":"external","relation":"references",
               "source_file":"app.go","source_location":"L11"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"owner","label":"Run()",
               "source_file":"app.go","source_location":"L10"},
              {"id":"external","label":"RawMessage"}
            ],"links":[
              {"source":"owner","target":"external","relation":"uses",
               "source_file":"app.go","source_location":"L11"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:qualified_external_binding",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_extends_occurrence_grounds_a_sourceless_placeholder(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"child","label":"Child","kind":"class",
               "source_file":"pkg/models.py","source_location":"L10","language":"python"},
              {"id":"base","label":"Storage","kind":"class",
               "source_file":"pkg/storage.py","source_location":"L2","language":"python"}
            ],"links":[
              {"source":"child","target":"base","relation":"extends",
               "source_file":"pkg/models.py","source_location":"L12"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"child","label":"Child",
               "source_file":"pkg/models.py","source_location":"L10"},
              {"id":"storage","label":"Storage"}
            ],"links":[
              {"source":"child","target":"storage","relation":"inherits",
               "source_file":"pkg/models.py","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_nodes"], 1)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:precise_inheritance_occurrence",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_inheritance_occurrence_rejects_a_wrong_anchored_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"child","label":"Child","kind":"class",
               "source_file":"pkg/models.py","source_location":"L10","language":"python"},
              {"id":"base","label":"Base","kind":"class",
               "source_file":"pkg/base.py","source_location":"L2","language":"python"},
              {"id":"wrong","label":"Wrong","kind":"class",
               "source_file":"pkg/wrong.py","source_location":"L2","language":"python"}
            ],"links":[
              {"source":"child","target":"base","relation":"extends",
               "source_file":"pkg/models.py","source_location":"L11"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"child","label":"Child",
               "source_file":"pkg/models.py","source_location":"L10"},
              {"id":"base","label":"Base",
               "source_file":"pkg/base.py","source_location":"L2"},
              {"id":"wrong","label":"Wrong",
               "source_file":"pkg/wrong.py","source_location":"L2"}
            ],"links":[
              {"source":"child","target":"wrong","relation":"inherits",
               "source_file":"pkg/models.py","source_location":"L10"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:exact_inheritance_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_imported_symbol_dominates_a_module_level_import(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"module","label":"app.py","kind":"file",
               "source_file":"app.py","source_location":"L1"},
              {"id":"symbol","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L20"}
            ],"links":[
              {"source":"module","target":"symbol","relation":"imports",
               "source_file":"app.py","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"module","label":"app.py",
               "source_file":"app.py","source_location":"L1"},
              {"id":"library","label":"lib.py",
               "source_file":"lib.py","source_location":"L1"}
            ],"links":[
              {"source":"module","target":"library","relation":"imports",
               "source_file":"app.py","source_location":"L2"}
            ]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "dominated:imported_symbol_definition",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_semantic_module_import_dominates_its_file_realization(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"consumer","label":"job.rs","kind":"file",
               "qualified_name":"crate::job","source_file":"src/job.rs",
               "source_location":"L1","language":"rust"},
              {"id":"module_file","label":"unwind.rs","kind":"file",
               "qualified_name":"crate::unwind","source_file":"src/unwind.rs",
               "source_location":"L1","language":"rust"},
              {"id":"import_owner","label":"job_impl","kind":"module",
               "qualified_name":"crate::job::job_impl","source_file":"src/job.rs",
               "source_location":"L1","language":"rust"},
              {"id":"module","label":"unwind","kind":"module",
               "qualified_name":"crate::unwind","source_file":"src/lib.rs",
               "source_location":"L1","language":"rust"}
            ],"links":[
              {"source":"import_owner","target":"module","relation":"imports",
               "source_file":"src/job.rs","source_location":"L1"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"consumer","label":"src/job.rs","kind":"file",
               "source_file":"src/job.rs","source_location":"L1",
               "language":"rust"},
              {"id":"module_file","label":"src/unwind.rs","kind":"file",
               "source_file":"src/unwind.rs","source_location":"L1",
               "language":"rust"}
            ],"links":[
              {"source":"consumer","target":"module_file","relation":"imports",
               "source_file":"src/job.rs","source_location":"L1"}
            ]}
            """,
        )
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:semantic_module_realization",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_precise_function_import_owner_dominates_a_file_owner(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"app.py","kind":"file",
               "source_file":"app.py","source_location":"L1"},
              {"id":"function","label":"run","kind":"function",
               "source_file":"app.py","source_location":"L10"},
              {"id":"symbol","label":"Widget","kind":"class",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"function","target":"symbol","relation":"imports",
               "source_file":"app.py","source_location":"L11"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"file","label":"app.py",
               "source_file":"app.py","source_location":"L1"},
              {"id":"function","label":"run()",
               "source_file":"app.py","source_location":"L10"},
              {"id":"symbol","label":"Widget",
               "source_file":"lib.py","source_location":"L2"}
            ],"links":[
              {"source":"file","target":"symbol","relation":"imports",
               "source_file":"app.py","source_location":"L11"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:precise_occurrence_owner",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_symbol_reexport_dominates_a_package_import(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"__init__.py","kind":"file",
               "source_file":"pkg/__init__.py","source_location":"L1"},
              {"id":"symbol","label":"Widget","kind":"class",
               "source_file":"pkg/widget.py","source_location":"L2"}
            ],"links":[
              {"source":"file","target":"symbol","relation":"exports",
               "source_file":"pkg/__init__.py","source_location":"L1"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"file","label":"__init__.py",
               "source_file":"pkg/__init__.py","source_location":"L1"},
              {"id":"symbol","label":"Widget",
               "source_file":"pkg/widget.py","source_location":"L2"}
            ],"links":[
              {"source":"file","target":"symbol","relation":"imports",
               "source_file":"pkg/__init__.py","source_location":"L1"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["dominated_graphify_edges"], 1)
        self.assertIn(
            "dominated:symbol_reexport",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_reexport_occurrence_rejects_a_wrong_local_import_target(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"file","label":"__init__.py","kind":"file",
               "source_file":"pkg/__init__.py","source_location":"L1"},
              {"id":"external","label":"os","kind":"import",
               "qualified_name":"os"},
              {"id":"wrong","label":"os.py","kind":"file",
               "source_file":"pkg/os.py","source_location":"L1"}
            ],"links":[
              {"source":"file","target":"external","relation":"exports",
               "source_file":"pkg/__init__.py","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"file","label":"__init__.py",
               "source_file":"pkg/__init__.py","source_location":"L1"},
              {"id":"wrong","label":"os.py",
               "source_file":"pkg/os.py","source_location":"L1"}
            ],"links":[
              {"source":"file","target":"wrong","relation":"imports",
               "source_file":"pkg/__init__.py","source_location":"L2"}
            ]}
            """,
        )
        self.assertTrue(result.passed, result.failures)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:reexport_target_conflict",
            result.metrics["graphify_edges_coverage_reasons"],
        )

    def test_exact_external_import_rejects_terminal_name_local_rebinding(self) -> None:
        result = compare_documents(
            """
            {"graph":{"diagnostics":[]},"nodes":[
              {"id":"module","label":"app.py","kind":"file",
               "source_file":"app.py","source_location":"L1"},
              {"id":"external","label":"inspect","kind":"import",
               "source_file":"app.py","source_location":"L2",
               "qualified_name":"inspect"}
            ],"links":[
              {"source":"module","target":"external","relation":"imports",
               "source_file":"app.py","source_location":"L2"}
            ]}
            """,
            """
            {"nodes":[
              {"id":"module","label":"app.py",
               "source_file":"app.py","source_location":"L1"},
              {"id":"wrong","label":"inspect.py",
               "source_file":"project/inspect.py","source_location":"L1"}
            ],"links":[
              {"source":"module","target":"wrong","relation":"imports",
               "source_file":"app.py","source_location":"L2"}
            ]}
            """,
        )
        self.assertFalse(result.passed)
        self.assertEqual(result.metrics["rejected_graphify_edges"], 1)
        self.assertEqual(result.metrics["missing_graphify_edges"], 0)
        self.assertIn(
            "rejected:qualified_external_import_rebound_to_local",
            result.metrics["graphify_edges_coverage_reasons"],
        )


if __name__ == "__main__":
    unittest.main()
