# Qualification corpora, baselines, and budget decision

Status: **RATIFIED UNCHANGED for Wave 5 qualification**
Decision date: 2026-08-28
Scope: future Surreal graph projection and dual-engine comparisons

## Decision basis

The thresholds below were written into the research document and phase plan
before C-012 ran. The immutable plan realization was pinned after the
2026-08-28T12:45:00Z verify-first handoff entry and before C-013 ratification;
its authoritative identity is SHA-256
`46e37804d513cf32ce8e7d008816642dffbb9d3b7b60fd3f2e82a482cd398ebf`.
No earlier filesystem timestamp is used as decision provenance.
The exact research budget section (lines 530–550 at ratification) has SHA-256
`d2f805da25a4904bb7137a0c13c1ab5a813a25c10875d24b2d81887e8298c744`.

Decision: accept every threshold unchanged. C-012's already-recorded disposable
engine measurements are explicitly not inputs to this ratification and are not
evaluated here. No Wave 5 Surreal projection result exists at this decision.

## Ratified profiles

| Profile | Nodes | Edges | Purpose |
| --- | ---: | ---: | --- |
| `qualification-semantic` | checked-in corpus | checked-in corpus | Exact semantic/evidence equivalence |
| `qualification-medium` | 100,000 | 250,000 | Footprint, query, recovery, and current-engine baseline |
| `qualification-large` | 1,000,000 | 2,500,000 | Scale sampling and boundedness |

## Ratified gates

| Gate | Threshold |
| --- | --- |
| Semantic equivalence | 100% preservation of identity, direction, parallel edges, confidence, source evidence, bounds, and pagination on the semantic corpus plus deterministic samples from both scale fixtures. |
| Default footprint | No linked Surreal dependencies and zero Surreal-attributable size delta when the feature is disabled. |
| Enabled footprint | Compressed artifact and peak RSS each no more than 2.0× current baseline on `qualification-medium`. |
| Core query regression | Search/callers/callees p95 no more than 1.10× current engine. |
| Native graph value | At least 10% lower p95 on depth-3+ impact/path and at least 15% fewer query calls or response tokens, with no evidence-recall or task-success loss. |
| Recovery | No partial generation visible; last active medium generation queryable within 30 seconds after a killed writer. |
| Agent tool value | On 30 versioned tasks, evidence recall and success are no worse than bounded raw traversal, and tool calls improve at least 20% or output tokens at least 15%. |
| Skill compatibility | Zero regressions on recorded umbrella invocations and zero incorrect/ambiguous selections on the focused-skill boundary-prompt suite. |

Changing a threshold requires a new dated decision before the affected backend is
measured. A failed gate is a falsifier, not permission to tune the corpus or
threshold after the fact.

## Artifact boundary

Versioned sources and results live under `benchmarks/qualification/`. Generated
graphs, query-index caches, binaries, and logs are disposable and must stay under
an explicitly selected `target/` or temporary directory.

## Current-engine baseline

The retained baseline uses workspace Compass 0.3.7 built with
`cargo build -p compass-cli --release --locked` on Apple arm64/macOS 26.7 with
Python 3.12.8 and zlib 1.2.12. The
exact `qualification-medium` graph is 220,023,199 bytes with SHA-256
`aca4e6a1a836780f4baa352c37469787be9815db19f8bf48e858cc87eeb4f054`.
Five measured samples follow one unmeasured cache warmup per query; p95 is the
nearest-rank value across separate CLI processes.

| Measurement | Current baseline |
| --- | ---: |
| Release binary | 146,726,928 B |
| Runtime-pinned gzip-9 binary | 25,945,073 B |
| Cold start (`--version`) p95 | 24,687 µs |
| Search p95 / max peak RSS | 11,290,305 µs / 898,514,944 B |
| Callers p95 / max peak RSS | 11,360,993 µs / 897,433,600 B |
| Callees p95 / max peak RSS | 11,374,780 µs / 897,449,984 B |
| Depth-3 impact p95 / max peak RSS | 11,238,889 µs / 912,834,560 B |
| Exact three-hop path p95 / max peak RSS | 3,959,473 µs / 1,990,197,248 B |
| Bounded raw 30-task run / peak RSS | 35,491,798 µs / 245,694,464 B |
| Raw per-task p95 after load | 14,376 µs |

These process-level timings include strict graph validation/load and are not
in-process service latency. The legacy `path` command exposes no depth flag; its
selected endpoints have a verified exact three-hop shortest path, while the
finite graph profile and 120-second process timeout bound the run. The raw
samples, deterministic output hashes,
binary/graph identities, Python/zlib host, commands, and limits are retained in
`benchmarks/qualification/current-engine-baseline-v1.json` so later comparisons
cannot silently switch measurement conditions. Source provenance includes the
tracked binary diff plus a bounded deterministic hash of all 638 existing
tracked or non-ignored untracked files across the root manifests/toolchain,
every workspace crate, and both vendored path dependencies. The 612-file tracked
and 26-file untracked subsets have separate counts, byte totals, and digests;
ignored ambient files are excluded from the identity.
