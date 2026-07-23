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
compass_bin=${COMPASS_BIN:-$compass_root/target/debug/compass}
if [[ $compass_bin != /* ]]; then
  compass_bin=$(cd "$(dirname "$compass_bin")" && pwd)/$(basename "$compass_bin")
fi
if [[ ! -x $compass_bin ]]; then
  echo "error: Compass binary is not executable: $compass_bin" >&2
  exit 1
fi

for command in git jq cmp perl; do
  command -v "$command" >/dev/null || {
    echo "error: required command not found: $command" >&2
    exit 1
  }
done

repository=$(git -C "$repository" rev-parse --show-toplevel)
if [[ -n $(git -C "$repository" status --porcelain=v1 --untracked-files=all) ]]; then
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

old_json=$validation_root/old.json
new_json=$validation_root/new.json
forward_json=$validation_root/forward.json
repeat_json=$validation_root/repeat.json
reverse_json=$validation_root/reverse.json
topology_json=$validation_root/topology.json

now_millis() {
  perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000'
}

(cd "$validation_repo" && "$compass_bin" history build "$old_commit" --code-only --format=json) >"$old_json"
(cd "$validation_repo" && "$compass_bin" history build "$new_commit" --profile-from "$old_commit" --format=json) >"$new_json"
(cd "$validation_repo" && "$compass_bin" diff "$old_commit" "$new_commit" --format=json) >"$forward_json"
full_started=$(now_millis)
(cd "$validation_repo" && "$compass_bin" diff "$old_commit" "$new_commit" --format=json) >"$repeat_json"
full_millis=$(( $(now_millis) - full_started ))
cmp "$forward_json" "$repeat_json"
(cd "$validation_repo" && "$compass_bin" diff "$new_commit" "$old_commit" --format=json) >"$reverse_json"

jq -S '[.changes[] | {
  record,
  key,
  change:(if .change == "added" then "removed" elif .change == "removed" then "added" else .change end),
  old:(.new // null),
  new:(.old // null)
}] | sort_by(.record, (.key | tostring), .change)' "$forward_json" >"$validation_root/forward-reversed.json"
jq -S '[.changes[] | {
  record,
  key,
  change,
  old:(.old // null),
  new:(.new // null)
}] | sort_by(.record, (.key | tostring), .change)' "$reverse_json" >"$validation_root/reverse-normalized.json"
cmp "$validation_root/forward-reversed.json" "$validation_root/reverse-normalized.json"

topology_started=$(now_millis)
(cd "$validation_repo" && "$compass_bin" diff "$old_commit" "$new_commit" --topology-only --format=json) >"$topology_json"
topology_millis=$(( $(now_millis) - topology_started ))
jq -e 'all(.changes[]; ((.record == "node" or .record == "edge") and (.change == "added" or .change == "removed")))' "$topology_json" >/dev/null
if (( topology_millis * 2 >= full_millis )); then
  echo "error: topology-only diff was not at least twice as fast as full diff (${topology_millis}ms vs ${full_millis}ms)" >&2
  exit 1
fi
(cd "$validation_repo" && "$compass_bin" history status "$new_commit" --format=json) |
  jq -e '.validation.valid == true' >/dev/null

if [[ -n $(git -C "$repository" status --porcelain=v1 --untracked-files=all) ]]; then
  echo "error: qualification changed the original repository" >&2
  exit 1
fi

jq -n \
  --arg repository "$repository" \
  --arg old "$old_commit" \
  --arg new "$new_commit" \
  --argjson old_graph "$(jq '{nodes,edges,hyperedges,analysis_records,metadata_records}' "$old_json")" \
  --argjson new_graph "$(jq '{nodes,edges,hyperedges,analysis_records,metadata_records}' "$new_json")" \
  --argjson diff_summary "$(jq '.summary' "$forward_json")" \
  --argjson topology_summary "$(jq '.summary' "$topology_json")" \
  --argjson full_millis "$full_millis" \
  --argjson topology_millis "$topology_millis" \
  '{repository:$repository,old:$old,new:$new,old_graph:$old_graph,new_graph:$new_graph,diff_summary:$diff_summary,topology_summary:$topology_summary,timing:{full_millis:$full_millis,topology_millis:$topology_millis},json_deterministic:true,reverse_symmetric:true,topology_materially_faster:true,original_checkout_clean:true}'
