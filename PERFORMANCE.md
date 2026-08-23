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

## Incremental code-graph qualification

Fact-neutral updates may bypass project-wide resolution only after the changed
files reproduce the prior normalized extraction-fact digests. The publisher
then refreshes inventory and full-file envelopes, validates the complete graph,
proves that node IDs, relationships, file keys, names, search terms, and
communities are unchanged, and point-updates only graph metadata and changed
node values. Any failed proof uses the complete graph publisher. Immutable-store
garbage collection remains bounded but is amortized across eight manifests so
ordinary one-file edits do not perform a full mark-and-sweep.

The 2026-08-15 release-build qualification used copied, read-only-derived
corpora from real repositories with SQLite storage, maximum inference, and
clustering/visualization disabled. Each observation appended or removed one
language comment and reported zero graph-assembly time:

| Language / corpus | Indexed files | Nodes / edges | Observed incremental wall | Extracted / cached |
| --- | ---: | ---: | ---: | ---: |
| Kotlin / Spring Framework Kotlin corpus | 388 | 10,708 / 15,650 | 1.01 s | 1 / 387 |
| Rust / ripgrep | 142 | 12,185 / 32,040 | 2.50 s | 1 / 141 |
| Go / go-git | 709 | 21,578 / 67,522 | 3.31 s | 1 / 708 |
| TypeScript / NestJS | 2,017 | 75,298 / 116,515 | 10.33 s | 25 / 1,992 |
| Python / FastAPI production package | 56 | 1,707 / 6,622 | 1.17 s | 1 / 55 |
| C# / ASP.NET Core Http.Extensions source | 35 | 1,130 / 1,291 | 0.61 s | 1 / 34 |

These are single observations, not medians. NestJS still exposes a separate
large-repository cost: 24 successful empty/unsupported inputs are not portable
AST cache entries and are rechecked with the edited file. The fact-neutral
proof nevertheless avoids resolution and complete index reconstruction.

Real-repository natural-query qualification materializes one SQLite-backed
query artifact per repository. Fresh latency/RSS is one direct `compass query`
process per observation; warm latency is measured inside one persistent MCP
session after an unmeasured iteration. The harness requires exact artifact
identity across compared runs, checks all seven discovery work counters, and
requires the complete eight-corpus run to include at least one query on a graph
of 50,000 or more nodes that inspects no
more than 25% of the graph's nodes during candidate recall. Current results
must carry the Rust-owned semantic digest. An explicitly enabled legacy
baseline may retain a labeled full-payload harness digest for timing reference,
but its quality failures remain visible and it cannot be promoted or used as a
current candidate.

## Query-relevance qualification

The native query-relevance gate keeps three intentionally separate evidence
sets: the checked-in 80-question reviewed synthetic corpus validates fixture,
schema, scoring, and metric behavior; a 500-question AI-reviewed synthetic
matrix executes all eight query classes on a digest-pinned graph; and a
23-question, production-shaped subset runs actual
`CodeQueryEngine::query_natural_profiled` requests for search, callers,
callees, impact, directed-path, and no-answer cases against the support graph.
The 500 records are approved synthetic equivalence cases, not production
telemetry or independent human judgments. The smaller executable subset
therefore qualifies the planner and typed operation together. It derives its
canonical graph digest, records measured latency and serialized response bytes,
and requires matching ordered observations from JSON, store, and a repeated
store execution. Timing is measured but removed before the deterministic
response-baseline comparison.

Run the gate with this checkout's external Cargo target:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-<checkout> \
  python3 scripts/qualify_query_relevance.py
```

The gate fails on generated-artifact, corpus/schema/digest drift,
non-deterministic backend output, or a reviewed minimum metric miss. The
500-query thresholds require perfect Success@1, Recall@5, intent macro-F1,
slot exact match, edge/direction recall, path acceptance, and no-answer
behavior for the deliberately small executable graph. It reports MRR, Recall@k, nDCG,
intent macro-F1, edge-kind/direction precision and recall, backend parity,
latency percentiles, and versioned query work. The profile records intent,
recall, ranking, execution, and total microseconds. Its logical work counters
measure candidate records observed before deduplication, term candidates
materialized by bounded posting lookup, nodes and edges examined by traversal
or relationship probes, and exact serialized response bytes. These counters
come from the query engine rather than being inferred from result size.

Single-edit fuzzy recall is capped at 192 variants per eligible term and 256
variants per query. An immutable per-engine LRU retains at most 512 fuzzy
name-lookup results, keyed by variant and requested result limit. This prevents
large paraphrase matrices from repeating empty SQLite probes while preserving
the same candidate and truncation contract. Qualification also checks every
observation against candidate, node, edge, and response-byte work limits.

Refresh a judged corpus, executable request, digest expectation, or threshold
only with a reviewed intent/identity change and updated deterministic evidence;
never regenerate expected judgments or thresholds from a proposed ranker.

The typed search path has hard-cut to `query-ranker/2`; no environment switch
can restore v1. The gate includes a reviewed production-versus-generated
exact-name ambiguity for which frozen test-only v1 loses at rank one and v2
wins, while all executable questions retain perfect Success@1. To expand the
baseline with real production vocabulary, run
`scripts/prepare_query_relevance_review.py` on an approved local JSONL export,
then follow the two-reviewer, graph-digest-pinned process in the relevance
fixture README. Importer output is a review queue, never a generated truth set.

Search work is bounded independently from response size: `maxCandidates`
limits the total multi-source recall pool and is never expanded to
`maxNodes`. The default ranks at most 20 recalled candidates. The existing
100,000-node in-process ceiling now covers direct search and natural-query
planning as well as callers, impact, and node trails.

### Shadow text-ranker qualification

The generic text-traversal ranker has an opt-in
`text-ranker/bm25-v1` qualification profile. It is not selected by the CLI or
MCP defaults. Run its controlled 100,000-node comparison with:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-<checkout> \
  scripts/qualify_text_ranker_bm25.sh
```

The script builds a release-mode qualification executable and runs each
profile in a separate process so OS-reported peak RSS is comparable. Each run
checks the exact top node ID, records the first query, then records 31 warm
samples. The synthetic question has one rare matching symbol; it is useful for
measuring indexed lookup overhead but does not represent common-term posting
scans or a real-repository relevance distribution.

One macOS Apple Silicon development run on 2026-08-08 produced:

