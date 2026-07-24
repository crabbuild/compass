from __future__ import annotations

import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "check_compatibility_manifest.py"
SPEC = importlib.util.spec_from_file_location("check_compatibility_manifest", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class CompatibilityManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.valid = CHECKER.load_manifest()

    def test_real_manifest_shape_is_valid(self) -> None:
        self.assertEqual(CHECKER.validate_manifest(self.valid), [])

    def test_short_sha_is_rejected(self) -> None:
        data = copy.deepcopy(self.valid)
        data["oracle"]["commit"] = "edec9ea"
        errors = CHECKER.validate_manifest(data)
        self.assertTrue(any("oracle.commit" in error for error in errors), errors)

    def test_short_release_sha_is_rejected(self) -> None:
        data = copy.deepcopy(self.valid)
        data["oracle"]["release_commit"] = "edec9ea"
        errors = CHECKER.validate_manifest(data)
        self.assertTrue(
            any("oracle.release_commit" in error for error in errors),
            errors,
        )

    def test_duplicate_capability_is_rejected(self) -> None:
        data = copy.deepcopy(self.valid)
        data["capability_audit"].append(copy.deepcopy(data["capability_audit"][0]))
        errors = CHECKER.validate_manifest(data)
        self.assertTrue(
            any("capability_audit.key contains duplicates" in error for error in errors),
            errors,
        )

    def test_unknown_disposition_is_rejected(self) -> None:
        data = copy.deepcopy(self.valid)
        data["capability_audit"][0]["status"] = "maybe"
        errors = CHECKER.validate_manifest(data)
        self.assertTrue(any("unknown disposition" in error for error in errors), errors)

    def test_mutable_workflow_ref_is_rejected(self) -> None:
        errors = CHECKER.validate_checkout_refs(
            "repository: Graphify-Labs/graphify\nref: v8\n",
            self.valid["oracle"]["commit"],
            "workflow.yml",
        )
        self.assertTrue(any("'v8'" in error for error in errors), errors)

    def test_pinned_workflow_ref_is_accepted(self) -> None:
        commit = self.valid["oracle"]["commit"]
        errors = CHECKER.validate_checkout_refs(
            f"repository: Graphify-Labs/graphify\nref: {commit}\n",
            commit,
            "workflow.yml",
        )
        self.assertEqual(errors, [])

    def test_repository_rejects_drifted_ledger(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            workflows = root / ".github" / "workflows"
            workflows.mkdir(parents=True)
            checkout = (
                "repository: Graphify-Labs/graphify\n"
                f"ref: {self.valid['oracle']['commit']}\n"
            )
            (workflows / "compass-ci.yml").write_text(checkout, encoding="utf-8")
            (workflows / "compass-hardening.yml").write_text(
                checkout, encoding="utf-8"
            )
            (root / "COMPATIBILITY.md").write_text(
                (
                    f"{self.valid['oracle']['commit']}\n"
                    f"{self.valid['oracle']['release_commit']}\n"
                    "compatibility.toml is authoritative\n"
                ),
                encoding="utf-8",
            )

            errors = CHECKER.validate_repository(root, self.valid)

        self.assertTrue(
            any("upstream main commit" in error for error in errors),
            errors,
        )


if __name__ == "__main__":
    unittest.main()
