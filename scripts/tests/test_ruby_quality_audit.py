"""Regression tests for the Ruby qualification oracle's closed-world join."""

from __future__ import annotations

import unittest

from benchmarks.performance.compass.occurrences import SourceConstruct
from scripts.build_ruby_quality_audit import (
    _has_local_target,
    _source_target_is_unambiguous,
)


def _construct(relation: str, target: str, owner: str = "Example") -> SourceConstruct:
    return SourceConstruct(
        source_file="example.rb",
        relation=relation,
        capability=relation,
        owner_qualified_name=owner,
        target_spelling=target,
        qualifier=None,
        start_byte=0,
        end_byte=1,
        start_line=1,
    )


class RubyQualityAuditTests(unittest.TestCase):
    def test_duplicate_methods_are_fail_closed(self) -> None:
        declarations = {"Example#run": ["method", "method"]}
        self.assertFalse(
            _source_target_is_unambiguous("calls", "Example#run", declarations)
        )
        self.assertFalse(
            _has_local_target(
                _construct("calls", "Example#run"),
                {"Example#run"},
                set(),
                declarations,
            )
        )

    def test_reopened_types_remain_local_construct_targets(self) -> None:
        declarations = {"Example": ["class", "class"]}
        self.assertTrue(
            _source_target_is_unambiguous("instantiates", "Example", declarations)
        )
        self.assertTrue(
            _has_local_target(
                _construct("instantiates", "Example#new"),
                {"Example"},
                {"Example"},
                declarations,
            )
        )

    def test_only_module_declarations_are_trait_targets(self) -> None:
        self.assertTrue(
            _source_target_is_unambiguous(
                "implements", "Support", {"Support": ["module", "module"]}
            )
        )
        self.assertFalse(
            _source_target_is_unambiguous(
                "implements", "Support", {"Support": ["module", "class"]}
            )
        )


if __name__ == "__main__":
    unittest.main()
