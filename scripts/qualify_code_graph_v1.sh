#!/usr/bin/env bash
set -euo pipefail

QUALIFY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUALIFY_TMP="$(mktemp -d "${TMPDIR:-/tmp}/compass-code-graph-v1.XXXXXX")"
trap 'chmod -R u+w "$QUALIFY_TMP" 2>/dev/null || true; rm -rf -- "$QUALIFY_TMP"' EXIT

# Qualification intentionally exercises large repositories. Set CARGO_TARGET_DIR
# before running if you want those build artifacts somewhere other than the
# checkout's own target directory.

usage() {
  cat >&2 <<EOF
usage:
  $0 --fixtures-only
  $0 --repositories <manifest> [--local-repository <path>]
EOF
  exit 2
}

MODE=
REPOSITORIES_MANIFEST=
LOCAL_REPOSITORY=
case "${1:-}" in
  --fixtures-only)
    MODE=fixtures
    shift
    ;;
  --repositories)
    MODE=repositories
    REPOSITORIES_MANIFEST="${2:-}"
    [[ -n "$REPOSITORIES_MANIFEST" ]] || usage
    shift 2
    if [[ "${1:-}" == "--local-repository" ]]; then
      LOCAL_REPOSITORY="${2:-}"
      [[ -n "$LOCAL_REPOSITORY" ]] || usage
      shift 2
    fi
    ;;
  *)
    usage
    ;;
esac
[[ "$#" -eq 0 ]] || usage

cd "$QUALIFY_ROOT"
MANIFEST="$QUALIFY_ROOT/tests/qualification/code-graph-v1-semantic.json"
CORPUS_MANIFEST="$QUALIFY_ROOT/tests/qualification/code-graph-v1-corpus.json"
CORPUS="$QUALIFY_TMP/corpus"
OUTPUT_PARENT="$QUALIFY_TMP/output"

active_graph() {
  python3 - "$1" <<'PY'
import pathlib
import sys

output = pathlib.Path(sys.argv[1]) / "compass-out"
pointer = output / "current-snapshot"
snapshot = pointer.read_text().strip()
if not snapshot.startswith("snapshot-") or "/" in snapshot or "\\" in snapshot:
    raise SystemExit(f"invalid active snapshot {snapshot!r}")
active = output / "snapshots" / snapshot
if not active.is_dir() or (active / "build-incomplete").exists():
    raise SystemExit(f"incomplete active snapshot {active}")
print(active / "graph.json")
PY
}

echo "[code-graph-v1] build qualifying production binary once"
cargo build --locked -p compass-cli --bin compass
QUALIFY_TARGET="$(cargo metadata --format-version 1 --no-deps | python3 -c \
  'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
COMPASS_BIN="$QUALIFY_TARGET/debug/compass"

if [[ "$MODE" == repositories ]]; then
  [[ -f "$REPOSITORIES_MANIFEST" ]] || {
    echo "repository manifest not found: $REPOSITORIES_MANIFEST" >&2
    exit 1
  }
  RELEASE_REPOSITORIES="$QUALIFY_TMP/release-repositories.tsv"
  python3 - "$REPOSITORIES_MANIFEST" >"$RELEASE_REPOSITORIES" <<'PY'
import pathlib
import sys
import tomllib

path = pathlib.Path(sys.argv[1])
manifest = tomllib.loads(path.read_text())
if manifest.get("schema") != "compass.code-graph-qualification/1":
    raise SystemExit(f"unsupported repository manifest schema in {path}")
repositories = [
    repository
    for repository in manifest.get("repository", [])
    if repository.get("release_gate") is True
]
if not repositories:
    raise SystemExit(f"no release-gate repositories declared in {path}")
for repository in repositories:
    required = {
        "name",
        "url",
        "commit",
        "size_class",
        "language_family",
        "release_gate",
        "required_validation_errors",
    }
    missing = required - repository.keys()
    if missing:
        raise SystemExit(
            f"repository {repository.get('name', '<unknown>')} missing {sorted(missing)}"
        )
    if repository["required_validation_errors"] != 0:
        raise SystemExit(
            f"repository {repository['name']} must require zero validation errors"
        )
    print(
        "\t".join(
            [
                repository["name"],
                repository["url"],
                repository["commit"],
                str(repository["required_validation_errors"]),
            ]
        )
    )
