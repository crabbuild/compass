# Low-inference evaluation against Graphify

## Result

The `low` inference level is a useful evidence-first profile, but it does not
yet make Compass an unconditional Graphify replacement.

On the pinned delta-rs corpus, Compass built a valid clustered graph 1.77x
faster than Graphify and reused an unchanged build 15.27x faster. The Compass
graph had no dangling edges and every node and relationship was source-backed.
Low inference also reduced Compass's graph by 71% of nodes and 73% of edges
relative to `max`, and reduced fresh JSON query latency by 3.94x.

The remaining deficits are material. A cold clustered Compass build used
7.21x Graphify's peak resident memory. A clustered incremental build used
5.70x as much memory and was only 1.09x faster. Natural queries against the
SQLite Compass artifact were 1.81x slower than Graphify, and an adversarial
no-answer check exposed Compass's willingness to return candidates after only
one generic subword matched. Low inference is therefore an opt-in precision
and size control, not evidence that the performance work is complete.

## Reproduction boundary

This diagnostic ran on 2026-08-10 with:

- Apple M2 Max, 12 cores, 32 GiB RAM, macOS 26.5.2;
- Compass commit `6680842cbd2f9d9f8af967125ace99de737dc072`, Rust
  1.97.1, and release binary SHA-256
  `96b741f4436371c7911cc6859ad2c823c9e10183984f2bdbe3df4d1117544cd6`;
- Graphify 0.9.37 from commit
  `09a34ad87a6c522757da1bfb8c2c209e523a4e55`;
- delta-rs commit `c27874c10043b5ccf0207d27eee148be0a033c6e`;
- three fresh processes for cold, unchanged-warm, one-file graph-neutral
  incremental, and restored-source workloads; and
- 20 source-verified natural questions with 29 required identifiers, plus 29
  exact-identifier queries repeated three times.

Every build used a fresh, detached, read-only corpus checkout. Cold output was
removed before every sample. Timing is wall-clock process time and memory is
peak RSS. Medians are reported. Compass and Graphify were each queried through
their native CLI; the primary Compass query result uses the default SQLite
artifact because the JSON artifact was also measured separately.

The diagnostic runner SHA-256 was
`c8edf791abfbf5ecd23196ded8b2065619bd9afec663ae5d86c19467cf53c6ea`;
the analysis script SHA-256 was
`0468e187cbbea53aab0d6ea05bdcaa63f00705a1a23fd13ab21e4d1bb019d6cd`;
and the suite SHA-256 was
`ff05b867e5405bd399631a93c39d7a25fb3fe42487250d3c57c1e27776e2f3c3`.
This PR adds `--inference-level` to the checked-in performance harness and
makes explicit comparison cluster both tools, so future runs do not need the
temporary argument wrappers used for this diagnostic.

## Graph construction

The primary build comparison enables community clustering for both tools. This
is the publishable profile: disabling Graphify clustering left 2,800 dangling
edges, so those faster raw results are not used for the main claim.

| Workload, clustered | Compass low | Graphify | Graphify / Compass | Compass RSS | Graphify RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Cold | 4.110 s | 7.262 s | 1.77x | 1,078.8 MiB | 149.6 MiB |
| Unchanged warm | 0.250 s | 3.809 s | 15.27x | 24.2 MiB | 229.8 MiB |
| One-file incremental | 4.111 s | 4.490 s | 1.09x | 1,312.2 MiB | 230.4 MiB |
| Restored source | 4.209 s | 5.710 s | 1.36x | 1,330.1 MiB | 231.6 MiB |

For diagnosis only, disabling clustering in both tools produced a 3.667 s
Compass cold median and 6.520 s Graphify median. Compass remained 1.78x faster,
but used 1,063.3 MiB versus 126.6 MiB. Unchanged warm was 0.312 s versus
3.327 s. Incremental was 1.259 s versus 3.031 s and used 117.2 MiB versus
143.6 MiB. The Graphify artifact in this profile was structurally incomplete,
so the row isolates extractor cost rather than publishable graph performance.