| Profile | First query | Warm p50 | Warm p95 | Maximum RSS | Top ID |
| --- | ---: | ---: | ---: | ---: | --- |
| `text-ranker/full-scan-v1` | 158.868 ms | 155.983 ms | 157.671 ms | 281,444,352 B | expected |
| `text-ranker/bm25-v1` | 366.502 ms | 0.005 ms | 0.010 ms | 322,453,504 B | expected |

The BM25 first query includes lazy construction of 202,008 terms for 100,000
documents. On this run it was about 131% slower on first use and maximum RSS
was about 14.6% higher, while warm rare-term lookup was substantially faster.
The very small warm measurements are timer-sensitive and should not be
generalized into a universal speedup.

The compact reviewed language goldens currently give both profiles 1.0 for
Success@1, MRR, Recall@5/20, and nDCG on four answerable Python/Rust/TypeScript/
Unicode questions, plus 1.0 no-answer precision on one negative question. That
small equal-score result demonstrates no regression on those cases; it does
not demonstrate the required 10% relative Recall@5 improvement. Because the
cold-query and memory observations also exceed the 10% review threshold, this
evidence does not support making BM25 the default. It remains a shadow profile
pending real-repository judgments and a decision about amortization or a more
compact/persistent index.

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

### Delta-rs low-inference diagnostic

A 2026-08-10 follow-up compared the new `low` inference profile with Graphify
0.9.37 on pinned delta-rs, including symmetric clustered and no-cluster build
profiles, cold/warm/incremental timing and RSS, graph integrity and provenance,
source-verified natural-query recall, exact identifiers, and adversarial
no-answer behavior. Compass won clustered cold and warm build time and
published a fully source-backed graph, but still lost query latency, strict
question pass rate, adversarial no-answer specificity, and cold/incremental
memory. The complete methodology, results, limitations, and prioritized gates
are in [Low-inference evaluation against Graphify](docs/implementation/low-inference-graphify-evaluation.md).

A 2026-08-11 resolver-admission follow-up preserved the ordered low graph while
moving rejected inference before graph-record allocation and compacting exact
duplicate Rust test candidates. Correcting streaming AST cache publication to
encode one entry at a time then produced three fresh exact-equivalent samples
with a 2.37 s wall-time median and 673.8 MiB peak-RSS median. These are
diagnostic development samples, not a promoted baseline: Compass still used
about 4.50x the documented Graphify cold RSS, so the memory gate remains
failed. The linked evaluation records the intermediate samples and explicit
CPU/memory tradeoffs.

Replacing the resolver's duplicate owned-ID hash keys with sorted fact tables
then produced a final three-sample median of 2.44 s and 610.7 MiB peak RSS,
again with exact ordered graph equivalence. Relative to the preceding cache-
only median, RSS decreased 9.4% while total time increased about 3.0%; the
universal-resolution stage itself rose from roughly 0.29 s to 0.40 s because
borrowed ID lookup is now binary search. Compass still used about 4.08x the
documented Graphify cold RSS.

Compacting relationship candidates into a resolver-private interned table as
the validated per-file batches are drained produced a subsequent three-sample
median of 2.63 s and 598.9 MiB. Ordered nodes and relationships remained byte-
equivalent in every sample. The 1.9% RSS reduction came with a 7.8% wall-time
increase and still used about 4.00x Graphify's documented cold RSS. This result
is retained as an honest intermediate bound: eliminating the remaining gap
requires compact or streamed evidence before corpus-wide per-file candidate
and occurrence objects coexist, not additional late filtering.

Moving the 88,447 validated occurrences into a resolver-private slot-backed
string table produced a further three-sample median of 2.65 s and 592.1 MiB.
The canonical 34,117,849-byte graph remained byte-identical to the preceding
artifact. Relative to the candidate-compaction median, RSS decreased 1.14%
and wall time increased 0.8%; Compass still used about 3.96x Graphify's
documented cold RSS. The compact table drops language, owner, and scope only
after evidence validation because no resolution stage reads those fields; the
public evidence and cache schema are unchanged. This small result reinforces
that resolver-local compaction is useful but cannot remove the earlier
producer/cache high-water mark or the later index/graph overlap.

Disabling mimalloc's process-wide reserved arena for one-shot builds then
produced three exact-equivalent clustered samples at `4.26`, `4.10`, and
`4.06` seconds, with peak RSS of `500,957,184`, `494,026,752`, and
`498,925,568` bytes. The median is 4.10 seconds and 475.8 MiB: 19.6% less RSS
than the preceding occurrence-table median, but 54.7% more wall time. Compass
still used about 3.18x the documented Graphify cold RSS, although it remained
1.79x faster than the retained 7.337-second Graphify cold median. All three
Compass graphs were the same canonical 34,117,849 bytes with SHA-256
`77c49b1f6bf4b6280898d005c1ffc53107feeef3e073dd1883025788e047722d`.
This improves the operating-system high-water mark without changing graph
semantics, but does not replace the required compact or streamed evidence
producer work.

Releasing the universal resolver's secondary name, member, hierarchy, and
language lookup indexes after every resolution decision is fixed, together
with consuming rather than cloning the typed/compatibility graph projection,
then produced fresh samples of `3.87`, `3.94`, and `3.96` seconds. Maximum RSS
was `465,977,344`, `466,878,464`, and `469,958,656` bytes, for medians of 3.94
seconds and 445.3 MiB. That is 3.9% faster and 6.4% less RSS than the preceding
no-arena median. Every graph remained exactly 34,117,849 bytes with SHA-256
`77c49b1f6bf4b6280898d005c1ffc53107feeef3e073dd1883025788e047722d`.
Compass therefore remains about 2.98x the retained 149.47 MiB Graphify cold
RSS; this closes an avoidable lifetime overlap but does not close the producer
evidence or graph-record representation gap.

Replaying the final admission binary established a new clean-head reference:
one clustered delta-rs low build completed in 4.12 seconds with 461,357,056
bytes (440.0 MiB) maximum RSS. Its 9,982 nodes, 25,206 relationships, 532
communities, and 1,212 structural test roles produced a 34,116,625-byte graph
with SHA-256
`971d588275cbad097ed1f7b5f54e32b86a80fadd8875760e3848b9948069f573`.
The earlier `77c49...` artifacts above remain evidence for their recorded
revisions but are not the semantic reference for changes after final resolver
stage admission.