PY
  repository_count="$(wc -l <"$RELEASE_REPOSITORIES" | tr -d ' ')"
  if [[ -n "$LOCAL_REPOSITORY" && "$repository_count" -ne 1 ]]; then
    echo "--local-repository requires exactly one release-gate repository" >&2
    exit 1
  fi

  while IFS=$'\t' read -r repository_name repository_url repository_commit required_errors; do
    repository="$QUALIFY_TMP/repositories/$repository_name"
    if [[ -n "$LOCAL_REPOSITORY" ]]; then
      repository="$(cd "$LOCAL_REPOSITORY" && pwd)"
    else
      git clone --quiet --no-checkout "$repository_url" "$repository"
      git -C "$repository" checkout --quiet --detach "$repository_commit"
    fi
    [[ "$(git -C "$repository" rev-parse HEAD)" == "$repository_commit" ]] || {
      echo "$repository_name is not at pinned commit $repository_commit" >&2
      exit 1
    }
    status_before="$(git -C "$repository" status --porcelain=v1 --untracked-files=all)"
    [[ -z "$status_before" ]] || {
      echo "$repository_name qualification repository must be clean" >&2
      exit 1
    }

    repository_output="$QUALIFY_TMP/repository-output/$repository_name"
    echo "[code-graph-v1] qualify pinned repository $repository_name@$repository_commit"
    "$COMPASS_BIN" update "$repository" \
      --out "$repository_output" --no-cluster --no-viz --inference-level max \
      >"$QUALIFY_TMP/$repository_name.log"
    repository_graph="$(active_graph "$repository_output")"
    "$COMPASS_BIN" benchmark "$repository_graph" \
      >"$QUALIFY_TMP/$repository_name.benchmark.json"
    python3 - "$repository_graph" "$repository_name" "$repository_commit" "$required_errors" <<'PY'
import hashlib
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
name = sys.argv[2]
commit = sys.argv[3]
required_errors = int(sys.argv[4])
graph_bytes = path.read_bytes()
graph = json.loads(graph_bytes)
metadata = graph.get("graph", {})
if metadata.get("schema") != "compass.graph/1":
    raise SystemExit(f"{name}: unexpected graph schema {metadata.get('schema')!r}")
diagnostics = metadata.get("diagnostics", [])
validation_errors = sum(
    diagnostic.get("severity") == "error" for diagnostic in diagnostics
)
if validation_errors != required_errors:
    raise SystemExit(
        f"{name}: expected {required_errors} validation errors, found {validation_errors}"
    )
print(
    json.dumps(
        {
            "schema": "compass.code-graph-repository-qualification/1",
            "repository": name,
            "commit": commit,
            "graphSha256": hashlib.sha256(graph_bytes).hexdigest(),
            "nodes": len(graph.get("nodes", [])),
            "edges": len(graph.get("links", [])),
            "validationErrors": validation_errors,
        },
        sort_keys=True,
        separators=(",", ":"),
    )
)
PY
    quality_json="$("$COMPASS_BIN" diagnose quality --graph "$repository_graph" --json)"
    python3 - "$quality_json" "$repository_name" <<'PY'
import json
import sys

summary = json.loads(sys.argv[1])
name = sys.argv[2]
diagnostics = summary.get("graph_diagnostics", {})
omitted_nodes = diagnostics.get("publication_omitted_nodes", 0)
omitted_edges = diagnostics.get("publication_omitted_edges", 0)
collisions = diagnostics.get("identity_collisions", 0)
if collisions:
    raise SystemExit(
        f"{name}: identity collisions are not acceptable "
        f"(identity_collisions={collisions})"
    )
if (omitted_nodes or omitted_edges) and "publication_omission_summary" not in diagnostics.get("by_code", {}):
    raise SystemExit(f"{name}: omissions are not accompanied by a publication summary")
