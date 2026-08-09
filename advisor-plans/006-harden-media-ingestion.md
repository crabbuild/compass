# Plan 006: Make media ingestion fail explicitly and remain memory-bounded

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `advisor-plans/README.md` unless a reviewer says they maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat 743a170..HEAD -- crates/compass-media crates/compass-semantic docs/design/security-and-privacy.md CHANGELOG.md`
> The working tree used to write this plan already contained unrelated changes.
> Preserve them. If an in-scope file has changed, compare the excerpts below to
> live code and stop if the named behavior no longer exists.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: security, bug
- **Planned at**: commit `743a170`, 2026-08-01

## Why this matters

Compass parses attacker-controlled Office/PDF inputs. ZIP-size limits exist,
but XLSX cell coordinates can still amplify a small XML member into an
unbounded vector allocation. Separately, the semantic reader converts any
DOCX/XLSX/PDF parse failure into an apparently successful empty source. That
violates Compass's explicit-limit invariant and can mark a file complete even
though no content was analyzed.

This plan hardens the existing string-returning API without introducing the
document IR planned in 007. It is intentionally safe to land first.

## Current state

- `crates/compass-media/src/lib.rs` owns bounded local extraction.
  - Lines 12–15 cap raw/archive/member bytes.
  - Lines 356–405 parse worksheet rows.
  - Lines 369–374 derive a column from arbitrary `c@r` text and call
    `values.resize(column + 1, ...)` without a column/cell bound.
  - Lines 418–428 saturate arbitrarily long column names to `usize::MAX`.
- `crates/compass-semantic/src/lib.rs:757-838` reads semantic files.
  - Lines 802–805 turn errors for `pdf`, `docx`, and `xlsx` into
    `Some(String::new())` with no warning.
- `docs/design/security-and-privacy.md:143-144` says a limit failure must not be
  reinterpreted as empty or complete.
- Tests use `Result<(), Box<dyn Error>>`, temporary files, and synthetic ZIP
  members; follow `crates/compass-media/src/lib.rs:460-580`.
- Rust policy: no unsafe code; no `unwrap`/`expect`/`panic`; deterministic
  collections or explicit sorting at contract boundaries.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Media tests | `cargo test -p compass-media --locked` | exit 0 |
| Semantic tests | `cargo test -p compass-semantic --locked` | exit 0 |
| Lint | `cargo clippy -p compass-media -p compass-semantic --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Boundary | `sh scripts/check_product_boundary.sh` | exit 0 |

If you redirect Cargo output with `CARGO_TARGET_DIR`, set it on every
invocation above and use a directory dedicated to this checkout.

## Scope

**In scope**:

- `crates/compass-media/src/lib.rs`
- `crates/compass-semantic/src/lib.rs`
- `crates/compass-semantic/src/tests.rs`
- `crates/compass-semantic/tests/edge_coverage.rs`
- `docs/design/security-and-privacy.md` only if diagnostics need clarification
- `CHANGELOG.md` for the release-visible hardening note

**Out of scope**:

- Adding PPTX, RTF, HTML, ODT, or EPUB support.
- Changing the public graph schema or relationship vocabulary.
- Building the document IR from plan 007.
- Running Graphify or adding Graphify fixtures/dependencies.
- Lowering the existing raw/archive byte ceilings without maintainer approval.

## Git workflow

- Suggested branch: `advisor/006-harden-media-ingestion`
- Use conventional commits such as `fix(media): bound worksheet coordinates`.
- Keep the XLSX allocation fix and semantic failure-policy change as separate
  logical commits if both are committed.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Add hostile XLSX regression tests

In the existing `compass-media` test module, add synthetic ZIP tests for:

1. a cell coordinate with enough letters to saturate the current fold;
2. a coordinate beyond Excel's last valid column (`XFD` is valid; the next
   column is invalid);
3. too many non-empty cells/rows according to new Compass processing limits;
4. malformed coordinates such as missing letters or non-ASCII letters;
5. exactly-at-limit input succeeding deterministically.

Tests must assert `MediaError::Rejected` or `MediaError::Parse`; they must not
accept a process panic as success.

**Verify**: run the media test command before implementation. At least the
hostile-coordinate tests must fail for the expected reason, not because their
fixture ZIP is malformed.

### Step 2: Replace coordinate amplification with checked limits

Add named constants near the current media limits:

