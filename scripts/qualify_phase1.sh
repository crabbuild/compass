#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compass_root="$(cd "$script_dir/.." && pwd)"
graphify_root="${GRAPHIFY_REPO_ROOT:-$(cd "$compass_root/.." && pwd)}"
corpus="${COMPASS_BENCH_CORPUS:-$graphify_root/graphify}"
python_bin="${GRAPHIFY_PYTHON:-$graphify_root/.venv/bin/python}"
repeats="${COMPASS_BENCH_REPEATS:-5}"
query="${COMPASS_BENCH_QUERY:-extract graph files}"
output="${COMPASS_BENCH_OUTPUT:-$compass_root/target/phase1-qualification.csv}"
baseline="${COMPASS_BENCH_BASELINE:-}"
compass_bin="$compass_root/target/release/compass"

if [[ ! -d "$corpus" ]]; then
  echo "error: corpus directory not found: $corpus" >&2
  exit 2
fi
if [[ ! -x "$python_bin" ]]; then
  echo "error: Python oracle not found: $python_bin" >&2
  exit 2
fi
if ! [[ "$repeats" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: COMPASS_BENCH_REPEATS must be a positive integer" >&2
  exit 2
fi

cargo build --manifest-path "$compass_root/Cargo.toml" \
  --workspace --all-features --release --locked
work_root="$(mktemp -d "${TMPDIR:-/tmp}/compass-qualification.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT
mkdir -p "$(dirname "$output")"
printf 'implementation,case,iteration,seconds,peak_rss_kib,indexed_files,nodes,edges,canonical_sha256,correct,files_per_second\n' > "$output"

copy_corpus() {
  local destination="$1"
  mkdir -p "$destination"
  tar -C "$corpus" --exclude='./compass-out' --exclude='*/__pycache__' -cf - . \
    | tar -C "$destination" -xf -
}

measure() {
  local stats="$1"
  local stdout="$2"
  shift 2
  "$python_bin" "$script_dir/measure_process.py" "$stdout" -- "$@" > "$stats"
  IFS=, read -r measured_seconds measured_rss < "$stats"
}

record_pair() {
  local case_name="$1"
  local iteration="$2"
  local python_seconds="$3"
  local python_rss="$4"
  local compass_seconds="$5"
  local compass_rss="$6"
  local python_graph="$7"
  local compass_graph="$8"
  local comparison
  comparison="$("$python_bin" "$script_dir/compare_phase1_graphs.py" --csv "$python_graph" "$compass_graph")"
  local correct nodes edges indexed_files digest
  IFS=, read -r correct nodes edges indexed_files digest <<< "$comparison"
  local python_throughput compass_throughput
  python_throughput="$(awk -v files="$indexed_files" -v seconds="$python_seconds" 'BEGIN {if (seconds > 0) printf "%.3f", files / seconds; else print "0"}')"
  compass_throughput="$(awk -v files="$indexed_files" -v seconds="$compass_seconds" 'BEGIN {if (seconds > 0) printf "%.3f", files / seconds; else print "0"}')"
  printf 'python,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$case_name" "$iteration" "$python_seconds" "$python_rss" "$indexed_files" "$nodes" "$edges" "$digest" "$correct" "$python_throughput" >> "$output"
  printf 'compass,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$case_name" "$iteration" "$compass_seconds" "$compass_rss" "$indexed_files" "$nodes" "$edges" "$digest" "$correct" "$compass_throughput" >> "$output"
}

run_pair() {
  local case_name="$1"
  local iteration="$2"
  shift 2
  local python_stats="$work_root/time-python-${case_name}-${iteration}.txt"
  local compass_stats="$work_root/time-compass-${case_name}-${iteration}.txt"
  measure "$python_stats" /dev/null env GRAPHIFY_OUT="$python_graph_dir" \
    "$python_bin" -m graphify update "$source_corpus" --no-cluster
  local python_seconds="$measured_seconds"
  local python_rss="$measured_rss"
  measure "$compass_stats" /dev/null env COMPASS_OUT="$compass_graph_dir" \
    "$compass_bin" update "$source_corpus" --no-cluster
  record_pair "$case_name" "$iteration" "$python_seconds" "$python_rss" \
    "$measured_seconds" "$measured_rss" \
    "$python_graph_dir/graph.json" "$compass_graph_dir/graph.json"
}

run_read_pair() {
  local case_name="$1"
  local iteration="$2"
  shift 2
  local python_stats="$work_root/time-python-${case_name}-${iteration}.txt"
  local compass_stats="$work_root/time-compass-${case_name}-${iteration}.txt"
  local python_output="$work_root/output-python-${case_name}-${iteration}.txt"
  local compass_output="$work_root/output-compass-${case_name}-${iteration}.txt"
  measure "$python_stats" \
    "$python_output" \
    "$python_bin" -m graphify "$@" \
    --graph "$python_graph_dir/graph.json"
  local python_seconds="$measured_seconds"
  local python_rss="$measured_rss"
  measure "$compass_stats" \
    "$compass_output" \
    "$compass_bin" "$@" \
    --graph "$compass_graph_dir/graph.json"
  if ! "$python_bin" "$script_dir/compare_phase1_read_outputs.py" \
    "$case_name" "$python_output" "$compass_output" \
    "$python_graph_dir/graph.json" "$compass_graph_dir/graph.json"; then
    echo "error: $case_name output differs between Python and Compass" >&2
    diff -u "$python_output" "$compass_output" | sed -n '1,120p' >&2 || true
    exit 1
  fi
  record_pair "$case_name" "$iteration" "$python_seconds" "$python_rss" \
    "$measured_seconds" "$measured_rss" \
    "$python_graph_dir/graph.json" "$compass_graph_dir/graph.json"
}

for ((iteration = 1; iteration <= repeats; iteration++)); do
  source_corpus="$work_root/source-$iteration"
  python_graph_dir="$work_root/python-graph-$iteration"
  compass_graph_dir="$work_root/compass-graph-$iteration"
  copy_corpus "$source_corpus"
  fixture_name="compass_phase1_benchmark_fixture.py"
  fixture_body='class CompassBenchmarkFixture:\n    def initial(self):\n        return "initial"\n'
  printf '%b' "$fixture_body" > "$source_corpus/$fixture_name"

  run_pair cold "$iteration"
  run_pair warm "$iteration"

  printf '%b' '\n    def changed(self):\n        return self.initial()\n' >> "$source_corpus/$fixture_name"
  run_pair incremental_change "$iteration"

  mv "$source_corpus/$fixture_name" "$source_corpus/compass_phase1_renamed_fixture.py"
  run_pair incremental_rename "$iteration"

  rm "$source_corpus/compass_phase1_renamed_fixture.py"
  run_pair incremental_delete "$iteration"

  # Python's baseline query renderer iterates a set for equal-degree ties, so
  # byte order varies with its hash seed. Request the complete result and let
  # the parity comparator require an exact header plus an exact line multiset.
  run_read_pair query "$iteration" query "$query" --budget 1000000
  IFS=$'\t' read -r benchmark_source benchmark_target < <(
    "$python_bin" - "$python_graph_dir/graph.json" <<'PY'
import json
import sys

graph = json.load(open(sys.argv[1], encoding="utf-8"))
links = graph.get("links", graph.get("edges", []))
if links:
    print(f"{links[0]['source']}\t{links[0]['target']}")
else:
    nodes = [node["id"] for node in graph.get("nodes", [])]
    if not nodes:
        raise SystemExit("benchmark graph has no nodes")
    print(f"{nodes[0]}\t{nodes[min(1, len(nodes) - 1)]}")
PY
  )
  run_read_pair path "$iteration" path "$benchmark_source" "$benchmark_target"
  run_read_pair explain "$iteration" explain "$benchmark_source"
  run_read_pair affected "$iteration" affected "$benchmark_source"
done

"$python_bin" - "$output" "$baseline" <<'PY'
import csv
import pathlib
import statistics
import sys
from collections import defaultdict

rows = list(csv.DictReader(open(sys.argv[1], encoding="utf-8")))
baseline_path = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2] else None
groups = defaultdict(list)
for row in rows:
    groups[(row["implementation"], row["case"])].append(row)
summaries = {}
for key in sorted(groups):
    values = groups[key]
    times = sorted(float(value["seconds"]) for value in values)
    rss = max(int(value["peak_rss_kib"]) for value in values)
    p95 = times[min(len(times) - 1, max(0, int(0.95 * len(times) + 0.999) - 1))]
    summaries[key] = (statistics.median(times), p95, rss)
    print(f"{key[0]:6} {key[1]:5} median={statistics.median(times):.3f}s p95={p95:.3f}s peak_rss={rss / 1024:.1f}MiB")

minimum_speedups = {
    "cold": 2.0,
    "warm": 5.0,
    "incremental_change": 5.0,
    "incremental_rename": 5.0,
    "incremental_delete": 5.0,
    "query": 5.0,
    "path": 5.0,
    "explain": 5.0,
    "affected": 5.0,
}
failures = []
for case, minimum in minimum_speedups.items():
    python = summaries.get(("python", case))
    compass = summaries.get(("compass", case))
    if python is None or compass is None:
        failures.append(f"missing measurements for {case}")
        continue
    speedup = python[0] / compass[0]
    if speedup < minimum:
        failures.append(f"{case} speedup {speedup:.2f}x is below {minimum:.1f}x")
    if compass[2] > python[2]:
        failures.append(
            f"{case} Compass peak RSS {compass[2]} KiB exceeds Python {python[2]} KiB"
        )
if baseline_path is not None and baseline_path.is_file():
    baseline_rows = list(csv.DictReader(baseline_path.open(encoding="utf-8")))
    baseline_groups = defaultdict(list)
    for row in baseline_rows:
        baseline_groups[(row["implementation"], row["case"])].append(row)
    for case in minimum_speedups:
        current = summaries.get(("compass", case))
        previous_rows = baseline_groups.get(("compass", case), [])
        if current is None or not previous_rows:
            failures.append(f"baseline is missing Compass measurements for {case}")
            continue
        previous_median = statistics.median(
            float(value["seconds"]) for value in previous_rows
        )
        if current[0] > previous_median * 1.10:
            failures.append(
                f"{case} Compass median regressed from {previous_median:.3f}s "
                f"to {current[0]:.3f}s (>10%)"
            )
if failures:
    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    raise SystemExit(1)
regression = " with the frozen regression baseline" if baseline_path and baseline_path.is_file() else ""
print(f"qualified: all parity, latency, peak-memory, and correctness gates passed{regression}")
PY
echo "raw results: $output"
