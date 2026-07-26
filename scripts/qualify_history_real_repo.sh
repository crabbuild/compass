#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 REPOSITORY OLD NEW" >&2
  exit 2
fi

repository=$1
old_revision=$2
new_revision=$3
script_dir=$(cd "$(dirname "$0")" && pwd)
compass_root=$(cd "$script_dir/.." && pwd)
compass_bin=${COMPASS_BIN:-$compass_root/target/release/compass}
measure=$script_dir/measure_process.py
if [[ $compass_bin != /* ]]; then
  compass_bin=$(cd "$(dirname "$compass_bin")" && pwd)/$(basename "$compass_bin")
fi
for command in git jq python3 shasum; do
  command -v "$command" >/dev/null || {
    echo "error: required command not found: $command" >&2
    exit 1
  }
done
if [[ ! -x $compass_bin ]]; then
  echo "error: Compass binary is not executable: $compass_bin" >&2
  exit 1
fi

repository=$(git -C "$repository" rev-parse --show-toplevel)
original_status=$(git -C "$repository" status --porcelain=v1 --untracked-files=all)
if [[ -n $original_status ]]; then
  echo "error: qualification repository must start clean: $repository" >&2
  exit 1
fi
old_commit=$(git -C "$repository" rev-parse --verify "$old_revision^{commit}")
new_commit=$(git -C "$repository" rev-parse --verify "$new_revision^{commit}")

validation_root=$(mktemp -d "${TMPDIR:-/tmp}/compass-history-real.XXXXXX")
trap 'rm -rf -- "$validation_root"' EXIT
validation_repo=$validation_root/repository
git clone --quiet --shared --no-checkout "$repository" "$validation_repo"
git -C "$validation_repo" checkout --quiet --detach "$new_commit"

run_measured() {
  local name=$1
  shift
  local output=$validation_root/$name.out
  local metrics=$validation_root/$name.metrics
  (cd "$validation_repo" && python3 "$measure" "$output" -- "$@") >"$metrics"
}

run_measured current_cold "$compass_bin" extract . --code-only --no-viz --out "$validation_root/current"
run_measured current_incremental "$compass_bin" extract . --code-only --no-viz --out "$validation_root/current"
run_measured history_cold "$compass_bin" history build "$old_commit" --code-only --format=json
run_measured history_adjacent "$compass_bin" history build "$new_commit" --profile-from "$old_commit" --format=json
run_measured history_noop "$compass_bin" history build "$new_commit" --profile-from "$old_commit" --format=json
run_measured semantic_first "$compass_bin" diff "$old_commit" "$new_commit" --format=json
run_measured semantic_repeat "$compass_bin" diff "$old_commit" "$new_commit" --format=json
run_measured viewer_first "$compass_bin" history export "$new_commit" --format=json --output "$validation_root/viewer-first.json"
run_measured viewer_repeat "$compass_bin" history export "$new_commit" --format=json --output "$validation_root/viewer-repeat.json"

cmp "$validation_root/semantic_first.out" "$validation_root/semantic_repeat.out"
cmp "$validation_root/viewer-first.json" "$validation_root/viewer-repeat.json"

operation_json() {
  local name=$1
  IFS=, read -r seconds peak_rss_kib <"$validation_root/$name.metrics"
  jq -n --argjson seconds "$seconds" --argjson peak "$peak_rss_kib" \
    '{seconds:$seconds,peak_rss_kib:$peak}'
}

if [[ $(git -C "$repository" status --porcelain=v1 --untracked-files=all) != "$original_status" ]]; then
  echo "error: qualification changed the original repository" >&2
  exit 1
fi

jq -n \
  --arg repository "$repository" \
  --arg old "$old_commit" \
  --arg new "$new_commit" \
  --arg binary "$compass_bin" \
  --arg semantic_first "$(shasum -a 256 "$validation_root/semantic_first.out" | awk '{print $1}')" \
  --arg semantic_repeat "$(shasum -a 256 "$validation_root/semantic_repeat.out" | awk '{print $1}')" \
  --arg viewer_first "$(shasum -a 256 "$validation_root/viewer-first.json" | awk '{print $1}')" \
  --arg viewer_repeat "$(shasum -a 256 "$validation_root/viewer-repeat.json" | awk '{print $1}')" \
  --argjson current_cold "$(operation_json current_cold)" \
  --argjson current_incremental "$(operation_json current_incremental)" \
  --argjson history_cold "$(operation_json history_cold)" \
  --argjson history_adjacent "$(operation_json history_adjacent)" \
  --argjson history_noop "$(operation_json history_noop)" \
  --argjson semantic_first_operation "$(operation_json semantic_first)" \
  --argjson semantic_repeat_operation "$(operation_json semantic_repeat)" \
  --argjson viewer_first_operation "$(operation_json viewer_first)" \
  --argjson viewer_repeat_operation "$(operation_json viewer_repeat)" \
  '{
    repository:$repository, old:$old, new:$new, binary:$binary,
    operations:{
      current_cold:$current_cold,
      current_incremental:$current_incremental,
      history_cold:$history_cold,
      history_adjacent:$history_adjacent,
      history_noop:$history_noop,
      semantic_first:$semantic_first_operation,
      semantic_repeat:$semantic_repeat_operation,
      viewer_first:$viewer_first_operation,
      viewer_repeat:$viewer_repeat_operation
    },
    digests:{
      semantic_first:$semantic_first,
      semantic_repeat:$semantic_repeat,
      viewer_first:$viewer_first,
      viewer_repeat:$viewer_repeat
    },
    original_checkout_clean:true
  }'