- `XLSX_MAX_COLUMNS = 16_384` (the XLSX format maximum);
- `XLSX_MAX_ROWS = 100_000` (Compass processing ceiling);
- `XLSX_MAX_CELLS = 1_000_000` (non-empty/visited cell ceiling);
- `OFFICE_MAX_TEXT_CHARS = 20_000_000` (aggregate normalized text ceiling).

Change `excel_column_index` to return a checked `Result<usize, MediaError>`.
Reject empty/malformed references, arithmetic overflow, and columns outside
`0..XLSX_MAX_COLUMNS`. In `parse_xlsx_rows`, track rows and visited cells with
checked/saturating counters and return `MediaError::Rejected` before resizing
or appending beyond a limit. Bound shared-string count and accumulated output
characters under the same policy.

Do not silently clamp a coordinate: truncating a sheet changes which value
belongs to which column and invents meaning. Return an actionable limit error.

**Verify**: media tests exit 0; the new hostile tests assert typed errors and a
normal sparse `A1/C1` fixture still renders identically.

### Step 3: Make semantic media failures observable

In `read_semantic_units`, remove the `is_compat_binary_document` branch that
turns `MediaError` into an empty source. On any `extract_text` error:

- do not add a `LoadedSemanticSource`;
- add one deterministic warning containing the logical path and error class;
- never include uncontrolled parser text beyond the existing diagnostic style;
- preserve valid empty documents as successful only when parsing itself
  returned `Ok(String::new())`.

Keep `extract_text_compat` for any explicitly documented compatibility caller;
do not use it in the semantic pipeline.

Add tests proving malformed PDF/DOCX/XLSX each produces a warning and no loaded
source, while a valid empty text file remains a loaded empty source. Assert
warning ordering follows input ordering.

**Verify**: semantic tests exit 0 and `rg 'is_compat_binary_document' crates/compass-semantic` returns no matches.

### Step 4: Propagate incomplete-file status to corpus orchestration

Trace `read_semantic_units` through `extract_semantic_units` and cached corpus
orchestration. A skipped parse must be represented as a failed/partial source,
not included among finalized cache entries and not counted as successfully
refreshed. Use existing `partial_files`, `failures`, and provider-warning
contracts rather than adding an unversioned side channel.

Add a corpus-level test that dispatches one valid Markdown file and one
malformed DOCX. The valid file may complete, the DOCX must remain partial or
failed, and the result must not write a complete per-file semantic cache entry
for the DOCX.

**Verify**: `cargo test -p compass-semantic --locked` exits 0; the new test
asserts the malformed path appears exactly once in incomplete diagnostics.

### Step 5: Document and verify the boundary

Add a concise changelog entry explaining that malformed/over-limit Office and
PDF inputs are reported as incomplete instead of treated as empty. Update the
security design only if the implementation introduces a named limit not
already described there.

Run all commands in the table. Inspect `git diff --check` and confirm no test
fixture or diagnostic contains private source material.

## Test plan

- Unit tests in `compass-media` for coordinate overflow, format bounds, total
  cells, total text, and exact-limit success.
- Semantic reader tests for malformed binary inputs, valid empty text, stable
  warning order, and no false successful source.
- Corpus cache test for incomplete media not being finalized/replayed.
- Existing six media tests and all semantic tests remain green.

## Done criteria

- [ ] No XLSX coordinate can request a vector beyond `XLSX_MAX_COLUMNS`.
- [ ] Worksheet rows, cells, shared strings, and normalized output are bounded.
- [ ] Media limit/parse errors never become successful empty semantic sources.
- [ ] Failed media is not finalized in semantic cache state.
- [ ] Targeted tests, Clippy, formatting, product boundary, and `git diff --check` pass.
- [ ] `git status --short` contains only in-scope changes plus pre-existing user changes.
- [ ] Plan 006 is marked `DONE` in `advisor-plans/README.md`.

## STOP conditions

Stop and report if:

- live code already replaced `Vec::resize` with a bounded sparse structure;
- changing failure semantics requires altering a published machine schema
  rather than existing warning/partial contracts;
- existing compatibility tests require malformed media to count as complete;
- an in-scope file overlaps unresolved user changes that cannot be preserved;
- any Cargo command would use a local `target/` directory.

## Maintenance notes

Every future document decoder must use the same explicit rejection policy.
Reviewers should look for allocations driven by declared indexes/counts, not
only byte-size caps. Plan 007 will centralize these limits in a document IR; do
not pre-empt that larger refactor here.
