# Plan 010: Preserve OOXML order and add native PPTX document graphs

> **Executor instructions**: Read this entire plan before editing. Execute each
> step and verification in order. Stop on a listed condition; do not substitute
> a subprocess converter. Update `advisor-plans/README.md` when complete unless
> a reviewer owns status updates.
>
> **Drift check (run first)**:
> `git diff --stat 743a170..HEAD -- crates/compass-media crates/compass-files crates/compass-semantic crates/compass-core fixtures docs CHANGELOG.md COMPATIBILITY.md Cargo.toml Cargo.lock`
> Plans 006–008 must be complete. Reconcile pre-existing user edits carefully.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `advisor-plans/006-harden-media-ingestion.md`, `advisor-plans/007-versioned-document-artifact.md`, `advisor-plans/008-lossless-document-fusion.md`
- **Category**: direction
- **Planned at**: commit `743a170`, 2026-08-01

## Why this matters

The current DOCX decoder collects paragraphs and tables in separate passes, so
it changes source order; XLSX expands sparse coordinates into rows of strings;
and PPTX is not classified as a semantic document. OOXML already contains rich
ordering, hierarchy, relationships, notes, and stable logical locations. A
bounded native decoder can preserve that evidence locally and deterministically
without LibreOffice, Python, credentials, or a model.

## Current state

- `crates/compass-media/src/lib.rs:74-112` reads DOCX paragraphs and tables
  separately and concatenates them, losing interleaving.
- The same file parses XLSX shared strings/sheets into Markdown-like rows;
  plan 006 adds coordinate/cell/text bounds.
- `crates/compass-files/src/detect.rs` lists `docx` and `xlsx` in semantic text
  extensions but not `pptx`.
- Current archive validation caps raw, archive, member, and compression-ratio
  sizes. Retain all limits and the explicit rejection policy.
- Plans 007–008 provide ordered artifact blocks, package locators, diagnostics,
  lossless semantic slices, and graph fusion.
- OOXML relationship targets are untrusted. Internal parts must remain within
  the package; external targets are link evidence only and must never be read.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Media | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-media --locked` | exit 0 |
| Files | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-files --locked` | exit 0 |
| Semantic | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-semantic --locked` | exit 0 |
| Core | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-core --locked` | exit 0 |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-media -p compass-files -p compass-semantic -p compass-core --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Boundary | `sh scripts/check_product_boundary.sh` | exit 0 |

## Scope

**In scope**:

- `crates/compass-media/src/lib.rs`
- `crates/compass-media/src/ooxml/mod.rs` (create)
- `crates/compass-media/src/ooxml/package.rs` (create)
- `crates/compass-media/src/ooxml/docx.rs` (create)
- `crates/compass-media/src/ooxml/xlsx.rs` (create)
- `crates/compass-media/src/ooxml/pptx.rs` (create)
- `crates/compass-media/tests/ooxml_contract.rs` and bounded XML/ZIP fixtures
- `crates/compass-media/Cargo.toml`, root dependency files only if required
- `crates/compass-files/src/detect.rs` and tests
- focused semantic/core integration tests
- document-processing design/reference docs, `COMPATIBILITY.md`, `CHANGELOG.md`

**Out of scope**:

- Legacy binary `.doc`, `.xls`, or `.ppt`.
- Macros, embedded OLE objects, ActiveX, media transcription, or image OCR.
- Executing formulas, macros, field code, external relationships, or links.
- Pixel-perfect slide rendering or presentation visual layout reconstruction.
- LibreOffice/Pandoc subprocesses, Python packages, or Graphify dependencies.
- Editing existing binary fixtures with opaque GUI-generated noise; prefer
  reviewable XML parts assembled into ZIPs by Rust test helpers.

## Git workflow

- Suggested branch: `advisor/010-native-ooxml-documents`
- Commit common package reader, DOCX, XLSX, PPTX, then integration/docs.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Build one bounded OPC package reader

Before editing, read `compass-media`'s `src/lib.rs`, `Cargo.toml`, and tests.
Move common ZIP/OOXML behavior into `ooxml/package.rs` without weakening plan
006 limits. The reader must:

- validate central-directory counts, member sizes, aggregate expansion, and
  compression ratios before allocating output;
- reject duplicate normalized member names;
- normalize `/`, `.`, and percent-independent package components and reject
  absolute or parent-escaping targets;
- parse `[Content_Types].xml` and `.rels` with bounded XML events/depth/text;
- distinguish `TargetMode="External"` and return it as inert link evidence;
- read any package part at most once per decoder, with a deterministic cache;
- reject missing required parts explicitly and diagnose optional missing parts.

Add hostile tests for duplicates, `../` targets, absolute targets, oversized
XML depth/event counts, external relationships, and ZIP amplification.

**Verify**: media tests exit 0; no decoder accesses ZIP members directly except
through the package reader.

### Step 2: Rebuild DOCX in body order

Walk `word/document.xml` body children in document order. Emit:

- paragraphs with preserved run text, tabs, explicit line/page breaks, and
  significant whitespace;
- headings from paragraph styles/outline levels;
- nested list/item evidence from `numbering.xml` without inventing sequence
  values when definitions are missing;
- tables at their exact interleaved position, with rows/cells and merged-cell
  metadata;
