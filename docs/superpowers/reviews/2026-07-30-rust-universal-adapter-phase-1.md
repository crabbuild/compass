# Rust Universal Adapter Phase 1 Evidence

**Date:** 2026-07-30

**Branch:** `codex/rust-universal-adapter`

## Delivered

- Added the versioned internal evidence schema
  `compass.languages.evidence/1`.
- Registered Rust as `UniversalCandidate`; all other registered languages
  remain on their established extraction algorithms.
- Advertised only the Rust capabilities emitted in this increment: exact impl
  ownership, calls, and qualified external packages.
- Preserved per-file Rust evidence batches through borrowed and owned
  collection resolution.
- Recorded exact call owner, spelling, qualifier, and byte range during the
  existing Rust tree traversal. No second parser or translated extractor runs.
- Resolved `Type::method()` only when the exact inherent impl owner has one
  matching local method.
- Retained qualified unresolved calls such as `HashMap::new()` as external
  candidates instead of rebinding them to same-named local methods.
- Projected a qualified external call into the legacy graph only when its
  terminal spelling collides with a local callable. This makes false local
  rebindings auditable without materializing every unresolved external
  candidate; the full candidate set remains in universal evidence.
- Stamped Rust declaration ranges before semantic reconciliation and made
  generic metadata attachment prefer exact ranges. This prevents one-line
  `impl` blocks from confusing an impl declaration with its method.

Rust remains a universal candidate, not a complete adapter. Namespaced calls,
declarations, scopes, imports, traits, annotations, inheritance, and macros
still need to emit and resolve through the universal evidence path before the
profile can become `UniversalComplete`.

## Quality comparison with Graphify

The comparison used all nine checked-in Rust fixtures available to both
extractors:

- Graphify `tests/fixtures/sample.rs`;
- the Compass Diesel, rich qualification, semantic-documentation, Actix,
  Axum, near-match, Rocket, and Program IR fixtures.

The normalized baseline key was relationship family, source location, and
target label. Graphify `method` and Compass `contains` were treated as the
same containment family. This checks preservation of shared facts but is not
an independent precision audit of every Compass-only edge.

| Metric | Compass | Graphify |
|---|---:|---:|
| Nodes | 107 | 62 |
| Edges | 161 | 100 |
| Calls | 11 | 8 |
| Graphify baseline edges covered | 100/100 | 100/100 |
| Graphify baseline calls covered | 8/8 | 8/8 |

Before this increment, Compass omitted `Graph::new()` in the shared Rust
fixture because scoped calls were discarded. After the change it publishes
both Graphify call occurrences while retaining exact byte ranges. The two
Compass-only calls come from stronger supported Rust extraction; they were not
counted as proof of correctness merely because they are additional.

## Fixture performance

The initial committed adapter was compiled as the optimized
`compass-languages` extraction example. Each tool was warmed once per fixture
and then invoked in five fresh processes. Timings include process startup and
JSON rendering. The later collision-only projection adds one node, one edge,
and one call across the nine-fixture set; it was measured separately on Bevy
below rather than reusing microbenchmark timing as a real-corpus claim.

- Median per-fixture speedup: **12.83x**
- Minimum per-fixture speedup: **11.26x**
- Compass median fixture latency range: **3.33–4.95 ms**
- Graphify median fixture latency range: **53.61–55.70 ms**

On the shared `sample.rs` fixture, five `/usr/bin/time -l` samples reported:

- Compass median peak RSS: **5,472,256 bytes**
- Graphify median peak RSS: **30,572,544 bytes**
- Compass used about **5.6x less** peak resident memory in this measurement.

These are extractor-level fixture results. They do not substitute for the
official Bevy cold, warm, incremental, query, and Graphify comparison gates.

## Phase 2 fixture qualification

Phase 2 replaced the one-off fixture normalization with the reusable,
occurrence-aware correctness classifier used by the real-corpus gate. Each of
the nine Rust fixtures was extracted independently by Compass and Graphify
three times. All Compass and Graphify canonical digests were stable across the
three repetitions.

The final isolated-fixture matrix contained 100 Compass nodes and 128 Compass
edges versus 62 Graphify nodes and 89 Graphify edges. Compass emitted 26 calls
on every repetition, compared with 8 Graphify calls. The shared-fact result
handled all 89 Graphify edges with no missing or ambiguous facts:

| Relation | Graphify | Exact | Dominated | Rejected | Missing | Ambiguous |
|---|---:|---:|---:|---:|---:|---:|
| calls | 8 | 6 | 0 | 2 | 0 | 0 |
| contains | 39 | 38 | 1 | 0 | 0 | 0 |
| extends | 1 | 1 | 0 | 0 | 0 | 0 |
| implements | 4 | 4 | 0 | 0 | 0 | 0 |
| imports | 2 | 0 | 2 | 0 | 0 | 0 |
| references | 35 | 7 | 5 | 23 | 0 | 0 |

The rejected facts are not omissions: they are Graphify terminal-name
rebindings contradicted by Compass's qualified or exact-occurrence evidence.
For example, a qualified Rust receiver is not rebound to an unrelated local
method solely because the terminal spelling matches.

