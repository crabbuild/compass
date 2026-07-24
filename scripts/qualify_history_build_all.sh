#!/usr/bin/env bash
set -euo pipefail

if (( $# < 2 || $# > 3 )); then
  echo "usage: $0 REPOSITORY REF [DEPTH]" >&2
  exit 2
fi

repository=$1
reference=$2
depth=${3:-5}
script_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
compass_bin=${COMPASS_BIN:-"$script_root/target/debug/compass"}

command -v jq >/dev/null
test -x "$compass_bin"
repository=$(git -C "$repository" rev-parse --show-toplevel)
before_status=$(git -C "$repository" status --porcelain=v1 --untracked-files=all)
qualification_root=$(mktemp -d)
trap 'rm -rf "$qualification_root"' EXIT
clone="$qualification_root/repository"

git clone --quiet --no-checkout --depth "$depth" "file://$repository" "$clone"
git -C "$clone" fetch --quiet --depth "$depth" origin "$reference"
git -C "$clone" checkout --quiet --detach FETCH_HEAD

(
  cd "$clone"
  "$compass_bin" history build HEAD --all --code-only --format=json >"$qualification_root/first.json"
  jq -e '
    .counts.total > 1 and
    .counts.failed == 0 and
    (.counts.built + .counts.rebuilt + .counts.skipped == .counts.total)
  ' "$qualification_root/first.json" >/dev/null

  "$compass_bin" history build HEAD --all --code-only --format=json >"$qualification_root/second.json"
  jq -e '
    .counts.failed == 0 and
    .counts.skipped == .counts.total
  ' "$qualification_root/second.json" >/dev/null

  expected=$(git rev-list --count HEAD)
  actual=$(jq -r '.counts.total' "$qualification_root/first.json")
  test "$actual" -eq "$expected"

  while read -r commit; do
    "$compass_bin" history status "$commit" --format=json |
      jq -e '.preferred != null and .validation.valid == true' >/dev/null
  done < <(git rev-list HEAD)
)

after_status=$(git -C "$repository" status --porcelain=v1 --untracked-files=all)
if [[ "$before_status" != "$after_status" ]]; then
  echo "source repository status changed during qualification" >&2
  exit 1
fi

jq '{ref, tip, scope, profile_digest, counts}' "$qualification_root/first.json"
echo "rerun skipped every reachable commit"