- hyperlinks resolved through relationships as inert internal/external links;
- footnotes, endnotes, comments, headers, and footers as separate located
  blocks when present and supported;
- diagnostics plus `complete=false` for bounded unsupported content.

Use package-part plus paragraph/table/run occurrence locators. Escape Markdown
only in the compatibility renderer. Do not expose raw relationship IDs as
stable user identity.

**Verify**: a fixture `paragraph → table → paragraph` produces that exact block
order; repeated equal paragraphs remain separate; pipes/backslashes survive IR
round-trip and render escaped.

### Step 3: Rebuild XLSX as a sparse, typed sheet artifact

Preserve workbook sheet order and actual row/cell coordinates without resizing
to attacker-declared widths. Parse bounded shared strings, inline strings,
booleans, errors, ISO dates where explicitly typed, cached formula values, and
formula text as separate evidence. Never execute a formula.

Represent sheets, rows, and non-empty cells with `Spreadsheet` locators.
Preserve merged ranges, hyperlinks, and named tables when valid. Missing shared
string indexes, invalid ranges, and unsupported cell types produce typed
diagnostics or errors according to whether extraction can remain coherent.
Keep plan 006's column/row/cell/text ceilings.

**Verify**: sparse `A1` and `XFD100000` do not allocate intermediate cells;
formula/error/inline/shared values remain distinguishable; sheet order and
serialization are deterministic.

### Step 4: Add native PPTX decoding

Detect `.pptx` as a supported semantic/document extension. Parse
`ppt/presentation.xml` and relationships to determine slide order; never infer
order from filenames. For each slide, walk shape tree order and emit:

- slide block and optional title;
- text shapes with paragraph/list hierarchy;
- tables with rows/cells;
- shape name, placeholder role, and alt text as bounded metadata;
- internal/external hyperlinks;
- speaker notes linked to the owning slide;
- diagnostics for charts, SmartArt, equations, animations, or media that are
  present but not structurally decoded.

Use slide number plus shape/tree occurrence logical locators. Do not claim
pixel geometry or reading order beyond XML/tree/placeholder evidence; if
multiple plausible visual orders exist, preserve tree order and diagnose the
limitation.

**Verify**: a three-slide fixture with non-lexical relationship targets follows
presentation order, includes notes and a table, skips an embedded object
safely, and reaches semantic fixture requests through plan 008 slices.

### Step 5: Integrate graph projection, cache version, and compatibility

Route all three formats through the plan-007 artifact and plan-008 structural
plus semantic flow. Increment `DOCUMENT_NORMALIZER_VERSION`, making old
semantic caches miss. Add cold/cached/reordered-ZIP-member tests: identical
logical packages with different ZIP member order/time metadata must publish
identical graphs and cache fingerprints based on source bytes plus normalizer
contract as documented.

If raw source-byte digest intentionally changes for byte-different equivalent
packages, graph output must still be identical; do not claim cache equivalence
unless canonical package hashing is separately designed and tested.

**Verify**: media/files/semantic/core tests exit 0; graph records contain no
absolute paths or ZIP metadata timestamps.

### Step 6: Document exact support and run gates

Update the format matrix with supported/partial/unsupported constructs for
DOCX, XLSX, and PPTX; locator guarantees; formulas/macros/external-link safety;
and no-rendering/OCR boundary. Add changelog and compatibility notes. Add a
migration note only if a user-facing option or schema requires action.

Run every command in the table and `git diff --check`.

## Test plan

- OPC containment, duplicate, external relationship, XML and ZIP limit tests.
- DOCX body order, headings/lists/tables/links/notes/repeated text/escaping.
- XLSX sparse extremes, typed values, formulas, merged ranges, invalid indexes.
- PPTX relationship-defined slide order, shapes, tables, notes, links, alt text,
  unsupported objects, and semantic slicing.
- Determinism across ZIP member ordering, repeated runs, and relocated roots.
- Malformed and over-limit packages never become empty successful documents.

## Done criteria

- [ ] All OOXML access goes through one bounded, containment-safe reader.
- [ ] DOCX block order matches document body order.
- [ ] XLSX remains sparse and never executes formulas.
- [ ] PPTX is natively detected, structurally extracted, and semantically sliced.
- [ ] External links/objects remain inert evidence.
- [ ] Normalizer/cache version and documentation are updated.
- [ ] Targeted tests, lint, format, boundary, and diff checks pass.
- [ ] Plan 010 is marked `DONE` in `advisor-plans/README.md`.

## STOP conditions

Stop and report if:

- plans 006–008 are incomplete;
- OOXML parsing would require an unbounded DOM or unchecked archive read;
- a proposed decoder needs to execute fields, formulas, macros, or objects;
- PPTX visual reading order would be presented as exact without evidence;
- a new dependency introduces runtime binaries, unsafe code in Compass, or
  network access and no bounded native alternative has been reviewed;
- public stable IDs must change without compatibility approval;
- an in-scope user edit cannot be preserved.

## Maintenance notes

Treat OOXML as an untrusted relationship graph, not merely a ZIP of XML.
Future chart/image/OCR support should add explicit evidence types and limits;
it must not overload text blocks or silently turn unsupported content into
complete extraction.
