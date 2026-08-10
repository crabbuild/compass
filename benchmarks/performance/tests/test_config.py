from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from benchmarks.performance.compass.config import load_suite


ROOT = Path(__file__).resolve().parents[1]


class ConfigTests(unittest.TestCase):
    def test_checked_in_suite_is_complete(self) -> None:
        suite = load_suite(ROOT / "repositories.toml")
        self.assertEqual(len(suite.repositories), 8)
        self.assertEqual(
            {item.name for item in suite.repositories},
            {"django", "spring", "rails", "laravel", "bevy", "aspnetcore", "angular", "entire"},
        )
        self.assertEqual(len(suite.digest), 64)
        self.assertEqual(sum(len(item.queries) for item in suite.repositories), 16)
        for repository in suite.repositories:
            for oracle in repository.queries:
                self.assertTrue(
                    oracle.expected_seeds or oracle.acceptable_seeds or oracle.allow_no_match
                )
                self.assertIn(oracle.expected_direction, {"incoming", "outgoing", "both"})
        self.assertTrue(all(len(item.commit) == 40 for item in suite.repositories))
        aspnetcore = next(item for item in suite.repositories if item.name == "aspnetcore")
        self.assertTrue(all(seed.source is None for seed in aspnetcore.queries[0].forbidden_seeds))
        for repository in suite.repositories:
            for oracle in repository.queries:
                self.assertTrue(
                    all(seed.source is not None for seed in oracle.expected_seeds)
                )
                self.assertTrue(
                    all(node.source is not None for node in oracle.relevant_nodes)
                )

    def test_unknown_field_is_rejected(self) -> None:
        raw = (ROOT / "repositories.toml").read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "suite.toml"
            path.write_text(raw.replace('name = "django"', 'name = "django"\nunknown = true', 1))
            with self.assertRaisesRegex(ValueError, "unknown fields"):
                load_suite(path)

    def test_duplicate_repository_is_rejected(self) -> None:
        raw = (ROOT / "repositories.toml").read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "suite.toml"
            path.write_text(raw.replace('name = "spring"', 'name = "django"', 1))
            with self.assertRaisesRegex(ValueError, "duplicate"):
                load_suite(path)

    def test_non_https_url_is_rejected(self) -> None:
        raw = (ROOT / "repositories.toml").read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "suite.toml"
            path.write_text(raw.replace("https://github.com/django/django.git", "git@github.com:django/django.git"))
            with self.assertRaisesRegex(ValueError, "HTTPS"):
                load_suite(path)

    def test_compass_oracle_cannot_fall_back_to_substring_only(self) -> None:
        raw = (ROOT / "repositories.toml").read_text(encoding="utf-8")
        raw = raw.replace(
            'expectedSeeds = [{ qualifiedName = "django.urls.resolvers.URLResolver::resolve", source = { file = "django/urls/resolvers.py", startLine = 670 } }]\n',
            "",
            1,
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "suite.toml"
            path.write_text(raw, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "needs expectedSeeds"):
                load_suite(path)

    def test_expected_seed_requires_source_even_when_forbidden_seed_does_not(self) -> None:
        raw = (ROOT / "repositories.toml").read_text(encoding="utf-8")
        raw = raw.replace(
            'expectedSeeds = [{ qualifiedName = "django.urls.resolvers.URLResolver::resolve", source = { file = "django/urls/resolvers.py", startLine = 670 } }]',
            'expectedSeeds = [{ qualifiedName = "django.urls.resolvers.URLResolver::resolve" }]',
            1,
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "suite.toml"
            path.write_text(raw, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "source must be an anchor table"):
                load_suite(path)

    def test_allow_no_match_cannot_also_declare_a_seed(self) -> None:
        raw = (ROOT / "repositories.toml").read_text(encoding="utf-8")
        raw = raw.replace(
            'expectedDirection = "both"\nrelevantNodes = [{ qualifiedName = "django.urls.resolvers.URLResolver::resolve"',
            'expectedDirection = "both"\nallowNoMatch = true\nrelevantNodes = [{ qualifiedName = "django.urls.resolvers.URLResolver::resolve"',
            1,
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "suite.toml"
            path.write_text(raw, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "cannot combine allowNoMatch"):
                load_suite(path)


if __name__ == "__main__":
    unittest.main()
