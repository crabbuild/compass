#!/usr/bin/env python3
"""CLI adapter for the importable Compass code-graph v1 semantic oracle."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from code_graph_v1_oracle import (
    QualificationError,
    canonical_bytes,
    load_json,
    load_manifest,
    manifest_fingerprint,
    qualify_graph,
    qualification_summary,
)

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MANIFEST = ROOT / "tests/qualification/code-graph-v1-semantic.json"
DEFAULT_CORPUS = ROOT / "tests/qualification/code-graph-v1-corpus.json"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--corpus-manifest", type=Path, default=DEFAULT_CORPUS)
    parser.add_argument("--fixture-root", type=Path, default=ROOT)
    parser.add_argument("--graph", type=Path)
    parser.add_argument("--compare", nargs=2, type=Path)
    parser.add_argument("--compass-revision")
    parser.add_argument("--comparisons", type=Path)
    args = parser.parse_args()

    corpus = load_json(args.corpus_manifest)
    if corpus.get("schema") != "compass.code-graph-qualification-corpus/1":
        raise QualificationError(
            f"corpus_manifest_schema [{args.corpus_manifest}]: unsupported schema"
        )
    if set(corpus) != {"schema", "files"} or not isinstance(corpus["files"], list):
        raise QualificationError(
            f"corpus_manifest_shape [{args.corpus_manifest}]: expected schema and files"
        )
    corpus_sources: set[str] = set()
    corpus_ids: set[str] = set()
    for item in corpus["files"]:
        if not isinstance(item, dict) or set(item) != {"id", "path", "language", "contents"}:
            raise QualificationError(
                f"corpus_manifest_file [{args.corpus_manifest}]: invalid file record"
            )
        path = Path(item["path"])
        if (
            not item["id"]
            or item["id"] in corpus_ids
            or not item["language"]
            or path.is_absolute()
            or ".." in path.parts
            or item["path"] in corpus_sources
        ):
            raise QualificationError(
                f"corpus_manifest_file [{item['id']}]: duplicate or unsafe identity"
            )
        corpus_ids.add(item["id"])
        corpus_sources.add(item["path"])
    manifest = load_manifest(args.manifest, args.fixture_root, corpus_sources)
    producer_version = manifest["languages"]["producerVersion"]
    manifest["_languageExpectations"] = [
        {
            "id": item["id"],
            "source": item["path"],
            "language": item["language"],
            "producerVersion": producer_version,
        }
        for item in corpus.get("files", [])
    ]
    if args.compare:
        left, right = (path.read_bytes() for path in args.compare)
        if left != right:
            raise QualificationError(
                f"deterministic_graph_bytes [{args.compare[0]}]: differs from {args.compare[1]}"
            )
    fingerprint = manifest_fingerprint((args.manifest, args.corpus_manifest))
    if args.graph is None:
        print(canonical_bytes({
            "schema": manifest["schema"],
            "fixtureManifestFingerprint": fingerprint,
            "flows": len(manifest["flows"]),
            "negatives": len(manifest["negatives"]),
            "nodeKinds": len(manifest["nodeProducers"]),
            "edgeKinds": len(manifest["edgeProducers"]),
            "languages": len(manifest["_languageExpectations"]),
        }).decode(), end="")
        return 0

    graph_bytes = args.graph.read_bytes()
    graph = json.loads(graph_bytes)
    assertions = qualify_graph(graph, manifest, args.fixture_root.resolve())
    comparisons = load_json(args.comparisons) if args.comparisons else {}
    summary = qualification_summary(
        compass_revision=args.compass_revision or "unknown",
        manifest_digest=fingerprint,
        graph_bytes=graph_bytes,
        graph=graph,
        assertions=assertions,
        comparisons=comparisons,
    )
    print(canonical_bytes(summary).decode(), end="")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, json.JSONDecodeError, QualificationError, ValueError) as error:
        print(f"code-graph-v1 qualification failed: {error}", file=sys.stderr)
        raise SystemExit(1)
