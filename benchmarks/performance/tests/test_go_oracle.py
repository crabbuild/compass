"""Go oracle fixtures use no product graph and require Go only when opted in."""

from __future__ import annotations

import copy
from contextlib import nullcontext
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.performance.compass import go_oracle
from benchmarks.performance.compass.occurrences import (
    has_independent_source_provider, independent_source_inventory,
    source_construct_inventory_sha256,
)


def payload_fixture(symbol="F"):
    raw = f"package p\nfunc {symbol}() {{}}\nfunc G() {{ {symbol}() }}\n".encode()
    spec = dict(path="main.go", sha256=hashlib.sha256(raw).hexdigest(), byteSize=len(raw))
    request = dict(root="/fixture", files=[spec])
    def anchor(start, end):
        return dict(startByte=start, endByte=end, startLine=raw[:start].count(b"\n") + 1)
    declarations = []
    for name in (symbol, "G"):
        body = raw.index(f"func {name}".encode())
        at = body + 5
        declarations.append(dict(name=name, qualifiedName=f"p.{name}", kind="function", bodyStartByte=body, bodyEndByte=raw.index(b"}", at) + 1, **anchor(at, at + len(name.encode()))))
    at = raw.index(f"{symbol}()".encode(), raw.index(b"func G"))
    definition = raw.index(symbol.encode())
    call = dict(owner="p.G", spelling=symbol, qualifier="", invocationStartByte=at, invocationEndByte=at + len(symbol.encode()) + 2, bindingStatus="local_concrete", target=dict(file="main.go", qualifiedName=f"p.{symbol}", **anchor(definition, definition + len(symbol.encode()))), **anchor(at, at + len(symbol.encode())))
    file = dict(**spec, status="parsed", typeCheckComplete=True, declarations=declarations, calls=[call], imports=[])
    payload = dict(schema=go_oracle.SCHEMA, provider=go_oracle.PROVIDER, toolchain="go1.26.0", platform="linux/amd64", selectionPolicy="all_selected_source_no_build_filter", parserAvailable=True, sourceDigest=go_oracle._inventory_digest(request["files"]), files=[file], diagnostics=[], counts=dict(files=1, parsed=1, declarations=2, calls=1, imports=0, diagnostics=0))
    return request, {"main.go": raw}, payload


