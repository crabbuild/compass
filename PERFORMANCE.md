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
The store workflow passes `--store sqlite`; ordinary builds remain JSON-only.
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
release harness verifies two-generation local retention and bounded
reachability deletion; distributed leases and service GC are separate future
qualification surfaces.

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
`957d0cee7167757ae221ffde59d2cf0a322e89c7` indexed 3,475 files and published
a 288,565,683-byte canonical graph with 82,654 nodes and 187,913 edges. The
store-qualified graph SHA-256 was
`14c8c07324355a4a383761a825d32bc1ec24154417eb1e5243012288dffc1511`.
The commands used a release binary, a fresh output root per mode, and:

```bash
compass update /Users/haipingfu/Github/django \
  --out OUTPUT --force --no-viz --no-cluster --no-program \
  --store json|sqlite --timing
```

The original merged implementation took 158.21 seconds for a fresh SQLite
build versus 13.88 seconds for JSON (11.4× slower), used about 1.05 GB for the
database, and performed roughly 16,410 durable transactions. It stored both a
legacy full-graph payload and individually written immutable objects, copied
the database through generation publication, and reconstructed the complete
graph for a store query.

The qualified implementation instead uses bounded immutable batches, no
existence pre-read per object, projected-only compressed snapshot roots, a
shared database plus generation selector, streamed canonical graph hashing,
direct projected queries, and two-generation reachability GC. Physical audit
reported 7,583 rows: 7,582 immutable objects plus one selector. Object values
totaled 115,336,290 bytes; the 16 KiB-page SQLite file was 190,087,168 bytes
(0.66× `graph.json`). `PRAGMA integrity_check` and `foreign_key_check` passed.
Point and partition-range `EXPLAIN QUERY PLAN` output both selected the
`WITHOUT ROWID` primary key.

Fresh and unchanged results on the same development runner were:

| Scenario | JSON | SQLite store | Store / JSON |
| --- | ---: | ---: | ---: |
| Fresh build, internal wall | 10.31 s | 12.82 s | 1.24× |
| Fresh build, process wall | 15.50 s | 12.87 s | 0.83× |
| Fresh build peak RSS | 2,681,487,360 B | 2,880,339,968 B | 1.07× |
| Unchanged update | 1.13 s | 1.18 s | 1.04× |

The store build published 7,582 new objects (115,336,290 value bytes) in 12
reported transactions, including activation and retention metadata. Mounted-
volume contention produced one slower outlier, so internal phase timing and
back-to-back controls are retained with the raw observations; the result is a
comparable dual publication, not a promise that every filesystem makes SQLite
faster than JSON.

All six typed query families (`search`, `callers`, `callees`, `impact`,
`explore`, and `node`) returned byte-identical JSON between engines on Django.
The search differential includes truncation at the real candidate bound and
therefore verifies candidate set, order, scores, diagnostics, nodes, and edges.
Representative warm-cache process measurements were:

| Query | JSON | SQLite store | Speedup |
| --- | ---: | ---: | ---: |
| `search django` | 11.19 s, 1,548,222,464 B RSS | 0.54 s, 41,238,528 B RSS | 20.7× |
| callers | 19.20 s | 0.01 s | 1,920× |
| impact | 10.55 s | 0.02 s | 527× |
| node trail | 10.46 s | 2.76 s | 3.8× |

The callers observation includes mounted-volume variability and should not be
treated as a stable ratio. The important release evidence is exact result
parity plus the absence of complete graph materialization on the store path.
JSON remains the default permanent engine; SQLite remains explicit with
`--store sqlite` and `--engine store`.

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

## Explicit Graphify comparison

An isolated diagnostic comparison was run on 2026-08-03 on Apple Silicon using
Graphify 0.9.32 (`00efd6e7969837ae4a9f11d8d504dcd3b20b09df`), Compass's release
binary from the current working tree, and fresh output directories for every
sample. The corpus commits were Django
`2cace96be6d64b7dde7eb66d3ffc2c7f08ef644f` and grpc-java
`e1fc64cada7a06c1b2fe7602980be051cf574378`.

