#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compass_root="$(cd "$script_dir/.." && pwd)"
cd "$compass_root"

version="$(cargo metadata --no-deps --format-version 1 \
  | jq -r '[.packages[] | select(.name != "compass-parity" and .name != "compass-tree-sitter-language-pack") | .version] | unique | if length == 1 then .[0] else error("workspace versions differ") end')"
expected_confirmation="publish-$version"
if [[ "${COMPASS_PUBLISH_CONFIRM:-}" != "$expected_confirmation" ]]; then
  echo "error: set COMPASS_PUBLISH_CONFIRM=$expected_confirmation" >&2
  exit 2
fi
if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  echo "error: refusing to publish from a dirty worktree" >&2
  exit 2
fi
if [[ "$(git describe --tags --exact-match 2>/dev/null || true)" != "compass-v$version" ]]; then
  echo "error: HEAD must be tagged compass-v$version" >&2
  exit 2
fi

cargo package --workspace --locked --no-verify

# Prove that compass-languages' normalized registry manifest points at the
# published static adapter by package name. A path-only dependency would work
# in this checkout yet make `cargo install compass-cli` silently lose grammars.
normalized_languages_manifest="$(
  tar -xOf "target/package/compass-languages-$version.crate" \
    "compass-languages-$version/Cargo.toml"
)"
if ! rg -q 'package = "compass-tree-sitter-language-pack"' \
  <<<"$normalized_languages_manifest"; then
  echo "error: packaged compass-languages does not select the static grammar adapter" >&2
  exit 2
fi

# A registry install cannot inherit this repository's .cargo/config.toml. The
# adapter therefore owns Compass's compile-time grammar selection and must be
# published before compass-languages. Its version follows the pinned upstream
# parser bundle, so subsequent Compass releases reuse the already-published crate.
if cargo info compass-tree-sitter-language-pack@1.13.1 >/dev/null 2>&1; then
  echo "compass-tree-sitter-language-pack 1.13.1 is already published"
else
  cargo publish --locked \
    --manifest-path vendor/compass-tree-sitter-language-pack/Cargo.toml
fi

# Cargo waits for each new package to become available in the registry index,
# so downstream crates can be published immediately in topological order.
crates=(
  compass-model
  compass-graphdb
  compass-files
  compass-media
  compass-whisper
  compass-cargo
  compass-google-workspace
  compass-prs
  compass-query
  compass-reflect
  compass-global
  compass-semantic
  compass-transcribe
  compass-ingest
  compass-languages
  compass-postgres
  compass-graph
  compass-resolve
  compass-output
  compass-core
  compass-mcp
  compass-cli
)
for crate in "${crates[@]}"; do
  cargo publish --locked -p "$crate"
done