class GoOracleContractTests(unittest.TestCase):
    def test_typed_payload_is_an_unjudged_candidate_inventory(self):
        request, sources, payload = payload_fixture()
        inventory = go_oracle.validate_payload(request, sources, payload)
        self.assertEqual((1, 1, ()), (inventory.scanned_files, inventory.parsed_files, inventory.rejected_files))
        self.assertEqual("p.G", inventory.constructs[0].owner_qualified_name)
        self.assertEqual("F", inventory.constructs[0].target_spelling)
        self.assertIn(("allTypesAvailable", "true"), inventory.provider_metadata)
        self.assertTrue(has_independent_source_provider("go"))
        self.assertFalse(has_independent_source_provider("rust"))

    def test_schema_coverage_counts_and_source_digests_fail_closed(self):
        request, sources, original = payload_fixture()
        for mutate in (
            lambda p: p.update(schema="compass.go-source-oracle/99"),
            lambda p: p.update(sourceDigest="0" * 64),
            lambda p: p.update(files=[]),
            lambda p: p["counts"].update(calls=2),
            lambda p: p["counts"].update(calls=True),
            lambda p: p["files"][0].update(sha256="0" * 64),
            lambda p: p["files"][0].update(unknown=True),
        ):
            payload = copy.deepcopy(original)
            mutate(payload)
            with self.assertRaises(RuntimeError):
                go_oracle.validate_payload(request, sources, payload)

    def test_target_proofs_require_complete_types_and_exact_declaration(self):
        request, sources, original = payload_fixture()
        for mutate in (
            lambda p: p["files"][0].update(typeCheckComplete=False),
            lambda p: p["files"][0]["calls"][0]["target"].update(file="../outside.go"),
            lambda p: p["files"][0]["calls"][0]["target"].update(qualifiedName="p.other"),
            lambda p: p["files"][0]["calls"][0].update(bindingStatus="interface_dispatch"),
            lambda p: p["files"][0]["calls"][0].update(owner="p.F"),
            lambda p: p["files"][0]["declarations"][0].update(name="wrong"),
        ):
            payload = copy.deepcopy(original)
            mutate(payload)
            with self.assertRaises(RuntimeError):
                go_oracle.validate_payload(request, sources, payload)

    def test_duplicates_and_invalid_invocation_ranges_are_rejected(self):
        request, sources, original = payload_fixture()
        payload = copy.deepcopy(original)
        payload["files"][0]["calls"].append(copy.deepcopy(payload["files"][0]["calls"][0]))
        payload["counts"]["calls"] = 2
        with self.assertRaisesRegex(RuntimeError, "duplicate"):
            go_oracle.validate_payload(request, sources, payload)
        payload = copy.deepcopy(original)
        payload["files"][0]["calls"][0]["invocationEndByte"] = 0
        with self.assertRaises(RuntimeError):
            go_oracle.validate_payload(request, sources, payload)

    def test_utf8_boundaries_are_validated_as_bytes(self):
        request, sources, payload = payload_fixture("é")
        go_oracle.validate_payload(request, sources, payload)
        payload["files"][0]["calls"][0]["startByte"] += 1
        with self.assertRaisesRegex(RuntimeError, "splits UTF-8"):
            go_oracle.validate_payload(request, sources, payload)

    def test_inventory_scope_is_complete_bounded_and_deterministic(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "z.go").write_text("package p\n")
            (root / "a.go").write_text("package p\n")
            request, sources = go_oracle._inputs(root, (), ())
            self.assertEqual(["a.go", "z.go"], list(sources))
            self.assertEqual(request, go_oracle._inputs(root, (), ())[0])
            self.assertEqual(["z.go"], list(go_oracle._inputs(root, ("z.go",), ())[1]))
            with patch.object(go_oracle, "MAX_SOURCE_BYTES", 1):
                with self.assertRaises(RuntimeError):
                    go_oracle._inputs(root, (), ())

    def test_symlink_sources_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.txt").write_text("package p\n")
            try:
                (root / "link.go").symlink_to(root / "source.txt")
            except OSError:
                self.skipTest("symlink creation unavailable")
            with self.assertRaises(RuntimeError):
                go_oracle._inputs(root, (), ())

    def test_missing_optional_runtime_is_an_explicit_error(self):
        with patch.dict(os.environ, {"COMPASS_GO_SOURCE_ORACLE": ""}):
            with self.assertRaisesRegex(RuntimeError, "explicitly built"):
                independent_source_inventory(Path.cwd(), "go")

    def test_subprocess_output_and_duration_are_bounded(self):
        popen = subprocess.Popen
        def run_script(script, **limits):
            def create(_argv, **kwargs):
                return popen([sys.executable, "-c", script], **kwargs)
            with patch.object(go_oracle.subprocess, "Popen", create):
                with patch.multiple(go_oracle, **limits) if limits else nullcontext():
                    return go_oracle._run(Path(sys.executable), b"{}")
        with self.assertRaisesRegex(RuntimeError, "byte limit"):
            run_script("import sys; sys.stdout.buffer.write(b'x'*8192)", MAX_OUTPUT_BYTES=128)
        with self.assertRaisesRegex(RuntimeError, "timed out"):
            run_script("import time; time.sleep(5)", TIMEOUT_SECONDS=0.03)
        with self.assertRaisesRegex(RuntimeError, "no partial"):
            run_script("import sys; sys.stdout.write('{}'); sys.exit(2)")


