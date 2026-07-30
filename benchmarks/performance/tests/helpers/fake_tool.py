#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path
import shutil


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    build = subparsers.add_parser("build")
    build.add_argument("--output", required=True)
    build.add_argument("--graph", required=True)
    query = subparsers.add_parser("query")
    query.add_argument("--text", required=True)
    args = parser.parse_args()
    if args.command == "query":
        print(args.text)
        return 0
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(args.graph, output / "graph.json")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