The clustered incremental result identifies clustering as a separate Compass
bottleneck. Low filtering happens before clustering, but clustering still
rebuilds large topology state rather than reusing the previous community
realization.

## Graph quality

| Property, clustered | Compass low | Graphify |
| --- | ---: | ---: |
| Nodes | 9,982 | 9,670 |
| Relationships | 25,206 | 27,173 |
| Graph JSON | 34.17 MB | 14.67 MB |
| Source-backed nodes | 100% | 64.86% |
| Node source files represented | 433 | 335 |
| Dangling edges | 0 | 0 |
| Duplicate node IDs | 0 | 0 |
| Validation-error diagnostics | 0 | 0 |
| Exact identifiers present | 29/29 | 29/29 |

All 25,206 Compass relationships had exact evidence and source occurrences.
Graphify labeled 26,142 relationships `EXTRACTED` and 1,031 `INFERRED`; its
7,404 calls consisted of 6,430 extracted and 974 inferred calls. Compass low
published 6,287 exact calls. These counts are breadth indicators, not recall:
neither tool is an independent ground-truth oracle.

The source-aware differential mapped 5,910 Graphify nodes uniquely (61.12%)
and matched 5,690 of 10,859 relationships in the comparable relation subset
(52.40%). Compass `max` matched 54.04% on the same denominator. The 1.64-point
drop shows the price of filtering, but Graphify agreement must not be called
accuracy because Graphify's own hypotheses include inferred relationships and
identity/taxonomy differences.

Compass recorded an inventory of 723 files and source-backed nodes from 433
files. Graphify reported that 100 SQL files contributed nothing because its
optional `tree_sitter_sql` dependency was absent, and that 106 source files
produced zero nodes. This environment reflects Graphify's tested default
installation and should not be generalized to an installation with its SQL
extra enabled.

Three cold Compass graphs were byte-identical. Three clustered Graphify cold
graphs had three different byte hashes, while its raw no-cluster graph was
byte-identical across three runs. This is only a byte-reproducibility result;
the test did not isolate whether Graphify's semantic graph changed.

Compass reported 1,997 repeated `(source, target, relation)` combinations and
117 self-loops. The repeated combinations can represent distinct source
occurrences and are not duplicate IDs, but they deserve a separate
multiplicity audit before graph size is treated as entirely useful signal.

## Query quality and performance

The natural-query oracle counted a required identifier only when it appeared
in the bounded result. A question passed strictly only when all its required
identifiers appeared and no paired forbidden identifier appeared. This is a
source-verified recall proxy, not human-adjudicated answer accuracy.

| Natural query metric, valid graph artifacts | Compass low, SQLite | Graphify | Compass max, JSON control |
| --- | ---: | ---: | ---: |
| Median fresh-process latency | 1.075 s | 0.593 s | 10.146 s |
| Median peak RSS | 84.0 MiB | 128.2 MiB | 566.6 MiB |
| Required-term recall | 11/29 (37.93%) | 10/29 (34.48%) | 10/29 (34.48%) |
| Strict question pass | 5/20 (25%) | 6/20 (30%) | 4/20 (20%) |
| Forbidden-term hits | 0 | 0 | 0 |

Low therefore improved term recall over both Graphify and Compass `max`, but
Graphify answered one more complete question and was faster. Compass's SQLite
artifact was 2.40x faster than its low JSON artifact (1.075 s versus 2.576 s),
yet remained 1.81x slower than Graphify. The low JSON graph was 3.94x faster
and used 3.12x less peak RSS than the max JSON control.

