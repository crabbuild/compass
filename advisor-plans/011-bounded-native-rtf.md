# Plan 011: Add a bounded native RTF decoder with explicit fidelity diagnostics

> **Executor instructions**: Follow every step and verification in order. Do
> not replace the native decoder with a system application or subprocess. Stop
> and report when a STOP condition applies. On completion, update the plan row
> in `advisor-plans/README.md` unless a reviewer owns the index.
>
> **Drift check (run first)**:
> `git diff --stat 743a170..HEAD -- crates/compass-media crates/compass-files crates/compass-semantic crates/compass-core docs CHANGELOG.md COMPATIBILITY.md Cargo.toml Cargo.lock`
> Plans 007 and 008 must be complete. Preserve all pre-existing user changes.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `advisor-plans/007-versioned-document-artifact.md`, `advisor-plans/008-lossless-document-fusion.md`
- **Category**: direction, security
- **Planned at**: commit `743a170`, 2026-08-01

## Why this matters

RTF is common in legal, medical, government, and legacy knowledge collections,
but its group state, destinations, binary escapes, code pages, and embedded
objects make regex extraction unsafe and incorrect. Compass can support useful
text, hierarchy, tables, fields, and provenance with a small streaming state
machine, provided it caps every attacker-controlled count and openly marks
unsupported fidelity. This expands local-first coverage without LibreOffice,
Word automation, Python, or network services.

## Current state

- `.rtf` is not a supported decoder in `compass-media` and is not in the
  semantic document extension list in `crates/compass-files/src/detect.rs`.
- Plan 007 defines `DocumentArtifact`, logical/exact locators, diagnostics,
  completeness, and shared limits. Plan 008 makes every artifact sliceable and
  fuses deterministic structure with optional semantic evidence.
- Compass forbids unbounded work and requires a limit error to remain distinct
  from an empty result. Inputs and diagnostic text are untrusted.
- Root `Cargo.toml` does not currently advertise an RTF subsystem. Prefer a
  focused internal parser; add an encoding dependency only after license,
  unsafe/transitive, and workspace policy review.

## Supported subset and safety contract

Support these constructs in the first release:

- groups, escaped braces/backslashes, control symbols, and control words;
- ANSI text and `\'hh` escapes under declared/common Windows code pages;
- signed UTF-16 `\uN` values with checked `\ucN` fallback skipping, including
  surrogate-pair handling;
- paragraphs/line breaks/tabs, basic heading styles, lists, and simple tables;
- `HYPERLINK` fields as inert link evidence;
- bounded document metadata from the `info` destination;
- exact raw-byte ranges for tokens/blocks when they are provable.

Skip these destinations safely and mark `complete=false` when content-bearing:
font/color/style tables after extracting needed decoding/style facts, pictures,
objects, data stores, files, themes, XML, drawing payloads, and unknown starred
destinations. Never execute, decompress, open, or emit embedded payload bytes.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Media | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-media --locked` | exit 0 |
| Files | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-files --locked` | exit 0 |
| Semantic | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-semantic --locked` | exit 0 |
| Core | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-core --locked` | exit 0 |
| Dependency policy | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo tree -p compass-media --locked` | exit 0; review only intended dependency changes |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-media -p compass-files -p compass-semantic -p compass-core --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Boundary | `sh scripts/check_product_boundary.sh` | exit 0 |

## Scope

**In scope**:

- `crates/compass-media/src/rtf.rs` (create)
- `crates/compass-media/src/lib.rs`, document/limits modules
- `crates/compass-media/tests/rtf_contract.rs` and reviewable text fixtures
- `crates/compass-media/Cargo.toml`, `Cargo.toml`, `Cargo.lock` only if a vetted
  text-encoding dependency is required
- `crates/compass-files/src/detect.rs` and tests
- focused semantic/core integration tests
- `docs/design/document-processing.md`, document-format reference,
  `docs/design/security-and-privacy.md`, `COMPATIBILITY.md`, `CHANGELOG.md`

**Out of scope**:

- Legacy Word `.doc`, Windows COM/Word automation, LibreOffice, or Pandoc.
- Rendering layout, fonts, colors, images, equations, OLE, or drawing objects.
- Executing RTF fields or opening file/network references.
- Lossy “best effort” that returns `complete=true` after skipped content.
- A general-purpose public RTF library; keep the parser owned by media needs.

## Git workflow

- Suggested branch: `advisor/011-bounded-native-rtf`
- Commit lexer/state/limits, text decoding, blocks/fields, integration/docs.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Lock limits and hostile grammar cases before parsing

Define named ceilings in the shared limits module, starting with:

- group nesting depth: 256;
- total tokens/control words: 1,000,000;
- control-word length: 64 ASCII letters;
- numeric parameter digits: 12 with checked signed conversion;
- destinations: 100,000;
- output text: the shared `OFFICE_MAX_TEXT_CHARS` or plan-007 equivalent;
- `\binN` skip length: remaining input and raw document size, checked before
  advancing;
- metadata fields/links/blocks: shared artifact limits.

Write failing tests for excessive nesting, unmatched braces, huge numeric
parameters, negative/overflow `\bin`, truncated hex escapes, token flood,
output amplification, malicious object/picture payloads, and exact-limit
success. Tests must assert typed `Rejected`/`Parse` errors, never panic/OOM.

**Verify**: media tests fail for missing RTF implementation, not malformed test
fixtures.

### Step 2: Implement a streaming lexer and group-state stack

Parse bounded bytes in one forward pass with an explicit state stack. Each
group state tracks destination, ignorable flag, Unicode fallback count,
encoding/code page, paragraph/style state, and pending field context. Use
checked arithmetic for every index/length and consume at least one byte per
loop iteration.

Recognize literal text, `{`, `}`, escaped literal symbols, control symbols,
control words with optional signed parameters/delimiters, hex escapes, and
`\binN`. `\binN` bytes must be skipped without tokenizing or copying. Unknown
control words are ignored per RTF semantics; unknown starred destinations are
skipped as a group and diagnosed when content-bearing.

Never store an entire skipped destination or an unbounded token stream. Keep
only bounded state plus normalized output/artifact builders.

**Verify**: lexer tests cover byte progress, nested state restore, starred
destinations, binary data containing braces/backslashes, and all limits.

### Step 3: Decode Unicode and code pages deterministically

Implement `\uN` as signed 16-bit code units, honoring the current bounded
`\ucN` fallback count. Combine valid surrogate pairs; emit U+FFFD plus a stable
diagnostic for unpaired values. Decode `\'hh` and literal high bytes under the
declared `\ansicpgN`/known charset. Support at minimum ASCII/UTF-8-safe input
and Windows-1252; unsupported code pages must use a documented deterministic
replacement policy with `complete=false`, never the host locale.

