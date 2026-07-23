#!/usr/bin/env bash
set -euo pipefail

QUALIFY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUALIFY_TMP="$(mktemp -d "${TMPDIR:-/tmp}/compass-program-ir.XXXXXX")"
trap 'chmod -R u+w "$QUALIFY_TMP" 2>/dev/null || true; rm -rf -- "$QUALIFY_TMP"' EXIT

cd "$QUALIFY_ROOT"

echo "[program-ir] build Compass once"
cargo build -p compass-cli --bin compass
COMPASS_BIN="$QUALIFY_ROOT/target/debug/compass"

echo "[program-ir] package and integration tests"
cargo test -p compass-ir
cargo test -p compass-program
cargo test -p compass-analysis
cargo test -p compass-files
cargo test -p compass-languages --test program_evidence
cargo test -p compass-core --test program_pipeline
cargo test -p compass-cli --test program_cli
cargo test -p compass-history
cargo test -p compass-output --test history_bundle

FIRST_ROOT="$QUALIFY_TMP/checkout-a"
SECOND_ROOT="$QUALIFY_TMP/checkout-b"
mkdir -p "$FIRST_ROOT/src" "$SECOND_ROOT/src"
cp fixtures/program-ir/rust/lib.rs "$FIRST_ROOT/src/lib.rs"
cp fixtures/program-ir/typescript/app.tsx "$FIRST_ROOT/src/app.tsx"
cp fixtures/program-ir/rust/lib.rs "$SECOND_ROOT/src/lib.rs"
cp fixtures/program-ir/typescript/app.tsx "$SECOND_ROOT/src/app.tsx"
cp "$FIRST_ROOT/src/lib.rs" "$QUALIFY_TMP/lib.rs.original"

echo "[program-ir] cold and warm builds"
COLD_LOG="$QUALIFY_TMP/cold.log"
WARM_LOG="$QUALIFY_TMP/warm.log"
"$COMPASS_BIN" update "$FIRST_ROOT" --no-cluster --no-viz >"$COLD_LOG"
cp "$FIRST_ROOT/compass-out/program.json" "$QUALIFY_TMP/program.cold.json"
"$COMPASS_BIN" update "$FIRST_ROOT" --no-cluster --no-viz >"$WARM_LOG"
cmp "$FIRST_ROOT/compass-out/program.json" "$QUALIFY_TMP/program.cold.json"
grep -Eq 'Program analysis: [1-9][0-9]* syntax analyzed' "$COLD_LOG"
grep -Eq 'Program analysis: 0 syntax analyzed, [1-9][0-9]* syntax reused' "$WARM_LOG"

echo "[program-ir] syntax change and restored incremental equivalence"
printf '\nfn qualification_change() {}\n' >>"$FIRST_ROOT/src/lib.rs"
"$COMPASS_BIN" update "$FIRST_ROOT" --no-cluster --no-viz >"$QUALIFY_TMP/syntax-change.log"
if cmp -s "$FIRST_ROOT/compass-out/program.json" "$QUALIFY_TMP/program.cold.json"; then
  echo "syntax change did not change program.json" >&2
  exit 1
fi
cp "$QUALIFY_TMP/lib.rs.original" "$FIRST_ROOT/src/lib.rs"
"$COMPASS_BIN" update "$FIRST_ROOT" --no-cluster --no-viz >"$QUALIFY_TMP/restored.log"
cmp "$FIRST_ROOT/compass-out/program.json" "$QUALIFY_TMP/program.cold.json"

echo "[program-ir] checkout-root and clean-build determinism"
"$COMPASS_BIN" update "$SECOND_ROOT" --no-cluster --no-viz >"$QUALIFY_TMP/second-root.log"
cmp "$SECOND_ROOT/compass-out/program.json" "$QUALIFY_TMP/program.cold.json"

echo "[program-ir] artifact-only invalidation, freshness, malformed input, and atomic output"
cargo test -p compass-core --test program_pipeline scip_cache_tracks_artifact_manifest_and_source_freshness
cargo test -p compass-core --test program_pipeline scip_cache_renormalizes_only_the_changed_document
cargo test -p compass-core --test program_pipeline malformed_discovered_scip_and_obstructed_output_fail_closed
cargo test -p compass-core watch_rebuilds_for_external_scip_and_companion_manifest_changes
cargo test -p compass-program --test scip
cargo test -p compass-program --test merge merge_preserves_conflicting_targets

echo "[program-ir] history ingest, reopen, diff, GC, and export"
cargo test -p compass-history --test roundtrip
cargo test -p compass-history --test diff
cargo test -p compass-history --test maintenance
cargo test -p compass-cli --test history_cli history_commands_inspect_prefer_and_export_published_realizations

echo "[program-ir] Graphify compatibility produces no program.json"
cargo test -p compass-cli --test program_cli graphify_rejects_program_artifacts_and_never_enables_program_output

echo "[program-ir] workspace qualification"
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

echo "[program-ir] qualification passed"
