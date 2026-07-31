from __future__ import annotations

import unittest

from benchmarks.performance.compass.model import ProcessMetrics, Sample
from benchmarks.performance.compass.stats import speedup, summarize


def sample(index: int, seconds: float, *, eligible: bool = True) -> Sample:
    metrics = ProcessMetrics(
        wall_seconds=seconds,
        user_seconds=seconds / 2,
        system_seconds=0.01,
        peak_rss_kib=100 + index,
        return_code=0,
        signal=None,
        timed_out=False,
        command=("fixture",),
        cwd="/tmp",
        stdout_path="/tmp/out",
        stderr_path="/tmp/err",
        stdout_sha256="a" * 64,
        stderr_sha256="b" * 64,
    )
    return Sample(str(index), "compass", "fixture", "cold", index, eligible, metrics)


class StatsTests(unittest.TestCase):
    def test_three_sample_summary_uses_median_and_nearest_rank_p95(self) -> None:
        result = summarize([sample(1, 3.0), sample(2, 1.0), sample(3, 2.0)])
        self.assertEqual(result.p50_seconds, 2.0)
        self.assertEqual(result.p95_seconds, 3.0)
        self.assertEqual(result.mad_seconds, 1.0)
        self.assertEqual(result.peak_rss_kib, 103)

    def test_ten_sample_p95_is_tenth_value(self) -> None:
        result = summarize([sample(index, float(index)) for index in range(1, 11)])
        self.assertEqual(result.p95_seconds, 10.0)

    def test_ineligible_samples_do_not_count(self) -> None:
        with self.assertRaisesRegex(ValueError, "three"):
            summarize([sample(1, 1.0), sample(2, 2.0), sample(3, 3.0, eligible=False)])

    def test_speedup_uses_medians(self) -> None:
        graphify = summarize([sample(1, 10.0), sample(2, 11.0), sample(3, 12.0)])
        compass = summarize([sample(1, 2.0), sample(2, 2.2), sample(3, 2.4)])
        self.assertEqual(speedup(graphify, compass), 5.0)


if __name__ == "__main__":
    unittest.main()
