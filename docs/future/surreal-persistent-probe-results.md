# SurrealDB 3.2.4 persistent-engine probe results

Status: **PASS — semantic/recovery gate only**
Run date: 2026-08-28
Host: Apple arm64, macOS 26.7 (25G224), Darwin 25.6.0
Toolchain: Rust 1.97.1 (`8bab26f4f`), Cargo 1.97.1

This is evaluation evidence, not a shipped Compass capability or performance
claim. It does not satisfy the separate BSL sign-off in
[`surrealdb-license-decision.md`](surrealdb-license-decision.md), and its
measurements are not evaluated against numeric budgets until C-013 ratifies
those budgets.

## Reproducible inputs

The retained fixture set is
[`scripts/fixtures/surreal-persistent-probes/`](../../scripts/fixtures/surreal-persistent-probes/).
`manifest-v1.json` pins the SHA-256 of every input, expected result, engine
result, and the exact tagged SurrealDB license bytes. The workload contains:

- three stable logical nodes;
- two same-direction parallel relations with distinct stable IDs;
- one reverse relation;
- exact source path, line, column, and finite-decimal confidence evidence;
- immutable `g-0001` and `g-0002` generations plus one active pointer; and
- 2,048 generated relations, for 2,051 total relations across 17 pages of 128.

`input-v1.json` retains the complete scale-edge expansion rule: zero-padded
IDs, alternating endpoints, fixed confidence, and deterministic source path,
line, and column formulas. `expected-v1.json` independently pins both the
newline-joined ordered-ID digest and the canonical expanded-relation digest.

The disposable runner was a standalone Cargo package under `/tmp`, pinned
`surrealdb = "=3.2.4"`, and compiled once with only `kv-surrealkv` and once with
only `kv-rocksdb`. Each engine used a separate empty release target. The
runner and every target/database directory were removed after the retained
results were verified.

## Semantic and recovery results

| Dimension | SurrealKV | RocksDB | Required result |
| --- | --- | --- | --- |
| Stable logical relation IDs | PASS | PASS | All 2,051 IDs round-trip exactly |
| Parallel directed multiplicity | PASS | PASS | Two distinct Alpha → Beta relations |
| Reverse direction | PASS | PASS | Beta → Alpha remains distinct |
| Provenance and confidence | PASS | PASS | Paths, coordinates, and six-decimal strings exact |
| Candidate isolation | PASS | PASS | Writing `g-0002` does not change active `g-0001` |
| Atomic activation | PASS | PASS | Final pointer switch selects `g-0002` |
| Kill during candidate write | PASS | PASS | Child killed after one durable candidate batch and before activation |
| Reopen after kill | PASS | PASS | Retained post-crash count/digest prove `g-0001` has all 2,051 records |
| Explicit ordering | PASS | PASS | Strict `stable_id` order, no duplicate/missing pages |
| Pagination | PASS | PASS | 17 pages concatenate to 2,051 records |
| Ordered-ID SHA-256 | `8d95d397…160d3` | `8d95d397…160d3` | Engine outputs equal |

The retained `.semantic` objects for the two engines are byte-identical after
canonical JSON sorting. Candidate rows written before the forced termination
may remain as unreachable orphan data; the probe's acceptance property is that
no incomplete candidate becomes visible through the active-generation read.

## Descriptive resource measurements

| Measurement | SurrealKV | RocksDB |
| --- | ---: | ---: |
| Selected normal dependency-tree lines | 418 | 411 |
| Clean release build wall time | 507.55 s | 289.65 s |
| Clean build maximum RSS | 405,012,480 B | 466,075,648 B |
| Release executable | 62,247,392 B | 70,950,208 B |
| Full semantic/recovery workload wall time | 2.73 s | 2.59 s |
| Workload maximum RSS | 54,575,104 B | 155,287,552 B |
| Median internal cold start (5 new stores) | 93,375 µs | 48,863 µs |
| Median cold-start maximum RSS | 35,012,608 B | 120,963,072 B |

Both release targets were empty, but registry sources and host caches were
already present from feature checks. The builds ran sequentially on one host;
the values are reproducible baselines for this environment, not universal or
statistically controlled comparisons.

## License capture

`SURREALDB-3.2.4-LICENSE.txt` is byte-identical to the official `v3.2.4`
tagged license and has SHA-256
`98a94ac615f88370865016487b436fa404560910bd329794ed7502277a94b805`.
It records BSL 1.1, the Database Service restriction, Change Date 2030-01-01,
and Apache License 2.0 as the Change License. License acceptance remains C-011,
not an inference from successful engine behavior.

## Disposition

C-012 passes: both required persistent engines completed every semantic,
ordering, pagination, generation, dirty-shutdown, measurement, and license
capture dimension. At C-012 completion on 2026-08-28, C-014/C-015 remained
blocked until C-011 recorded explicit user/legal approval and C-013 ratified
measurement budgets. If either gate rejected the work, Compass retained its
current SQLite/redb/`graph.json` stack. Later gate outcomes belong to their own
dated decision records, not this prerequisite probe record.

No SurrealDB dependency is present in Compass `Cargo.toml` files or
`Cargo.lock`; the probe does not alter default builds or published behavior.
The pre-probe and post-disposal checksum ledgers were byte-identical. Each
ledger contains 33 sorted Compass `Cargo.toml` SHA-256 records followed by the
`Cargo.lock` SHA-256 record; the ledger SHA-256 is
`a98ca39857724b88a12a306b6647299ff93fed69f4c881bee54fddc230b45e6c`.
