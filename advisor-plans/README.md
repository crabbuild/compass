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

Plan 013 is a third, self-contained TypeScript/JavaScript code-graph quality
program. It was planned against Compass commit `a8a6a80` with read-only
diagnostic comparison to Graphify and official TypeScript/SCIP references.
Graphify remains comparison context only. The program preserves Compass's
native, compiler-free structural tier and makes any compiler/SCIP enrichment
explicit, optional, fresh, and provenance-preserving.

Plan 014 ships a typed pull-request risk review report and a reusable GitHub
Action. It consumes immutable history and semantic diff evidence while keeping
advisory risk separate from deterministic merge gates.

Plans 015–018 are self-contained notebook, PHP framework, execution-flow, and
MCP workflow programs planned at Compass commit `6680842c` on 2026-08-10.

Plan 019 is the Ruby universal-evidence program. It was planned at Compass
commit `b53c3ea2` on 2026-08-16. It freezes established Ruby evidence, builds an
independent Ripper oracle and qualification-only producer, adds conservative
Ruby project/resolution semantics, converts Rails to a universal framework
pack, performs one atomic hard cut, and then measures optimization and complete
quality gates. The pinned three-corpus audit now passes (89,981 accepted
relationships, 100% observed precision, 98.5567% recall); Ruby remains
`Qualifying` until a separate promotion decision.

Plan 020 is the Swift, Dart, Scala, and Groovy universal-evidence program. It
was planned at Compass commit `88abe4c0` on 2026-08-21. All four languages are
already recognized and have established extraction, so the program freezes
that behavior, builds independent source oracles and qualification-only
candidates, performs one atomic hard cut per language, preserves existing
Vapor/Dart/Play/Spock/Gradle behavior through evidence-backed boundaries, and
finishes with a mixed-language release gate. Swift, Dart, and Scala candidates
can proceed independently after the shared baseline; Groovy reuses Scala's
exact-language JVM boundary.
The production hard cut, deterministic fixture baselines, pinned manifests,
parser-backed source-oracle providers, audit builder, mixed fixture gate, and
three-corpus quality audits are implemented. The plan is `DONE`; all four
registry entries intentionally remain version-1 `Qualifying` until a separate
promotion decision. The mounted qualification target records the pinned
SwiftSyntax, Dart Analyzer, scala.meta, and Groovy CompilationUnit toolchains
and the immutable audit results.

Plan 021 hardens the React frontend framework graph after Plan 013's
TypeScript/JavaScript production hard cut. It adds occurrence-preserving render
evidence, conservative component and runtime-boundary roles, deep Next.js and
TanStack route semantics, parsed Vite configuration, and agent-facing context
and impact workflows. The seven-family corpus and independent scorecards are
checked in, but the last pinned release artifact predates the current branch
revision; the exact release gate must be rerun before promotion. TanStack Start
remains explicitly pre-stable and is not promoted by aggregate results.

Plan 022 extends the native document program with selective, local OCR for
scanned PDF pages and images embedded in DOCX, PPTX, and XLSX. Native package
and PDF text remains authoritative. OCR is off by default, model/profile and
geometry provenance are explicit, model installation is separate from
extraction, and the recommended engine must beat a pinned baseline on a
Compass-owned corpus before support or quality claims ship.

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
| 009 | Make Markdown and HTML first-class structural documents | P1 | L | 007, 008 | DONE |
| 010 | Preserve OOXML order and add native PPTX document graphs | P2 | L | 006, 007, 008 | TODO |
| 011 | Add a bounded native RTF decoder with explicit fidelity diagnostics | P2 | L | 007, 008 | TODO |
| 012 | Qualify document graphs across formats, limits, and determinism | P1 | M | 009, 010, 011 | TODO |
| 013 | Make TypeScript and JavaScript code graphs best in class | P1 | XL | —; final gate should consume 005 or equivalent | IN PROGRESS |
| 014 | Ship typed pull-request risk review and a reusable GitHub Action | P1 | L | Immutable history and semantic diff; coordinate with Compass Guard | DONE |
| 015 | Add bounded Jupyter and Databricks notebook extraction | P1 | L | — | TODO |
| 016 | Complete Composer, Blade, and Eloquent framework resolution | P1 | L | — | TODO |
| 017 | Derive bounded, ranked execution flows from entry points | P2 | L | Existing universal call graph | TODO |
| 018 | Expose five native MCP workflow prompts | P2 | M | — | TODO |
| 019 | Hard-cut Ruby to a qualifying universal evidence pipeline | P1 | XL | —; final gate should consume 005 or equivalent | IN PROGRESS |
| 020 | Hard-cut Swift, Dart, Scala, and Groovy to universal evidence | P1 | XXL | —; final gate should consume 005 or equivalent | DONE |
| 021 | Make React frontend framework graphs enterprise-ready | P1 | XXL | 013 production hard cut; final gate should consume 005 or equivalent | IN PROGRESS |
| 022 | Add bounded, quality-gated OCR to document processing | P1 | XL | 006, 007, 008, 010 | IN PROGRESS |

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
- Plan 013 is deliberately staged: independent truth and project semantics land
  before a test-only universal evidence pipeline, production changes in one hard cut, and
  compiler/framework enhancements follow only after the native graph qualifies.
  Its final public claim should consume plan 005's exact-production-evidence
  model or an equivalent release-candidate gate.
- Plan 014 consumes immutable history and semantic diff evidence, preserves the
  boundary between advisory risk and deterministic gates, and ships the
  reusable GitHub review Action.
- Plans 015 and 016 are independent language/framework enrichments. Plan 017
  can consume their facts later but does not depend on them. Plan 018 is an
  independent MCP/DX addition.
- Plan 019 is deliberately staged: established behavior and independent truth
  are frozen first; identity precedes extraction; the emitter, resolver, and
  Rails pack stay qualification-only until one atomic production hard cut;
  optimization follows semantic parity; and complete promotion remains gated
  by the 2,000-record quality audit.
- Plan 020 is one program with four independent language tracks. Phase 0
  freezes shared baselines and independent truth. Swift, Dart, and Scala
  candidates may then proceed in parallel; Groovy may also proceed but must
  reuse the exact-language JVM boundary established for Scala. Each language
  has a separate candidate and atomic hard-cut phase, and the mixed-language
  release gate runs only after all four cuts.
- Plan 021 builds on Plan 013's hard-cut TypeScript/JavaScript evidence rather
  than creating another parser path. It lands the public graph vocabulary and
  universal framework substrate first, then qualifies React, Next.js,
  TanStack, React Router/Remix, and Vite independently. Its final claim should
consume Plan 005's exact-production-evidence model or an equivalent gate.

- Plan 022 starts only after the document safety, artifact, slicing/fusion, and
  OOXML plans. It adds a separate OCR qualification gate rather than weakening
  Plan 012's credential-free native document gate. Plan 012 and Plan 022 may
  reuse fixture-manifest infrastructure, but neither may make OCR models or
helper runtimes prerequisites for native document support.

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
- **Generative slide-layout, chart, and formula understanding:** Plan 022 now
  owns bounded text OCR and geometry for scanned pages and embedded images.
  Generative VLM interpretation remains deferred because it needs separate
  inferred-evidence, hallucination, cost, and qualification contracts.
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
