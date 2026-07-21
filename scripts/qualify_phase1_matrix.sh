#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rust_root="$(cd "$script_dir/.." && pwd)"
repo_root="$(cd "$rust_root/.." && pwd)"
output_dir="${TRAIL_BENCH_MATRIX_OUTPUT:-$rust_root/target/phase1-qualification-matrix}"
work_root="$(mktemp -d "${TMPDIR:-/tmp}/trail-matrix.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT

mkdir -p "$output_dir"

copy_tree() {
  local source="$1"
  local destination="$2"
  mkdir -p "$destination"
  tar -C "$source" --exclude='graphify-out' --exclude='*/__pycache__' -cf - . \
    | tar -C "$destination" -xf -
}

prepare_corpus() {
  local tier="$1"
  local corpus="$work_root/$tier"
  mkdir -p "$corpus"
  case "$tier" in
    small)
      copy_tree "$repo_root/tests/fixtures" "$corpus/fixtures"
      ;;
    medium)
      copy_tree "$repo_root/graphify" "$corpus/graphify"
      copy_tree "$repo_root/tests/fixtures" "$corpus/fixtures"
      ;;
    large)
      copy_tree "$repo_root/graphify" "$corpus/graphify"
      copy_tree "$repo_root/tests" "$corpus/tests"
      copy_tree "$rust_root/crates" "$corpus/rust-crates"
      copy_tree "$repo_root/docs" "$corpus/docs"
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
  TRAIL_BENCH_CORPUS="$corpus" \
    TRAIL_BENCH_OUTPUT="$output_dir/$tier.csv" \
    "$script_dir/qualify_phase1.sh"
done

echo "matrix results: $output_dir"