Dropping resolver-only candidate and occurrence lookup IDs once each edge's
typed relation, occurrence rule, exact anchor, endpoints, and provenance were
materialized then produced three fresh samples at 4.30, 4.36, and 4.06
seconds, with maximum RSS of 455,966,720, 455,131,136, and 462,143,488 bytes.
The medians are 4.30 seconds and 434.8 MiB. All three graphs were byte-identical
to the clean-head reference above, including the test-role count. Relative to
the single clean-head replay this is a diagnostic 1.2% RSS reduction, not a
controlled baseline replacement; Compass still used about 2.91x Graphify's
retained 149.47 MiB cold RSS. The result removes dead transient identity
strings without weakening multiplicity or provenance, but confirms that the
remaining gap requires a compact edge representation rather than additional
attribute pruning.

Reusing the build's existing bounded Rayon pool for AST extraction, rather
than creating and destroying a second pool before resolution, then produced
fresh SQLite samples at 4.49, 4.56, and 4.70 seconds. Maximum RSS was
430,489,600, 431,833,088, and 431,194,112 bytes, for medians of 4.56 seconds
and 411.2 MiB. Every graph remained exactly 34,116,625 bytes with SHA-256
`971d588275cbad097ed1f7b5f54e32b86a80fadd8875760e3848b9948069f573`.
The worker policy still bounds mid-sized extraction at eight deterministic
chunks and honors `--max-workers`; the chunks now execute on the long-lived
pipeline pool so their allocator pages can be reused by resolution. Relative
to the preceding median this reduced RSS by 5.4% while increasing wall time by
6.0%. Compass still used about 2.75x Graphify's retained 149.47 MiB cold RSS,
so compact or streamed producer evidence remains the required next step.

A bounded streaming multiplicity audit of that same canonical graph found
25,206 unique edge IDs, 23,209 semantic `(source, relation, target)` pairs, and
1,076 pairs with parallel occurrences. Those repeated pairs represent 3,073
edges at distinct source sites; collapsing each pair to one edge would discard
1,997 real occurrences. The audit found zero duplicate edge IDs, zero duplicate
pair/site records, and zero missing relationship sites. Calls account for 533
parallel pairs, tests for 295, references for 173, and the remaining 75 span
instantiation, reads, exports, returns, and type relationships.

The relationship array serialized to 22,808,206 bytes, 66.9% of the complete
34,116,625-byte graph, with a mean of 904.9 bytes per relationship. This proves
both sides of the serialization gap: relationship representation is the main
artifact cost, but semantic-pair deduplication is not a valid remedy. Further
compaction must preserve every distinct site, direction, occurrence rule, and
provenance record.

A focused all-level delta-rs replay then exercised `medium`, `high`, and `max`
with the same clustered default-SQLite profile and forced each build a second
time. Every profile reproduced its exact graph SHA-256:

| Level | Nodes | Relationships | Communities | JSON bytes | First build | Peak RSS | SHA-256 prefix |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| low | 9,982 | 25,206 | 532 | 34,116,625 | 4.30 s median | 434.8 MiB median | `971d5882` |
| medium | 9,982 | 25,219 | 528 | 34,137,117 | 4.43 s | 433.0 MiB | `27492c72` |
| high | 21,368 | 56,640 | 749 | 85,047,812 | 5.91 s | 722.8 MiB | `8a70927b` |
| max | 34,384 | 93,247 | 1,078 | 143,936,676 | 9.02 s | 1,120.9 MiB | `dc87abf2` |

The low row retains the three-sample result above; the other performance rows
are single samples plus an unmeasured exact forced rebuild, not promoted
baselines. This proves deterministic nested publication on the pinned corpus
but not complete-suite qualification. It also exposes a separate high/max
memory problem: admitting external and deferred inference more than doubles
the clustered graph and drives max past 1 GiB, so low-only improvements cannot
stand in for all-level performance.

A subsequent query-hydration replay used the same pinned delta-rs SQLite
artifact and seven fresh release-binary processes per independently labeled
negative identifier. Moving the already-established absent composite-
identifier admission rule ahead of generic posting hydration reduced median
latency from `0.04`--`0.52` seconds to `0.01`--`0.05` seconds. The slowest row
fell from `0.52` to `0.02` seconds while decoded candidate work fell from
3,471 nodes to zero; every row performed only exact-ID and exact-name probes.
All five negative oracles remained no-answer, and the exact composite
`AddColumnBuilder` still resolved through `exact_name`. This is a focused
warm-filesystem diagnostic rather than a promoted cross-platform baseline.

A later operation-admission replay added a compact immutable term index over
source-backed operation-role declarations. One fresh process per 20 positive
and five negative source-reviewed labels, with `--max-nodes 3 --max-edges 1`
to isolate seed admission from neighborhood expansion, produced 19/20 Top-1,
20/20 Top-3, and 5/5 no-answer results. Median latency was 0.066 seconds, the
observed p95 was 0.542 seconds, and the maximum was 0.892 seconds. Median
candidate work was 25 decoded nodes; operation rows commonly decoded 5--44.
The remaining broad representation rows decoded as many as 6,407 nodes and
account for the tail. The sole Top-1 disagreement ranked the Python
`CommitProperties` declaration ahead of the independently labeled Rust
`CommitProperties`; the labeled declaration remained second. This focused
result does not claim default 500-node neighborhood latency, which remains a
separate traversal/hydration target.

Running the checked-in correctness validator once per label with the default
discovery limits produced 21/25 strict passes: 16/20 positives plus all five
negative controls. Positive Top-1 was 19/20, MRR@10 was 0.975, and recall@10
was 100%. The four failed positive rows were the same `CommitProperties`
cross-language Top-1 disagreement plus unexpected ambiguity on that row and
three otherwise correct Top-1 rows. Only strict passes are timing-eligible in
the comparison harness, so these quality results do not manufacture a latency
claim from failed rows.

