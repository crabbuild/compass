# Compass enhancement advisor plans

Generated from a deep comparison on 2026-07-23.

Upstream snapshots:

- Compass: `3837b411197771351b387cff935e4ae1e0eb8750`
- Graphify `origin/main`: `91f4d120b630ee35c79bf3c75ccd186870a808f9`
- Graphify v0.9.20 release base: `edec9eabeceeae6aa2375eddb3835efa1a32c0a3`
- Graphify qualified R-support oracle: `de0806be7c95d97aa7ff40371a235da899d6edb0`

Graphify `origin/main` is a divergent v1 product line, not a newer commit on
the frozen v8 oracle's ancestry. Read
[`000-origin-main-audit.md`](000-origin-main-audit.md) before executing a plan.

## Execution order and status

| Plan | Title | Priority | Effort | Depends on | Status |
|---|---|---:|---:|---|---|
| 001 | Make upstream compatibility lineage machine-checkable | P1 | M | — | DONE |
| 002 | Restore incoming evidence in directed wiki exports | P1 | S | 001 | DONE |
| 003 | Return structured provenance from path and discovery queries | P1 | M | 001 | TODO |
| 004 | Publish current outputs as one observable generation | P1 | L | 001 | TODO |
| 005 | Gate pull requests and releases on production qualification | P1 | M | 001 | TODO |

Status values: `TODO`, `IN PROGRESS`, `DONE`, `BLOCKED`, or `REJECTED`.

## Dependency notes

- Plan 001 establishes the exact upstream line and evidence vocabulary used by
  all later compatibility decisions.
- Plan 002 is deliberately small and should land before the broader structured
  evidence work in plan 003.
- Plans 003 and 004 are independent after plan 001.
- Plan 005 should consume the manifest and evidence targets introduced by plan
  001 rather than introducing another compatibility configuration.

## Direction options not promoted to implementation plans

- **First-class hyperedge queries:** high architectural adjacency, but the
  identity, role, history, and CompassQL semantics need an approved design
  before implementation.
- **Versioned mixed-corpus workspace profile:** Graphify main leads with this
  workflow and Compass already has most primitives. Product priority and
  provider/cost defaults need maintainer approval.
- **Natural-query token/trigram index:** high-confidence performance
  opportunity, but benchmark scale curves should establish priority after the
  release qualification gate is trustworthy.
- **Pluggable Leiden-quality community engine:** feature parity is incomplete,
  but expected user value must be proven with modularity, connectivity,
  stability, latency, and memory measurements.
- **Linux and Windows release artifacts:** clear distribution gap; deferred
  only to keep this first plan set at five items.

## Findings considered and rejected

- Reimplement Graphify main's Python pipeline architecture: rejected because
  Compass's typed native workspace, deterministic indexes, bounded CompassQL,
  immutable history, and safety limits are deeper Modules with stronger
  Interfaces.
- Treat all 74 failures from running `compass-parity` against Graphify main as
  Compass regressions: rejected. The suite is designed for the v8 lineage and
  many failures are missing v8 fixtures, commands, or modules on the divergent
  main branch.
- Duplicate Graphify main's hyperedge storage and shaded HTML: rejected because
  Compass already preserves, reports, visualizes, and versions hyperedges.
- Replace Compass watch and hook implementations with Graphify main's versions:
  rejected because focused Compass tests pass and the native implementation
  already handles custom hook paths, safe managed blocks, background refresh,
  history, and external SCIP changes.
