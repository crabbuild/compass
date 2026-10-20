"""Exposure integrity contracts, not independent attestations or source labels."""
import copy
import unittest

from benchmarks.agent_query.broad_query_review import (
    _repository_key, exposure_failures, sha, validate_registration,
)
from benchmarks.agent_query.tests.test_broad_query_review import registration


EVIDENCE = b'{"historicalCapture":"fixture; no real tool execution"}'


def with_lineage(reg=None):
    reg = copy.deepcopy(reg if reg is not None else registration())
    reg.update(designation="heldout", qualificationEligible=True)
    for task in reg["tasks"]:
        task.update(split="confirmation", mechanisms=["store/persistence"])
    reg["exposureLineage"] = dict(
        schema="compass.broad-query-exposure-lineage/1", frozenAt=reg["frozenAt"],
        historyCompleteThrough=reg["frozenAt"], historyComplete=True,
        author="fixture-history-author", repositoryAliases=[], mechanismLinks=[],
        independentReview=dict(status="confirmed", reviewer="fixture-history-reviewer",
                               reviewerType="ai", sourceAware=True, independentOfAuthor=True,
                               reviewedAt=reg["frozenAt"]),
        history=[dict(id="old-capture", observedAt="2025-12-01T00:00:00Z", kind="query_capture",
                      repositoryUrl="https://example.invalid/other", commit="b" * 40,
                      mechanisms=["other/mechanism"], evidenceSha256=sha(EVIDENCE))])
    return reg