A subsequent checked-in Delta qualification on 2026-08-11 used the focused
64-node/128-edge defaults, ID-first multi-term intersection, and full-rank
ambiguity comparison. Source review independently confirmed that the unscoped
`CommitProperties` question legitimately names both the Rust-core and
Python-facing public declarations, so the oracle now requires that ambiguity
instead of forcing a language preference. All 20 positive labels and all five
negative controls passed, and every one of 250 fresh-process plus 250
persistent-MCP measured samples was correctness-eligible. Across the 25 rows,
fresh-process p50 ranged from 0.0481 to 0.5500 seconds and p95 from 0.0483 to
0.5647 seconds; every row stayed below the 0.59-second p95 target. Positive
persistent-MCP p50 ranged from 0.1613 to 0.4806 seconds, while the five
constant-work negative controls ranged from 0.00147 to 0.00169 seconds. Fresh
peak RSS ranged from 19.64 to 84.31 MiB. The shared MCP server peaked at 147.48
MiB for the complete session, which is process high-water rather than per-query
memory. The harness performs one unmeasured warmup per row, so “fresh” means a
new CLI process over warmed filesystem state, not a cold-page-cache claim. The
complete run passed its correctness gate; raw evidence is retained outside the
repository under the qualification workspace and is not a promoted
cross-platform baseline.

A subsequent declaration-admission replay at Compass revision `2e9d1455`
projected the existing exact term postings onto source-backed type
declarations. The store can finish from this compact channel only when it read
the complete declaration set and the unchanged ranker proves every requested
seed outranks omitted non-types. The canonical graph artifacts did not change:
low retained SHA-256 `971d5882...` and max retained `dc87abf2...`. The SQLite
store grew from 35,995,648 to 36,405,248 bytes for low (1.14%) and from
113,033,216 to 113,278,976 bytes for max (0.22%).

Across all 25 reviewed questions, decoded candidate IDs fell from 21,543 to
897 in low (95.84%) and from 25,978 to 913 in max (96.49%). The four formerly
broad declaration rows changed as follows:

| Question | Low candidate IDs | Max candidate IDs |
| --- | ---: | ---: |
| Delta table state | 6,407 -> 245 | 6,985 -> 251 |
| commit properties | 5,958 -> 64 | 6,357 -> 65 |
| table snapshot | 4,691 -> 143 | 6,917 -> 149 |
| log-store abstraction | 4,132 -> 90 | 5,364 -> 93 |

Every low and max matrix again passed all 25 labels in both 250-sample fresh
CLI and 250-sample persistent-MCP modes. Low retained all ranked seed lists.
Max changed one rank-3 seed for the state question from `DeltaTableFactory` to
the more specific `DeltaTable`; Top-1 and the independently labeled result were
unchanged, and low/max now agree for that row instead of allowing max-only
inferred relationship evidence to promote the factory.

The max run improved aggregate fresh p95 from 1.067 to 0.518 seconds and peak
RSS from 126.72 to 91.75 MiB; persistent p95 improved from 1.259 to 0.367
seconds and the server high-water fell from 237.52 to 176.23 MiB. Two complete
low runs exposed runner noise in different modes: fresh p50 was stable at
0.252--0.264 seconds, but aggregate p95 ranged from 0.290 to 0.609 seconds;
persistent p50 was 0.193--0.198 seconds while p95 ranged from 0.244 to 0.542
seconds. The slow samples switched from persistent to fresh between repeats,
so this result does not promote the better timing or claim a cross-platform
latency bound. Candidate work, correctness, graph identity, and store size were
deterministic across those runs.

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

A 2026-08-05 TypeScript follow-up used the same pinned 444-file Zod checkout,
fresh JSON output roots, and the retained Graphify 0.9.32 artifact. Three fresh
Compass processes produced byte-identical graphs with 11,322 nodes and 14,921
occurrence-backed relationships. External wall times were 6.95, 1.47, and
1.41 seconds (1.47-second median); the first run was a host cold-launch outlier
and is retained rather than discarded. The retained 4.918-second Graphify
sample is therefore 3.35× the Compass median, while Compass used 339--350 MB
peak RSS versus Graphify's 250 MB.

The TypeScript variance and value-namespace correction removed all four valid
syntax parser-recovery quarantines. One source-invalid inheritance edge remains
explicitly omitted. Against 5,928 Graphify edge hypotheses, exact matches rose
from 3,482 to 4,195, semantically dominated matches rose from 292 to 547, and
missing hypotheses fell from 2,127 to 1,073. Seventy-seven formerly missing
hypotheses are now rejected by stronger exact source facts: Graphify attributes
methods declared by `_ZodBigInt` to `ZodBigInt`, for example, and attributes
`ZodAny extends _ZodType` to `ZodType`. Nine former exact agreements are also
correctly rejected: type-only imports of `ZodParsedType` and `ZodIssueCode`
target their type aliases, while Graphify targets the same-named runtime values.
The 92 missing Graphify node hypotheses are unchanged. These comparator
classifications and targeted source checks are diagnostic, not a statistical
precision or recall claim.

A 2026-08-05 Rust follow-up used the pinned 191-file Rayon checkout and fresh
JSON output roots. Three release-binary Compass processes produced
byte-identical graphs (SHA-256 `11d35db33a3ac2ad1c6914cd59b53ec6b5fbe637046c04307aad57978edb91fb`)
with 8,828 nodes and 18,422 occurrence-backed relationships. Wall times were
2.24, 2.14, and 2.30 seconds (2.24-second median), with a 326.7 MB maximum peak
RSS. The retained Graphify 0.9.32 sample took 1.947 seconds and 84.6 MB, so this
small-repository sample is not a Compass performance win; Compass publishes
2.39× as many relationships.

Preserving nested Rust declarations added 261 nodes and 501 relationships over
the preceding Compass artifact. Resolving repository-local wildcard calls
then made eight additional Graphify call hypotheses exact without redirecting
same-module calls through the wildcard. In the final comparison, 2,159 of
7,701 Graphify relationship hypotheses matched exactly, 1,795 were
semantically dominated, 2,965 were rejected as unsafe projections, and 782
remained missing. Of 4,197 node hypotheses, 2,754 matched exactly, 271 were
dominated, 1,083 unverifiable placeholders were rejected, and 89 remained
missing. These classifications use source-compatible endpoints and occurrence
anchors; they are diagnostic hypotheses, not precision or recall percentages.

A second 2026-08-05 Rayon run qualified scoped Rust type, lifetime, and const
generic parameters. Three fresh outputs were byte-identical (SHA-256
`2b9111ed6f21748749c8046c8c3f8730c9991ab9b218be63d5e60c4a6f9eab80`)
at 11,963 nodes and 24,929 occurrence-backed relationships. External wall
times were 7.94, 2.01, and 2.04 seconds; the first copied-binary launch is
retained as a host cold-launch outlier, and the 2.04-second median remains
slightly slower than the retained 1.947-second Graphify sample. Peak RSS was
377,454,592 bytes. Four pre-existing unrelated edges were omitted; no generic
parameter or bound relationship was omitted.