The fair structural workload used Compass `extract --code-only --no-cluster --no-viz --force`
and Graphify's native code-only, no-clustering profile.
Compass now omits Program IR by default; `--program` is a separate, explicit
workload. Compass `--code-only` also excludes document extractors, while the
file inventory remains available for diagnostics.

The latest three-sample cold-build observations were:

| Repository | Compass p50 | Graphify p50 | Graphify / Compass | Compass graph |
| --- | ---: | ---: | ---: | --- |
| Django | 10.38 s | 40.74 s | 3.92× | 80,989 nodes / 187,293 edges / 286,522,634 B |
| grpc-java | 11.31 s | 39.073 s | 3.45× | 94,423 nodes / 241,093 edges / 377,825,842 B |

The grpc-java sample ran while the shared development volume was contended,
so its wall-time row is diagnostic rather than a clean qualification. Compass
still publishes a richer contract: Graphify's matching graphs were about 51,526
/ 165,454 / 78,312,513 B for Django and 34,460 / 126,990 / 77,983,036 B for
grpc-java. These observations do not yet meet the requested 5×–10× end-to-end
build target; dropping provenance, typed records, or Program output would not
be a valid comparison.

After one untimed index warmup and ten measured fresh Compass processes, the
current Django code-only graph kept deterministic result digests and met the
5× query gate:

| Query | Compass p50 | Graphify p50 | Graphify / Compass |
| --- | ---: | ---: | ---: |
| `where is URL resolution implemented` | 0.525 s | 2.675 s | 5.10× |
| `how does a model save data` | 0.550 s | 2.905 s | 5.28× |

These cross-tool observations are diagnostic evidence rather than a promoted
Compass baseline. The remaining build gap is primarily the richer graph size
and durable publication contract, not an unresolved duplicate atomic flush.

## Incremental and language-hardening observations

A follow-up release-binary smoke run on 2026-08-04 used four small,
read-only real repositories under the mounted qualification volume: Cobra
(Go, 37 files), Zod (TypeScript, 444 files), Rayon (Rust, 191 files), and
Click (Python, 91 files). The run used `extract --code-only --no-cluster
--no-viz --force --store sqlite` and measured one cold process per repository:

| Repository | Wall time | Nodes | Edges |
| --- | ---: | ---: | ---: |
| Cobra | 0.82 s | 1,204 | 5,701 |
| Zod | 1.59 s | 11,047 | 9,632 |
| Rayon | 2.39 s | 8,807 | 17,942 |
| Click | 1.47 s | 3,876 | 9,471 |

These are diagnostic observations, not promoted baselines or a claim of a
4×–5× cold-build advantage. Extraction with fewer than 32 missing files stays
sequential to avoid multiplying parser/AST working sets; the automatic path
uses a default cap of 8 local workers below 1,024 missing files and 4 workers
at or above that boundary, while `--max-workers` remains the explicit
override. Portable AST publication
also streams one compressed value at a time instead of retaining the entire
compressed batch in memory. Incremental cache hits now remain in that portable
representation and decode directly into typed extraction records when loaded
by the pipeline, so only freshly extracted files pay the source-path
normalization walk; the public cache reader continues to return its established
absolute-path representation. The bounded graph-delta candidate check walks
the already canonical node/link order without allocating full ID maps, and the
framework resolver skips target-index construction when no framework facts
exist. These are internal optimizations: the immutable snapshot validator and
the full graph publication contract remain authoritative.

The same Cobra checkout was cold-built into a fresh SQLite output and then
received a comment-only edit. The edit extracted 1 file and reused 36 cached
files; store publication created 5 objects and wrote 32,325 bytes, compared
with 192 objects and 2,072,182 bytes for the cold publication. Snapshot tests
also verify that the Edges, Incoming, Outgoing, Files, Names, Terms,
Communities, and Diagnostics roots remain immutable for a file-only change.

