# Plan 008: Chunk rich documents losslessly and fuse structural with semantic evidence

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If a STOP condition occurs, stop and report. Update this plan's
> status row in `advisor-plans/README.md` when complete unless instructed not to.
>
> **Drift check (run first)**:
> `git diff --stat 743a170..HEAD -- crates/compass-semantic crates/compass-core crates/compass-graph crates/compass-media crates/compass-model docs/design CHANGELOG.md`
> Plan 007 must already be complete. If the live `DocumentArtifact` contract
> differs from the assumptions below, stop and revise this plan with a reviewer.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `advisor-plans/007-versioned-document-artifact.md`
- **Category**: bug, perf, direction
- **Planned at**: commit `743a170`, 2026-08-01

## Why this matters

Rich documents currently bypass normal source slicing: their raw ZIP/PDF byte
size is used for planning, then extracted text is capped at 20,000 characters.
Large documents therefore lose content silently and cannot be retried in
smaller units. At the same time, `compass-core` suppresses deterministic
Markdown structure whenever semantic extraction covers the file. This plan
makes normalized document text gap-free and sliceable, then publishes native
structure and provider semantics together instead of forcing a choice.

## Current state

- `crates/compass-semantic/src/lib.rs:742-838` defines `SemanticUnit::File` and
  `SemanticUnit::Slice`; rich media is extracted only while reading a file.
- `crates/compass-semantic/src/orchestration.rs:94-138` slices using source-file
  character estimates before DOCX/XLSX/PDF normalization.
- The same reader truncates loaded content at roughly 20,000 characters.
- `crates/compass-core/src/pipeline.rs:484-510` builds a
  `semantic_documents` set and excludes those paths from language extraction.
- `crates/compass-graph/src/lib.rs:456-478` has `doc_twin_remap`, a heuristic
  that recognizes semantic document twins by a `_doc` suffix.
- The artifact from plan 007 provides ordered blocks, locators, completeness,
  diagnostics, `DOCUMENT_SCHEMA`, and `DOCUMENT_NORMALIZER_VERSION`.
- Preserve relationship direction, multiplicity, source anchors, provider
  provenance, deterministic order, and explicit ambiguity.

## Target flow

```text
bounded bytes
  -> validated DocumentArtifact (once)
  -> deterministic structural projection
  -> lossless DocumentSlice sequence
  -> bounded semantic requests/retries
  -> semantic nodes linked to proven document/block identities
  -> one graph containing both evidence classes
```

A slice sequence is lossless when concatenating slice text in ordinal order
reconstructs the exact normalized semantic text, including separators. Empty
or repeated blocks must not disappear. Slices should prefer artifact block
boundaries and split an oversized block only at checked UTF-8 character
boundaries.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Semantic | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-semantic --locked` | exit 0 |
| Core | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-core --locked` | exit 0 |
| Graph | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-graph --locked` | exit 0 |
| CLI contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-cli --test compass_product --locked` | exit 0 |
| Qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main ./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-semantic -p compass-core -p compass-graph --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |

Stop if the external target volume is unavailable; never build into the repo.

## Scope

**In scope**:

- `crates/compass-semantic/src/lib.rs`
- `crates/compass-semantic/src/orchestration.rs`
- focused semantic tests under `crates/compass-semantic/src/` and `tests/`
- `crates/compass-core/src/pipeline.rs` and its focused tests
- `crates/compass-graph/src/lib.rs` and graph publication tests
- `crates/compass-media/src/document.rs` only for a deterministic semantic
  rendering/slice helper missing from plan 007
- `crates/compass-model` only if an existing provenance field must accept a new
  typed value without changing schema meaning
- `docs/design/document-processing.md`, `COMPATIBILITY.md`, `CHANGELOG.md`

**Out of scope**:

- Improving format parsing; plans 009–011 own extractors.
- Changing provider protocols or adding a provider.
- Embeddings, vector databases, natural-language querying, or Graphify.
- Deduplicating two nodes merely because their labels/text are similar.
- Altering stable graph identity without a compatibility review and migration.

## Git workflow

- Suggested branch: `advisor/008-lossless-document-fusion`
- Prefer commits for prepared corpus/slices, retry behavior, graph fusion, docs.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Specify lossless prepared-document units

Extend `SemanticUnit` with a document-backed unit or introduce a nearby
`PreparedSemanticUnit`. It must carry:

- canonical relative source path;
- shared normalized content or an owned bounded slice (avoid N full copies);
- checked start/end character offsets in normalized content;
- first/last artifact block ordinal and locators;
- slice ordinal and total count;
- document schema/normalizer fingerprint;
- completeness/diagnostics needed by orchestration.

Write failing tests for a synthetic artifact containing headings, an empty
paragraph, repeated paragraphs, a table, a Unicode paragraph, and a single
oversized block. Assert concatenation reconstructs normalized text exactly,
all offsets lie on UTF-8 boundaries, every block occurrence is covered once,
and repeated text remains distinct by ordinal/locator.

**Verify**: semantic tests fail only because document-preparation behavior is
not implemented.

### Step 2: Decode before packing and slice by normalized content