The source-compatible comparison improved to 2,160 exact and 2,349 dominated
Graphify edge hypotheses, with 2,520 rejected and 672 missing. Four formerly
missing Graphify node hypotheses now match exact scoped parameters, leaving 85
missing nodes. Of the 110-edge reduction in missing hypotheses, one is a newly
covered typed reference and 109 are now rejected because Graphify targets a
same-named generic parameter from the wrong lexical owner. Another 599
previously unverifiable generic edges are dominated by Compass's exact
occurrence targets. The remaining missing-node set consists almost entirely
of Graphify's synthetic Rust impl-receiver spellings such as `Vec<T>` and
`&'a [T]`; Compass graph schema version 1 has no first-class impl-block node.
That representation gap and the remaining exact-endpoint call, type, and
import hypotheses require separate source audits rather than synthetic count
inflation.

A third 2026-08-05 Rayon run resolved repository-local Rust imports to their
unique semantic module instead of dropping the relationship when the module's
physical file had the same qualified identity. Three fresh graphs were
byte-identical (SHA-256
`21711c58282858cf0fc6873ac538ea88445f6a0718159ee776397eb359aa4af4`)
at 11,963 nodes and 25,095 occurrence-backed relationships. Wall times were
7.47, 1.96, and 2.45 seconds in the committed-revision rerun; the copied-binary
cold-launch outlier is retained and the median was 2.45 seconds. Maximum peak
RSS was 380,092,416 bytes. Four
pre-existing unrelated edges were omitted.

The source-compatible comparison now reports 2,160 exact, 2,408 dominated,
2,512 rejected, and 621 missing Graphify edge hypotheses. It recognizes 52
exact-occurrence imports where Graphify names the physical Rust file and
Compass names the same qualified semantic module, including imports owned by a
more precise nested source scope. The comparison requires a mapped file target,
identical qualified module identity, exact occurrence file and line, and one
compatible Compass import; ambiguity still fails closed. Relative to the
scoped-generic run, the resolver moved eight unsafe projections to dominated
evidence and the representation-aware comparison reduced missing hypotheses by
51. Node classifications remain 2,758 exact, 271 dominated, 1,083 rejected,
and 85 missing. These counts remain diagnostic hypotheses, not precision or
recall percentages.

A fourth 2026-08-05 Rayon run preserved Rust associated types as exact
trait- or implementation-scoped aliases and resolved `Self::Type` returns to
the unique lexical declaration. Three forced, fresh-output graphs from Compass
revision `5d414b11c710` were byte-identical (SHA-256
`aeb329d04c758d275d06456188722e048afc55525e9a62485e39aa88eefee09e`)
at 12,144 nodes and 26,378 occurrence-backed relationships. External wall
times were 7.31, 1.56, and 1.42 seconds; the first host cold-launch outlier is
retained and the median was 1.56 seconds. Maximum peak RSS was 392,937,472
bytes. The retained Graphify 0.9.32 sample took 1.947 seconds and 84,639,744
bytes, so Compass was 1.25x faster at the median while publishing 3.43x as many
raw relationships. This is a single small-repository diagnostic comparison,
not a promoted performance baseline.

The source-compatible comparison reports 2,160 exact, 2,421 dominated, 2,612
rejected, and 508 missing Graphify edge hypotheses; node classifications stay
at 2,758 exact, 271 dominated, 1,083 rejected, and 85 missing. Exact return
occurrences now distinguish the associated alias from its concrete realization:
one Graphify concrete-return hypothesis is dominated by the stronger two-edge
representation, and 110 terminal-name projections are rejected because they
target an unrelated same-named trait or type. The targeted return-type subset
fell from 128 missing hypotheses to 17. Those 17 consist of multiline or
duplicated source-owner mapping cases, cross-impl associated types inherited
through a related trait, and two Graphify synthetic slice endpoints; they
remain open for separate source audits rather than being guessed. Overall
missing edge hypotheses fell by 113 from the module-import run. These are
comparator classifications with source and occurrence constraints, not a
statistical precision or recall claim.

A fifth 2026-08-05 Rayon run resolved unqualified Rust symbols across multiple
visible repository-local glob imports. Three forced, fresh-output graphs from
Compass revision `acc1f7992f85` were byte-identical (SHA-256
`4b0d37f89ab94df99164d8511daf69ab58c81a6ed4761d8c1875841b619c5a15`)
at 12,111 nodes and 26,537 occurrence-backed relationships. External wall
times were 7.43, 1.51, and 1.38 seconds; the first host cold-launch outlier is
retained and the median was 1.51 seconds. Maximum peak RSS was 382,681,088
bytes. Against the retained 1.947-second Graphify sample, Compass was 1.29x
faster at the median while publishing 3.45x as many raw relationships. Four
pre-existing unrelated edges were omitted.

The source-compatible comparison reports 2,209 exact, 2,495 dominated, 2,542
rejected, and 455 missing Graphify edge hypotheses; node classifications stay
at 2,758 exact, 271 dominated, 1,083 rejected, and 85 missing. Forty-nine
previously missing `bridge` or `bridge_unindexed` calls now match exactly at
their source occurrences. Another 74 Graphify hypotheses are dominated by
source-backed implementation owners, inheritance occurrences, field types, or
typed references after the same glob resolution rewired 84 implementation
relationships away from placeholders. The resolver unions only bounded local
glob scopes with one compatible declaration. Competing local declarations,
mixed local/external glob sets, and truncated scope or re-export searches fail
closed. Missing edge hypotheses fell by 53 from the associated-type run; the
remaining set contains 53 calls, 111 type/signature references, nine
implementation relationships, 170 containment relationships, and 112
hypotheses with uncovered non-containment endpoints. These diagnostic
classifications are not statistical precision or recall measurements.