The language fixtures cover the remaining attribution gaps directly: Go
variadic range elements, type assertions, and nested closure parameters now
resolve to their receiver methods; Rust generic `impl` and struct bounds now
carry through direct and field receivers, resolving calls to the exact trait
method while preserving fail-closed ambiguity. Rust generic impl calls also
resolve to the exact impl owner both within one file and across an imported
module. On the
same filtered Graphify Go comparison, exact comparable edges improved from
1,469 to 1,492 and missing edges fell from 38 to 15. These quality numbers are
diagnostic and do not imply that all Graphify gaps are closed.

The extraction handoff now releases excess capacity from the AST fact working
set before project-wide resolution. Large nested JSON fact maps are rebuilt
only above a high cardinality threshold; this keeps the common per-node and
per-edge path allocation-neutral while bounding the large-map case. The
compaction is lossless: a threshold-tuned release-binary smoke run produced
the same graph SHA-256 for all four pinned repositories (`78ef5b7b` Cobra,
`c25d3b97` Click, `06014539` Rayon, and `f620afe4` Zod). The observed peak RSS
was 103, 168, 270, and 309 MiB respectively. These are single-run diagnostic
measurements, not a promoted memory baseline.

The automatic AST pool now uses the default cap of 8 workers below 1,024
missing files and 4 workers at or above that repository-size boundary. An
explicit `--max-workers` value continues to override the automatic cap. On a
fresh 3,096-file Django extraction from the pinned 2026-08-05 qualification
corpus, the adaptive path reported 11.75 seconds internally and measured
18.07 seconds with `/usr/bin/time -l`, with 2.17 GiB peak RSS and the same
80,976-node/195,164-edge graph. This is a single mounted-volume observation;
the pinned three-sample Graphify comparison remains 40.76 seconds p50, so the
universal 4× target is not established by this change.

The default SQLite publication path now uses the same bounded canonical graph
stream as JSON-only publication while the immutable snapshot indexes are built
in parallel. This removes per-record hashing overhead from the generic serde
writer without changing the atomic-write, digest, or snapshot contracts. On a
release-binary A/B smoke run over the pinned Go Cobra, Rust Rayon, and
TypeScript Zod repositories, the resulting `graph.json` bytes were byte-for-byte
identical in every comparison; observed internal cold-build wall times were
0.7 s, 1.1 s, and 1.4 s respectively for the changed path versus 0.6 s, 1.5 s,
and 1.5 s for the control. Mounted-volume contention makes these diagnostic
observations rather than a promoted baseline.

On the same 444-file Zod checkout, a release-binary smoke measurement recorded
about 332 MiB peak RSS with the previous host-sized worker pool, 289 MiB with
the new default cap of 8 workers, and 257 MiB with an explicit cap of 4. The
8-worker run retained essentially the previous wall time; the 4-worker run
was about 0.26 s slower. These are diagnostic measurements on one macOS
runner, not a cross-platform performance guarantee.

A follow-up release build from the merged `origin/main` tip was measured with
three fresh cold processes per repository on the same four pinned corpora. The
Graphify values are the authoritative three-sample baselines from the pinned
comparison run; the Compass samples used the compact JSON graph path:

| Repository | Compass p50 | Graphify p50 | Graphify / Compass | Compass RSS p50 | Graphify RSS | Compass graph (nodes / edges) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Cobra | 0.32 s | 0.718 s | 2.24× | 121.00 MiB | 64.53 MiB | 1,206 / 5,711 |
| Click | 0.54 s | 1.764 s | 3.27× | 181.34 MiB | 99.45 MiB | 3,876 / 9,471 |
| Rayon | 0.82 s | 2.025 s | 2.47× | 298.38 MiB | 80.95 MiB | 8,563 / 17,941 |
| Zod | 1.14 s | 4.330 s | 3.80× | 295.44 MiB | 234.62 MiB | 11,058 / 9,644 |

The run is deterministic across all three samples per corpus and no longer
reports a partial graph for Zod: its recovered TypeScript `implements`
relationship now targets the structural `ParseInput` type alias. These
measurements still do not establish the requested 4× speed target for every
repository or the Graphify RSS gate; the richer typed graph remains the
correctness priority while publication and resolver costs are reduced.

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
