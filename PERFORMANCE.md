# Compass performance qualification

Compass performance is measured against Compass-owned baselines. Qualification
must never trade away graph correctness, deterministic output, resource bounds,
or complete error reporting.

The reproducible real-repository harness, operator commands, correctness gates,
and optional explicit Graphify comparison are documented in
[`benchmarks/performance/README.md`](benchmarks/performance/README.md).

## Baseline policy

A benchmark records:

- Compass version and commit;
- operating system, architecture, CPU, memory, and Rust toolchain;
- corpus identity and canonical graph digest;
- cold, unchanged-warm, and incremental latency;
- peak resident memory;
- indexed files, nodes, edges, and throughput;
- query latency and result digest.

Compare a proposed change with a previously approved Compass result captured on
the same runner and corpus. A median regression above 10% requires explicit
review and evidence explaining the tradeoff.

## Compass Store release qualification

The local store release harness records the adapter's build/query timings,
peak child RSS, graph and database bytes, request/byte counters, write
amplification, immutable-object reuse, GC state, and JSON/typed-query/
CompassQL differential results. It also measures a small real CLI workflow:
clean build, unchanged update, one-file update, and cold JSON/store search.
Run it from a checkout with the required external target directory:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-store-<checkout> \
  scripts/qualify_compass_store_release.sh
