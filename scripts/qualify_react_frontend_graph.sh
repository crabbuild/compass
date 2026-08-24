#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-/Volumes/Workspace/crabbuild-target/compass-021-react-frontend}"
PARSER_ROOT="${TSLP_PARSER_SOURCE_DIR:-/Volumes/Workspace/crabbuild-target/compass-parser-sources}"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/compass-react-frontend.XXXXXX")"
trap 'chmod -R u+w "$TMP" 2>/dev/null || true; rm -rf -- "$TMP"' EXIT

MODE="pinned"
MANIFEST="$ROOT/tests/qualification/react-frontend-repositories.toml"
ARTIFACT_ROOT="/Volumes/Workspace/crabbuild-target/compass-021-react-frontend/qualification/react-frontend"
BASELINE=""
AUDIT_ONLY=0
if [[ "${1:-}" == "--fixtures-only" ]]; then
  MODE="fixtures"
  shift
elif [[ "${1:-}" == "--pinned" ]]; then
  shift
  if [[ "${1:-}" == "--manifest" ]]; then
    MANIFEST="${2:-}"
    [[ -n "$MANIFEST" ]] || { echo "--manifest requires a path" >&2; exit 2; }
    shift 2
  fi
  if [[ "${1:-}" == "--artifact-root" ]]; then
    ARTIFACT_ROOT="${2:-}"
    [[ -n "$ARTIFACT_ROOT" ]] || { echo "--artifact-root requires a path" >&2; exit 2; }
    shift 2
  fi
  if [[ "${1:-}" == "--baseline" ]]; then
    BASELINE="${2:-}"
    [[ -n "$BASELINE" ]] || { echo "--baseline requires a path" >&2; exit 2; }
    shift 2
  fi
  if [[ "${1:-}" == "--audit-only" ]]; then
    AUDIT_ONLY=1
    shift
  fi
elif [[ "$#" -ne 0 ]]; then
  echo "usage: $0 [--fixtures-only | --pinned [--manifest PATH] [--artifact-root PATH] [--baseline PATH] [--audit-only]]" >&2
  exit 2
fi
[[ "$#" -eq 0 ]] || { echo "unexpected argument: $1" >&2; exit 2; }

if [[ "$TARGET" == /Volumes/Workspace/* ]]; then
  [[ -d /Volumes/Workspace && -d "$(dirname "$TARGET")" && -w "$(dirname "$TARGET")" ]] || {
    echo "[react-frontend] /Volumes/Workspace and the selected target parent must be mounted and writable" >&2
    exit 1
  }
else
  [[ -d "$(dirname "$TARGET")" && -w "$(dirname "$TARGET")" ]] || {
    echo "[react-frontend] the selected target parent must be mounted and writable: $TARGET" >&2
    exit 1
  }
fi
[[ -f "$PARSER_ROOT/sources/language_definitions.json" && -d "$PARSER_ROOT/parsers" ]] || {
  echo "[react-frontend] offline qualification requires a pre-provisioned parser source bundle at $PARSER_ROOT (set TSLP_PARSER_SOURCE_DIR)" >&2
  exit 1
}

cd "$ROOT"
echo "[react-frontend] build release production binary ($MODE mode)"
PROJECT_ROOT="$PARSER_ROOT" TSLP_OFFLINE=1 CARGO_TARGET_DIR="$TARGET" \
  cargo build --release --locked -p compass-cli --bin compass
BIN="$TARGET/release/compass"
[[ -x "$BIN" ]] || { echo "missing production binary: $BIN" >&2; exit 1; }

if [[ "$MODE" == "fixtures" ]]; then
  echo "[react-frontend] run package/config precedence matrix with the production binary"
  python3 "$ROOT/scripts/qualify_react_frontend_matrix.py" --compass "$BIN"
fi

if [[ "$MODE" == "pinned" ]]; then
  if [[ "$AUDIT_ONLY" -eq 0 && -z "$BASELINE" ]]; then
    echo "[react-frontend] pinned production qualification requires --baseline; use --audit-only for evidence without a performance comparison" >&2
    exit 2
  fi
  echo "[react-frontend] qualify immutable pinned corpora from $MANIFEST"
  PINNED_ARGS=(
    --manifest "$MANIFEST" \
    --compass "$BIN" \
    --artifact-root "$ARTIFACT_ROOT"
  )
  if [[ -n "$BASELINE" ]]; then
    PINNED_ARGS+=(--baseline "$BASELINE")
  fi
  if [[ "$AUDIT_ONLY" -eq 1 ]]; then
    PINNED_ARGS+=(--audit-only)
  fi
  python3 "$ROOT/scripts/qualify_react_frontend_pinned.py" "${PINNED_ARGS[@]}"
  echo "[react-frontend] pinned production qualification passed"
  exit 0
fi

POSITIVE="$TMP/frontend-react"
NEGATIVE="$TMP/frontend-react-negative"
POSITIVE_OUT="$TMP/positive-output"
POSITIVE_REPEAT_OUT="$TMP/positive-repeat-output"
NEGATIVE_OUT="$TMP/negative-output"
cp -R "$ROOT/fixtures/code-graph/frontend-react" "$POSITIVE"
cp -R "$ROOT/fixtures/code-graph/frontend-react-negative" "$NEGATIVE"

active_graph() {
  python3 - "$1" <<'PY'
import pathlib
import sys

out = pathlib.Path(sys.argv[1]) / "compass-out"
pointer = out / "current-snapshot"
snapshot = pointer.read_text(encoding="utf-8").strip()
if not snapshot.startswith("snapshot-") or "/" in snapshot or "\\" in snapshot:
    raise SystemExit(f"invalid active snapshot {snapshot!r}")
graph = out / "snapshots" / snapshot / "graph.json"
if not graph.is_file():
    raise SystemExit(f"missing active graph {graph}")
print(graph)
PY
}

run_update() {
  local project="$1"
  local output="$2"
  shift 2
  # TSLP_OFFLINE applies to parser-source acquisition only. The production
  # binary is statically linked; this proves runtime qualification needs no
  # network or credential boundary.
  TSLP_OFFLINE=1 "$BIN" update "$project" --out "$output" --no-cluster --no-viz \
    --no-gitignore --inference-level max "$@" >"$TMP/$(basename "$output").log"
  active_graph "$output"
}

echo "[react-frontend] qualify positive corpus"
positive_graph="$(run_update "$POSITIVE" "$POSITIVE_OUT")"
echo "[react-frontend] repeat positive production build"
repeat_graph="$(run_update "$POSITIVE" "$POSITIVE_REPEAT_OUT" --force)"
cmp "$positive_graph" "$repeat_graph"

echo "[react-frontend] qualify activation negative"
negative_graph="$(run_update "$NEGATIVE" "$NEGATIVE_OUT")"
node "$ROOT/scripts/react_frontend_source_oracle.mjs" --root "$POSITIVE" --framework fixture --output "$TMP/frontend-source-oracle.json"
python3 "$ROOT/scripts/qualify_react_frontend_graph.py" \
  "$positive_graph" "$negative_graph" \
  --source-oracle "$TMP/frontend-source-oracle.json" \
  --min-precision 1 --min-recall 1 \
  --result "$TMP/react-frontend-result.json"
echo "[react-frontend] deterministic, offline, activation, endpoint, and independent-anchor checks passed"
