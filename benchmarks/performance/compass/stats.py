"""Deterministic benchmark statistics."""

from __future__ import annotations

import math
import statistics
from collections.abc import Sequence

from .model import Aggregate, Sample


def summarize(samples: Sequence[Sample]) -> Aggregate:
    eligible = [
        sample
        for sample in samples
        if sample.eligible
        and not sample.metrics.timed_out
        and sample.metrics.return_code == 0
        and sample.metrics.wall_seconds > 0
    ]
    if len(eligible) < 3:
        raise ValueError("at least three eligible successful samples are required")
    seconds = sorted(sample.metrics.wall_seconds for sample in eligible)
    median = statistics.median(seconds)
    deviations = [abs(value - median) for value in seconds]
    p95_index = max(0, math.ceil(0.95 * len(seconds)) - 1)
    return Aggregate(
        samples=len(seconds),
        p50_seconds=median,
        p95_seconds=seconds[p95_index],
        min_seconds=seconds[0],
        max_seconds=seconds[-1],
        mad_seconds=statistics.median(deviations),
        peak_rss_kib=max(sample.metrics.peak_rss_kib for sample in eligible),
    )


def speedup(graphify: Aggregate, compass: Aggregate) -> float:
    if graphify.p50_seconds <= 0 or compass.p50_seconds <= 0:
        raise ValueError("speedup requires positive medians")
    return graphify.p50_seconds / compass.p50_seconds
