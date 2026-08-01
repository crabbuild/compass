#!/usr/bin/env python3
"""Analyze Compass and Graphify graphs produced for the same source corpora."""

from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import asdict
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
from typing import Iterator


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
if str(REPOSITORY_ROOT) not in sys.path:
    sys.path.insert(0, str(REPOSITORY_ROOT))

from benchmarks.performance.compass.correctness import (  # noqa: E402
    GraphSummary,
    canonical_graph_digest,
    compare_graphs,
    index_graph,
)
from benchmarks.performance.compass.jsonstream import (  # noqa: E402
    iter_top_level_array,
)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--workspace",
        type=Path,
        required=True,
        help="run directory containing outputs/, logs/, and metrics/",
    )
    parser.add_argument(
        "--corpora",
        type=Path,
        required=True,
        help="JSON manifest describing corpus names, source paths, and metadata",
    )
    parser.add_argument(
        "--compass-binary",
        type=Path,
        default=Path("compass"),
        help="Compass executable used to record version evidence (default: compass)",
    )
    parser.add_argument(
        "--compass-source",
        type=Path,
        default=REPOSITORY_ROOT,
        help="Compass source checkout used to record the build commit",
    )
    parser.add_argument(
        "--graphify-binary",
        type=Path,
        default=Path("graphify"),
        help="Graphify executable used to record version evidence (default: graphify)",
    )
    return parser


