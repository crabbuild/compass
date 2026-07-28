#!/usr/bin/env python3
"""Fail closed when Compass code-graph v1 trust claims lose coverage."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REQUIRED_FRAMEWORKS = {
    "django": "python",
    "flask": "python",
    "fastapi": "python",
    "express": "typescript",
    "nestjs": "typescript",
    "laravel": "php",
    "drupal": "php",
    "rails": "ruby",
    "spring": "jvm",
    "play": "jvm",
    "gin": "go",
    "chi": "go",
    "gorilla-mux": "go",
    "axum": "rust",
    "actix": "rust",
    "rocket": "rust",
    "aspnet": "csharp",
    "vapor": "swift",
    "react-router": "typescript",
    "sveltekit": "typescript",
    "vue-router": "typescript",
    "nuxt": "typescript",
    "astro": "typescript",
}
FRAMEWORK_TOKENS = {
    **{name: (name,) for name in REQUIRED_FRAMEWORKS},
    "gorilla-mux": ("gorilla", "gorilla-mux"),
    "aspnet": ("aspnet", "asp.net"),
    "react-router": ("react-router", "react_router"),
}
PRODUCER_ROOTS = (
    "crates/compass-languages/src",
    "crates/compass-resolve/src",
    "crates/compass-graph/src",
    "crates/compass-postgres/src",
    "crates/compass-core/src",
)


def fail(message: str) -> None:
    raise ValueError(message)


def enum_variants(source: str, name: str) -> list[str]:
    match = re.search(rf"pub enum {re.escape(name)} \{{(.*?)\n\}}", source, re.S)
    if not match:
        fail(f"could not locate {name}")
    return re.findall(r"^\s*([A-Z][A-Za-z0-9]+),", match.group(1), re.M)


def production_text() -> str:
    chunks: list[str] = []
    for relative in PRODUCER_ROOTS:
        for source in sorted((ROOT / relative).rglob("*.rs")):
            chunks.append(source.read_text(encoding="utf-8", errors="replace"))
    return "\n".join(chunks)


def check_vocabulary() -> dict[str, int]:
    model = (ROOT / "crates/compass-model/src/code_graph.rs").read_text()
    producers = production_text()
    counts: dict[str, int] = {}
    for enum_name, prefix in (("NodeKind", "node"), ("EdgeKind", "edge")):
        variants = enum_variants(model, enum_name)
        missing = [
            variant
            for variant in variants
            if f"{enum_name}::{variant}" not in producers
        ]
        if missing:
            fail(f"{enum_name} variants have no production producer: {', '.join(missing)}")
        counts[f"{prefix}_kinds"] = len(variants)
    return counts


def check_framework_fixtures() -> dict[str, int]:
    corpus_paths = [
        *sorted((ROOT / "fixtures/code-graph/routes").rglob("*")),
        *sorted((ROOT / "crates/compass-resolve/tests").glob("*.rs")),
        *sorted((ROOT / "crates/compass-languages/tests").glob("*.rs")),
    ]
    corpus = "\n".join(
        path.read_text(encoding="utf-8", errors="replace").lower()
        for path in corpus_paths
        if path.is_file()
    )
    for framework, language in REQUIRED_FRAMEWORKS.items():
        if not any(token in corpus for token in FRAMEWORK_TOKENS[framework]):
            fail(f"{framework} has no positive fixture/test coverage")
        near = ROOT / f"fixtures/code-graph/routes/{language}"
        if not any(
            "near" in path.name.lower() and "match" in path.name.lower()
            for path in near.iterdir()
        ):
            fail(f"{framework} has no {language} near-match fixture")
    resolution = (ROOT / "crates/compass-resolve/tests/framework_routes.rs").read_text()
    for marker in (
        "ResolutionState::Exact",
        "ResolutionState::Ambiguous",
        "ResolutionState::Unresolved",
        "wiring_site",
        "incremental_resolution_replaces",
    ):
        if marker not in resolution:
            fail(f"shared framework qualification lacks {marker}")
    return {"frameworks": len(REQUIRED_FRAMEWORKS)}


def check_contract_fingerprint() -> str:
    contracts = ROOT / "fixtures/contracts"
    manifest_path = contracts / "compass-query-v1.manifest.json"
    manifest_bytes = manifest_path.read_bytes()
    expected = (contracts / "compass-query-v1.fingerprint").read_text().strip()
    actual = f"sha256:{hashlib.sha256(manifest_bytes).hexdigest()}"
    if actual != expected:
        fail(f"query contract fingerprint drift: expected {expected}, got {actual}")
    manifest = json.loads(manifest_bytes)
    typescript = (
        ROOT / "packages/compass-viewer/src/contracts/codeQuery.ts"
    ).read_text()
    for group in manifest["enums"].values():
        for value in group:
            if f'"{value}"' not in typescript:
                fail(f"TypeScript query contract is missing enum variant {value}")
    integrations = {
        "cli": ROOT / "crates/compass-cli/src/code_query_commands.rs",
        "mcp": ROOT / "crates/compass-mcp/src/code_query.rs",
        "vscode": ROOT / "editors/vscode/src/views/codeQueryClient.ts",
    }
    required_reference = {
        "cli": "CodeQueryResponse",
        "mcp": "CodeQueryLimits",
        "vscode": "CodeQueryResponseSchema",
    }
    for client, path in integrations.items():
        if required_reference[client] not in path.read_text():
            fail(f"{client} does not consume the shared query contract")
    return actual


def load_repositories(path: Path) -> list[dict]:
    repositories = tomllib.loads(path.read_text()).get("repository", [])
    if not repositories:
        fail("repository lock contains no [[repository]] entries")
    seen_names: set[str] = set()
    seen_cells: set[tuple[str, str]] = set()
    framework_flows: dict[str, list[dict]] = defaultdict(list)
    for repository in repositories:
        name = repository.get("name", "")
        commit = repository.get("commit", "")
        url = repository.get("url", "")
        cell = (repository.get("size_class", ""), repository.get("language_family", ""))
        if not name or name in seen_names:
            fail(f"repository name is empty or duplicated: {name!r}")
        if not re.fullmatch(r"[0-9a-f]{40}", commit):
            fail(f"{name}: commit must be an immutable lowercase 40-hex object ID")
        if not re.match(r"^(https://|ssh://|git@|file://)", url):
            fail(f"{name}: URL must be an explicit Git transport")
        if not all(cell) or cell in seen_cells:
            fail(f"{name}: size/language qualification cell is empty or duplicated")
        seen_names.add(name)
        seen_cells.add(cell)
        declared = set(repository.get("frameworks", []))
        unknown = declared - REQUIRED_FRAMEWORKS.keys()
        if unknown:
            fail(f"{name}: unknown frameworks: {', '.join(sorted(unknown))}")
        for flow in repository.get("flows", []):
            framework = flow.get("framework", "")
            if framework not in declared:
                fail(f"{name}: flow framework {framework!r} is not declared")
            if not flow.get("name") or not flow.get("query") or not flow.get("source"):
                fail(f"{name}: every flow requires name, query, and source")
            framework_flows[framework].append(flow)
    for framework in REQUIRED_FRAMEWORKS:
        flows = framework_flows[framework]
        names = {flow["name"] for flow in flows}
        if len(flows) < 3 or len(names) < 3:
            fail(f"{framework} requires at least three named locked flows")
    return repositories


def check_graph(graph_path: Path, checkout: Path | None, repositories: list[dict]) -> dict:
    graph = json.loads(graph_path.read_text())
    if graph.get("graph", {}).get("schema") != "compass.graph/1":
        fail(f"{graph_path}: expected compass.graph/1")
    nodes = graph.get("nodes", [])
    links = graph.get("links", [])
    node_ids = {node["id"] for node in nodes}
    if any(edge.get("source") not in node_ids or edge.get("target") not in node_ids for edge in links):
        fail(f"{graph_path}: dangling edge endpoint")
    for edge in links:
        for evidence in edge.get("evidence", []):
            if evidence.get("origin") == "heuristic":
                if not evidence.get("rule") or not evidence.get("wiringSite"):
                    fail(f"{edge.get('id')}: heuristic edge lacks rule or wiringSite")
    kind_counts = Counter(node.get("kind") for node in nodes)
    edge_counts = Counter(edge.get("kind") for edge in links)
    handler_routes = {
        edge["source"]
        for edge in links
        if edge.get("kind") == "routes_to"
        and edge.get("details", {}).get("type") == "route"
        and edge.get("details", {}).get("data", {}).get("stage") == "handler"
    }
    route_resolutions = Counter(
        node.get("details", {}).get("data", {}).get("resolution", "unknown")
        for node in nodes
        if node.get("kind") == "route"
    )
    false_exact = [
        node["id"]
        for node in nodes
        if node.get("kind") == "route"
        and node.get("details", {}).get("data", {}).get("resolution") == "exact"
        and node["id"] not in handler_routes
    ]
    if false_exact:
        fail(
            f"{graph_path}: {len(false_exact)} exact route(s) have no handler routes_to edge"
        )
    heuristic_edges = sum(
        1
        for edge in links
        if any(evidence.get("origin") == "heuristic" for evidence in edge.get("evidence", []))
    )
    if checkout is not None:
        for repository in repositories:
            for flow in repository.get("flows", []):
                if not (checkout / flow["source"]).is_file():
                    fail(f"{repository['name']}: flow source is missing: {flow['source']}")
        present_frameworks = {
            node.get("framework")
            for node in nodes
            if node.get("kind") == "route"
        }
        for framework in REQUIRED_FRAMEWORKS:
            aliases = FRAMEWORK_TOKENS[framework]
            if not any(
                isinstance(value, str)
                and any(value == alias or value.startswith(f"{alias}-") for alias in aliases)
                for value in present_frameworks
            ):
                fail(f"{graph_path}: no route nodes were emitted for {framework}")
    return {
        "nodes": len(nodes),
        "edges": len(links),
        "node_kinds": len(kind_counts),
        "edge_kinds": len(edge_counts),
        "coverage_records": len(graph.get("graph", {}).get("coverage", [])),
        "diagnostics": len(graph.get("graph", {}).get("diagnostics", [])),
        "heuristic_edges": heuristic_edges,
        "route_resolutions": dict(sorted(route_resolutions.items())),
        "false_exact_resolutions": len(false_exact),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--repositories",
        type=Path,
        default=ROOT / "tests/qualification/code-graph-v1-repositories.toml",
    )
    parser.add_argument("--list-repositories", action="store_true")
    parser.add_argument("--list-flows")
    parser.add_argument("--graph", type=Path)
    parser.add_argument("--checkout", type=Path)
    parser.add_argument("--compare", nargs=2, type=Path)
    args = parser.parse_args()
    repositories = load_repositories(args.repositories)
    if args.list_repositories:
        for repository in repositories:
            print("\t".join((
                repository["name"],
                repository["url"],
                repository["commit"],
            )))
        return 0
    if args.list_flows:
        repository = next(
            (item for item in repositories if item["name"] == args.list_flows),
            None,
        )
        if repository is None:
            fail(f"unknown repository {args.list_flows}")
        for flow in repository.get("flows", []):
            print("\t".join((flow["name"], flow["framework"], flow["query"])))
        return 0
    if args.compare:
        left, right = (path.read_bytes() for path in args.compare)
        if left != right:
            fail("clean and incremental graph bytes differ")
    report = {
        **check_vocabulary(),
        **check_framework_fixtures(),
        "query_contract_fingerprint": check_contract_fingerprint(),
        "repositories": len(repositories),
    }
    if args.graph:
        report.update(check_graph(args.graph, args.checkout, repositories))
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        print(f"code-graph-v1 qualification failed: {error}", file=sys.stderr)
        raise SystemExit(1)
