# Compass Phase 1 performance qualification

The release gate compares the pinned Python oracle and a locked, release-mode
Compass binary on the same machine and copied corpus. Every measured cold pair is
checked for deterministic node and edge parity before warm and query results are
accepted.

Run the qualification harness from any directory:

```bash
rust/scripts/qualify_phase1.sh
```

The release-grade corpus matrix is equally explicit:

```bash
rust/scripts/qualify_phase1_matrix.sh
```

It prepares reproducible small (`tests/fixtures`), medium (`graphify` plus the
multilingual fixtures), and large (`graphify`, tests, native crates, and docs)
corpora, runs every gate for at least five iterations, and retains one CSV per
tier. Weekly hardening CI runs this matrix and uploads the raw evidence.

Configuration is explicit through `COMPASS_BENCH_CORPUS`,
`COMPASS_BENCH_REPEATS`, `COMPASS_BENCH_QUERY`, `COMPASS_BENCH_OUTPUT`, and
`GRAPHIFY_PYTHON`. Set `COMPASS_BENCH_BASELINE` to a previously approved raw CSV,
or `COMPASS_BENCH_BASELINE_DIR` for the corpus matrix. In that mode, any Compass
median more than 10% slower than the frozen baseline fails qualification. Raw
per-run data is written to
`target/phase1-qualification.csv` by default.
Timing uses a monotonic high-resolution clock around the child process; peak
RSS comes from the operating system's child-resource counters. Each row records
latency, peak RSS, indexed-file throughput, node and edge
counts, the canonical graph SHA-256, and whether the Python/Compass topology pair
matched. Cold, unchanged-warm, one-file change, rename, delete, query, path,
explain, and affected cases are measured independently on fresh corpus copies.
Query qualification requests the complete untruncated result and compares its
exact header and line multiset, avoiding Python hash-set tie-order instability.

## Qualified local baseline

Baseline recorded 2026-07-20 on Apple M2 Max (12-core ARM64), 32 GiB RAM,
macOS 26.5, Rust 1.97.1, and Python 3.12.13. Every tier used five independent
corpus copies and `--no-cluster` to isolate deterministic AST behavior. The
first binary launch remains in the sample; no warm-up observation is discarded.

| Tier | Files | Nodes / edges | Cold median speedup | Python / Compass cold peak RSS | Canonical SHA-256 |
|---|---:|---:|---:|---:|---|
| Small | 106 | 711 / 878 | 2.8× | 53.7 / 49.2 MiB | `6be92324a1dc5f5a...` |
| Medium | 322 | 3,668 / 7,079 | 5.1× | 184.0 / 156.1 MiB | `5fe36acc17ec0a83...` |
| Large | 850 | 15,151 / 38,374 | 2.85× | 351.3 / 291.2 MiB | `c25817b1c1acc685...` |

The large tier is the binding performance and memory qualification. Its full
results were:

| Case | Python median / p95 | Compass median / p95 | Median speedup | Python peak RSS | Compass peak RSS |
|---|---:|---:|---:|---:|---:|
| Cold AST build | 10.766 / 11.121 s | 3.774 / 5.856 s | 2.85× | 351.3 MiB | 291.2 MiB |
| Warm unchanged update | 12.180 / 12.575 s | 0.232 / 0.236 s | 52.5× | 477.2 MiB | 141.9 MiB |
| One-file change | 12.155 / 12.720 s | 2.112 / 2.193 s | 5.8× | 476.5 MiB | 310.6 MiB |
| File rename | 12.142 / 12.559 s | 2.141 / 2.213 s | 5.7× | 477.1 MiB | 309.2 MiB |
| File delete | 12.134 / 12.708 s | 2.137 / 2.266 s | 5.7× | 476.6 MiB | 310.4 MiB |
| Query | 0.746 / 0.801 s | 0.132 / 0.137 s | 5.7× | 173.3 MiB | 104.6 MiB |
| Path | 0.655 / 0.701 s | 0.107 / 0.112 s | 6.1× | 174.7 MiB | 80.4 MiB |
| Explain | 0.609 / 0.658 s | 0.087 / 0.091 s | 7.0× | 173.0 MiB | 80.4 MiB |
| Affected | 0.250 / 0.265 s | 0.034 / 0.037 s | 7.4× | 158.9 MiB | 52.2 MiB |

Every graph pair and every read output matched the Python oracle. The harness
fails unless cold median speedup reaches 2×, every warm, incremental, and read
median reaches 5×, and Compass's worst observed RSS is no greater than Python's
for each case. The single retained parser is deliberate: it preserves a safe
60 MiB cold-memory margin on the large multilingual corpus while exceeding the
required cold latency target.

This local baseline is evidence, not a substitute for release CI. The hardening
workflow repeats the small, medium, and large matrix, freezes the first approved
runner result, and rejects a later Compass median regression greater than 10%.
Release workflows require that frozen baseline before publishing.

## CompassQL qualification

CompassQL measures compile/plan latency, indexed fixed matches, one-hop and
bounded-path expansion, aggregation, optional matching, cached-plan lookup,
cancellation latency, expanded relationships, returned rows, and peak RSS.
Run `scripts/benchmark_compassql.sh [GRAPH_JSON]`; the default graph is
`graphify-out/graph.json`.

The release gate rejects a cached-plan or query median regression above 10%, a
working-memory budget violation, any partial result after cancellation/limit,
or a cancellation checkpoint delay above 100 ms. Raw observations belong under
`target/compassql-benchmark.csv`; no local observation is promoted to an
approved baseline without the cross-platform release workflow artifact.