A source-audited comparator follow-up at Compass revision `34e23ca7bd38`
kept the graph digest, counts, and timing evidence unchanged while separating
additional Graphify projection errors from native Compass gaps. Exact scoped
child types reject 46 Graphify field or signature targets that bind to an
unrelated same-named parameter; 19 multi-component field types remain
ambiguous instead of selecting one constituent. Exact anchored signature
occurrences reject 11 edges that Graphify assigns to an earlier same-named
Rust impl method, and exact return occurrences reject two edges assigned to an
earlier same-named function. Placeholder targets are not accepted as
disproof, and multiple occurrence candidates fail closed as ambiguous. The
updated comparison reports 2,209 exact, 2,495 dominated, 2,601 rejected, 19
ambiguous, and 377 missing Graphify edge hypotheses. The residual set contains
53 calls, 51 type/signature references, 103 implementation relationships, and
170 containment relationships. Node classifications remain 2,758 exact, 271
dominated, 1,083 rejected, and 85 missing. These are source-constrained
comparator classifications, not statistical precision or recall measurements.

A subsequent 2026-08-05 Rayon qualification added exact receiver identity,
complete Rust supertrait traversal, and bounded parent-module glob visibility.
The fresh graph contains 12,091 nodes and 26,790 relationships. Relative to the
preceding 12,111-node, 26,537-relationship graph, 22 repeated external
`Producer`/`UnindexedProducer` relationships retarget to their exact local
traits, eliminating 20 duplicate placeholder nodes; all nine previously
missing inherited `Self::Reducer` returns resolve;
and inherited `ParallelIterator::Item` uses resolve across exact
`IndexedParallelIterator` implementations. The only two old non-placeholder
relationships removed were source-invalid bindings from `Add<Output = T>` to
unrelated `ProducerCallback::Output` aliases. The comparator reports 2,242
exact, 2,518 dominated, 2,588 rejected, 19 ambiguous, and 334 missing Graphify
edge hypotheses, reducing the missing set by 43 from the preceding audited
run. Node classifications remain 2,758 exact, 271 dominated, 1,083 rejected,
and 85 missing. These are source-constrained comparator classifications, not
statistical precision or recall measurements.

A further 2026-08-05 Rayon qualification made Rust wildcard completeness aware
of exact source-present sibling crates. Three clean release builds were byte
identical at 12,060 nodes, 26,675 relationships, and graph SHA-256
`52df1abb0e84468f165b1e1066a254b45b1d8dafb200b83519d38c767a17e99b`.
Relative to the preceding graph, 34 call and 20 test occurrences were added,
including exact `empty`, `once`, `repeat_n`, `scope`, and `Board::random`
targets. The graph removed 177 placeholder relationships whose targets were
not declarations in the indexed wildcard modules, including false
`rayon::prelude::Vec`, `rayon::prelude::Send`, and
`rayon::prelude::FnOnce` identities. The comparator reports 2,251 exact, 2,516
dominated, 2,585 rejected, 19 ambiguous, and 330 missing Graphify edge
hypotheses: nine more exact matches and four fewer missing hypotheses than the
preceding run. Node classifications remain 2,758 exact, 271 dominated, 1,083
rejected, and 85 missing. These are source-constrained comparator
classifications, not statistical precision or recall measurements.

The next 2026-08-05 Rayon qualification added exact Rust implementation-header
type references. Three clean release builds were byte-identical at 12,060
nodes, 26,809 relationships, and graph SHA-256
`91b5e3c97c926d08a576341ef2fff991acac519a2791bf60fc628d7aa30550da`.
All 134 added relationships are exact source occurrences: 131 target scoped
implementation parameters, two target source structs, and one targets an
explicitly imported type alias. A targeted source audit confirmed
`WorkerThread -> ThreadBuilder`, `CollectReducer -> CollectResult`,
`ListReducer -> LinkedList`, and tuple-parameter references on `Unzip` against
their implementation headers. No node identities, node counts, or preceding
relationships changed; Rust adapter metadata advances to version 7. The
comparator reports 2,253 exact, 2,604 dominated, 2,497 rejected, 19
ambiguous, and 328 missing Graphify edge hypotheses. Two formerly missing
references are now exact, while 88 unverifiable generic placeholders are
dominated by Compass's exact implementation-scoped parameter identities. Node
classifications remain 2,758 exact, 271 dominated, 1,083 rejected, and 85
missing. These are source-constrained comparator classifications, not
statistical precision or recall measurements.

The following 2026-08-05 Rayon qualification published Rust blanket trait
implementations from their exact impl-scoped generic parameter declarations.
Three clean release-binary builds were byte-identical at 12,060 nodes, 26,814
relationships, and graph SHA-256
`e073a4d8f83ade7b9fd01cc6ebcd09903e6f115c3191c48a78acc16238314f2c`.
The five added `implements` relationships are source-anchored at the blanket
implementation headers for `IntoParallelRefIterator`,
`IntoParallelRefMutIterator`, `IntoParallelIterator`, `ParallelBridge`, and
`Pattern`; no preceding node or relationship was removed. Publication still
omits only the four pre-existing invalid macro-containment relationships. The
comparator reports 2,257 exact, 2,605 dominated, 2,497 rejected, 19 ambiguous,
and 323 missing Graphify edge hypotheses: four formerly missing blanket
implementations are exact, while Graphify's single node for two distinct `I`
parameters is dominated by Compass's occurrence-scoped owners. Node
classifications remain 2,758 exact, 271 dominated, 1,083 rejected, and 85
missing. Rust adapter metadata advances to version 8. These are
source-constrained comparator classifications, not statistical precision or
recall measurements.

The following 2026-08-05 Rayon qualification preserved calls through the
mutually exclusive `#[cfg(unix)]` and `#[cfg(windows)]` reexports of
`get_cpu_time`, together with the source-declared fallback for other targets.
Three clean release-binary builds were byte-identical at 12,060 nodes, 26,820
relationships, and graph SHA-256
`08e304704903ecaf5df14cfa8f828c0563cf7df255833c5a5bd8a818c953f73b`.
The only graph delta was six exact, source-anchored `calls` relationships: each
of the two call sites in `rayon-demo/src/cpu_time/mod.rs` targets the Unix,
Windows, and fallback declarations. No node or preceding relationship changed.
Against the same pinned Graphify artifact used by the preceding qualification,
the comparator reports 2,258 exact, 2,605 dominated, 2,497 rejected, 19
ambiguous, and 322 missing Graphify edge hypotheses; the line-33
`get_cpu_time` hypothesis moved from missing to exact. Rust adapter metadata
advances to version 9. A separate fresh Graphify run was excluded from change
attribution because it produced 4,185 nodes, 8,183 relationships, and 497
dangling relationships, rather than the pinned artifact's 4,197 nodes, 7,701
relationships, and zero dangling relationships. These are source-constrained
comparator classifications, not statistical precision or recall measurements.

