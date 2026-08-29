import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from check_universal_evidence_promotion import (  # noqa: E402
    MANIFEST,
    MAX_MANIFEST_BYTES,
    PromotionError,
    load,
    validate,
)


class UniversalEvidencePromotionTests(unittest.TestCase):
    def test_checked_in_decision_promotes_every_registry_pipeline(self) -> None:
        document = load(MANIFEST)
        self.assertEqual(document["decision"], "promote")
        self.assertEqual(document["scope"], "advertised-bounded-capabilities")
        self.assertEqual(document["review"]["status"], "approved")
        self.assertEqual(len(document["pipelines"]), 14)
        self.assertTrue(all(item["decision"] == "qualified" for item in document["pipelines"]))

    def test_pipeline_order_and_versions_are_contractual(self) -> None:
        document = load(MANIFEST)
        invalid = copy.deepcopy(document)
        invalid["pipelines"][0], invalid["pipelines"][1] = (
            invalid["pipelines"][1],
            invalid["pipelines"][0],
        )
        with self.assertRaisesRegex(PromotionError, "exactly the sorted"):
            validate(invalid)

    def test_unqualified_pipeline_is_rejected(self) -> None:
        document = json.loads(MANIFEST.read_text(encoding="utf-8"))
        document["pipelines"][0]["decision"] = "qualifying"
        with self.assertRaisesRegex(PromotionError, "not qualified"):
            validate(document)

    def test_policy_drift_is_rejected(self) -> None:
        document = json.loads(MANIFEST.read_text(encoding="utf-8"))
        document["requiredGates"]["minimumCapabilityRecall"] = 0.9
        with self.assertRaisesRegex(PromotionError, "requiredGates"):
            validate(document)

    def test_oversized_record_is_rejected_before_json_parsing(self) -> None:
        with tempfile.TemporaryDirectory(prefix="compass-promotion-test-") as directory:
            path = Path(directory) / "oversized.json"
            path.write_bytes(b" " * (MAX_MANIFEST_BYTES + 1))
            with self.assertRaisesRegex(PromotionError, "1 MiB"):
                load(path)


if __name__ == "__main__":
    unittest.main()
