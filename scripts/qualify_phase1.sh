#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rust_root="$(cd "$script_dir/.." && pwd)"
repo_root="$(cd "$rust_root/.." && pwd)"
corpus="${TRAIL_BENCH_CORPUS:-$repo_root/graphify}"
python_bin="${GRAPHIFY_PYTHON:-$repo_root/.venv/bin/python}"
repeats="${TRAIL_BENCH_REPEATS:-5}"
query="${TRAIL_BENCH_QUERY:-extract graph files}"
output="${TRAIL_BENCH_OUTPUT:-$rust_root/target/phase1-qualification.csv}"
trail_bin="$rust_root/target/release/trail"

if [[ ! -d "$corpus" ]]; then
  echo "error: corpus directory not found: $corpus" >&2
  exit 2
fi
if [[ ! -x "$python_bin" ]]; then
  echo "error: Python oracle not found: $python_bin" >&2
  exit 2
fi
if ! [[ "$repeats" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: TRAIL_BENCH_REPEATS must be a positive integer" >&2
  exit 2
fi

cargo build --manifest-path "$rust_root/Cargo.toml" --release --locked --bins
work_root="$(mktemp -d "${TMPDIR:-/tmp}/trail-qualification.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT
mkdir -p "$(dirname "$output")"
printf 'implementation,case,iteration,seconds,peak_rss_kib,indexed_files,nodes,edges,canonical_sha256,correct,files_per_second\n' > "$output"

copy_corpus() {
  local destination="$1"
  mkdir -p "$destination"
  tar -C "$corpus" --exclude='./graphify-out' --exclude='*/__pycache__' -cf - . \
    | tar -C "$destination" -xf -
}

measure() {
  local stats="$1"
  shift
  if [[ "$(uname -s)" == "Darwin" ]]; then
    /usr/bin/time -lp "$@" > /dev/null 2> "$stats"
    measured_seconds="$(awk '$1 == "real" {print $2}' "$stats" | tail -1)"
    measured_rss="$(awk '/maximum resident set size/ {printf "%.0f", $1 / 1024}' "$stats" | tail -1)"
  else
    /usr/bin/time -f 'TRAIL_TIME %e %M' "$@" > /dev/null 2> "$stats"
    measured_seconds="$(awk '$1 == "TRAIL_TIME" {print $2}' "$stats" | tail -1)"
    measured_rss="$(awk '$1 == "TRAIL_TIME" {print $3}' "$stats" | tail -1)"
  fi
}

record_pair() {
  local case_name="$1"
  local iteration="$2"
  local python_seconds="$3"
  local python_rss="$4"
  local trail_seconds="$5"
  local trail_rss="$6"
  local python_graph="$7"
  local trail_graph="$8"
  local comparison
  comparison="$("$python_bin" "$script_dir/compare_phase1_graphs.py" --csv "$python_graph" "$trail_graph")"
  local correct nodes edges indexed_files digest
  IFS=, read -r correct nodes edges indexed_files digest <<< "$comparison"
  local python_throughput trail_throughput
  python_throughput="$(awk -v files="$indexed_files" -v seconds="$python_seconds" 'BEGIN {if (seconds > 0) printf "%.3f", files / seconds; else print "0"}')"
  trail_throughput="$(awk -v files="$indexed_files" -v seconds="$trail_seconds" 'BEGIN {if (seconds > 0) printf "%.3f", files / seconds; else print "0"}')"
  printf 'python,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$case_name" "$iteration" "$python_seconds" "$python_rss" "$indexed_files" "$nodes" "$edges" "$digest" "$correct" "$python_throughput" >> "$output"
  printf 'trail,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$case_name" "$iteration" "$trail_seconds" "$trail_rss" "$indexed_files" "$nodes" "$edges" "$digest" "$correct" "$trail_throughput" >> "$output"
}

run_pair() {
  local case_name="$1"
  local iteration="$2"
  shift 2
  local python_stats="$work_root/time-python-${case_name}-${iteration}.txt"
  local trail_stats="$work_root/time-trail-${case_name}-${iteration}.txt"
  measure "$python_stats" "$python_bin" -m graphify update "$python_corpus" --no-cluster
  local python_seconds="$measured_seconds"
  local python_rss="$measured_rss"
  measure "$trail_stats" "$trail_bin" graph update "$trail_corpus" --no-cluster
  record_pair "$case_name" "$iteration" "$python_seconds" "$python_rss" \
    "$measured_seconds" "$measured_rss" \
    "$python_corpus/graphify-out/graph.json" "$trail_corpus/graphify-out/graph.json"
}

run_read_pair() {
  local case_name="$1"
  local iteration="$2"
  shift 2
  local python_stats="$work_root/time-python-${case_name}-${iteration}.txt"
  local trail_stats="$work_root/time-trail-${case_name}-${iteration}.txt"
  measure "$python_stats" \
    "$python_bin" -m graphify "$@" \
    --graph "$python_corpus/graphify-out/graph.json"
  local python_seconds="$measured_seconds"
  local python_rss="$measured_rss"
  measure "$trail_stats" \
    "$trail_bin" graph "$@" \
    --graph "$trail_corpus/graphify-out/graph.json"
  record_pair "$case_name" "$iteration" "$python_seconds" "$python_rss" \
    "$measured_seconds" "$measured_rss" \
    "$python_corpus/graphify-out/graph.json" "$trail_corpus/graphify-out/graph.json"
}

for ((iteration = 1; iteration <= repeats; iteration++)); do
  python_corpus="$work_root/python-$iteration"
  trail_corpus="$work_root/trail-$iteration"
  copy_corpus "$python_corpus"
  copy_corpus "$trail_corpus"
  fixture_name="trail_phase1_benchmark_fixture.py"
  fixture_body='class TrailBenchmarkFixture:\n    def initial(self):\n        return "initial"\n'
  printf '%b' "$fixture_body" > "$python_corpus/$fixture_name"
  printf '%b' "$fixture_body" > "$trail_corpus/$fixture_name"

  run_pair cold "$iteration"
  run_pair warm "$iteration"

  printf '%b' '\n    def changed(self):\n        return self.initial()\n' >> "$python_corpus/$fixture_name"
  printf '%b' '\n    def changed(self):\n        return self.initial()\n' >> "$trail_corpus/$fixture_name"
  run_pair incremental_change "$iteration"

  mv "$python_corpus/$fixture_name" "$python_corpus/trail_phase1_renamed_fixture.py"
  mv "$trail_corpus/$fixture_name" "$trail_corpus/trail_phase1_renamed_fixture.py"
  run_pair incremental_rename "$iteration"

  rm "$python_corpus/trail_phase1_renamed_fixture.py"
  rm "$trail_corpus/trail_phase1_renamed_fixture.py"
  run_pair incremental_delete "$iteration"

  run_read_pair query "$iteration" query "$query"
  IFS=$'\t' read -r benchmark_source benchmark_target < <(
    "$python_bin" - "$python_corpus/graphify-out/graph.json" <<'PY'
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

"$python_bin" - "$output" <<'PY'
import csv
import statistics
import sys
from collections import defaultdict

rows = list(csv.DictReader(open(sys.argv[1], encoding="utf-8")))
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
    trail = summaries.get(("trail", case))
    if python is None or trail is None:
        failures.append(f"missing measurements for {case}")
        continue
    speedup = python[0] / trail[0]
    if speedup < minimum:
        failures.append(f"{case} speedup {speedup:.2f}x is below {minimum:.1f}x")
    if trail[2] > python[2]:
        failures.append(
            f"{case} Trail peak RSS {trail[2]} KiB exceeds Python {python[2]} KiB"
        )
if failures:
    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    raise SystemExit(1)
print("qualified: all parity, latency, and peak-memory gates passed")
PY
echo "raw results: $output"