A comparator-only follow-up reused those byte-identical Compass and pinned
Graphify artifacts. Graphify places some multiline Rust return-type references
on the callable declaration line, while Compass preserves the exact returned
symbol on a later line. The comparator now treats that projection as dominated
only when the mapped callable and returned type are exact, the Graphify context
is `return_type` or `generic_arg`, the Graphify occurrence is the callable
declaration, and exactly one Compass `returns` fact supports the pair. Eight
Rayon hypotheses moved from `missing:no_matching_relationship_occurrence` to
`dominated:precise_return_type_declaration_projection`: returns from
`init_global_registry`, `Registry::new`, `get_in_place_thread_registry`,
`Counters::increment_jobs_event_counter_if`, `ThreadPool::build`, and
`Board::new_with_custom_rules`. Exact, rejected, ambiguous, and all other
reason counts were unchanged. The resulting edge classifications are 2,258
exact, 2,613 dominated, 2,497 rejected, 19 ambiguous, and 314 missing. This
changes no Compass graph node, relationship, identity, or digest.

The next 2026-08-05 Rayon qualification removed fabricated external fallbacks
for ambiguous Rust `self.par_extend(...)` dispatch. When the indexed receiver
already has multiple local trait-method declarations, Compass now leaves the
call unresolved instead of publishing a same-named external or deferred
placeholder. Three clean release builds were byte-identical at 12,057 nodes,
26,817 relationships, and graph SHA-256
`623b619089b6391f6eaabf112e57e143f1c2d2516d009c07aa635a158beb715c`.
The exact delta removed only the `LinkedList::par_extend`,
`String::par_extend`, and `Vec::par_extend` placeholder nodes and their three
source-anchored calls in `src/iter/extend.rs`; no retained record changed and
no record was added. The regression fixture also proves that a genuine
external inherent call such as `String::push_str` remains published. Against
the same pinned Graphify artifact, every node and edge classification stayed
unchanged at 2,758 exact, 271 dominated, 1,083 rejected, and 85 missing nodes,
plus 2,258 exact, 2,613 dominated, 2,497 rejected, 19 ambiguous, and 314
missing edges. This is expected: Graphify assigned the three forwarding calls
to an unrelated unit implementation, so the removed Compass placeholders had
not supported those hypotheses. Rust adapter metadata advances to version 10.
The first timing sample was a mounted-volume outlier; the other clean process
times were 2.40 and 2.13 seconds, so no timing claim is made from this run.

Rust adapter version 11 was then qualified on the same pinned Rayon checkout
after adding source-proven associated-function result dispatch. Three clean
release builds were byte-identical at 11,859 nodes, 26,604 relationships, and
graph SHA-256
`b99fda44928991ec8ef5265b7fa869541cfb75ff24e8f2fe393a0c1de7287b53`.
Both `DrainGuard::new(...).par_drain(...)` occurrences resolve exactly to the
single local `ParallelDrainRange` implementation. Relative to version 10, the
graph removes 296 inferred external function placeholders, adds 98 corrected
placeholders, and adds 233 exact concrete-`Self` return relationships; no
source-backed declaration node is removed. The Graphify comparator records
2,258 exact, 2,613 dominated, 2,493 rejected, 19 ambiguous, and 318 missing
edge hypotheses. The four-net increase in `missing` is a comparator-label
effect: two incorrect Graphify `par_drain` targets move from missing to
rejected, while six formerly rejected hypotheses become missing after the
malformed Compass placeholders that contradicted them are removed. These
counts are diagnostic classifications, not a precision or recall claim. The
three process times were 3.39, 2.99, and 2.92 seconds on a nearly full mounted
workspace, so no performance claim is made from this run.

Rust adapter version 12 was qualified on that pinned Rayon checkout after
adding exact same-file method-result receiver evidence. Three clean release
builds were byte-identical at 11,858 nodes, 26,604 relationships, and graph
SHA-256
`f8afecf1b1c51cf8c1621fe8eab4641ad8857650182dc4181fddbe0616724900`.
At `rayon-core/src/lib.rs:324`, the chained result of
`ThreadPoolBuilder::spawn_handler(...)` now targets the source-backed
`ThreadPoolBuilder::build` method with exact evidence. This replaces one
inferred edge to a multiline expression-text placeholder and removes that
placeholder node; every other node and relationship is unchanged. The
Graphify comparator classifications remain 2,258 exact, 2,613 dominated,
2,493 rejected, 19 ambiguous, and 318 missing edge hypotheses because the
pinned Graphify graph does not represent this corrected call. Process times
were 7.94, 2.10, and 2.08 seconds; the first was a mounted-volume startup
outlier, so no performance claim is made from this run.

Rust adapter version 13 was qualified on the same pinned Rayon checkout after
adding bounded cross-file and trait-default method-result chains. Three fresh
release builds were byte-identical at 11,998 nodes, 27,090 relationships, and
graph SHA-256
`e7179312b42de8dcadcfc85b728b78625460f651438956e57ebf508320422aae`.
Relative to version 12, 169 new relationship occurrences are exact and 13
formerly inferred occurrences become exact, with no exact occurrence lost.
This includes the complete `par_split -> filter -> drive_unindexed` chains in
`src/str.rs` and repeated `ThreadPoolBuilder` builder chains. The graph has no
dangling or duplicate edges, every relationship has a source occurrence, and
publication reports zero omitted nodes, four omitted edges, and zero identity
collisions.

The retained Graphify 0.9.32 comparison contains 7,701 relationships versus
Compass's 27,090. Its classifications are 2,258 exact, 2,613 dominated, 2,498
rejected, 19 ambiguous, and 313 missing. Six formerly missing Graphify call
hypotheses are now rejected by exact source-occurrence targets; one formerly
rejected hypothesis becomes missing because Compass no longer treats a
`*const WorkerThread` return as a `WorkerThread` method receiver. These are
quality classifications, not precision or recall. The inferred surface also
remains a gap: 199 inferred placeholder identities were added and 59 removed
relative to version 12, for a net increase of 140. They remain occurrence
anchored but require further language-aware return modeling before their raw
count can be treated as useful graph coverage. Process times were 3.51, 3.30,
and 3.38 seconds versus the retained 1.947-second Graphify sample, so this
small-repository run makes no performance-win claim.

