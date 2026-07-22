#!/usr/bin/env python3
"""Measure one command with a monotonic clock and child peak RSS."""

from __future__ import annotations

from pathlib import Path
import resource
import subprocess
import sys
import time


def main() -> int:
    if len(sys.argv) < 4 or sys.argv[2] != "--":
        print("usage: measure_process.py STDOUT -- COMMAND [ARG ...]", file=sys.stderr)
        return 2
    output = Path(sys.argv[1])
    started = time.perf_counter()
    with output.open("wb") as stream:
        completed = subprocess.run(sys.argv[3:], stdout=stream, check=False)
    elapsed = time.perf_counter() - started
    peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    # ru_maxrss is bytes on Darwin and KiB on Linux/BSD.
    peak_kib = int(peak / 1024) if sys.platform == "darwin" else int(peak)
    print(f"{elapsed:.9f},{peak_kib}")
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