if summary.get("output_consistency", {}).get("stats_match_graph") is False:
    raise SystemExit(f"{name}: output stats do not match canonical graph counts")
PY
    status_after="$(git -C "$repository" status --porcelain=v1 --untracked-files=all)"
    [[ "$status_after" == "$status_before" ]] || {
      echo "$repository_name source repository changed during qualification" >&2
      exit 1
    }
  done <"$RELEASE_REPOSITORIES"
  exit 0
fi

echo "[code-graph-v1] validate strict manifests"
python3 scripts/check_code_graph_v1_coverage.py \
  --manifest "$MANIFEST" \
  --corpus-manifest "$CORPUS_MANIFEST"

echo "[code-graph-v1] enforce in-process scale ceilings"
cargo test --locked -p compass-core --test pipeline_scale
cargo test --locked -p compass-query --test code_query_scale
cargo test --locked -p compass-resolve --test framework_resolution_scale

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
  # This gate qualifies the complete language/relation support superset.
  # Product-contract tests separately prove that an omitted CLI option is
  # byte-identical to explicit low inference.
  "$COMPASS_BIN" update "$CORPUS" \
    --out "$OUTPUT_PARENT" --no-cluster --no-viz --no-gitignore \
    --inference-level max "$@" \
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

echo "[code-graph-v1] alternate-checkout production update"
CHECKOUT_CORPUS="$QUALIFY_TMP/alternate/corpus"
CHECKOUT_OUTPUT="$QUALIFY_TMP/alternate/output"
mkdir -p "$CHECKOUT_CORPUS"
cp -R "$CORPUS/." "$CHECKOUT_CORPUS/"
"$COMPASS_BIN" update "$CHECKOUT_CORPUS" \
  --out "$CHECKOUT_OUTPUT" --no-cluster --no-viz --no-gitignore \
  --inference-level max \
  >"$QUALIFY_TMP/alternate-checkout.log"
checkout_graph="$(active_graph "$CHECKOUT_OUTPUT")"
cp "$checkout_graph" "$QUALIFY_TMP/checkout.graph.json"
cmp "$QUALIFY_TMP/clean.graph.json" "$QUALIFY_TMP/checkout.graph.json"

fixture_digest_after="$(fixture_digest)"
[[ "$fixture_digest_before" == "$fixture_digest_after" ]]

cat >"$QUALIFY_TMP/comparisons.json" <<'JSON'
{"cleanEqualsCheckout":true,"cleanEqualsRebuild":true,"cleanEqualsRestored":true,"cleanEqualsWarm":true,"sourceFixtureUnchanged":true}
JSON

echo "[code-graph-v1] execute semantic assertions over production graph"
python3 scripts/check_code_graph_v1_coverage.py \
  --manifest "$MANIFEST" \
  --corpus-manifest "$CORPUS_MANIFEST" \
  --fixture-root "$CORPUS" \
  --graph "$QUALIFY_TMP/restored.graph.json" \
  --compass-revision "$(git rev-parse HEAD)" \
  --comparisons "$QUALIFY_TMP/comparisons.json"

quality_json="$("$COMPASS_BIN" diagnose quality --graph "$QUALIFY_TMP/restored.graph.json" --json)"
python3 - "$quality_json" <<'PY'
import json
import sys

summary = json.loads(sys.argv[1])
diagnostics = summary.get("graph_diagnostics", {})
if diagnostics.get("identity_collisions", 0):
    raise SystemExit(f"fixture publication has identity collisions: {diagnostics}")
if (
    diagnostics.get("publication_omitted_nodes", 0)
    or diagnostics.get("publication_omitted_edges", 0)
) and "publication_omission_summary" not in diagnostics.get("by_code", {}):
    raise SystemExit(f"fixture omissions lack a publication summary: {diagnostics}")
if summary.get("output_consistency", {}).get("stats_match_graph") is False:
    raise SystemExit("fixture output stats do not match canonical graph counts")
PY
