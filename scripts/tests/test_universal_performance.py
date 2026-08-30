import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from qualify_universal_language import compare_performance_baseline


class UniversalPerformanceBaselineTests(unittest.TestCase):
    def _baseline(self, directory: Path) -> Path:
        path = directory / "baseline.json"
        path.write_text(
            json.dumps(
                {
                    "compassRevision": "88abe4c0",
                    "cold": {
                        "medianSeconds": 1.0,
                        "first": {"seconds": 1.0, "rssBytes": 100},
                        "second": {"seconds": 1.0, "rssBytes": 100},
                    },
                    "warm": {
                        "medianSeconds": 0.5,
                        "samples": [{"seconds": 0.5, "rssBytes": 100}],
                    },
                    "factNeutral": {"seconds": 0.5, "rssBytes": 100},
                    "semanticEdit": {"seconds": 0.5, "rssBytes": 100},
                    "forced": {"seconds": 0.5, "rssBytes": 100},
                    "alternateCheckout": {"seconds": 0.5, "rssBytes": 100},
                    "restore": {"seconds": 0.5, "rssBytes": 100},
                    "performanceGates": {
                        "coldMultiplier": 1.1,
                        "warmMultiplier": 1.15,
                        "factNeutralAdditiveSeconds": 0.25,
                        "peakRssMultiplier": 1.15,
                        "peakRssAdditiveBytes": 32,
                    },
                }
            ),
            encoding="utf-8",
        )
        return path

    def test_comparison_accepts_values_inside_all_gates(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            baseline = self._baseline(Path(directory))
            result = compare_performance_baseline(
                {
                    "cold": {"seconds": 1.5},
                    "warm": {"medianSeconds": 0.6},
                    "factNeutral": {"seconds": 0.7},
                    "peakRssBytes": 120,
                },
                baseline,
            )
        self.assertTrue(result["passed"])

    def test_comparison_rejects_cold_regression(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            baseline = self._baseline(Path(directory))
            result = compare_performance_baseline(
                {
                    "cold": {"seconds": 2.1},
                    "warm": {"medianSeconds": 0.6},
                    "factNeutral": {"seconds": 0.7},
                    "peakRssBytes": 120,
                },
                baseline,
            )
        self.assertFalse(result["passed"])


if __name__ == "__main__":
    unittest.main()
