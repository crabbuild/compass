# Plan 007: Resolve the JSON/store search-normalization parity blocker

> This remediation is intentionally independent of Plan 001 because Plan 001
> stopped on this defect. It must preserve immutable historical snapshots and
> must not hide the failing parity case. After this plan is complete, rerun
> Plan 001; the broader analyzer and exact-first retrieval work remains in Plan
> 002.

## Status

- **Execution status**: `DONE` at commit `a76583bb`. The formerly ignored
  JSON/store parity qualification passes for accents, Unicode case, identifier
  separators, control misses, edge responses, and repeated execution. The
  repository-wide graph baseline still has the unrelated Markdown identity
  failure described in **Execution reconciliation** below.
- **Priority**: P1
- **Effort**: M
- **Risk**: HIGH
- **Depends on**: none; unblocks Plan 001
- **Planned at**: commit `64dcbf60`, 2026-08-07

## Problem and evidence

The Phase 1 parity qualification uses the existing compact graph fixture. For
the query `cafe`, JSON search returns the node named `café`, while the store
engine returns no result. This is not a ranking disagreement: SQLite FTS is
configured with `unicode61 remove_diacritics 2` in
`crates/compass-query/src/index.rs`, while immutable term postings are built by
`crates/compass-graph/src/snapshot.rs:3059-3064`, which only lowercases terms.
Typed query terms in `crates/compass-query/src/code_query.rs:1307-1334` also
only lowercase. The failing reproduction is the ignored test
`crates/compass-query/tests/relevance_qualification.rs`:
`backend_parity_subset_exposes_known_unicode_normalization_mismatch`.

The fix must make analysis portable and identical at index-build and query
time. It must not change public graph identities, invent fuzzy matches, or
silently reinterpret old immutable snapshots.

## Scope

In scope:

- `crates/compass-model/src/search.rs` and `crates/compass-model/src/lib.rs`
  for a small, pure, versioned term-normalization contract, if that is the
  lowest shared ownership boundary;
- `crates/compass-graph/src/snapshot.rs` for immutable term-posting creation;
- `crates/compass-store/src/lib.rs` for the shared snapshot-layout validator
  when the derived index layout is bumped;
- `crates/compass-query/src/code_query.rs` and `src/index.rs` for query/index
  analysis;
- relevant `compass-model`, `compass-graph`, and `compass-query` tests,
  including `tests/relevance_qualification.rs` and the existing
  `tests/store_engine.rs` lint-only fixture cleanup;
- `COMPATIBILITY.md`, `MIGRATION.md`, and
  `docs/implementation/query-engine.md` only when the snapshot/index version
  or rebuild behavior changes.

Out of scope:

- BM25/fuzzy/PageRank ranking, intent planning, traversal, CLI/MCP contracts;
- changing graph node/edge identities or source evidence;
- network/model/vector dependencies;
- rewriting historical snapshot objects in place;
- weakening or deleting the ignored parity reproduction.

## Required design

1. Define one analyzer version and one deterministic normalization function for
   Unicode case and combining-mark removal. Keep term length and query-byte
   limits bounded. Preserve exact raw identifier lookup semantics where the
   existing contract requires it; do not turn this remediation into broad
   typo-fuzzy retrieval.
2. Use that function for both immutable store term postings and query terms.
   JSON FTS may still use its accelerator, but the public candidate set and
   ordering must remain backend-neutral.
3. Treat existing store layouts as immutable. If the term encoding changes,
   bump the store graph-index layout/analyzer version or add an explicit
   rebuild/rejection path. A v2 snapshot must never be silently treated as a
   v3 snapshot.
4. Add table-driven parity cases for `café`/`cafe`, `résumé`/`resume`, Unicode
   case, snake/camel identifiers, and a control case that must not match. Run
   each case twice and compare ordered IDs, diagnostics, truncation, edge
   kind/direction, and serialized responses.
5. Remove only the `#[ignore]` marker from the Phase 1 parity test once the
   production fix and version/rebuild behavior are verified. The test must
   remain a real JSON/store differential test.

## Verification gates

Before every Cargo command verify `/Volumes/Workspace` is mounted and use a
checkout-specific target directory below `/Volumes/Workspace/crabbuild-target`.

```text
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/<checkout> cargo test -p compass-model --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/<checkout> cargo test -p compass-graph --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/<checkout> cargo test -p compass-query --test relevance_qualification --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/<checkout> cargo test -p compass-query --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/<checkout> cargo clippy -p compass-model -p compass-graph -p compass-query --all-targets --all-features --locked -- -D warnings
cargo fmt --all -- --check
git diff --check
```

The relevance parity test must pass without `--ignored`; no test may be made
green by accepting a backend-specific result. Verify old-layout rejection or
rebuild with a round-trip fixture and document the compatibility consequence.

## Stop conditions

Stop and report instead of improvising if the old snapshot format cannot be
rejected/rebuilt without violating historical immutability, if the portable
normalizer changes stable public result semantics beyond the documented parity
fix, if a dependency or network download is required, or if the same compact
fixture still produces different ordered semantic results after the fix.

## Completion evidence

- A versioned analyzer/normalizer is used at both posting-build and query time.
- The ignored Phase 1 parity test runs normally and passes for all cases.
- Existing v2 snapshots are either read compatibly with explicit dual behavior
  or rejected/rebuilt through a documented path; no historical object changes.
- Full targeted tests, Clippy, format, and diff checks pass.
- Plan 001 can be rerun with no ignored parity tests and can then add its
  qualification runner/docs without masking backend differences.

## Execution reconciliation

The remediation's touched paths have been independently verified: the
relevance qualification (including the formerly ignored parity test), graph
snapshot tests, package Clippy, formatting, and diff checks pass. The broader
`cargo test -p compass-graph --locked` gate is currently red only at the
pre-existing `tests/markdown_identity.rs` test
`repeated_markdown_headings_use_stable_hierarchical_identities`; no Markdown
identity source or test is changed by this plan. Its failure is recorded as a
baseline exception and must not be attributed to the search-normalization
patch. The full graph gate remains required for the repository baseline, while
the remediation can be checkpointed with this explicit exception and Plan 001
must remain blocked until the repository owner resolves or waives that
unrelated baseline failure.