For 29 exact source identifiers, both tools selected the correct identifier in
29/29 cases across three repetitions. Compass's median fresh-process time was
1.504 s with 81.5 MiB peak RSS; Graphify's was 0.546 s with 127.3 MiB. An
initial text-only evaluator incorrectly marked `AddColumnBuilder`,
`CreateBuilder`, and `VacuumMetrics` as Compass misses because the formatter
printed their exact seed IDs without their display names. Inspecting the typed
seed evidence showed `source=exact_name` as the first result in every case; the
incorrect 26/29 score is excluded.

Five adversarial identifiers were confirmed absent from the corpus. Each
combined a unique prefix with otherwise plausible concepts. Graphify returned
no answer for 5/5. Compass returned no answer for only 1/5 because it stemmed
and accepted one generic component such as `sync`, `resolve`, `sentinel`, or
`activity`. This small adversarial check is not a population precision
estimate, but it demonstrates a real specificity defect that ordinary
forbidden-term scoring did not expose.

## Low versus max

| Compass property | Low | Max | Low change |
| --- | ---: | ---: | ---: |
| Nodes | 9,982 | 34,384 | -70.97% |
| Relationships | 25,206 | 93,247 | -72.97% |
| JSON bytes | 33.65 MB | 141.94 MB | -76.29% |
| Source-backed nodes | 100% | 29.03% | +70.97 points |
| Natural-query median, JSON | 2.576 s | 10.146 s | 3.94x faster |
| Natural-query peak RSS, JSON | 181.4 MiB | 566.6 MiB | 3.12x lower |

Cold build time and memory did not improve with the smaller published graph.
Low filtering currently runs after full extraction, cross-file resolution, and
normalization, so the expensive inferred graph is still constructed before it
is discarded. Moving evidence admission earlier is required for low inference
to become a cold-build optimization rather than primarily an output and query
optimization.

## Work required to surpass Graphify

1. **Move inference admission before materialization.** Thread the selected
   evidence budget into resolver stages so low mode never creates deferred
   receiver, heuristic call, or inferred placeholder records. Gate success on
   unchanged low graph semantics plus cold peak RSS at or below Graphify on the
   full qualification suite.
2. **Make clustering incremental and bounded.** Reuse unchanged community
   assignments, update only affected topology, and avoid simultaneous full
   graph/adjacency/community copies. The delta-rs target is below 250 MiB for a
   one-file clustered update, not the measured 1.31 GiB.
3. **Fix query specificity without sacrificing recall.** Preserve the 29/29
   exact fast path, but require stronger multi-concept coverage before a
   generic subword can produce a positive answer. Add the adversarial cases to
   the reviewed no-answer corpus and require 5/5 while retaining or improving
   the 11/29 natural required-term recall.
4. **Reduce fresh query startup and hydration cost.** The SQLite path already
   cuts low-query latency by 2.40x. Profile process startup, store opening,
   candidate hydration, traversal, and text rendering independently; qualify
   both fresh CLI and persistent MCP sessions. The immediate cross-tool gate
   is at most 0.59 s on this corpus without increasing query work or weakening
   deterministic results.
5. **Create an independent quality denominator.** Graphify overlap is useful
   differential evidence, not truth. Expand compiler/source oracles and
   adjudicated samples for declarations, imports, calls, members, returns,
   negative targets, and duplicate names. Report precision, recall, F1,
   ambiguity, and unsupported-file coverage by relation and language.
6. **Audit multiplicity and serialization cost.** Prove which repeated semantic
   pairs are distinct occurrences, then compact representation without
   dropping provenance, direction, or parallel-edge meaning. Graphify's JSON
   was less than half the bytes even though node and relationship counts were
   similar.
7. **Qualify every inference level on the complete corpus suite.** Keep `max`
   as the compatibility default until low, medium, and high have deterministic
   build, incremental, query, and source-oracle evidence across languages.

This run is diagnostic evidence from one mixed Rust/Python/SQL repository. It
is not a promoted performance baseline, a broad language-quality claim, or a
human-adjudicated accuracy study.
