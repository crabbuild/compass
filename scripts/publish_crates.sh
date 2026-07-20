#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rust_root="$(cd "$script_dir/.." && pwd)"
cd "$rust_root"

version="$(cargo metadata --no-deps --format-version 1 \
  | jq -r '[.packages[] | select(.name != "trail-parity") | .version] | unique | if length == 1 then .[0] else error("workspace versions differ") end')"
expected_confirmation="publish-$version"
if [[ "${TRAIL_PUBLISH_CONFIRM:-}" != "$expected_confirmation" ]]; then
  echo "error: set TRAIL_PUBLISH_CONFIRM=$expected_confirmation" >&2
  exit 2
fi
if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  echo "error: refusing to publish from a dirty worktree" >&2
  exit 2
fi
if [[ "$(git describe --tags --exact-match 2>/dev/null || true)" != "trail-v$version" ]]; then
  echo "error: HEAD must be tagged trail-v$version" >&2
  exit 2
fi

cargo package --workspace --locked --no-verify

# Cargo waits for each new package to become available in the registry index,
# so downstream crates can be published immediately in topological order.
crates=(
  trail-model
  trail-files
  trail-languages
  trail-graph
  trail-output
  trail-resolve
  trail-query
  trail-core
  trail-cli
)
for crate in "${crates[@]}"; do
  cargo publish --locked -p "$crate"
done
