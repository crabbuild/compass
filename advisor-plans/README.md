# Compass enhancement advisor plans

Generated from deep product/code audits on 2026-07-23 and 2026-08-01.

Upstream snapshots:

- Compass: `3837b411197771351b387cff935e4ae1e0eb8750`
- Graphify `origin/main`: `91f4d120b630ee35c79bf3c75ccd186870a808f9`
- Graphify v0.9.20 release base: `edec9eabeceeae6aa2375eddb3835efa1a32c0a3`
- Graphify qualified R-support oracle: `de0806be7c95d97aa7ff40371a235da899d6edb0`

Graphify `origin/main` is a divergent v1 product line, not a newer commit on
the frozen v8 oracle's ancestry. Read
[`000-origin-main-audit.md`](000-origin-main-audit.md) before executing a plan.

Plans 006–012 are a second, self-contained sequence for native document and
text processing. They were planned against Compass commit `743a170` and a
read-only local inspection of Graphify v0.9.26 at `66d8110`. That comparison is
context only: the plans prohibit Graphify runtime, fixture, test, or fallback
dependencies. Each executor should read the selected plan in full; no audit or
conversation context is required beyond the plan file.

## Execution order and status

| Plan | Title | Priority | Effort | Depends on | Status |
|---|---|---:|---:|---|---|
| 001 | Make upstream compatibility lineage machine-checkable | P1 | M | — | DONE |
| 002 | Restore incoming evidence in directed wiki exports | P1 | S | 001 | DONE |
| 003 | Return structured provenance from path and discovery queries | P1 | M | 001 | TODO |
| 004 | Publish current outputs as one observable generation | P1 | L | 001 | TODO |
| 005 | Gate pull requests and releases on production qualification | P1 | M | 001 | TODO |
| 006 | Make media ingestion fail explicitly and remain memory-bounded | P1 | M | — | TODO |
| 007 | Introduce a versioned, provenance-preserving document artifact | P1 | L | 006 | TODO |
| 008 | Chunk rich documents losslessly and fuse structural with semantic evidence | P1 | L | 007 | TODO |
| 009 | Make Markdown and HTML first-class structural documents | P1 | L | 007, 008 | TODO |
| 010 | Preserve OOXML order and add native PPTX document graphs | P2 | L | 006, 007, 008 | TODO |
| 011 | Add a bounded native RTF decoder with explicit fidelity diagnostics | P2 | L | 007, 008 | TODO |
| 012 | Qualify document graphs across formats, limits, and determinism | P1 | M | 009, 010, 011 | TODO |

Status values: `TODO`, `IN PROGRESS`, `DONE`, `BLOCKED`, or `REJECTED`.

## Dependency notes

- Plan 001 establishes the exact upstream line and evidence vocabulary used by
  all later compatibility decisions.
- Plan 002 is deliberately small and should land before the broader structured
  evidence work in plan 003.
- Plans 003 and 004 are independent after plan 001.
- Plan 005 should consume the manifest and evidence targets introduced by plan
  001 rather than introducing another compatibility configuration.
- Plan 006 closes correctness and denial-of-service gaps before expanding the
  media surface.
- Plan 007 establishes the one typed document contract, locator vocabulary,
  completeness policy, and cache version used by every later format.
- Plan 008 makes that artifact losslessly sliceable and allows deterministic
  structure to coexist with optional provider semantics.
- Plans 009, 010, and 011 can proceed independently after their listed shared
  prerequisites. They own Markdown/HTML, OOXML/PPTX, and RTF respectively.
- Plan 012 runs last because it converts the actual shipped support of all
  three format plans into one stable qualification and documentation gate.
- Plan 012 complements plan 005 rather than depending on it: plan 005 owns the
  broad production release gate; plan 012 owns document-specific evidence.

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
- **Image OCR and slide-layout understanding:** valuable for scanned PDFs and
  diagram-heavy decks, but require a separate bounded media/provenance design;
  these plans deliberately cover native text and package evidence first.
- **Legacy binary Office formats (`.doc`, `.xls`, `.ppt`):** require a safe
  parser/conversion boundary distinct from OOXML and are not implied by plan
  010.
- **ODT/ODS/ODP and EPUB:** architecturally fit the document artifact after the
  core formats are qualified, but have their own package and semantic rules.

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
- Add Pandoc, LibreOffice, Word automation, or Python document libraries as a
  normal runtime fallback: rejected because it violates Compass's native,
  local-first, bounded, portable product boundary and weakens deterministic
  failure semantics.
- Keep flattening every document to one Markdown string: rejected because it
  discards order, hierarchy, locators, diagnostics, completeness, and stable
  block-level provenance needed for higher-quality graph construction.
- Suppress structural parsing whenever semantic extraction runs: rejected
  because deterministic evidence and provider-derived concepts answer different
  questions and should coexist under a proven source identity.
- Make Graphify a CI oracle for document support: rejected because qualification
  must be Compass-owned, offline, deterministic, and independent of another
  product's changing implementation and optional dependencies.
