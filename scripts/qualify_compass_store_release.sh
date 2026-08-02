#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
compass_root=$(cd "$script_dir/.." && pwd)
target_dir=${CARGO_TARGET_DIR:-/Volumes/Workspace/crabbuild-target/compass-store-phase9}
output_dir=${COMPASS_STORE_QUALIFICATION_OUTPUT:-$target_dir/compass-store-release-qualification-$(date -u +%Y%m%dT%H%M%SZ)}
sizes=${COMPASS_STORE_QUALIFICATION_SIZES:-"32 128 512"}
measure="$script_dir/measure_process.py"

usage() {
  echo "usage: $0 [--output DIR] [--sizes 'N N ...']" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      [[ $# -ge 2 ]] || usage
      output_dir=$2
      shift 2
      ;;
    --sizes)
      [[ $# -ge 2 ]] || usage
      sizes=$2
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done

if [[ ! -d /Volumes/Workspace || ! -w /Volumes/Workspace ]]; then
  echo "error: /Volumes/Workspace must be mounted and writable" >&2
  exit 1
fi
mkdir -p "$target_dir"
if [[ ! -w "$target_dir" ]]; then
  echo "error: target directory is not writable: $target_dir" >&2
  exit 1
fi
if [[ -e "$output_dir" ]]; then
  echo "error: qualification output already exists; choose a new --output directory: $output_dir" >&2
  exit 1
fi
mkdir -p "$output_dir"

export CARGO_TARGET_DIR="$target_dir"
cargo build --release --locked -p compass-cli --bin compass
cargo build --release --locked -p compass-store-qualification --bin compass-store-qualification
compass_bin="$target_dir/release/compass"
qualification_bin="$target_dir/release/compass-store-qualification"

if [[ ${COMPASS_STORE_QUALIFICATION_SKIP_GATES:-0} != 1 ]]; then
  cargo test -p compass-store --features test-support --locked
  cargo test -p compass-store-redb --locked
  cargo test -p compass-graph --test store_snapshot --locked
  cargo test -p compass-query --test store_engine --locked
  cargo test -p compass-query --test opencypher_tck --locked
  cargo test -p compass-cli --test store_cli --locked
  cargo test -p compass-cli --test compass_product --locked
  python3 "$compass_root/scripts/check_compassql_support.py"
  sh "$compass_root/scripts/check_product_boundary.sh"
  sh "$compass_root/scripts/test_release_scripts.sh"
  "$compass_root/scripts/qualify_code_graph_v1.sh" --fixtures-only
fi

mkdir -p "$output_dir/store" "$output_dir/cli"
printf 'adapter,nodes,seconds,peak_rss_kib,graph_bytes,database_bytes,write_amplification,build_requests,query_requests,canonical_json_equal,compassql_equal,gc_executed,gc_supported\n' \
  > "$output_dir/store/metrics.csv"

for adapter in sqlite redb; do
  for nodes in $sizes; do
    if ! [[ "$nodes" =~ ^[1-9][0-9]*$ ]]; then
      echo "error: invalid graph size: $nodes" >&2
      exit 2
    fi
    raw="$output_dir/store/${adapter}-${nodes}.json"
    metrics=$(python3 "$measure" "$raw" -- \
      "$qualification_bin" --adapter "$adapter" --nodes "$nodes")
    IFS=, read -r seconds peak_rss_kib <<<"$metrics"
    python3 - "$raw" "$adapter" "$nodes" "$seconds" "$peak_rss_kib" \
      "$output_dir/store/metrics.csv" <<'PY'
import csv
import json
import sys

raw, adapter, nodes, seconds, peak, csv_path = sys.argv[1:]
with open(raw, encoding="utf-8") as stream:
    report = json.load(stream)
with open(csv_path, "a", newline="", encoding="utf-8") as stream:
    csv.writer(stream).writerow([
        adapter,
        int(nodes),
        seconds,
        int(peak),
        report["graphBytes"],
        report["databaseBytes"],
        report["writeAmplification"],
        json.dumps(report["buildRequests"], sort_keys=True, separators=(",", ":")),
        json.dumps(report["queryRequests"], sort_keys=True, separators=(",", ":")),
        report["canonicalJsonEqual"],
        report["compassqlEqual"],
        report["gc"]["executed"],
        report["gc"]["supported"],
    ])
if not report["canonicalJsonEqual"] or not report["compassqlEqual"]:
    raise SystemExit(f"{adapter}/{nodes}: differential qualification failed")
PY
  done
done

work_root=$(mktemp -d "${TMPDIR:-/tmp}/compass-store-release.XXXXXX")
cleanup() {
  rm -rf -- "$work_root"
}
trap cleanup EXIT HUP INT TERM
project="$work_root/project"
mkdir -p "$project/src"
printf 'pub fn symbol_zero() -> usize { 0 }\n' > "$project/src/lib.rs"
printf 'pub fn symbol_one() -> usize { super::symbol_zero() + 1 }\n' > "$project/src/other.rs"

measure_cli() {
  local name=$1
  shift
  local raw="$output_dir/cli/${name}.stdout"
  local metrics="$output_dir/cli/${name}.metrics"
  python3 "$measure" "$raw" -- "$@" > "$metrics"
}

measure_cli clean_build "$compass_bin" init "$project" --yes
measure_cli no_change_build "$compass_bin" update "$project" --no-program --no-viz
printf '\npub fn symbol_two() -> usize { 2 }\n' >> "$project/src/lib.rs"
measure_cli small_change_build "$compass_bin" update "$project" --no-program --no-viz
graph="$project/compass-out/graph.json"
measure_cli cold_query_json "$compass_bin" search symbol --graph "$graph" --engine json --format json
measure_cli cold_query_store "$compass_bin" search symbol --graph "$graph" --engine store --format json

python3 - "$output_dir" "$compass_root" "$compass_bin" <<'PY'
import csv
import json
import pathlib
import subprocess
import sys

output, root, binary = map(pathlib.Path, sys.argv[1:])
metrics = output / "store" / "metrics.csv"
rows = list(csv.DictReader(metrics.open(encoding="utf-8")))
for row in rows:
    assert row["canonical_json_equal"] == "True", row
    assert row["compassql_equal"] == "True", row
    assert row["gc_executed"] == "False", row
    assert row["gc_supported"] == "False", row

reports = {}
for path in (output / "store").glob("*.json"):
    report = json.loads(path.read_text(encoding="utf-8"))
    reports.setdefault(report["nodes"], {})[report["adapter"]] = report
for nodes, engines in reports.items():
    assert set(engines) == {"sqlite", "redb"}, (nodes, engines)
    assert engines["sqlite"]["graphDigest"] == engines["redb"]["graphDigest"], nodes
    assert engines["sqlite"]["snapshotId"] == engines["redb"]["snapshotId"], nodes
    assert engines["sqlite"]["manifestDigest"] == engines["redb"]["manifestDigest"], nodes

cli = output / "cli"
for name in ["clean_build", "no_change_build", "small_change_build", "cold_query_json", "cold_query_store"]:
    metric = (cli / f"{name}.metrics").read_text(encoding="utf-8").strip()
    seconds, peak = metric.split(",")
    assert float(seconds) >= 0, name
    assert int(peak) > 0, name
assert (cli / "cold_query_json.stdout").read_bytes()
assert (cli / "cold_query_store.stdout").read_bytes()

commit = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
summary = {
    "schema": "compass.store.release-summary/1",
    "commit": commit,
    "compassBinary": str(binary),
    "storeMetrics": str(metrics),
    "cliMetrics": str(cli),
    "reports": sorted(str(path.relative_to(output)) for path in (output / "store").glob("*.json")),
    "graphDigests": {
        str(nodes): engines["sqlite"]["graphDigest"]
        for nodes, engines in sorted(reports.items())
    },
}
(output / "release-summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
with (output / "release-summary.md").open("w", encoding="utf-8") as stream:
    stream.write("# Compass Store release qualification\n\n")
    stream.write(f"Commit: `{commit}`\n\n")
    stream.write("Store adapters produced byte-identical graph digests, snapshot IDs, and manifest digests for every requested size. Both typed search and CompassQL differential checks passed. Raw observations are in `store/metrics.csv`; CLI build/query observations are in `cli/*.metrics`.\n")
PY

echo "Compass Store release qualification written to $output_dir"
