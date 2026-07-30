#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compass_root="$(cd "$script_dir/.." && pwd)"
cd "$compass_root"

version="$(cargo metadata --no-deps --format-version 1 \
  | jq -r '[.packages[] | select(.name != "compass-tree-sitter-language-pack") | .version] | unique | if length == 1 then .[0] else error("workspace versions differ") end')"
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

# Private inference crates remain workspace-native, but have no registry graph
# and therefore are intentionally excluded from the release package set.
cargo package --workspace --locked --no-verify \
  --exclude compass-transcribe \
  --exclude compass-whisper

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

# A publishable registry crate may only depend on other registry crates. Compute
# the complete dependency-first order from workspace metadata so newly added
# product crates cannot be silently omitted from a release.
mapfile -t crates < <(
  cargo metadata --no-deps --format-version 1 \
    | python3 scripts/publishable_crates.py
)

# Normalized manifests are Cargo's actual registry contracts. Keep inference
# crates out of both the ingest boundary and the installable CLI package.
for packaged_crate in compass-ingest compass-cli; do
  normalized_manifest="$(
    tar -xOf "target/package/$packaged_crate-$version.crate" \
      "$packaged_crate-$version/Cargo.toml"
  )"
  if rg -q 'compass-(transcribe|whisper)' <<<"$normalized_manifest"; then
    echo "error: packaged $packaged_crate reaches an internal inference crate" >&2
    exit 2
  fi
done

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
for crate in "${crates[@]}"; do
  cargo publish --locked -p "$crate"
done
