#!/usr/bin/env bash
set -euo pipefail

QUALIFY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/Volumes/Workspace/crabbuild-target/compass-main}"

usage() {
  echo "usage: $0 --fixtures-only" >&2
  exit 2
}

[[ "${1:-}" == "--fixtures-only" && "$#" -eq 1 ]] || usage
[[ -d /Volumes/Workspace && -w /Volumes/Workspace/crabbuild-target ]] || {
  echo "[agent-graph] /Volumes/Workspace/crabbuild-target is unavailable" >&2
  exit 1
}

cd "$QUALIFY_ROOT"

echo "[agent-graph] validate frozen JSON contracts"
python3 scripts/check_agent_graph_contracts.py

echo "[agent-graph] qualify Grounding, identity, CRUD, conflicts, rebase, limits, corruption, and audit"
cargo test -p compass-agent-graph --locked

echo "[agent-graph] qualify exact Effective Graph query and current/historical orchestration"
cargo test -p compass-query --test effective_graph --locked
cargo test -p compass-core --test agent_graph --locked

echo "[agent-graph] qualify CLI and MCP authorization adapters"
cargo test -p compass-cli --test agent_graph_cli --locked
cargo test -p compass-mcp --test agent_graph_tools --locked
cargo test -p compass-mcp --test agent_graph_http_auth --locked

echo "[agent-graph] fixture qualification passed"
