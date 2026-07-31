# Rust Universal Adapter Phase 1 Evidence

**Date:** 2026-07-30

**Branch:** `codex/rust-universal-adapter`

## Delivered

- Added the versioned internal evidence schema
  `compass.languages.evidence/1`.
- Registered Rust as `UniversalCandidate`; all other registered languages
  remain on their existing legacy algorithms.
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
| Nodes | 106 | 62 |
| Edges | 160 | 100 |
| Calls | 10 | 8 |
| Graphify baseline edges covered | 100/100 | 100/100 |
| Graphify baseline calls covered | 8/8 | 8/8 |

Before this increment, Compass omitted `Graph::new()` in the shared Rust
fixture because scoped calls were discarded. After the change it publishes
both Graphify call occurrences while retaining exact byte ranges. The two
Compass-only calls come from stronger supported Rust extraction; they were not
counted as proof of correctness merely because they are additional.

## Fixture performance

The final code was compiled as the optimized `compass-languages` extraction
example. Each tool was warmed once per fixture and then invoked in five fresh
processes. Timings include process startup and JSON rendering.

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

## Real-corpus gate

The Bevy doctor had enough disk space but correctly refused to run while the
feature worktree was dirty:

```text
disk: 7,011,647,488 bytes free — pass
compass-worktree: Compass source checkout is dirty — fail
```

The official Bevy comparison must run from a clean commit. Until it passes,
this review claims only checked-in fixture quality and performance, not
real-repository qualification or complete universal Rust support.
