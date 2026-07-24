#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
compass_repo=$(cd "$script_dir/.." && pwd -P)
podman_input=${PODMAN_ROOT:-/Volumes/Workspace/Github/podman}
if [[ ! -d "$podman_input" ]]; then
  echo "error: Podman root does not exist: $podman_input" >&2
  exit 2
fi
podman_root=$(cd "$podman_input" && pwd -P)
user_home=$(cd && pwd -P)
if [[ -z "$podman_root" || "$podman_root" == "/" || "$podman_root" == "$user_home" ]]; then
  echo "error: unsafe Podman root: $podman_root" >&2
  exit 2
fi
if [[ ! -d "$podman_root/.git" ]]; then
  echo "error: Podman root is not a Git checkout: $podman_root" >&2
  exit 2
fi

compass_bin=${COMPASS_BIN:-"$compass_repo/target/release/compass"}
graphify_python=${GRAPHIFY_PYTHON:-/Users/haipingfu/graphify/.venv/bin/python}
parity_samples=${PARITY_SAMPLES:-3}
query_samples=${QUERY_SAMPLES:-5}
query_text=${PARITY_QUERY:-update}
compass_output="$podman_root/compass-out"
graphify_output="$podman_root/graphify-out"
results_dir=$(mktemp -d "${TMPDIR:-/tmp}/compass-graphify-parity.XXXXXX")
timings="$results_dir/timings.tsv"

for count in "$parity_samples" "$query_samples"; do
  if [[ ! "$count" =~ ^[1-9][0-9]*$ ]] || ((count % 2 == 0)); then
    echo "error: sample counts must be positive odd integers" >&2
    exit 2
  fi
done
if [[ ! -x "$graphify_python" ]]; then
  echo "error: Graphify Python is not executable: $graphify_python" >&2
  exit 2
fi

reset_output() {
  local target=$1
  if [[ "$target" != "$compass_output" && "$target" != "$graphify_output" ]]; then
    echo "error: refusing to reset unexpected output: $target" >&2
    exit 2
  fi
  if [[ "$(dirname "$target")" != "$podman_root" ]]; then
    echo "error: refusing to reset output outside Podman: $target" >&2
    exit 2
  fi
  if [[ -e "$target" ]]; then
    rm -rf -- "$target"
  fi
}

prepare_graphify_output() {
  mkdir -p "$graphify_output"
  printf '%s\n' '{"excludes":["/compass-out/"]}' \
    >"$graphify_output/.graphify_build.json"
}

graph_counts() {
  local graph=$1
  "$graphify_python" - "$graph" <<'PY'
import json
import sys
from pathlib import Path

data = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(f"{len(data.get('nodes', []))}\t{len(data.get('links', data.get('edges', [])))}")
PY
}

measure() {
  local tool=$1
  local operation=$2
  local sample=$3
  local graph=$4
  shift 4
  local label="${tool}-${operation}-${sample}"
  local stdout_file="$results_dir/$label.stdout"
  local stderr_file="$results_dir/$label.stderr"

  if ! (
    cd "$podman_root"
    /usr/bin/time -p "$@"
  ) >"$stdout_file" 2>"$stderr_file"; then
    cat "$stderr_file" >&2
    echo "error: $tool $operation sample $sample failed" >&2
    exit 1
  fi
  local seconds
  seconds=$(awk '$1 == "real" { value=$2 } END { print value }' "$stderr_file")
  if [[ -z "$seconds" ]]; then
    echo "error: no elapsed time recorded for $label" >&2
    exit 1
  fi
  local nodes edges
  IFS=$'\t' read -r nodes edges < <(graph_counts "$graph")
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$tool" "$operation" "$sample" "$seconds" "$nodes" "$edges" >>"$timings"
  echo "$tool $operation $sample: ${seconds}s, $nodes nodes, $edges edges"
}

median() {
  local tool=$1
  local operation=$2
  local count=$3
  local middle=$((count / 2 + 1))
  awk -F $'\t' -v tool="$tool" -v operation="$operation" \
    '$1 == tool && $2 == operation { print $4 }' "$timings" |
    LC_ALL=C sort -n |
    sed -n "${middle}p"
}

