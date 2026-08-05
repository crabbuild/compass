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
4×–5× cold-build advantage. Small-project extraction now stays sequential by
default to avoid multiplying parser/AST working sets; `--max-workers` remains
the explicit opt-in for parallel extraction. Portable AST cache publication
also streams one compressed value at a time instead of retaining the entire
compressed batch in memory.

The same Cobra checkout was cold-built into a fresh SQLite output and then
received a comment-only edit. The edit extracted 1 file and reused 36 cached
files; store publication created 5 objects and wrote 32,325 bytes, compared
with 192 objects and 2,072,182 bytes for the cold publication. Snapshot tests
also verify that the Edges, Incoming, Outgoing, Files, Names, Terms,
Communities, and Diagnostics roots remain immutable for a file-only change.

The language fixtures cover the remaining attribution gaps directly: Go
variadic range elements, type assertions, and nested closure parameters now
resolve to their receiver methods; Rust generic impl calls resolve to the
exact impl owner both within one file and across an imported module. On the
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

The extraction handoff now releases excess capacity from the AST fact working
set before project-wide resolution. Large nested JSON fact maps are rebuilt
only above a high cardinality threshold; this keeps the common per-node and
per-edge path allocation-neutral while bounding the large-map case. The
compaction is lossless: a threshold-tuned release-binary smoke run produced
the same graph SHA-256 for all four pinned repositories (`78ef5b7b` Cobra,
`c25d3b97` Click, `06014539` Rayon, and `f620afe4` Zod). The observed peak RSS
was 103, 168, 270, and 309 MiB respectively. These are single-run diagnostic
measurements, not a promoted memory baseline.

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
