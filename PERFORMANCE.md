# Compass performance qualification

Compass performance is measured against Compass-owned baselines. Qualification
must never trade away graph correctness, deterministic output, resource bounds,
or complete error reporting.

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
