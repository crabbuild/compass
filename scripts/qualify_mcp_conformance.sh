#!/usr/bin/env bash
set -euo pipefail

readonly CONFORMANCE_REF="74edef34d674f563537be8c6587cebaa58e830ca"
readonly SPEC_VERSION="2026-07-28"
readonly REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly COMPASS_BIN="${COMPASS_BIN:-$REPOSITORY_ROOT/target/debug/compass}"
readonly FIXTURE_ROOT="$REPOSITORY_ROOT/scripts/fixtures/mcp-conformance-project"
readonly EXPECTED_FAILURES="$REPOSITORY_ROOT/scripts/fixtures/mcp-conformance-expected-failures.yaml"
readonly PORT="${COMPASS_MCP_CONFORMANCE_PORT:-39091}"
readonly RUNNER_TIMEOUT_SECONDS="${COMPASS_MCP_CONFORMANCE_TIMEOUT_SECONDS:-300}"
readonly WORK_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/compass-mcp-conformance.XXXXXX")"
readonly GRAPH_PATH="$WORK_ROOT/compass-out/graph.json"
readonly SERVER_LOG="$WORK_ROOT/server.log"

server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$WORK_ROOT"
}
trap cleanup EXIT

case "$RUNNER_TIMEOUT_SECONDS" in
  ''|*[!0-9]*)
    echo "COMPASS_MCP_CONFORMANCE_TIMEOUT_SECONDS must be an integer from 1 through 1800" >&2
    exit 1
    ;;
esac
if [ "$RUNNER_TIMEOUT_SECONDS" -lt 1 ] || [ "$RUNNER_TIMEOUT_SECONDS" -gt 1800 ]; then
  echo "COMPASS_MCP_CONFORMANCE_TIMEOUT_SECONDS must be an integer from 1 through 1800" >&2
  exit 1
fi

run_bounded() {
  python3 - "$RUNNER_TIMEOUT_SECONDS" "$@" <<'PY'
import subprocess
import sys

timeout_seconds = int(sys.argv[1])
command = sys.argv[2:]
try:
    completed = subprocess.run(command, timeout=timeout_seconds, check=False)
except subprocess.TimeoutExpired:
    print(
        f"MCP conformance runner exceeded {timeout_seconds} seconds: {command[0]}",
        file=sys.stderr,
    )
    raise SystemExit(124)
raise SystemExit(completed.returncode)
PY
}

if [ ! -x "$COMPASS_BIN" ]; then
  echo "MCP conformance requires a built Compass binary: $COMPASS_BIN" >&2
  exit 1
fi

"$COMPASS_BIN" update "$FIXTURE_ROOT" \
  --store json \
  --out "$WORK_ROOT" \
  --force \
  --no-cluster \
  --no-viz >"$WORK_ROOT/update.log" 2>&1

if [ ! -f "$GRAPH_PATH" ]; then
  sed -n '1,160p' "$WORK_ROOT/update.log" >&2
  echo "Compass did not publish the MCP conformance graph: $GRAPH_PATH" >&2
  exit 1
fi

"$COMPASS_BIN" serve "$GRAPH_PATH" \
  --transport http \
  --host 127.0.0.1 \
  --port "$PORT" \
  --json-response >"$SERVER_LOG" 2>&1 &
server_pid=$!

ready=0
for _attempt in $(seq 1 60); do
  if ! kill -0 "$server_pid" 2>/dev/null; then
    sed -n '1,160p' "$SERVER_LOG" >&2
    echo "Compass MCP HTTP server exited before becoming ready" >&2
    exit 1
  fi
  if curl --silent --output /dev/null --connect-timeout 1 "http://127.0.0.1:$PORT/mcp"; then
    ready=1
    break
  fi
  sleep 0.25
done
if [ "$ready" -ne 1 ]; then
  sed -n '1,160p' "$SERVER_LOG" >&2
  echo "Compass MCP HTTP server did not become ready" >&2
  exit 1
fi

cd "$WORK_ROOT"
for scenario in \
  server-stateless \
  tools-list \
  resources-list \
  prompts-list \
  sep-2164-resource-not-found \
  dns-rebinding-protection \
  http-header-validation
do
  run_bounded npx --yes "github:modelcontextprotocol/conformance#$CONFORMANCE_REF" \
    server \
    --url "http://127.0.0.1:$PORT/mcp" \
    --scenario "$scenario" \
    --spec-version "$SPEC_VERSION" \
    --expected-failures "$EXPECTED_FAILURES" \
    --output-dir "$WORK_ROOT/results"
done

python3 - "$WORK_ROOT/results" <<'PY'
import json
import pathlib
import sys

result_root = pathlib.Path(sys.argv[1])
matches = []
for path in result_root.glob("server-http-header-validation-*/checks.json"):
    with path.open(encoding="utf-8") as handle:
        checks = json.load(handle)
    matches.extend(
        check
        for check in checks
        if check.get("id") == "sep-2243-server-accepts-whitespace-header-value"
    )
if len(matches) != 1:
    raise SystemExit(
        "expected exactly one whitespace-header conformance result, "
        f"found {len(matches)}"
    )
check = matches[0]
details = check.get("details")
body = details.get("responseBody") if isinstance(details, dict) else None
error = body.get("error") if isinstance(body, dict) else None
if not (
    check.get("status") == "FAILURE"
    and details.get("responseStatus") == 400
    and isinstance(error, dict)
    and error.get("code") == -32602
):
    raise SystemExit(
        "whitespace-header baseline no longer has the reviewed invalid-tool-arguments "
        f"cause: {json.dumps(check, sort_keys=True)}"
    )
print("Whitespace-header expected failure cause verified: invalid tool arguments (-32602)")
PY

echo "MCP HTTP conformance passed: reference@$CONFORMANCE_REF spec=$SPEC_VERSION; stdio is the separate protocol_conformance test"
