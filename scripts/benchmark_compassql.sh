#!/bin/sh
set -eu

graph=${1:-graphify-out/graph.json}
binary=${COMPASS_BIN:-target/release/compass}
output=${COMPASSQL_BENCH_OUTPUT:-target/compassql-benchmark.csv}
repeats=${COMPASSQL_BENCH_REPEATS:-5}
baseline=${COMPASSQL_BENCH_BASELINE:-}
summary=${output%.csv}-summary.csv

if [ ! -x "$binary" ]; then
    cargo build --release --locked -p compass-cli --bin compass
fi
if [ ! -f "$graph" ]; then
    echo "graph not found: $graph" >&2
    exit 2
fi

mkdir -p "$(dirname "$output")"
echo "case,iteration,seconds,peak_kib,expanded_relationships,rows,working_memory_bytes" > "$output"

run_case() {
    case_name=$1
    query=$2
    iteration=1
    while [ "$iteration" -le "$repeats" ]; do
        result_file="target/compassql-output-$case_name.json"
        measurement=$(python3 scripts/measure_process.py "$result_file" -- \
            "$binary" query --cql "$query" --graph "$graph" --format json)
        metrics=$(python3 - "$result_file" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
profile = value.get("profile") or {}
print(f"{profile.get('expanded_relationships', 0)},{len(value.get('rows', []))},{profile.get('peak_memory_bytes', 0)}")
PY
)
        echo "$case_name,$iteration,$measurement,$metrics" >> "$output"
        iteration=$((iteration + 1))
    done
}

run_case cold_plan "EXPLAIN MATCH (n) RETURN n.id LIMIT 100"
run_case scan "PROFILE MATCH (n) RETURN n.id LIMIT 100"
run_case anchored "PROFILE MATCH (n {id:'__compass_missing__'}) RETURN n"
run_case one_hop "PROFILE MATCH (a)-[r]->(b) RETURN a.id, type(r), b.id LIMIT 100"
run_case bounded_path "PROFILE MATCH p=(a)-[*1..4]->(b) RETURN length(p) LIMIT 100"
run_case aggregate "PROFILE MATCH (n) RETURN count(n) AS nodes"
run_case optional "PROFILE MATCH (n) OPTIONAL MATCH (n)-[r]->(m) RETURN n.id, m.id LIMIT 100"
run_case policy_shape "PROFILE MATCH (domain {source_file:'src/domain'})-[:CALLS*1..8]->(database) RETURN domain.id, database.id LIMIT 100"

python3 - "$output" "$summary" "$baseline" <<'PY'
import csv, statistics, sys
raw, summary, baseline = sys.argv[1:]
groups = {}
with open(raw, newline="", encoding="utf-8") as stream:
    for row in csv.DictReader(stream):
        groups.setdefault(row["case"], []).append(float(row["seconds"]))
with open(summary, "w", newline="", encoding="utf-8") as stream:
    writer = csv.writer(stream)
    writer.writerow(["case", "samples", "p50_seconds", "p95_seconds"])
    for name, values in sorted(groups.items()):
        ordered = sorted(values)
        p95 = ordered[min(len(ordered) - 1, max(0, int((len(ordered) * 0.95 + 0.999999)) - 1))]
        writer.writerow([name, len(values), f"{statistics.median(values):.9f}", f"{p95:.9f}"])
if baseline:
    with open(baseline, newline="", encoding="utf-8") as stream:
        approved = {row["case"]: float(row["p50_seconds"]) for row in csv.DictReader(stream)}
    regressions = []
    for name, values in groups.items():
        if name in approved and statistics.median(values) > approved[name] * 1.10:
            regressions.append(name)
    if regressions:
        raise SystemExit("CompassQL median regression above 10%: " + ", ".join(sorted(regressions)))
PY

echo "CompassQL benchmark written to $output and $summary"