A separate combined-corpus stress run placed all nine unrelated fixtures in
one source tree. Both tools remained deterministic, but Graphify rebound six
`std::result::Result` uses in the Program IR fixture to the unrelated custom
`Result` declared in `sample.rs`. Compass conservatively emitted no such
cross-module references. The graph-only classifier reports those six edges as
missing because the unresolved standard-library identity is intentionally not
materialized; manual source inspection establishes that preserving those
Graphify edges would reduce correctness. The qualification gate therefore
uses fixture-isolated parity, while the combined run is retained as an
adversarial false-rebinding audit.

## Verification

The final implementation passed:

```text
cargo fmt --all -- --check
cargo test --locked -p compass-languages
cargo test --locked -p compass-resolve
```

The full language suite covered 30 crate unit tests plus all integration
targets, including the new universal registry, occurrence, qualification,
Unicode, repetition, ambiguity, and one-line impl cases. The full resolver
suite included the new two-file evidence-preservation test and all existing
framework, language, SQL, and occurrence suites.

`graphify update .` completed from the parent repository and reported no
code-graph topology change in the parent outputs.

## Official pinned-Bevy comparison

The official comparison ran from clean Compass commit `83adb16` against Bevy
commit `25368b78ce5e9b15dc770cdf2af4595602cc8a7b` and Graphify commit
`4fe11092ccbe9f543608f140c790f68d5d83cae4`. It used three cold, warm, and
incremental samples per tool. Graphify writes
`graphify-out/cache/stat-index.json` inside the corpus, so the run used a
process-scoped Git exclude for that generated directory; the selected Rust
mutation and every source-file status check remained active.

| Tool/workload | Eligible | p50 | p95 | Peak RSS |
|---|---:|---:|---:|---:|
| Compass cold | 3/3 | 12.863 s | 13.425 s | 4,656.69 MiB |
| Graphify cold | 3/3 | 37.766 s | 49.193 s | 388.08 MiB |
| Compass warm | 3/3 | 1.777 s | 1.836 s | 691.88 MiB |
| Graphify warm | 3/3 | 14.989 s | 16.351 s | 478.69 MiB |
| Compass incremental | 3/3 | 27.939 s | 37.798 s | 6,591.91 MiB |
| Graphify incremental | 3/3 | ineligible | ineligible | ineligible |

Compass was **2.936x faster cold** and **8.437x faster warm**. It did not meet
the suite's 5x cold target and used more memory in every comparable workload.
Compass's cold, warm, incremental, and restore graphs were deterministic.
Graphify's incremental and restore canonical graph digests changed on all
three repetitions, so the harness correctly withheld an incremental
performance comparison.

The strict shared-fact gate indexed 117,094 Compass nodes and 188,238 Compass
edges versus 43,812 Graphify nodes and 115,860 Graphify edges. Compass covered
43,643 of 43,812 unambiguous Graphify nodes (99.61%) and 77,177 of 115,860
Graphify edges (66.61%). It reported 120 missing and 49 ambiguous Graphify
nodes, plus 38,399 missing and 284 ambiguous Graphify edges. The largest edge
gaps were:

| Relation | Graphify edges | Strictly covered | Missing | Ambiguous |
|---|---:|---:|---:|---:|
| references | 63,320 | 39,516 | 23,584 | 220 |
| contains | 27,511 | 20,276 | 7,187 | 48 |
| calls | 16,825 | 11,409 | 5,411 | 5 |
| implements | 4,631 | 3,485 | 1,140 | 6 |
| imports | 3,133 | 2,110 | 1,020 | 3 |
| extends | 440 | 381 | 57 | 2 |

This official gate therefore **failed**. More total Compass facts are not proof
of better quality, and the phase does not claim suite-level Graphify parity.

## Post-gate Rust call refinement

Inspection showed that many missing Graphify calls were terminal-name false
rebindings. For example, Bevy line 24 calls `World::new()`, while Graphify
targets the same-file `Benchmark::new()`. The follow-up collision-only
projection publishes an unresolved `World.new` target at the exact occurrence,
allowing the comparator to reject rather than silently count that baseline
edge.

On one fresh extraction of the same pinned Bevy tree, the refined graph had
119,966 nodes and 195,155 edges. Call coverage rose from **67.81% to 88.67%**:
3,414 Graphify call edges were identified as qualified-external calls rebound
to local symbols, and missing calls fell from 5,411 to 1,901. Overall strict
edge coverage rose from **66.61% to 69.64%**. The remaining dominant gaps are
21,132 reference occurrences and 6,966 containment paths.

Current-code spot measurements on the same machine were:

- cold: 13.77 s wall, 4,942,512,128-byte peak RSS;
- warm: 3.19 s wall, 2,830,024,704-byte peak RSS.

Against the official Graphify medians these individual samples were 2.74x
faster cold and 4.70x faster warm. They are single-sample follow-ups, not a
replacement qualification matrix. The refinement materially improves call
quality while preserving a latency advantage, but memory, cold 5x speed,
reference/containment coverage, and Graphify-parity gates remain open.