@unittest.skipUnless(os.environ.get("COMPASS_GO_SOURCE_ORACLE"), "explicit Go oracle build required; native/ordinary tests stay Go-free")
class GoOracleSourceFixtureTests(unittest.TestCase):
    def invoke(self, root):
        request, sources = go_oracle._inputs(root, (), ())
        raw = go_oracle._run(Path(os.environ["COMPASS_GO_SOURCE_ORACLE"]), json.dumps(request).encode())
        payload = json.loads(raw)
        return payload, go_oracle.validate_payload(request, sources, payload)

    def test_local_interface_function_values_conversions_and_same_line_occurrences(self):
        source = """package p
type R struct{}
func (R) Emit() {}
type I interface { Emit() }
type Number int
func café() {}
func Caller(r R, i I, f func()) {
    café(); café()
    r.Emit(); i.Emit(); f()
    _ = int(1); _ = Number(2); _ = len([]int{1})
    { café := f; café() }
    func() { café() }()
}
"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "main.go").write_text(source)
            payload, inventory = self.invoke(root)
            self.assertTrue(payload["files"][0]["typeCheckComplete"])
            calls = payload["files"][0]["calls"]
            statuses = {c["bindingStatus"] for c in calls}
            self.assertTrue({"local_concrete", "interface_dispatch", "function_value", "builtin", "type_conversion"} <= statuses)
            self.assertTrue(all(c["target"] is None for c in calls if c["bindingStatus"] != "local_concrete"))
            same_line = [c for c in inventory.constructs if c.target_spelling == "café" and c.start_line == 8]
            self.assertEqual(2, len(same_line))
            self.assertNotEqual(same_line[0].start_byte, same_line[1].start_byte)
            self.assertTrue(any("closure@" in c.owner_qualified_name for c in inventory.constructs))
            self.assertEqual(inventory, self.invoke(root)[1])
            self.assertEqual(source_construct_inventory_sha256("go", inventory), source_construct_inventory_sha256("go", self.invoke(root)[1]))

    def test_public_provider_recomputes_inventory_and_pins_helper_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "main.go").write_text("package p\nfunc F(){}\nfunc G(){ F() }\n")
            first = independent_source_inventory(root, "go")
            self.assertEqual(first, independent_source_inventory(root, "go"))
            metadata = dict(first.provider_metadata)
            self.assertEqual(64, len(metadata["oracleBinarySha256"]))
            self.assertEqual(64, len(metadata["oracleHelperSha256"]))
            self.assertEqual("deny_external_exports_no_subprocess", metadata["importPolicy"])

    def test_external_imports_never_run_and_unavailable_types_remain_unresolved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "main.go").write_text('package p\nimport "example.invalid/dependency"\nfunc F(){ dependency.Run() }\n')
            (root / "go.mod").write_text("this is deliberately not a valid module\n")
            payload, inventory = self.invoke(root)
            self.assertEqual((1, 1), (inventory.scanned_files, inventory.parsed_files))
            self.assertFalse(payload["files"][0]["typeCheckComplete"])
            self.assertTrue(payload["diagnostics"])
            self.assertTrue(all(c["bindingStatus"] == "unresolved_call_or_conversion" and c["target"] is None for c in payload["files"][0]["calls"]))
            self.assertEqual({"calls", "imports"}, {c.relation for c in inventory.constructs})

    def test_parse_errors_and_duplicate_declarations_cannot_claim_concrete_targets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "main.go").write_text("package p\nfunc F(){}\nfunc G(){ F() }\n")
            (root / "bad.go").write_text("package p\nfunc (")
            payload, inventory = self.invoke(root)
            self.assertEqual((2, 1, ("bad.go",)), (inventory.scanned_files, inventory.parsed_files, inventory.rejected_files))
            self.assertTrue(all(c["target"] is None for f in payload["files"] for c in f["calls"]))
            (root / "bad.go").write_text("package p\nfunc F(){}\n")
            payload, inventory = self.invoke(root)
            self.assertEqual((2, 2), (inventory.scanned_files, inventory.parsed_files))
            self.assertTrue(all(c["target"] is None for f in payload["files"] for c in f["calls"]))

    def test_line_directives_do_not_rewrite_physical_source_anchors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "main.go").write_text("package p\nfunc F(){}\n//line imaginary.go:900\nfunc G(){ F() }\n")
            payload, inventory = self.invoke(root)
            self.assertTrue(payload["files"][0]["typeCheckComplete"])
            self.assertEqual(("main.go", 4), (inventory.constructs[0].source_file, inventory.constructs[0].start_line))

    def test_oversized_identity_rejects_the_entire_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "main.go").write_text("package p\nfunc " + "F" * 4097 + "(){}\n")
            with self.assertRaisesRegex(RuntimeError, "no partial"):
                self.invoke(root)


if __name__ == "__main__":
    unittest.main()