assert_at_most() {
  local label=$1
  local value=$2
  local maximum=$3
  if ! awk -v value="$value" -v maximum="$maximum" \
    'BEGIN { exit(value <= maximum ? 0 : 1) }'; then
    echo "error: $label median ${value}s exceeds ${maximum}s" >&2
    exit 1
  fi
}

normalize_query() {
  local input=$1
  local output=$2
  # Community labels may differ when Compass adds superset-only topology, and
  # edge rows compete for the shared token budget after all result nodes are
  # printed. Compare stable node identity here; the full graph comparator above
  # already proves exact Graphify node attributes and edge inclusion.
  sed -n 's/^NODE \(.*\) community=[^]]*]$/NODE \1]/p' "$input" |
    LC_ALL=C sort -u >"$output"
}

printf 'tool\toperation\tsample\tseconds\tnodes\tedges\n' >"$timings"

echo "Building release Compass and parity comparator"
(cd "$compass_repo" && cargo build --release -p compass-cli)
(cd "$compass_repo" && cargo build --release -p compass-parity --bin compare-graphs)
if [[ ! -x "$compass_bin" ]]; then
  echo "error: Compass binary is not executable: $compass_bin" >&2
  exit 2
fi
comparator="$compass_repo/target/release/compare-graphs"

for sample in $(seq 1 "$parity_samples"); do
  reset_output "$compass_output"
  measure compass cold "$sample" "$compass_output/graph.json" \
    env COMPASS_OUT=compass-out "$compass_bin" update .
done
for sample in $(seq 1 "$parity_samples"); do
  measure compass warm "$sample" "$compass_output/graph.json" \
    env COMPASS_OUT=compass-out "$compass_bin" update .
done

for sample in $(seq 1 "$parity_samples"); do
  reset_output "$graphify_output"
  prepare_graphify_output
  measure graphify cold "$sample" "$graphify_output/graph.json" \
    env GRAPHIFY_OUT=graphify-out "$graphify_python" -m graphify update .
done
for sample in $(seq 1 "$parity_samples"); do
  measure graphify warm "$sample" "$graphify_output/graph.json" \
    env GRAPHIFY_OUT=graphify-out "$graphify_python" -m graphify update .
done

echo "Checking graph superset parity"
"$comparator" "$compass_output/graph.json" "$graphify_output/graph.json" |
  tee "$results_dir/parity.txt"

for sample in $(seq 1 "$query_samples"); do
  measure compass query "$sample" "$compass_output/graph.json" \
    "$compass_bin" query "$query_text" --graph "$compass_output/graph.json"
  measure graphify query "$sample" "$graphify_output/graph.json" \
    "$graphify_python" -m graphify query "$query_text" --graph "$graphify_output/graph.json"
done

normalize_query "$results_dir/compass-query-${query_samples}.stdout" \
  "$results_dir/compass-query.normalized"
normalize_query "$results_dir/graphify-query-${query_samples}.stdout" \
  "$results_dir/graphify-query.normalized"
comm -23 \
  "$results_dir/graphify-query.normalized" \
  "$results_dir/compass-query.normalized" >"$results_dir/query-missing.txt"
if [[ -s "$results_dir/query-missing.txt" ]]; then
  echo "error: Compass query is missing Graphify result rows" >&2
  sed -n '1,50p' "$results_dir/query-missing.txt" >&2
  exit 1
fi

compass_cold=$(median compass cold "$parity_samples")
compass_warm=$(median compass warm "$parity_samples")
graphify_cold=$(median graphify cold "$parity_samples")
graphify_warm=$(median graphify warm "$parity_samples")
compass_query=$(median compass query "$query_samples")
graphify_query=$(median graphify query "$query_samples")

assert_at_most "Compass cold" "$compass_cold" 32
assert_at_most "Compass warm" "$compass_warm" 10

{
  echo "Compass cold median: ${compass_cold}s"
  echo "Compass warm median: ${compass_warm}s"
  echo "Graphify cold median: ${graphify_cold}s"
  echo "Graphify warm median: ${graphify_warm}s"
  echo "Compass query median: ${compass_query}s"
  echo "Graphify query median: ${graphify_query}s"
  echo "Query node inclusion: pass"
} | tee "$results_dir/summary.txt"

echo "Qualification evidence: $results_dir"
