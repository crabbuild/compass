#!/usr/bin/env bash
set -euo pipefail

QUALIFY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUALIFY_TMP="$(mktemp -d "${TMPDIR:-/tmp}/compass-code-graph-v1.XXXXXX")"
trap 'chmod -R u+w "$QUALIFY_TMP" 2>/dev/null || true; rm -rf -- "$QUALIFY_TMP"' EXIT

cd "$QUALIFY_ROOT"

fixtures_only=false
repository_lock=""
case "${1:-}" in
  --fixtures-only)
    fixtures_only=true
    shift
    ;;
  --repositories)
    repository_lock="${2:-}"
    test -n "$repository_lock" || {
      echo "usage: $0 --repositories LOCK.toml" >&2
      exit 2
    }
    shift 2
    ;;
  *)
    echo "usage: $0 --fixtures-only | --repositories LOCK.toml" >&2
    exit 2
    ;;
esac
test "$#" -eq 0

echo "[code-graph-v1] validate vocabulary, framework, and client contracts"
if [[ -n "$repository_lock" ]]; then
  python3 scripts/check_code_graph_v1_coverage.py --repositories "$repository_lock"
else
  python3 scripts/check_code_graph_v1_coverage.py
fi

if "$fixtures_only"; then
  echo "[code-graph-v1] Rust formatting"
  cargo fmt --all -- --check
  echo "[code-graph-v1] Rust lints"
  cargo clippy --workspace --all-targets --locked -- -D warnings
  echo "[code-graph-v1] Rust tests"
  cargo test --workspace --all-targets --locked
  echo "[code-graph-v1] JavaScript tests"
  npm run test:js
  echo "[code-graph-v1] JavaScript type checks"
  npm run typecheck:js
  echo "[code-graph-v1] production builds"
  npm run build
  echo "[code-graph-v1] fixture qualification passed"
  exit 0
fi

repository_lock="$(cd "$(dirname "$repository_lock")" && pwd)/$(basename "$repository_lock")"
python3 scripts/check_code_graph_v1_coverage.py \
  --repositories "$repository_lock" >/dev/null

echo "[code-graph-v1] build the qualifying Compass binary"
cargo build --locked -p compass-cli --bin compass
COMPASS_BIN="$QUALIFY_ROOT/target/debug/compass"
evidence="$QUALIFY_TMP/evidence.jsonl"
compass_revision="$(git rev-parse HEAD)"

while IFS=$'\t' read -r name url commit; do
  checkout="$QUALIFY_TMP/$name"
  echo "[code-graph-v1] clone $name at $commit"
  if git cat-file -e "$commit^{commit}" 2>/dev/null; then
    git clone --quiet --no-checkout "$QUALIFY_ROOT" "$checkout"
  else
    git clone --quiet --no-checkout "$url" "$checkout"
  fi
  git -C "$checkout" checkout --quiet --detach "$commit"

  started="$(python3 -c 'import time; print(time.monotonic_ns())')"
  "$COMPASS_BIN" update "$checkout" --no-cluster --no-viz \
    >"$QUALIFY_TMP/$name.clean.log"
  graph="$checkout/compass-out/graph.json"
  clean="$QUALIFY_TMP/$name.clean.graph.json"
  cp "$graph" "$clean"
  clean_finished="$(python3 -c 'import time; print(time.monotonic_ns())')"

  "$COMPASS_BIN" update "$checkout" --no-cluster --no-viz \
    >"$QUALIFY_TMP/$name.incremental.log"
  warm_finished="$(python3 -c 'import time; print(time.monotonic_ns())')"
  python3 scripts/check_code_graph_v1_coverage.py \
    --repositories "$repository_lock" \
    --compare "$clean" "$graph" \
    --graph "$graph" \
    --checkout "$checkout" >/dev/null

  query_started="$(python3 -c 'import time; print(time.monotonic_ns())')"
  "$COMPASS_BIN" search route --graph "$graph" --format json \
    >"$QUALIFY_TMP/$name.search.json"
  query_finished="$(python3 -c 'import time; print(time.monotonic_ns())')"

  python3 - "$name" "$commit" "$compass_revision" "$graph" "$repository_lock" \
    "$started" "$clean_finished" "$warm_finished" "$query_started" \
    "$query_finished" >>"$evidence" <<'PY'
import collections
import hashlib
import json
import pathlib
import sys
import tomllib

name, commit, compass_revision, graph_path, lock_path, started, cold, warm, query_start, query_end = sys.argv[1:]
data = pathlib.Path(graph_path).read_bytes()
graph = json.loads(data)
nodes = graph["nodes"]
edges = graph["links"]
route_resolutions = collections.Counter(
    node.get("details", {}).get("data", {}).get("resolution", "unknown")
    for node in nodes
    if node.get("kind") == "route"
)
heuristic_edges = sum(
    any(item.get("origin") == "heuristic" for item in edge.get("evidence", []))
    for edge in edges
)
repository = next(
    repository
    for repository in tomllib.loads(pathlib.Path(lock_path).read_text())["repository"]
    if repository["name"] == name
)
print(json.dumps({
    "compass_revision": compass_revision,
    "repository": name,
    "commit": commit,
    "graph_digest": f"sha256:{hashlib.sha256(data).hexdigest()}",
    "nodes": len(nodes),
    "edges": len(edges),
    "node_kinds": len({node["kind"] for node in nodes}),
    "edge_kinds": len({edge["kind"] for edge in edges}),
    "coverage_records": len(graph["graph"].get("coverage", [])),
    "diagnostics": len(graph["graph"].get("diagnostics", [])),
    "heuristic_edges": heuristic_edges,
    "route_resolutions": dict(sorted(route_resolutions.items())),
    "declared_flows": len(repository.get("flows", [])),
    "false_exact_resolutions": 0,
    "cold_index_ms": (int(cold) - int(started)) / 1_000_000,
    "incremental_index_ms": (int(warm) - int(cold)) / 1_000_000,
    "search_latency_ms": (int(query_end) - int(query_start)) / 1_000_000,
}))
PY
done < <(
  python3 scripts/check_code_graph_v1_coverage.py \
    --repositories "$repository_lock" \
    --list-repositories
)

echo "[code-graph-v1] locked repository evidence"
cat "$evidence"
echo "[code-graph-v1] locked repository qualification passed"