Introduce an explicit preparation phase before the current request packer:

1. read each file once with existing bounded readers;
2. decode rich documents to `DocumentArtifact` once;
3. derive normalized semantic text plus block-boundary indexes;
4. calculate request size from that normalized content;
5. emit gap-free slices under `FILE_CHAR_CAP`/request limits.

Replace or wrap `expand_oversized_semantic_files`; callers must no longer use
compressed/raw byte length as the rich-document text estimate. Retain the
existing plain-text fast path if it produces the same unit contract. No unit
may truncate at 20,000 characters. A hard size/parse error remains incomplete
and must not be cached as success.

**Verify**: add a synthetic rich document over 20,000 characters with unique
sentinels near the beginning, middle, and end. Provider-fixture requests must
contain every sentinel exactly once across ordered units.

### Step 3: Make retries bisect document slices

Route oversized-provider responses through the same checked `bisect_slice`
policy used for text. Bisect only within the failed prepared unit; preserve its
parent source, locator range, schema fingerprint, and deterministic left/right
order. Stop at the existing minimum size and surface an explicit partial
result if the provider still rejects it.

Cache only a fully completed set for the document normalizer/prompt namespace.
Partial retry results may contribute to the current response under existing
policy but must remain marked partial and must not masquerade as a finalized
document cache.

**Verify**: a fake provider that rejects above N characters causes a single
document unit to bisect deterministically and eventually covers every sentinel;
a provider that always rejects returns one bounded failure without a loop.

### Step 4: Publish deterministic structure even with semantic coverage

In `compass-core/src/pipeline.rs`, remove the exclusion that prevents normal
language extraction for paths in `semantic_documents`. Keep semantic refresh
selection logic, but always run a registered deterministic extractor for the
same file. Add a regression test using Markdown: with semantic mode enabled,
the graph must contain the document/heading/link structure and semantic
concepts/relationships.

This step must not double-count a source file in discovery statistics or
create two canonical file resources. Make the canonical relative path the
join key; do not use display labels.

**Verify**: core tests assert one source resource, structural headings present,
semantic nodes present, and stable output across two identical builds.

### Step 5: Replace suffix-based twin merging with evidence-based fusion

Refactor `doc_twin_remap` in `compass-graph`. A semantic document root may map
to a structural document root only when all of these are true:

- canonical relative source identity is equal;
- provenance identifies one node as semantic and one as deterministic
  structure;
- the candidate is unique in both directions;
- document schema/normalizer evidence is compatible.

Do not use `_doc`, labels, text similarity, or first-candidate selection.
Ambiguous candidates remain separate and emit/preserve existing unresolved
evidence instead of an invented merge. Preserve semantic provider provenance
on remapped nodes/edges and all structural source anchors.

Add tests for unique mapping, two structural candidates, two semantic
candidates, same label/different path, and same path/different realization.

**Verify**: graph tests exit 0 and `rg '_doc' crates/compass-graph/src/lib.rs`
returns no match associated with document-twin selection.

### Step 6: Document compatibility and run gates

Update document-processing design with prepared units, completeness, retries,
and fusion. Add a changelog entry for removal of rich-document truncation and
simultaneous structural/semantic evidence. Update `COMPATIBILITY.md` if node or
cache realization behavior changes; add `MIGRATION.md` only if users must act.

Run all commands in the table, `sh scripts/check_product_boundary.sh`, and
`git diff --check`.

## Test plan

- Gap-free slicing over empty, repeated, Unicode, table, and oversized blocks.
- Rich-document source above 20,000 characters reaches provider fixtures in
  full and in deterministic order.
- Adaptive retry bisects and terminates under both eventual-success and
  permanent-failure fixtures.
- Semantic Markdown graph retains structural headings/links and concepts.
- Evidence-based twin mapping covers uniqueness and ambiguity negatives.
- Incremental/cached rebuild yields the same realized graph as a cold build.

## Done criteria

- [ ] Rich documents are decoded before request-size planning.
- [ ] No fixed 20,000-character truncation remains in semantic reading.
- [ ] Ordered slices reconstruct normalized content exactly.
- [ ] Retry and cache completeness operate at document-slice granularity.
- [ ] Deterministic structure and semantic evidence coexist for one source.
- [ ] Twin fusion uses explicit source/provenance evidence, never suffixes.
- [ ] All targeted tests, qualification, lint, format, boundary, and diff checks pass.
- [ ] Plan 008 is marked `DONE` in `advisor-plans/README.md`.

## STOP conditions

Stop and report if:

- plan 007's artifact cannot reconstruct normalized semantic text without
  losing separators or occurrence identity;
- fusion requires selecting among ambiguous source candidates;
- eliminating suppression changes public stable IDs without an approved
  compatibility/migration decision;
- provider request schemas would need an unversioned breaking change;
- an in-scope user change cannot be preserved;
- any verification would write Cargo artifacts to local `target/`.

## Maintenance notes

Every future decoder receives slicing/retry/fusion automatically by producing
a valid artifact; do not add format-specific semantic readers. Reviewers should
scrutinize gap coverage, cache completion, and identity proof rather than only
checking concept counts.