class ExposureTests(unittest.TestCase):
    def check(self, reg):
        return exposure_failures(reg, lambda digest: EVIDENCE)

    def test_recorded_clean_split_passes_only_the_exposure_check(self):
        reg = with_lineage()
        self.assertEqual(self.check(reg), [])
        self.assertEqual(validate_registration(reg), reg)
        self.assertIn("bytes were not verified", " ".join(exposure_failures(reg)))

    def test_legacy_shape_remains_readable_but_cannot_qualify_without_lineage(self):
        reg = registration()
        self.assertEqual(validate_registration(reg), reg)
        self.assertIn("exposure lineage is missing", " ".join(exposure_failures(reg)))

    def test_new_id_question_and_commit_cannot_erase_repository_exposure(self):
        reg = with_lineage()
        reg["exposureLineage"]["history"][0]["repositoryUrl"] = reg["repositories"][0]["url"]
        reg["tasks"][0].update(id="renamed", question="Entirely new wording")
        self.assertIn("previously exposed", " ".join(self.check(reg)))

    def test_github_origin_spelling_does_not_create_fresh_source(self):
        reg = with_lineage()
        reg["repositories"][0]["url"] = "https://github.com/Owner/Repo"
        reg["exposureLineage"]["history"][0]["repositoryUrl"] = "https://WWW.GITHUB.COM.:443/OWNER/REPO.git/"
        self.assertIn("previously exposed", " ".join(self.check(reg)))

    def test_explicit_repository_rename_links_are_transitive(self):
        reg = with_lineage()
        reg["exposureLineage"]["repositoryAliases"] = [
            [reg["repositories"][0]["url"], "https://example.invalid/intermediate"],
            ["https://example.invalid/intermediate", "https://example.invalid/other"]]
        self.assertIn("previously exposed", " ".join(self.check(reg)))

    def test_explicit_shared_mechanisms_are_transitive_across_repositories(self):
        reg = with_lineage()
        reg["exposureLineage"]["mechanismLinks"] = [
            ["store/persistence", "shared/serialization"],
            ["other/mechanism", "shared/serialization"]]
        self.assertIn("previously exposed", " ".join(self.check(reg)))

    def test_same_commit_under_an_unlinked_fork_url_is_still_exposed_source(self):
        reg = with_lineage()
        reg["repositories"][0]["commit"] = reg["exposureLineage"]["history"][0]["commit"]
        self.assertIn("previously exposed", " ".join(self.check(reg)))

    def test_github_tree_url_cannot_masquerade_as_a_new_repository(self):
        with self.assertRaisesRegex(ValueError, "GitHub repository root"):
            _repository_key("https://github.com/owner/repo/tree/new-branch")

    def test_development_controls_remain_capturable_but_cannot_be_heldout(self):
        reg = with_lineage()
        reg["tasks"][0]["split"] = "development"
        self.assertIn("development task", " ".join(self.check(reg)))
        reg["designation"] = "development"
        reg["qualificationEligible"] = False
        self.assertEqual(self.check(reg), [])

    def test_all_tasks_need_explicit_split_and_mechanism_lineage(self):
        for field in ("split", "mechanisms"):
            reg = with_lineage()
            del reg["tasks"][0][field]
            with self.assertRaises(ValueError):
                self.check(reg)

    def test_aliasing_two_registered_repositories_does_not_add_independent_units(self):
        reg = with_lineage()
        other = copy.deepcopy(reg["repositories"][0])
        other["id"] = "other-id"
        reg["repositories"].append(other)
        with self.assertRaisesRegex(ValueError, "repeats an aliased repository"):
            self.check(reg)

    def test_two_fork_urls_at_the_same_commit_do_not_add_independent_units(self):
        reg = with_lineage()
        other = copy.deepcopy(reg["repositories"][0])
        other.update(id="fork", url="https://example.invalid/new-fork")
        reg["repositories"].append(other)
        with self.assertRaisesRegex(ValueError, "repeats a source commit"):
            self.check(reg)

    def test_independent_history_acceptance_is_required_and_not_self_issued(self):
        for change in (dict(status="pending"), dict(independentOfAuthor=False),
                       dict(reviewer="fixture-history-author"),
                       dict(reviewer="AI source author"), dict(reviewer=None)):
            reg = with_lineage()
            reg["exposureLineage"]["independentReview"].update(change)
            self.assertTrue(self.check(reg))
        reg = with_lineage()
        reg["exposureLineage"]["historyComplete"] = False
        self.assertIn("incomplete", " ".join(self.check(reg)))

    def test_stale_cutoff_post_freeze_review_and_post_review_event_are_rejected(self):
        for field in ("frozenAt", "historyCompleteThrough"):
            reg = with_lineage()
            reg["exposureLineage"][field] = "2025-12-31T23:59:59Z"
            with self.assertRaisesRegex(ValueError, "match source freeze"):
                self.check(reg)
        reg = with_lineage()
        reg["exposureLineage"]["independentReview"]["reviewedAt"] = "2026-01-02T00:00:00Z"
        with self.assertRaisesRegex(ValueError, "follows source freeze"):
            self.check(reg)
        reg = with_lineage()
        reg["exposureLineage"]["history"][0]["observedAt"] = "2026-01-02T00:00:00Z"
        with self.assertRaisesRegex(ValueError, "follows independent history review"):
            self.check(reg)

    def test_changed_evidence_bytes_unknown_schema_and_duplicate_events_fail(self):
        reg = with_lineage()
        with self.assertRaisesRegex(ValueError, "evidence bytes differ"):
            exposure_failures(reg, lambda digest: b"changed history")
        reg["exposureLineage"]["schema"] = "compass.broad-query-exposure-lineage/99"
        with self.assertRaisesRegex(ValueError, "unsupported exposure lineage"):
            self.check(reg)
        reg = with_lineage()
        reg["exposureLineage"]["history"] *= 2
        with self.assertRaisesRegex(ValueError, "duplicate exposure event"):
            self.check(reg)

    def test_history_is_bounded_and_source_url_aliases_cannot_hide_in_escapes(self):
        reg = with_lineage()
        reg["exposureLineage"]["history"] *= 8193
        with self.assertRaisesRegex(ValueError, "0..8192"):
            self.check(reg)
        for url in ("https://host/owner/%72epo", "https://host/owner/../repo",
                    "https://user@host/repo", "https://host/repo?alias=other", "https://host/a\\b"):
            with self.assertRaises(ValueError):
                _repository_key(url)


if __name__ == "__main__":
    unittest.main()