If a new encoding crate is proposed, inspect its license, default features,
unsafe/transitive footprint, and no-network behavior. Add it at the root as a
workspace dependency. Do not write a large ad hoc code-page table silently.

**Verify**: tests cover ASCII, Windows-1252 punctuation, hex escapes, Unicode
fallback bytes, surrogate pairs, unpaired surrogates, unsupported code page,
and identical results under different process locales.

### Step 4: Emit ordered blocks, tables, fields, and diagnostics

Map paragraph boundaries and explicit line/tab controls into ordered artifact
blocks while preserving exact token byte ranges. Support heading evidence from
recognized styles/outline controls only when the style definition is proven.
Represent list paragraphs and simple `\trowd`/`\cell`/`\row` tables without
inventing missing cells or nesting.

Parse `FIELD` instruction/result destinations only enough to recognize a
strictly bounded, quoted/unquoted `HYPERLINK` target. Emit an inert link from the
containing block. Other fields retain visible result text and a diagnostic;
they are never executed. Extract bounded `title`, `subject`, `author`,
`keywords`, and creation/modification timestamps from `info` as metadata,
without trusting them as configuration.

Set `complete=false` for skipped content-bearing destinations, unsupported
encodings/styles, or recoverable malformed structures. A broken group stack,
limit breach, or incoherent byte stream remains a hard error.

**Verify**: fixtures cover paragraphs, headings, nested lists, a simple table,
hyperlinks, field result text, metadata, skipped picture/object, and malformed
but recoverable input. Assert ordering, occurrences, byte ranges, link source,
diagnostics, and completeness.

### Step 5: Integrate detection, semantic slicing, graph projection, and cache

Add `.rtf` detection only after the decoder and negative tests pass. Route RTF
through `decode_document`, plan-008 slicing/retry, and the shared deterministic
projector. Increment `DOCUMENT_NORMALIZER_VERSION` so old caches miss.

Add an integration fixture above one semantic request cap. Fake-provider input
must contain first/middle/last sentinels exactly once across slices. With
semantic mode enabled, the graph must retain RTF blocks/links and add semantic
evidence without replacing structure.

**Verify**: files, semantic, and core tests exit 0; malformed RTF is incomplete
or an explicit error and never a successful empty document.

### Step 6: Publish the fidelity and security boundary

Update format-reference and security docs with the supported subset, exact
limits, `complete` policy, code-page behavior, embedded-object policy, and no
layout fidelity claim. Add changelog/compatibility notes. Do not market RTF as
fully supported while unsupported content yields diagnostics.

Run all commands in the table and `git diff --check`.

## Test plan

- Lexer progress, grammar, state restoration, malformed groups, and every limit.
- Unicode/code-page/fallback/locale independence.
- Ordered paragraph/list/table/field/link/metadata artifacts.
- Embedded destinations remain inert and bounded.
- Semantic lossless slicing and structural-plus-semantic graph integration.
- Repeated and relocated-root runs serialize identically.

## Done criteria

- [ ] RTF parsing is native, single-pass/bounded, and locale-independent.
- [ ] Unicode fallback, code pages, binary skips, and group restoration are tested.
- [ ] Embedded objects/fields never execute or cause external reads.
- [ ] Unsupported fidelity is represented by diagnostics and `complete=false`.
- [ ] Detection, graph projection, semantic slicing, and cache version are wired.
- [ ] Targeted tests, dependency review, lint, format, boundary, and diff checks pass.
- [ ] Plan 011 is marked `DONE` in `advisor-plans/README.md`.

## STOP conditions

Stop and report if:

- plans 007–008 are incomplete;
- required fidelity depends on executing Word/LibreOffice or embedded fields;
- the parser cannot guarantee forward progress or bounded stack/token/output;
- a dependency adds runtime network/process requirements or violates license/
  unsafe policy without explicit approval;
- skipped content would need to be reported as `complete=true`;
- stable graph identity changes without compatibility approval;
- an in-scope user change cannot be preserved.

## Maintenance notes

RTF's permissive grammar invites accidental scope growth. Add a malicious and
normal fixture for every newly recognized destination/control word, document
whether it affects completeness, and keep binary/media handling outside the
text decoder until an explicit bounded media design exists.
