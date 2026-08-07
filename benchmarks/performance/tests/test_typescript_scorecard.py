from __future__ import annotations

import copy
import json
from pathlib import Path
import tempfile
import unittest

from benchmarks.performance.compass.typescript_scorecard import (
    SCORECARD_SCHEMA,
    TypeScriptScorecardError,
    scorecard_result,
)


def record(
    record_id: str,
    *,
    pool: str,
    judgment: str,
    corpus: str = "zod",
    capability: str = "calls",
    relation: str = "calls",
    cluster: str = "local::target",
    judgment_source: str = "manual",
    reason: str | None = None,
) -> dict[str, object]:
    value: dict[str, object] = {
        "id": record_id,
        "corpus": corpus,
        "adapter": "typescript",
        "language": "typescript",
        "capability": capability,
        "relation": relation,
        "pool": pool,
        "targetCluster": cluster,
        "sourceFile": "src/main.ts",
        "startByte": 1,
        "endByte": 5,
        "judgment": judgment,
        "judgmentSource": judgment_source,
    }
    if judgment != "correct":
        value["reason"] = reason or f"reviewed {judgment}"
    elif reason is not None:
        value["reason"] = reason
    return value


def scorecard(*, mode: str = "diagnostic") -> dict[str, object]:
    return {
        "schema": SCORECARD_SCHEMA,
        "mode": mode,
        "provider": "typescript_checker_api_5_9_3",
        "oracleScriptSha256": "a" * 64,
        "candidateAdapter": "compass.typescript.candidate",
        "corpora": [
            {"name": "axios", "commit": "1" * 40},
            {"name": "nest", "commit": "2" * 40},
            {"name": "vite", "commit": "3" * 40},
            {"name": "zod", "commit": "4" * 40},
        ],
        "releaseGateCorpora": ["axios", "nest", "vite", "zod"],
        "advertisedCapabilities": [
            {"adapter": "typescript", "capability": "calls"},
            {"adapter": "typescript", "capability": "members"},
        ],
        "requiredRelations": ["accesses", "calls"],
        "comparators": [
            {
                "name": "graphify",
                "version": "0.9.26",
                "scopeDigest": "b" * 64,
                "equivalentScope": False,
                "adjudicated": False,
            },
            {
                "name": "scip_typescript",
                "version": "0.4.0",
                "scopeDigest": "c" * 64,
                "equivalentScope": False,
                "adjudicated": False,
            },
        ],
        "records": [],
    }


class TypeScriptScorecardTests(unittest.TestCase):
    def write(self, value: dict[str, object]) -> Path:
        temporary = Path(self.temporary.name)
        path = temporary / "scorecard.json"
        path.write_text(
            json.dumps(value, sort_keys=True, separators=(",", ":")),
            encoding="utf-8",
        )
        return path

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)

    def test_diagnostic_result_is_deterministic_and_not_claim_eligible(self) -> None:
        value = scorecard()
        value["records"] = [
            record("accepted-correct", pool="accepted", judgment="correct"),
            record(
                "accepted-invalid",
                pool="accepted",
                judgment="invalid",
                capability="members",
                relation="accesses",
            ),
            record("oracle-correct", pool="source_oracle", judgment="correct"),
            record(
                "oracle-missing",
                pool="source_oracle",
                judgment="missing",
                capability="members",
                relation="accesses",
            ),
        ]
        path = self.write(value)
        first = scorecard_result(path)
        second = scorecard_result(path)

        self.assertEqual(first, second)
        self.assertTrue(first["passed"])
        self.assertFalse(first["eligibleForQualityClaim"])
        self.assertEqual(first["precision"]["numerator"], 1)
        self.assertEqual(first["precision"]["denominator"], 2)
        self.assertEqual(first["recall"]["numerator"], 1)
        self.assertEqual(first["recall"]["denominator"], 2)
        self.assertEqual(first["judgments"], {"correct": 2, "invalid": 1, "missing": 1})

    def test_qualification_requires_release_corpora_and_strata_minimums(self) -> None:
        value = scorecard(mode="qualification")
        value["releaseGateCorpora"] = ["zod"]
        value["records"] = [record("one", pool="accepted", judgment="correct")]
        result = scorecard_result(self.write(value))

        self.assertFalse(result["passed"])
        self.assertTrue(any("releaseGateCorpora" in failure for failure in result["failures"]))
        self.assertTrue(any("2000 required" in failure for failure in result["failures"]))
        self.assertTrue(any("400 required" in failure for failure in result["failures"]))
        self.assertTrue(any("100 required" in failure for failure in result["failures"]))

    def test_missing_or_unrecognized_judgments_fail_closed(self) -> None:
        missing = scorecard()
        missing["records"] = [record("missing", pool="accepted", judgment="correct")]
        del missing["records"][0]["judgment"]
        with self.assertRaisesRegex(TypeScriptScorecardError, "missing fields: judgment"):
            scorecard_result(self.write(missing))

        unknown = scorecard()
        unknown["records"] = [record("unknown", pool="accepted", judgment="invented")]
        with self.assertRaisesRegex(TypeScriptScorecardError, "judgment is unknown"):
            scorecard_result(self.write(unknown))

    def test_automatic_judgment_source_is_rejected(self) -> None:
        value = scorecard()
        value["records"] = [
            record(
                "automatic",
                pool="accepted",
                judgment="correct",
                judgment_source="automatic",
            )
        ]
        with self.assertRaisesRegex(TypeScriptScorecardError, "judgmentSource"):
            scorecard_result(self.write(value))

    def test_non_correct_judgments_require_review_reason(self) -> None:
        value = scorecard()
        value["records"] = [
            record(
                "missing-reason",
                pool="source_oracle",
                judgment="missing",
                reason="",
            )
        ]
        del value["records"][0]["reason"]
        with self.assertRaisesRegex(TypeScriptScorecardError, "reason is required"):
            scorecard_result(self.write(value))

    def test_records_must_be_sorted_and_unique(self) -> None:
        value = scorecard()
        value["records"] = [
            record("z", pool="accepted", judgment="correct"),
            record("a", pool="accepted", judgment="correct"),
        ]
        with self.assertRaisesRegex(TypeScriptScorecardError, "sorted by unique id"):
            scorecard_result(self.write(value))

        duplicate = copy.deepcopy(value)
        duplicate["records"] = [
            record("a", pool="accepted", judgment="correct"),
            record("a", pool="accepted", judgment="correct"),
        ]
        with self.assertRaisesRegex(TypeScriptScorecardError, "sorted by unique id"):
            scorecard_result(self.write(duplicate))

    def test_leadership_requires_adjudicated_equivalent_comparators(self) -> None:
        value = scorecard(mode="leadership")
        value["records"] = [record("one", pool="accepted", judgment="correct")]
        result = scorecard_result(self.write(value))

        self.assertFalse(result["passed"])
        self.assertTrue(
            any("leadership comparator 'graphify'" in failure for failure in result["failures"])
        )
        self.assertTrue(
            any(
                "leadership comparator 'scip_typescript'" in failure
                for failure in result["failures"]
            )
        )

    def test_critical_semantic_judgment_is_a_failure(self) -> None:
        value = scorecard(mode="qualification")
        value["records"] = [
            record("critical", pool="accepted", judgment="fabricated_occurrence")
        ]
        result = scorecard_result(self.write(value))
        self.assertTrue(
            any("fabricated_occurrence" in failure for failure in result["failures"])
        )


if __name__ == "__main__":
    unittest.main()
