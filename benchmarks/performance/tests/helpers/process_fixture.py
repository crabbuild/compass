#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exit", type=int, default=0)
    parser.add_argument("--allocate-mib", type=int, default=0)
    parser.add_argument("--sleep", type=float, default=0)
    parser.add_argument("--stdout", default="")
    parser.add_argument("--spawn-child", action="store_true")
    args = parser.parse_args()
    child = None
    if args.spawn_child:
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        print(child.pid, file=sys.stderr, flush=True)
    allocation = bytearray(args.allocate_mib * 1024 * 1024)
    if allocation:
        allocation[0] = 1
        allocation[-1] = 1
    if args.stdout:
        print(args.stdout, flush=True)
    time.sleep(args.sleep)
    if child is not None:
        child.wait()
    return args.exit


if __name__ == "__main__":
    raise SystemExit(main())

