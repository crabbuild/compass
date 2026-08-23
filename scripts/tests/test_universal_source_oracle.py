"""Regression tests for the four bounded universal source-oracle contracts."""

from __future__ import annotations

import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from benchmarks.performance.compass.occurrences import SourceConstruct  # noqa: E402
from build_universal_quality_audit import _declaration_overlap_matches  # noqa: E402
from independent_language_oracle import canonical_bytes, run_oracle  # noqa: E402


class UniversalSourceOracleTests(unittest.TestCase):
    def test_groovy_declaration_overlap_requires_one_target(self) -> None:
        construct = SourceConstruct(
            "Module.groovy",
            "implements",
            "base_types",
            "wave.UserStore",
            "Store",
            None,
            100,
            200,
            1,
        )
        edge = {
            "anchor": ("Module.groovy", 150, 155),
            "target": "store-id",
            "targetNode": {"qualifiedName": "wave.Store"},
        }
        self.assertEqual(
            [edge],
            _declaration_overlap_matches(
                "groovy",
                construct,
                frozenset(("implements",)),
                {("Module.groovy", "implements"): [edge]},
            ),
        )
        ambiguous = {**edge, "target": "other-store-id"}
        self.assertEqual(
            [],
            _declaration_overlap_matches(
                "groovy",
                construct,
                frozenset(("implements",)),
                {("Module.groovy", "implements"): [edge, ambiguous]},
            ),
        )

    def test_dart_ownership_overlap_requires_one_target(self) -> None:
        construct = SourceConstruct(
            "lib/system.dart",
            "contains",
            "ownership",
            "SystemChannels",
            "SystemChannels",
            None,
            100,
            400,
            1,
        )
        edge = {
            "anchor": ("lib/system.dart", 110, 125),
            "target": "system-id",
            "targetNode": {"qualifiedName": "SystemChannels"},
        }
        self.assertEqual(
            [edge],
            _declaration_overlap_matches(
                "dart",
                construct,
                frozenset(("contains",)),
                {("lib/system.dart", "contains"): [edge]},
            ),
        )

    def test_metadata_is_reported_without_changing_inventory_digest(self) -> None:
        cases = (
            ("swift", ".swift", "SwiftSyntax provider unavailable"),
            ("dart", ".dart", "Dart Analyzer provider unavailable"),
            ("scala", ".scala", "scala.meta provider unavailable"),
            ("groovy", ".groovy", "Groovy CompilationUnit provider unavailable"),
        )
        for language, suffix, provider_name in cases:
            with self.subTest(language=language), tempfile.TemporaryDirectory(
                prefix=f"compass-{language}-oracle-test-"
            ) as directory:
                root = Path(directory)
                source = root / f"main{suffix}"
                source.write_text(
                    "class Sample { void run() { helper(); } void helper() {} }\n",
                    encoding="utf-8",
                )
                document = run_oracle(
                    root,
                    language=language,
                    provider=f"{language}-provider",
                    toolchain="pinned test toolchain",
                    implementation=f"bounded_lexical_scanner; {provider_name}",
                    parser_available=False,
                    suffixes=(suffix,),
                )
                self.assertFalse(document["parserAvailable"])
                self.assertIn(provider_name, document["implementation"])
                inventory = {
                    "language": document["language"],
                    "provider": document["provider"],
                    "toolchain": document["toolchain"],
                    "rootRelativeFiles": [item["path"] for item in document["files"]],
                    "files": document["files"],
                }
                expected = hashlib.sha256(
                    canonical_bytes(inventory).rstrip(b"\n")
                ).hexdigest()
                self.assertEqual(expected, document["inventorySha256"])

    def test_include_and_exclude_globs_define_the_complete_inventory(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-universal-oracle-globs-") as directory:
            root = Path(directory)
            (root / "lib").mkdir()
            (root / "tests").mkdir()
            (root / "lib" / "main.swift").write_text("class Main {}\n", encoding="utf-8")
            (root / "tests" / "main.swift").write_text("class Test {}\n", encoding="utf-8")
            document = run_oracle(
                root,
                language="swift",
                provider="swift-provider",
                toolchain="pinned test toolchain",
                suffixes=(".swift",),
                include_globs=("lib/**/*.swift", "tests/**/*.swift"),
                exclude_globs=("tests/**",),
            )
            self.assertEqual(["lib/main.swift"], [item["path"] for item in document["files"]])


if __name__ == "__main__":
    unittest.main()
