# Trail Phase 1 performance qualification

The release gate compares the pinned Python oracle and a locked, release-mode
Trail binary on the same machine and copied corpus. Every measured cold pair is
checked for deterministic node and edge parity before warm and query results are
accepted.

Run the qualification harness from any directory:

```bash
rust/scripts/qualify_phase1.sh
```

Configuration is explicit through `TRAIL_BENCH_CORPUS`,
`TRAIL_BENCH_REPEATS`, `TRAIL_BENCH_QUERY`, `TRAIL_BENCH_OUTPUT`, and
`GRAPHIFY_PYTHON`. Raw per-run data is written to
`rust/target/phase1-qualification.csv` by default.
Each row records latency, peak RSS, indexed-file throughput, node and edge
counts, the canonical graph SHA-256, and whether the Python/Trail topology pair
matched. Cold, unchanged-warm, one-file change, rename, delete, and query cases
are measured independently on fresh corpus copies.

## Qualified local baseline

Baseline recorded 2026-07-19 on Apple M2 Max (12-core ARM64), 32 GiB RAM,
macOS 26.5, Rust 1.97.1, and Python 3.12.13. The corpus was the 216-file,
3.8 MiB `graphify/` Python package and builds used `--no-cluster` to isolate the
deterministic AST pipeline.

| Case | Python median / p95 | Trail median / p95 | Median speedup | Python peak RSS | Trail peak RSS |
|---|---:|---:|---:|---:|---:|
| Cold AST build | 2.64 / 2.64 s | 0.34 / 1.96 s | 7.8× | 180.1 MiB | 140.3 MiB |
| Warm unchanged update | 2.66 / 2.67 s | 0.04 / 0.04 s | 66.5× | 197.9 MiB | 41.3 MiB |
| One-file change | 2.66 / 2.67 s | 0.22 / 0.22 s | 12.1× | 197.7 MiB | 119.1 MiB |
| File rename | 2.66 / 2.68 s | 0.22 / 0.22 s | 12.1× | 198.0 MiB | 123.5 MiB |
| File delete | 2.66 / 2.67 s | 0.22 / 0.22 s | 12.1× | 198.5 MiB | 120.7 MiB |
| Query | 0.21 / 0.22 s | 0.02 / 0.02 s | 10.5× | 61.2 MiB | 24.7 MiB |

The first Trail process had cold executable pages and accounts for the 1.96 s
cold p95. It is retained in the sample rather than discarded; later corpus-cold
runs were 0.33–0.34 s. The harness fails unless the cold median exceeds 2×,
every warm/incremental/query median exceeds 5×, every graph pair matches the
oracle, and Trail peak RSS is no worse than Python for every case.

This baseline is local qualification evidence, not a substitute for release CI.
Before a Phase 1 release, the harness must also be run for at least five
iterations on approved small, medium, and large multilingual corpora, with
median and p95 results retained as release artifacts. The local baseline above
qualifies the complete Graphify Python package, including real incremental
change, rename, and delete operations.