```

The default graph sizes are 32, 128, and 512 generated nodes. Raw JSON and
CSV observations are written below the selected target directory and must not
be committed. Set `COMPASS_STORE_QUALIFICATION_SKIP_GATES=1` only for a local
measurement rerun; a release run also executes adapter conformance, snapshot,
query, CLI, CompassQL, product-boundary, packaging, and fixture gates. The
release harness reports GC as `discovery-only` because general retention and
deletion are not shipped in the local store.

One macOS development-run observation (Apple Silicon, Rust 1.97.1, release
binary, 2026-08-02) was:

| Adapter | Nodes | Process seconds | Peak RSS KiB | Graph bytes | Database bytes | Write amplification | Build requests (get/put) | Query requests (get/scan) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| SQLite | 32 | 0.688 | 11,520 | 33,833 | 110,592 | 3.27× | 24 / 12 | 10 / 0 |
| SQLite | 128 | 0.127 | 22,368 | 135,361 | 348,160 | 2.57× | 38 / 19 | 10 / 0 |
| SQLite | 512 | 0.379 | 45,840 | 542,017 | 1,347,584 | 2.49× | 112 / 56 | 26 / 0 |
| redb (library) | 32 | 0.118 | 11,904 | 33,833 | 561,152 | 16.59× | 24 / 12 | 10 / 0 |
| redb (library) | 128 | 0.214 | 22,512 | 135,361 | 593,920 | 4.39× | 38 / 19 | 10 / 0 |
| redb (library) | 512 | 0.531 | 46,880 | 542,017 | 2,379,776 | 4.39× | 112 / 56 | 26 / 0 |

The same sample project measured CLI process times of 6.215 s (clean
`init`), 0.090 s (unchanged update), 0.098 s (small source change), 0.025 s
(cold JSON search), and 0.028 s (cold store search), with peak RSS of 31,536,
24,496, 27,840, 14,320, and 15,072 KiB respectively. These are reproducible
diagnostic observations, not a claim that every repository or backend is
faster. The release gate remains correctness plus the existing 10% regression
policy; a performance cutover requires a comparable approved JSON baseline.

### Django store audit

A real-repository audit on Django commit
`957d0cee7167757ae221ffde59d2cf0a322e89c7` used 7,074 detected files and
published a 292,582,885-byte graph with 82,654 nodes and 187,913 edges. The
canonical graph SHA-256 was
`30897dc82a8452512b8d4be6149beeae6d0929bbb09966a8abf838b70a436595`.

The initial JSON tree encoding could not publish this graph because a valid
qualified name exceeded the portable key limit. After bounded hashed name
keys, a graph-scale item limit, and compact MessagePack tree objects were
added, the SQLite sidecar was 1,051,348,992 bytes (3.59× graph bytes), down
from 2,672,005,120 bytes with JSON tree objects. The compact reader also
successfully validated the older JSON-object store.

On the same output and release binary, cold typed search returned
byte-identical 740,534-byte JSON from both engines. Store search measured
17.371 s and 2,292,272 KiB peak RSS; JSON search measured 19.476 s and
1,758,864 KiB. Store startup was 10.8% faster but used 30.3% more peak memory.
`compass store status` completed in 7.837 s with 2,755,360 KiB peak RSS.

Fresh publication remains outside the default-cutover budget: the compact
store build completed in 206.357 s with 4,221,296 KiB peak RSS, versus the
JSON-only control at 15.357 s and 3,628,336 KiB. Two unchanged store updates
measured 4.300 and 4.561 s versus the JSON-only control at 1.833 s. The
current store engine reconstructs the complete graph before query execution,
and SQLite publishes immutable tree objects as individual durable writes.
Direct projection execution, batched durable publication, and bounded
generation retention remain required before claiming that Compass Store
improves build performance or before treating it as an unconditional
large-repository default.

## CompassQL qualification

CompassQL measures compile/plan latency, indexed fixed matches, one-hop and
bounded-path expansion, aggregation, optional matching, cached-plan lookup,
cancellation latency, expanded relationships, returned rows, and peak RSS.

Run:

```bash
scripts/benchmark_compassql.sh [GRAPH_JSON]
```

The default graph is `compass-out/graph.json`. Raw observations belong under
`target/compassql-benchmark.csv`.

The release gate rejects:

- a cached-plan or query median regression above 10%;
- a working-memory budget violation;
- any successful partial result after cancellation or a hard limit;
- a cancellation checkpoint delay above 100 ms.

Local observations are diagnostic evidence. Promote a baseline only from a
controlled Compass CI run with retained artifacts.

## Django cold-build regression qualification

The 0.2.1 code-graph performance hardening was qualified against Django commit
`5e32c82a5a896e1d942cfc9dd9a2ebbe86741258`. Three isolated cold builds, each
with a fresh output directory and no reusable cache entries, completed in
9.31 seconds with internal phase profiling enabled and 9.04 and 9.06 seconds
without profiling on the development runner. Every run published 3,105 files,
80,961 nodes, 187,173 edges, 2,583 communities, 3,038 Program modules, and
32,049 Program summaries. The canonical graph and Program SHA-256 digests were
identical across all three runs.

After rebasing the hardening onto Compass revision `347d9b1`, the default scan
contract included the newly shipped deterministic HTML and Markdown adapters.
On the local Django checkout at `957d0cee7167757ae221ffde59d2cf0a322e89c7`,
that expanded the cold workload to 3,475 files, 82,654 nodes, 187,913 edges,
and 3,379 communities. Three profiled fresh-output builds reported 10.39,
10.60, and 10.37 seconds internally; after the first copied-binary launch,
external wall times were 10.65 and 10.42 seconds. Two additional unprofiled
runs completed in 10.55 and 10.67 seconds externally. All five runs produced
the same graph and Program SHA-256 digests. The sub-10-second claim above
therefore applies to the pinned 3,105-file corpus; the expanded 3,475-file
default scan is a separate baseline rather than an equivalent workload.

The qualification includes durable artifact publication. Storage throughput
must be recorded separately when the output resides on a mounted volume: the
same 277 MB graph can add several seconds when the volume is saturated, even
when extraction and graph construction remain below their approved baseline.
These local measurements are diagnostic evidence and do not replace a
controlled CI baseline.

## Versioned history qualification

Build a release binary, then measure a clean real repository:

```bash
cargo build --release -p compass-cli
scripts/qualify_history_real_repo.sh /path/to/repository OLD NEW
```

The qualification records cold/current extraction, cold/adjacent/no-op history
builds, first/repeated semantic diff, first/repeated viewer projection, peak
RSS, and deterministic output digests. Existing sealed realizations and cached
diff/view projections are expected to be constant- or bounded-read paths;
explicit `history verify` remains the full integrity scan.
