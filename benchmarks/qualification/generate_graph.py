#!/usr/bin/env python3
"""Generate deterministic, bounded Compass qualification graphs."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys
from typing import BinaryIO, Iterable

if __package__ in {None, ""}:
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from benchmarks.qualification.io import atomic_binary_writer, atomic_write_text

SCHEMA = "compass.qualification-graph-generator/1"
GRAPH_SCHEMA = "compass.graph/1"
SOURCE_PATH = "qualification/generated.rs"
EXTRACTOR = "compass.qualification.generator"
MAX_NODES = 1_000_000
MAX_EDGES = 2_500_000


def _json_bytes(value: object) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def _stable_id(domain: str, values: Iterable[str]) -> str:
    digest = hashlib.sha256()
    for value in (GRAPH_SCHEMA, domain, *values):
        encoded = value.encode("utf-8")
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
    return f"sha256:{digest.hexdigest()}"


def _node_id(index: int) -> str:
    return f"n:qualification:{index:07d}"


def _anchor(index: int) -> dict[str, object]:
    return {
        "file": SOURCE_PATH,
        "startByte": index,
        "endByte": index + 1,
        "startLine": index + 1,
        "startColumn": 0,
        "endLine": index + 1,
        "endColumn": 1,
    }


def _evidence(index: int) -> list[dict[str, object]]:
    return [
        {
            "anchors": [_anchor(index)],
            "confidence": "exact",
            "extractor": EXTRACTOR,
            "origin": "artifact",
        }
    ]


def node_record(index: int) -> dict[str, object]:
    anchor = _anchor(index)
    return {
        "evidence": _evidence(index),
        "id": _node_id(index),
        "kind": "function",
        "language": "rust",
        "name": f"Node{index:07d}",
        "qualifiedName": f"qualification::Node{index:07d}",
        "source": anchor,
    }


def edge_endpoints(ordinal: int, nodes: int) -> tuple[int, int]:
    chain_edges = nodes - 1
    if ordinal < chain_edges:
        return ordinal, ordinal + 1
    chord = ordinal - chain_edges
    if chord == 0:
        return 0, 1
    source = (chord - 1) % nodes
    band = (chord - 1) // nodes
    offset = 2 + (band % (nodes - 2))
    target = (source + offset) % nodes
    return source, target


def edge_record(ordinal: int, nodes: int) -> dict[str, object]:
    source_index, target_index = edge_endpoints(ordinal, nodes)
    source = _node_id(source_index)
    target = _node_id(target_index)
    anchor = _anchor(ordinal)
    canonical_anchor = (
        f"{SOURCE_PATH}:{ordinal}:{ordinal + 1}:"
        f"{ordinal + 1}:0:{ordinal + 1}:1"
    )
    edge_id = _stable_id(
        "edge", (source, "calls", target, canonical_anchor, "")
    )
    return {
        "evidence": _evidence(ordinal),
        "id": edge_id,
        "key": edge_id,
        "kind": "calls",
        "relationshipSite": anchor,
        "source": source,
        "target": target,
        "weight": 1.0,
    }


def _profile(path: Path, name: str) -> dict[str, object]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != "compass.qualification-profiles/1":
        raise ValueError(f"unsupported profile schema in {path}")
    profiles = document.get("profiles")
    if not isinstance(profiles, list):
        raise ValueError("profiles document requires a profiles array")
    if any(not isinstance(item, dict) for item in profiles):
        raise ValueError("profile entries must be objects")
    matches = [item for item in profiles if item.get("name") == name]
    if len(matches) != 1:
        raise ValueError(f"profile {name!r} must resolve exactly once")
    profile = matches[0]
    profile_name = profile.get("name")
    nodes_value = profile.get("nodes")
    edges_value = profile.get("edges")
    if not isinstance(profile_name, str) or not profile_name:
        raise ValueError("profile name must be a non-empty string")
    if isinstance(nodes_value, bool) or not isinstance(nodes_value, int):
        raise ValueError(f"profile {profile_name!r} nodes must be an integer")
    if isinstance(edges_value, bool) or not isinstance(edges_value, int):
        raise ValueError(f"profile {profile_name!r} edges must be an integer")
    nodes = nodes_value
    edges = edges_value
    if nodes < 3 or nodes > MAX_NODES:
        raise ValueError(f"nodes must be in 3..={MAX_NODES}")
    if edges < nodes or edges > MAX_EDGES:
        raise ValueError(f"edges must be in nodes..={MAX_EDGES}")
    sample_ordinals = profile.get("sampleOrdinals", [])
    if not isinstance(sample_ordinals, list):
        raise ValueError("sampleOrdinals must be an array")
    for value in sample_ordinals:
        if isinstance(value, bool) or not isinstance(value, int):
            raise ValueError("sampleOrdinals must contain integers")
        if value < 0 or value >= nodes:
            raise ValueError(f"sample ordinal {value} must be in 0..{nodes - 1}")
    return profile


def _write_array(stream: BinaryIO, records: Iterable[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    first = True
    for record in records:
        encoded = _json_bytes(record)
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
        if not first:
            stream.write(b",")
        stream.write(encoded)
        first = False
    return digest.hexdigest()


def _logical_digest(records: Iterable[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    for record in records:
        encoded = _json_bytes(record)
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
    return digest.hexdigest()


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def generation_metadata(profile: dict[str, object], *, complete: bool) -> dict[str, object]:
    nodes = int(profile["nodes"])
    edges = int(profile["edges"])
    samples = [int(value) for value in profile.get("sampleOrdinals", [])]
    metadata: dict[str, object] = {
        "schema": SCHEMA,
        "profile": profile["name"],
        "nodes": nodes,
        "edges": edges,
        "topology": {
            "chainEdges": nodes - 1,
            "parallelEdge": {"ordinal": nodes - 1, "source": 0, "target": 1},
            "chordFormula": "source=(chord-1)%nodes; offset=2+(((chord-1)//nodes)%(nodes-2)); target=(source+offset)%nodes",
        },
        "sampleNodes": [node_record(value) for value in samples],
        "sampleEdges": [edge_record(value, nodes) for value in samples],
        "completeLogicalDigests": complete,
    }
    if complete:
        metadata["nodeRecordsSha256"] = _logical_digest(
            node_record(index) for index in range(nodes)
        )
        metadata["edgeRecordsSha256"] = _logical_digest(
            edge_record(index, nodes) for index in range(edges)
        )
    return metadata


def write_graph(path: Path, profile: dict[str, object]) -> dict[str, object]:
    nodes = int(profile["nodes"])
    edges = int(profile["edges"])
    file_size = max(nodes, edges) + 1
    file_id = _stable_id("file", (SOURCE_PATH,))
    metadata = {
        "schema": GRAPH_SCHEMA,
        "build": {
            "builderVersion": SCHEMA,
            "schemaFingerprint": _stable_id("qualification", ("schema-v1",)),
            "sourceTreeDigest": _stable_id("qualification", (str(profile["name"]),)),
            "configurationDigest": _stable_id("qualification", ("profiles-v1",)),
            "generationId": _stable_id(
                "qualification", (str(profile["name"]), str(nodes), str(edges))
            ),
        },
        "files": [
            {
                "byteSize": file_size,
                "contentDigest": _stable_id(
                    "qualification",
                    ("generated-source", str(profile["name"]), str(nodes), str(edges)),
                ),
                "extractionStatus": "generated",
                "extractorVersions": [SCHEMA],
                "generated": True,
                "id": file_id,
                "language": "rust",
                "path": SOURCE_PATH,
            }
        ],
    }
    with atomic_binary_writer(path) as stream:
        stream.write(b'{"directed":true,"graph":')
        stream.write(_json_bytes(metadata))
        stream.write(b',"links":[')
        edge_digest = _write_array(
            stream, (edge_record(index, nodes) for index in range(edges))
        )
        stream.write(b'],"multigraph":true,"nodes":[')
        node_digest = _write_array(stream, (node_record(index) for index in range(nodes)))
        stream.write(b"]}\n")
    result = generation_metadata(profile, complete=False)
    result.update(
        {
            "graphBytes": path.stat().st_size,
            "graphSha256": _file_sha256(path),
            "nodeRecordsSha256": node_digest,
            "edgeRecordsSha256": edge_digest,
            "completeLogicalDigests": True,
        }
    )
    return result


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True)
    parser.add_argument(
        "--profiles",
        type=Path,
        default=Path(__file__).with_name("profiles-v1.json"),
    )
    parser.add_argument("--output", type=Path)
    parser.add_argument("--metadata-output", type=Path)
    parser.add_argument(
        "--plan-only",
        action="store_true",
        help="Validate counts/topology and emit samples without iterating every record",
    )
    parser.add_argument(
        "--metadata-only",
        action="store_true",
        help="Iterate every record and emit logical digests without a graph file",
    )
    args = parser.parse_args(argv)
    modes = sum((args.output is not None, args.plan_only, args.metadata_only))
    if modes != 1:
        parser.error("choose exactly one of --output, --plan-only, or --metadata-only")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    profile = _profile(args.profiles, args.profile)
    if args.output is not None:
        metadata = write_graph(args.output, profile)
    else:
        metadata = generation_metadata(profile, complete=args.metadata_only)
    payload = json.dumps(metadata, indent=2, sort_keys=True) + "\n"
    if args.metadata_output is None:
        sys.stdout.write(payload)
    else:
        atomic_write_text(args.metadata_output, payload)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (KeyError, OSError, TypeError, ValueError, json.JSONDecodeError) as error:
        print(f"graph generation failed: {error}", file=sys.stderr)
        raise SystemExit(2) from error
