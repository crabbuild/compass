#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compass_root="$(cd "$script_dir/.." && pwd)"
graphify_root="${GRAPHIFY_REPO_ROOT:-$(cd "$compass_root/.." && pwd)}"
output_dir="${COMPASS_BENCH_MATRIX_OUTPUT:-$compass_root/target/phase1-qualification-matrix}"
baseline_dir="${COMPASS_BENCH_BASELINE_DIR:-}"
work_root="$(mktemp -d "${TMPDIR:-/tmp}/compass-matrix.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT

mkdir -p "$output_dir"

copy_tree() {
  local source="$1"
  local destination="$2"
  mkdir -p "$destination"
  tar -C "$source" --exclude='compass-out' --exclude='*/__pycache__' -cf - . \
    | tar -C "$destination" -xf -
}

prepare_corpus() {
  local tier="$1"
  local corpus="$work_root/$tier"
  mkdir -p "$corpus"
  case "$tier" in
    small)
      copy_tree "$graphify_root/tests/fixtures" "$corpus/fixtures"
      ;;
    medium)
      copy_tree "$graphify_root/graphify" "$corpus/graphify"
      copy_tree "$graphify_root/tests/fixtures" "$corpus/fixtures"
      ;;
    large)
      copy_tree "$graphify_root/graphify" "$corpus/graphify"
      copy_tree "$graphify_root/tests" "$corpus/tests"
      copy_tree "$compass_root/crates" "$corpus/compass-crates"
      copy_tree "$graphify_root/docs" "$corpus/docs"
      ;;
    *)
      echo "error: unsupported benchmark tier: $tier" >&2
      exit 2
      ;;
  esac
  printf '%s\n' "$corpus"
}

for tier in small medium large; do
  corpus="$(prepare_corpus "$tier")"
  echo "qualifying $tier multilingual corpus"
  baseline=""
  if [[ -n "$baseline_dir" && -f "$baseline_dir/$tier.csv" ]]; then
    baseline="$baseline_dir/$tier.csv"
  fi
  COMPASS_BENCH_CORPUS="$corpus" \
    COMPASS_BENCH_OUTPUT="$output_dir/$tier.csv" \
    COMPASS_BENCH_BASELINE="$baseline" \
    "$script_dir/qualify_phase1.sh"
done

echo "matrix results: $output_dir"
