from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from benchmarks.performance.compass_perf.config import load_suite


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


if __name__ == "__main__":
    unittest.main()

