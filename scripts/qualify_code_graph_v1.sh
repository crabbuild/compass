#!/usr/bin/env bash
set -euo pipefail

QUALIFY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUALIFY_TMP="$(mktemp -d "${TMPDIR:-/tmp}/compass-code-graph-v1.XXXXXX")"
trap 'chmod -R u+w "$QUALIFY_TMP" 2>/dev/null || true; rm -rf -- "$QUALIFY_TMP"' EXIT

usage() {
  echo "usage: $0 --fixtures-only" >&2
  exit 2
}

[[ "${1:-}" == "--fixtures-only" ]] || usage
shift
[[ "$#" -eq 0 ]] || usage

cd "$QUALIFY_ROOT"
MANIFEST="$QUALIFY_ROOT/tests/qualification/code-graph-v1-semantic.json"
CORPUS_MANIFEST="$QUALIFY_ROOT/tests/qualification/code-graph-v1-corpus.json"
CORPUS="$QUALIFY_TMP/corpus"
OUTPUT_PARENT="$QUALIFY_TMP/output"

echo "[code-graph-v1] validate strict manifests"
python3 scripts/check_code_graph_v1_coverage.py \
  --manifest "$MANIFEST" \
  --corpus-manifest "$CORPUS_MANIFEST"

echo "[code-graph-v1] build qualifying production binary once"
cargo build --locked -p compass-cli --bin compass
COMPASS_BIN="$QUALIFY_ROOT/target/debug/compass"

mkdir -p "$CORPUS/fixtures/code-graph"
cp -R "$QUALIFY_ROOT/fixtures/code-graph/." "$CORPUS/fixtures/code-graph/"
python3 - "$CORPUS_MANIFEST" "$CORPUS" <<'PY'
import json
import pathlib
import sys

manifest = json.loads(pathlib.Path(sys.argv[1]).read_text())
root = pathlib.Path(sys.argv[2])
for fixture in manifest["files"]:
    path = root / fixture["path"]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(fixture["contents"].encode("utf-8"))
PY

active_graph() {
  python3 - "$1" <<'PY'
import pathlib
import sys

output = pathlib.Path(sys.argv[1]) / "compass-out"
pointer = output / ".compass-active-generation"
generation = pointer.read_text().strip()
if not generation.startswith("generation-") or "/" in generation or "\\" in generation:
    raise SystemExit(f"invalid active generation {generation!r}")
active = output / ".compass-generations" / generation
if not active.is_dir() or (active / ".compass-build-incomplete").exists():
    raise SystemExit(f"incomplete active generation {active}")
print(active / "graph.json")
PY
}

fixture_digest() {
  python3 - "$QUALIFY_ROOT/fixtures/code-graph" <<'PY'
import hashlib
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
digest = hashlib.sha256()
for path in sorted(item for item in root.rglob("*") if item.is_file()):
    digest.update(path.relative_to(root).as_posix().encode())
    digest.update(b"\0")
    digest.update(path.read_bytes())
    digest.update(b"\0")
print(digest.hexdigest())
PY
}

fixture_digest_before="$(fixture_digest)"

run_update() {
  local mode="$1"
  shift
  "$COMPASS_BIN" update "$CORPUS" \
    --out "$OUTPUT_PARENT" --no-cluster --no-viz --no-gitignore "$@" \
    >"$QUALIFY_TMP/$mode.log"
  active_graph "$OUTPUT_PARENT"
}

echo "[code-graph-v1] clean production update"
clean_graph="$(run_update clean)"
cp "$clean_graph" "$QUALIFY_TMP/clean.graph.json"

echo "[code-graph-v1] unchanged warm production update"
warm_graph="$(run_update warm)"
cp "$warm_graph" "$QUALIFY_TMP/warm.graph.json"
cmp "$QUALIFY_TMP/clean.graph.json" "$QUALIFY_TMP/warm.graph.json"

echo "[code-graph-v1] forced clean production rebuild"
rebuild_graph="$(run_update rebuild --force)"
cp "$rebuild_graph" "$QUALIFY_TMP/rebuild.graph.json"
cmp "$QUALIFY_TMP/clean.graph.json" "$QUALIFY_TMP/rebuild.graph.json"

edit_path="$CORPUS/fixtures/code-graph/routes/rust/axum.rs"
cp "$edit_path" "$QUALIFY_TMP/axum.original"
printf '\n' >>"$edit_path"
run_update incremental-edit >/dev/null
cp "$QUALIFY_TMP/axum.original" "$edit_path"
restored_graph="$(run_update incremental-restore)"
cp "$restored_graph" "$QUALIFY_TMP/restored.graph.json"
cmp "$QUALIFY_TMP/clean.graph.json" "$QUALIFY_TMP/restored.graph.json"

fixture_digest_after="$(fixture_digest)"
[[ "$fixture_digest_before" == "$fixture_digest_after" ]]

cat >"$QUALIFY_TMP/comparisons.json" <<'JSON'
{"cleanEqualsRebuild":true,"cleanEqualsRestored":true,"cleanEqualsWarm":true,"sourceFixtureUnchanged":true}
JSON

echo "[code-graph-v1] execute semantic assertions over production graph"
python3 scripts/check_code_graph_v1_coverage.py \
  --manifest "$MANIFEST" \
  --corpus-manifest "$CORPUS_MANIFEST" \
  --fixture-root "$CORPUS" \
  --graph "$QUALIFY_TMP/restored.graph.json" \
  --compass-revision "$(git rev-parse HEAD)" \
  --comparisons "$QUALIFY_TMP/comparisons.json"