def command(*args: str, cwd: Path | None = None) -> str:
    completed = subprocess.run(
        args,
        cwd=cwd,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    return completed.stdout.strip()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_corpora(manifest_path: Path) -> list[dict[str, str | Path]]:
    manifest_path = manifest_path.resolve()
    document = json.loads(manifest_path.read_text(encoding="utf-8"))
    raw_corpora = document.get("corpora") if isinstance(document, dict) else None
    if not isinstance(raw_corpora, list) or not raw_corpora:
        raise ValueError("corpus manifest must contain a non-empty 'corpora' array")

    corpora: list[dict[str, str | Path]] = []
    names: set[str] = set()
    for index, raw in enumerate(raw_corpora):
        if not isinstance(raw, dict):
            raise ValueError(f"corpora[{index}] must be an object")
        values: dict[str, str] = {}
        for key in ("name", "language", "framework", "source"):
            value = raw.get(key)
            if not isinstance(value, str) or not value.strip():
                raise ValueError(f"corpora[{index}].{key} must be a non-empty string")
            values[key] = value.strip()
        name = values["name"]
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", name):
            raise ValueError(f"invalid corpus name: {name!r}")
        if name in names:
            raise ValueError(f"duplicate corpus name: {name}")
        names.add(name)
        source = Path(values["source"]).expanduser()
        if not source.is_absolute():
            source = manifest_path.parent / source
        source = source.resolve()
        if not source.is_dir():
            raise ValueError(f"corpus source is not a directory: {source}")
        corpora.append(
            {
                "name": name,
                "language": values["language"],
                "framework": values["framework"],
                "source": source,
            }
        )
    return corpora


def compass_graph(workspace: Path, name: str) -> Path:
    output = workspace / "outputs" / "compass" / name / "compass-out"
    generation = (output / ".compass-active-generation").read_text(
        encoding="utf-8"
    ).strip()
    graph = output / ".compass-generations" / generation / "graph.json"
    if not graph.is_file():
        raise FileNotFoundError(graph)
    return graph


def graphify_graph(workspace: Path, name: str) -> Path:
    graph = workspace / "outputs" / "graphify" / name / "graphify-out" / "graph.json"
    if not graph.is_file():
        raise FileNotFoundError(graph)
    return graph


def timing(workspace: Path, tool: str, name: str) -> dict[str, int | float]:
    text = (workspace / "logs" / f"{tool}-{name}.log").read_text(
        encoding="utf-8", errors="replace"
    )
    wall_matches = re.findall(r"^\s*([0-9]+(?:\.[0-9]+)?) real", text, re.MULTILINE)
    rss_matches = re.findall(
        r"^\s*([0-9]+)\s+maximum resident set size", text, re.MULTILINE
    )
    if not wall_matches or not rss_matches:
        raise ValueError(f"timing evidence missing: {tool}/{name}")
    return {
        "wall_seconds": float(wall_matches[-1]),
        "peak_rss_bytes": int(rss_matches[-1]),
    }


def git_identity(source: Path) -> dict[str, str]:
    return {
        "commit": command("git", "rev-parse", "HEAD", cwd=source),
        "tree": command("git", "rev-parse", "HEAD^{tree}", cwd=source),
        "origin": command("git", "remote", "get-url", "origin", cwd=source),
    }


def records(path: Path, kind: str) -> Iterator[dict[str, object]]:
    keys = ("nodes",) if kind == "nodes" else ("links", "edges")
    for key in keys:
        try:
            yield from iter_top_level_array(path, key)
            return
        except KeyError:
            continue
    raise KeyError(f"graph has no {kind!r} array: {path}")


def source_file(record: dict[str, object]) -> str:
    source = record.get("source")
    nested = source if isinstance(source, dict) else {}
    return str(record.get("source_file") or nested.get("file") or "")


def occurrence_file(record: dict[str, object]) -> str:
    site = record.get("relationshipSite", record.get("relationship_site"))
    nested = site if isinstance(site, dict) else {}
    return str(record.get("source_file") or nested.get("file") or "")


def graph_profile(path: Path) -> dict[str, object]:
    node_ids: set[str] = set()
    node_payloads: Counter[str] = Counter()
    kind_counts: Counter[str] = Counter()
    language_counts: Counter[str] = Counter()
    sourced_nodes = 0
    placeholder_nodes = 0
    unverified_nodes = 0
    for node in records(path, "nodes"):
        identifier = str(node.get("id", ""))
        node_ids.add(identifier)
        payload = json.dumps(
            node, sort_keys=True, separators=(",", ":"), ensure_ascii=False
        )
        node_payloads[payload] += 1
        kind = str(
            node.get(
                "kind",
                node.get("file_type", node.get("type", node.get("node_type", ""))),
            )
        ).lower()
        kind_counts[kind] += 1
        language_counts[str(node.get("language", node.get("lang", ""))).lower()] += 1
        if source_file(node):
            sourced_nodes += 1
        if (
            node.get("placeholder") is True
            or node.get("unresolved") is True
            or node.get("external") is True
            or str(node.get("resolution", "")).lower() in {"deferred", "unresolved"}
        ):
            placeholder_nodes += 1
        if str(node.get("verification", "")).lower() == "unverified":
            unverified_nodes += 1

    relation_counts: Counter[str] = Counter()
    edge_payloads: Counter[str] = Counter()
    edge_keys: Counter[tuple[str, str, str, str, str]] = Counter()
    dangling_edges = 0
    self_loops = 0
    occurrence_edges = 0
    route_edges = 0
    for edge in records(path, "edges"):
        source = str(edge.get("source", ""))
        target = str(edge.get("target", ""))
        relation = str(edge.get("relation", edge.get("kind", ""))).lower()
        relation_counts[relation] += 1
        payload = json.dumps(
            edge, sort_keys=True, separators=(",", ":"), ensure_ascii=False
        )
        edge_payloads[payload] += 1
        edge_keys[
            (
                source,
                target,
                relation,
                occurrence_file(edge),
                str(edge.get("source_location", "")),
            )
        ] += 1
        if source not in node_ids or target not in node_ids:
            dangling_edges += 1
        if source == target:
            self_loops += 1
        if occurrence_file(edge):
            occurrence_edges += 1
        if relation in {"routes_to", "route"}:
            route_edges += 1

    return {
        "file_bytes": path.stat().st_size,
        "sha256": sha256(path),
        "nodes": len(node_ids),
        "edges": sum(relation_counts.values()),
        "sourced_nodes": sourced_nodes,
        "source_occurrence_edges": occurrence_edges,
        "placeholder_nodes": placeholder_nodes,
        "unverified_nodes": unverified_nodes,
        "dangling_edges": dangling_edges,
        "self_loops": self_loops,
        "exact_duplicate_nodes": sum(count - 1 for count in node_payloads.values()),
        "exact_duplicate_edges": sum(count - 1 for count in edge_payloads.values()),
        "duplicate_semantic_edge_keys": sum(count - 1 for count in edge_keys.values()),
        "node_kinds": dict(kind_counts.most_common()),
        "languages": dict(language_counts.most_common()),
        "relations": dict(relation_counts.most_common()),
        "route_edges": route_edges,
    }


def omission_counts(workspace: Path, name: str) -> dict[str, int]:
    text = (workspace / "logs" / f"compass-{name}.log").read_text(
        encoding="utf-8", errors="replace"
    )
    match = re.search(
        r"omitting ([0-9]+) nodes and ([0-9]+) edges; "
        r"([0-9]+) identity collisions",
        text,
    )
    if not match:
        return {"nodes": 0, "edges": 0, "identity_collisions": 0}
    return {
        "nodes": int(match.group(1)),
        "edges": int(match.group(2)),
        "identity_collisions": int(match.group(3)),
    }


def _summary_after_validation_error(
    database: sqlite3.Connection,
    tool: str,
    validation_errors: int,
) -> GraphSummary:
    nodes = int(
        database.execute(
            "SELECT COUNT(*) FROM nodes WHERE tool = ?", (tool,)
        ).fetchone()[0]
    )
    edges = int(
        database.execute(
            "SELECT COUNT(*) FROM edges WHERE tool = ?", (tool,)
        ).fetchone()[0]
    )
    dangling = int(
        database.execute(
            """
            SELECT COUNT(*)
            FROM edges AS edge
            LEFT JOIN nodes AS source
              ON source.tool = edge.tool AND source.id = edge.source
            LEFT JOIN nodes AS target
              ON target.tool = edge.tool AND target.id = edge.target
            WHERE edge.tool = ?
              AND (source.id IS NULL OR target.id IS NULL)
            """,
            (tool,),
        ).fetchone()[0]
    )
    if dangling:
        raise ValueError(f"{tool} graph has {dangling} dangling edges")
    digest = canonical_graph_digest(database, tool)
    database.execute(
        "INSERT INTO summaries VALUES (?, ?, ?, ?, ?)",
        (tool, nodes, edges, validation_errors, digest),
    )
    database.commit()
    return GraphSummary(tool, nodes, edges, validation_errors, digest)


def compare(
    database_path: Path,
    compass_path: Path,
    graphify_path: Path,
    source_root: Path,
) -> dict[str, object]:
    if database_path.exists():
        database_path.unlink()
    database = sqlite3.connect(database_path)
    try:
        compass_diagnostics_stream_limit = False
        compass_validation_errors = 0
        try:
            compass_summary = index_graph("compass", compass_path, database)
        except ValueError as error:
            validation_match = re.fullmatch(
                r"Compass graph reports ([0-9]+) validation errors", str(error)
            )
            if str(error) == f"JSON record exceeds limit in {compass_path}":
                compass_diagnostics_stream_limit = True
            elif validation_match is not None:
                compass_validation_errors = int(validation_match.group(1))
            else:
                raise
            compass_summary = _summary_after_validation_error(
                database, "compass", compass_validation_errors
            )

        sanitized_dangling_edges = 0
        try:
            graphify_summary = index_graph("graphify", graphify_path, database)
        except ValueError as error:
            match = re.fullmatch(
                r"graphify graph has ([0-9]+) dangling edges", str(error)
            )
            if match is None:
                raise
            sanitized_dangling_edges = int(match.group(1))
            database.execute(
                """
                DELETE FROM edges
                WHERE tool = 'graphify'
                  AND (
                    source NOT IN (SELECT id FROM nodes WHERE tool = 'graphify')
                    OR target NOT IN (SELECT id FROM nodes WHERE tool = 'graphify')
                  )
                """
            )
            graphify_summary = _summary_after_validation_error(database, "graphify", 0)

        result = compare_graphs(database, source_root)
        return {
            "passed": result.passed,
            "failures": list(result.failures),
            "metrics": result.metrics,
            "compass_diagnostics_stream_limit": compass_diagnostics_stream_limit,
            "compass_validation_errors": compass_validation_errors,
            "sanitized_dangling_graphify_edges": sanitized_dangling_edges,
            "summaries": {
                "compass": asdict(compass_summary),
                "graphify": asdict(graphify_summary),
            },
        }
    finally:
        database.close()


def pct(numerator: int, denominator: int) -> float:
    return round(100.0 * numerator / denominator, 2) if denominator else 0.0


def markdown_report(payload: dict[str, object]) -> str:
    tools = payload["tools"]
    corpora = payload["corpora"]
    assert isinstance(tools, dict)
    assert isinstance(corpora, list)
    compass_tool = tools["compass"]
    graphify_tool = tools["graphify"]
    assert isinstance(compass_tool, dict)
    assert isinstance(graphify_tool, dict)
    lines = [
        "# Compass vs Graphify real-world graph evaluation",
        "",
        f"- Compass: `{compass_tool['version']}` at "
        f"`{str(compass_tool['commit'])[:12]}`",
        f"- Graphify: `{graphify_tool['version']}`",
        f"- Corpora: `{len(corpora)}` pinned real-world repositories",
        "",
        "## Graph size and build evidence",
        "",
        "| Corpus | Lang / framework | Compass nodes | Graphify nodes | Compass edges | Graphify edges | Compass s | Graphify s |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for corpus in corpora:
        c = corpus["compass"]
        g = corpus["graphify"]
        lines.append(
            f"| {corpus['name']} | {corpus['language']} / {corpus['framework']} | "
            f"{c['nodes']:,} | {g['nodes']:,} | {c['edges']:,} | {g['edges']:,} | "
            f"{c['timing']['wall_seconds']:.2f} | {g['timing']['wall_seconds']:.2f} |"
        )
    lines.extend(
        (
            "",
            "## Integrity and publication quality",
            "",
            "| Corpus | Tool | Dangling | Exact dup edges | Occurrence-backed edges | "
            "Source-backed nodes | Omitted nodes | Omitted edges | Unverified nodes |",
            "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        )
    )
    for corpus in corpora:
        for tool in ("compass", "graphify"):
            item = corpus[tool]
            omissions = (
                corpus["compass_omissions"]
                if tool == "compass"
                else {"nodes": 0, "edges": 0}
            )
            lines.append(
                f"| {corpus['name']} | {tool} | {item['dangling_edges']:,} | "
                f"{item['exact_duplicate_edges']:,} | "
                f"{item['source_occurrence_edges']:,} "
                f"({pct(item['source_occurrence_edges'], item['edges']):.2f}%) | "
                f"{item['sourced_nodes']:,} "
                f"({pct(item['sourced_nodes'], item['nodes']):.2f}%) | "
                f"{omissions['nodes']:,} | {omissions['edges']:,} | "
                f"{item['unverified_nodes']:,} |"
            )
    lines.extend(
        (
            "",
            "## Graphify hypothesis coverage in Compass",
            "",
            "These are comparator classifications, not ground-truth precision or recall. "
            "`rejected` means a known unsafe/fabricated Graphify projection; `missing` "
            "means no source-compatible Compass fact was found.",
            "",
            "| Corpus | Exact nodes | Dominated nodes | Ambiguous nodes | Missing nodes | "
            "Exact edges | Dominated edges | Rejected edges | Ambiguous edges | "
            "Missing edges |",
            "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        )
    )
    for corpus in corpora:
        metrics = corpus["comparison"]["metrics"]
        lines.append(
            f"| {corpus['name']} | "
            f"{metrics.get('exact_graphify_nodes', 0):,} | "
            f"{metrics.get('dominated_graphify_nodes', 0):,} | "
            f"{metrics.get('ambiguous_graphify_nodes', 0):,} | "
            f"{metrics.get('missing_graphify_nodes', 0):,} | "
            f"{metrics.get('exact_graphify_edges', 0):,} | "
            f"{metrics.get('dominated_graphify_edges', 0):,} | "
            f"{metrics.get('rejected_graphify_edges', 0):,} | "
            f"{metrics.get('ambiguous_graphify_edges', 0):,} | "
            f"{metrics.get('missing_graphify_edges', 0):,} |"
        )
    lines.extend(
        (
            "",
            "## Important limitations",
            "",
            "- Node and edge counts measure representation density, not correctness.",
            "- The cross-tool comparator treats Graphify as a recall-hypothesis source, "
            "not ground truth.",
            "- No manual stratified source audit was performed, so this report does not "
            "claim a statistical precision/recall percentage.",
            "",
        )
    )
    return "\n".join(lines)


def tool_identity(
    binary: Path, *, compass: bool, source: Path | None = None
) -> dict[str, str]:
    executable = str(binary)
    version = command(executable, "--version")
    if not compass:
        version = version.splitlines()[-1]
    identity = {
        "version": version,
        "binary": executable,
    }
    if binary.is_file():
        identity["binary_sha256"] = sha256(binary)
    if compass:
        assert source is not None
        identity["commit"] = command("git", "rev-parse", "HEAD", cwd=source)
    return identity


def analyze(
    workspace: Path,
    manifest_path: Path,
    compass_binary: Path,
    compass_source: Path,
    graphify_binary: Path,
) -> tuple[Path, Path]:
    workspace = workspace.resolve()
    metrics_directory = workspace / "metrics"
    metrics_directory.mkdir(parents=True, exist_ok=True)
    corpora: list[dict[str, object]] = []
    for spec in load_corpora(manifest_path):
        name = str(spec["name"])
        print(f"analyzing {name}", flush=True)
        compass_path = compass_graph(workspace, name)
        graphify_path = graphify_graph(workspace, name)
        compass_profile = graph_profile(compass_path)
        graphify_profile = graph_profile(graphify_path)
        compass_profile["timing"] = timing(workspace, "compass", name)
        graphify_profile["timing"] = timing(workspace, "graphify", name)
        source = spec["source"]
        assert isinstance(source, Path)
        corpora.append(
            {
                "name": name,
                "language": spec["language"],
                "framework": spec["framework"],
                "source": str(source),
                "git": git_identity(source),
                "compass_graph": str(compass_path),
                "graphify_graph": str(graphify_path),
                "compass": compass_profile,
                "graphify": graphify_profile,
                "compass_omissions": omission_counts(workspace, name),
                "comparison": compare(
                    metrics_directory / f"{name}.sqlite",
                    compass_path,
                    graphify_path,
                    source,
                ),
            }
        )

    payload: dict[str, object] = {
        "schema": "compass-graphify-real-world-evaluation/1",
        "tools": {
            "compass": tool_identity(
                compass_binary, compass=True, source=compass_source.resolve()
            ),
            "graphify": tool_identity(graphify_binary, compass=False),
        },
        "corpora": corpora,
    }
    results = metrics_directory / "results.json"
    results.write_text(
        json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    report = workspace / "REPORT.md"
    report.write_text(markdown_report(payload), encoding="utf-8")
    return results, report


def main() -> None:
    args = build_parser().parse_args()
    try:
        results, report = analyze(
            args.workspace,
            args.corpora,
            args.compass_binary,
            args.compass_source,
            args.graphify_binary,
        )
    except (FileNotFoundError, KeyError, ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"error: {error}") from error
    print(results)
    print(report)


if __name__ == "__main__":
    main()