A later current-branch Rayon qualification followed the source-present named
reexport behind `use rayon::*` before resolving the associated
`ThreadPoolBuilder` chain in `tests/named-threads.rs`. Relative to the preceding
deterministic graph, exactly eight `calls`/`tests` occurrences changed:
`new`, `thread_name`, and `build_global` now target their source-backed
`rayon_core::ThreadPoolBuilder` declarations, while `unwrap` targets the
canonical `std::result::Result` endpoint. Four malformed expression
placeholders were removed and one canonical external endpoint was added, for
a net change from 12,069 to 12,066 nodes; the 27,367 relationship count is
unchanged. No unrelated occurrence changed. Two clean release builds were
byte-identical at graph SHA-256
`c77d3f0edf3170a39efc01ab78a946bbcafcac7855fbd10cd9d3e6a73dc240e1`.
Competing named reexports, cycles, lowercase local receivers, and direct-path
collisions remain fail-closed. A mounted-volume startup delay affected one
process sample, so this qualification makes no timing claim.

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
missing files and a host-aware, bounded cap of up to 12 workers at or above
that repository-size boundary. The build-wide Rayon pool uses the same
host-aware ceiling; an explicit `--max-workers` value continues to override
both automatic choices. The cap is deliberately finite rather than an
unbounded host-sized pool, because parser and graph-publication working sets
scale with concurrency.

A clean three-sample follow-up on the pinned 2026-08-05 qualification corpora
measured the release binary with JSON output and fresh output directories on
each run. Graphify values are the paired three-sample p50 baselines:

| Repository | Compass p50 | Graphify p50 | Graphify / Compass | Compass RSS p50 |
| --- | ---: | ---: | ---: | ---: |
| Cobra (Go) | 0.30 s | 0.95 s | 3.17× | 123.5 MiB |
| Click (Python) | 0.50 s | 1.94 s | 3.88× | 180.7 MiB |
| Rayon (Rust) | 0.80 s | 1.90 s | 2.38× | 284.5 MiB |
| Zod (TypeScript) | 1.00 s | 4.80 s | 4.80× | 284.8 MiB |
| Django (Python) | 10.80 s | 40.76 s | 3.77× | 2,248 MiB |

The Django graph remained 80,976 nodes / 195,164 edges with zero validation
errors, and the four small-corpus graph populations and normalized digests
remained unchanged. The host-aware default therefore improves the large
repository path, but these measurements still do not establish the requested
universal 4× target; the higher Compass RSS is also an explicit tradeoff to
monitor in future qualification.

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

### Django viewer qualification

The 2026-08-05 large-graph viewer hardening was exercised against Django
commit `957d0cee7167757ae221ffde59d2cf0a322e89c7` using the same published
294,854,948-byte graph for both exports. The graph contains 78,064 nodes,
205,291 edges, and 3,376 communities; its prepared community overview contains
3,376 nodes and 10,437 aggregate edges.

The previous standalone export embedded every community detail and occupied
1,127,391,375 bytes. The bounded export embeds only complete details within a
shared 5,000-node / 40,000-edge budget and occupied 13,570,416 bytes, a 98.8%
reduction. Eight complete community details fit this particular graph; all
other communities remain available through VS Code or explicit JSON export.
No canonical graph records were removed or rewritten.

On the local Apple Silicon development runner, Chromium reached the exported
graph controls in 751 ms over a local HTTP server. The overview rendered a
deterministic hub-centered layout and a strongest-edge connectivity backbone
of 4,000 relationships. The browser qualification fixture mirrors this shape
with 3,400 communities and 10,500 aggregate edges; it requires the useful
controls, the static layout, the 200-row community DOM bound, and the visible
edge disclosure to appear within three seconds. These are runner-specific
diagnostic observations, not a cross-platform latency guarantee.

### Django parallel fact-state qualification

The 2026-08-05 large-repository fact-state hardening was measured from Compass
revision `726df22ade0bcd3181b7b0e146783b781a87bd98` plus the candidate change,
Django revision `dfc52e53f1d19a2730854d68b602fb4dba8bf0c5`, and Graphify revision
`07b9143d4b90b1e1cb88dc71423f742a501efd29` (package 0.9.34). Both tools read
the same pinned Django checkout and wrote each sample to a distinct fresh
output root on the native APFS volume. The release Compass command used the
comparison profile `--code-only --no-cluster --no-viz --store json`.

Serial AST fact digest construction accounted for 1.98 seconds of the cold
path. Indexed parallel digest collection reduced that stage to about 0.39
seconds. Compass also constructs the unchanged pre-merge digest concurrently
with portable AST cache publication, and deterministically recomputes it after
any declaration/definition merge that changes facts.

| Tool | Cold samples (s) | p50 | Graphify / Compass |
| --- | --- | ---: | ---: |
| Compass before | 9.971, 9.816, 9.821 | 9.821 s | 4.44x |
| Compass candidate | 8.037, 8.045, 7.878 | 8.037 s | 5.43x |
| Graphify 0.9.34 | 43.643, 43.544, 44.746 | 43.643 s | — |

All three candidate runs published byte-identical 75,604-node / 203,871-edge
graphs with SHA-256
`a5dfcfbe8d77ba2308cf2f0585472132e16dd6f80efa428d5ecf879cb5600ea1`.
Candidate peak RSS remained informational for this qualification and had a
three-sample median of about 2,353 MiB. These native-volume measurements avoid
the multi-second publication variance observed on the mounted workspace, but
remain runner-specific rather than a cross-platform guarantee.

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

## Document and OCR qualification

Document decoding and OCR are governed by correctness and resource gates, not
an unmeasured speed claim. Qualification records per-format wall time, peak RSS,
page/image count, aggregate pixels, cache hit/miss behavior, and deterministic
artifact digests. A warm complete document cache must perform zero OCR
inference calls. Partial or corrupt entries are not warm hits.

Run the focused deterministic gate with:

```bash
scripts/qualify_document_ocr_v1.sh --fixtures-only
```

Native format fixtures run without a model. The optional model-backed corpus
runs only when its exact verified profile is already installed; ordinary CI
does not download model weights.

On 2026-08-23, the installed `pp-ocrv6-small` acceptance test on the local
aarch64 macOS host initialized the statically linked runtime and processed its
blank plus clean synthetic-English rasters in 1.77 seconds under a debug test
build. The clean sample had 0 CER. This single smoke result is not a production
latency, memory, degraded-input, cross-architecture, or multilingual claim.
