"""Regression tests for the Ruby source oracle and qualification entry point."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import qualify_ruby_universal as qualification  # noqa: E402


class RubyQualificationTests(unittest.TestCase):
    def test_fixture_oracle_is_byte_deterministic_and_has_required_families(self) -> None:
        report = qualification.run_deterministic_oracle(
            ROOT / "fixtures" / "code-graph" / "qualification",
            ROOT / "scripts" / "ruby_source_oracle.rb",
        )
        self.assertTrue(report["deterministic"])
        self.assertGreaterEqual(report["declarations"], 1)
        self.assertGreaterEqual(report["relationFamilies"].get("extends", 0), 1)
        self.assertGreaterEqual(report["relationFamilies"].get("uses_trait", 0), 1)
        self.assertGreaterEqual(report["relationFamilies"].get("imports", 0), 1)

    def test_oracle_marks_malformed_and_invalid_utf8_as_partial(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-ruby-oracle-test-") as directory:
            root = Path(directory)
            (root / "malformed.rb").write_text("class Broken\n  def nope(\n", encoding="utf-8")
            (root / "invalid.rb").write_bytes(b"class Invalid\n\xff\nend\n")
            document, _raw = qualification.run_oracle(
                root,
                ROOT / "scripts" / "ruby_source_oracle.rb",
            )
            statuses = {item["path"]: item["status"] for item in document["files"]}
            self.assertEqual(statuses, {"invalid.rb": "partial", "malformed.rb": "partial"})

    def test_oracle_keeps_qualified_mixin_paths_and_exact_utf8_ranges(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-ruby-oracle-mixin-") as directory:
            root = Path(directory)
            source = "class Account\n  include Billing::Auditable\n  include(dynamic_target)\nend\n"
            path = root / "account.rb"
            path.write_text(source, encoding="utf-8")
            document, _raw = qualification.run_oracle(
                root,
                ROOT / "scripts" / "ruby_source_oracle.rb",
            )
            relations = [
                item
                for item in document["files"][0]["relations"]
                if item["relation"] == "uses_trait"
            ]
            self.assertEqual(len(relations), 1)
            relation = relations[0]
            self.assertEqual(relation["target"], "Billing::Auditable")
            start = source.index("Billing")
            end = start + len("Billing::Auditable".encode("utf-8"))
            self.assertEqual(relation["anchor"]["startByte"], start)
            self.assertEqual(relation["anchor"]["endByte"], end)

    def test_oracle_excludes_dynamic_dispatch_from_call_evidence(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-ruby-oracle-dynamic-") as directory:
            root = Path(directory)
            (root / "dynamic.rb").write_text(
                "class Account\n"
                "  send(:save)\n"
                "  public_send(:save)\n"
                "  include Billing::Auditable\n"
                "end\n",
                encoding="utf-8",
            )
            document, _raw = qualification.run_oracle(
                root,
                ROOT / "scripts" / "ruby_source_oracle.rb",
            )
            calls = [
                item
                for item in document["files"][0]["relations"]
                if item["relation"] == "calls"
            ]
            self.assertEqual(calls, [])

    def test_oracle_excludes_unowned_and_dynamic_mixin_sites(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-ruby-oracle-mixin-owner-") as directory:
            root = Path(directory)
            (root / "dynamic.rb").write_text(
                "include TopLevelMixin\n"
                "class Account\n"
                "  def install\n"
                "    include MethodMixin\n"
                "  end\n"
                "  Class.new do\n"
                "    include AnonymousMixin\n"
                "  end\n"
                "  include OwnedMixin\n"
                "end\n",
                encoding="utf-8",
            )
            document, _raw = qualification.run_oracle(
                root,
                ROOT / "scripts" / "ruby_source_oracle.rb",
            )
            relations = [
                item
                for item in document["files"][0]["relations"]
                if item["relation"] == "uses_trait"
            ]
            self.assertEqual([item["target"] for item in relations], ["OwnedMixin"])

    def test_cli_report_is_canonical_machine_json(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-ruby-report-test-") as directory:
            output = Path(directory) / "report.json"
            self.assertEqual(
                qualification.main(
                    [
                        "--mode",
                        "fixture",
                        "--output",
                        str(output),
                    ]
                ),
                0,
            )
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(report["schema"], qualification.SCHEMA)
            self.assertEqual(
                output.read_bytes(),
                qualification.canonical_bytes(report),
            )


if __name__ == "__main__":
    unittest.main()
