# Plan 012: Qualify document graphs across formats, limits, and determinism

> **Executor instructions**: This plan turns completed decoder work into a
> release claim. Follow the steps and run every command. Do not weaken expected
> fixtures to make a failure pass. Stop and report on a STOP condition. Update
> `advisor-plans/README.md` after completion unless a reviewer owns the index.
>
> **Drift check (run first)**:
> `git diff --stat 743a170..HEAD -- crates/compass-media crates/compass-languages crates/compass-semantic crates/compass-core tests fixtures scripts docs PERFORMANCE.md CHANGELOG.md COMPATIBILITY.md .github/workflows/compass-ci.yml Makefile`
> Plans 009, 010, and 011 must be complete. Reconcile the live support matrix
> with this plan before adding qualification expectations.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `advisor-plans/009-structural-markdown-html.md`, `advisor-plans/010-native-ooxml-documents.md`, `advisor-plans/011-bounded-native-rtf.md`
- **Category**: tests, perf, docs
- **Planned at**: commit `743a170`, 2026-08-01

## Why this matters

Format checkboxes do not establish a better knowledge-graph product. Compass
needs repeatable evidence that document graphs preserve hierarchy, links,
tables, order, provenance, completeness, and bounded failure across cold,
cached, relocated, and semantic builds. This plan creates a Compass-owned
qualification corpus and release gate; it does not depend on Graphify, private
documents, model credentials, or network access.

## Current state

- `./scripts/qualify_code_graph_v1.sh --fixtures-only` is the closest existing
  fixture qualification pattern. Read it and its expected artifacts before
  adding a document-specific gate.
- Root `AGENTS.md` requires regression tests at the lowest owner plus a public
  contract test, deterministic fixtures, bounded failures, and performance
  evidence before performance claims.
- Plans 009–011 should leave a format matrix in docs and native fixtures in
  their owning crates. This plan promotes a minimal cross-format subset to a
  stable qualification corpus.
- The local Graphify checkout observed during planning used regex Markdown and
  optional Python Office libraries. That is audit context only. Do not invoke
  it, copy its fixtures, or add it to CI/product boundaries.

## Qualification contract

For every shipped format, qualification must assert:

1. deterministic document/block identity and order;
2. heading/list/table containment and multiplicity where supported;
3. link direction, containing-block source, target/unresolved state;
4. exact text locators or honest format-specific logical locators;
5. provenance and completeness/diagnostics;
6. structural-only and structural-plus-semantic coexistence;
7. cold, warm-cache, repeated, and relocated-root equivalence;
8. malformed/limit inputs fail explicitly and leave no coherent-looking
   partial artifact set;
9. machine output contains no absolute path, credential, or private source.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Document qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main ./scripts/qualify_document_graph_v1.sh --fixtures-only` | exit 0; all format cases pass |
| Code qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main ./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Product contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-cli --test compass_product --locked` | exit 0 |
| Workspace tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test --workspace --lib --bins --locked` | exit 0 |
| Workspace lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy --workspace --lib --bins --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Boundary | `sh scripts/check_product_boundary.sh` | exit 0 |

The first command does not exist before this plan. Build it in step 3. If the
external volume is unavailable, stop and do not use a local Cargo target.

## Scope

**In scope**:

- `tests/qualification/documents/` or the repository's live qualification
  fixture location after inspecting the code-graph exemplar
- focused native fixture builders under affected crate tests when ZIP/package
  assembly is required
- `scripts/qualify_document_graph_v1.sh` (create)
- `scripts/check_document_support.py` only if the repository's support-matrix
  checks are already Python-based; this is a developer gate, never runtime
- `Makefile` and `.github/workflows/compass-ci.yml` for one named CI target
- `docs/reference/document-formats.md` (or the live format matrix)
- `docs/implementation/document-qualification.md` (create)
- `docs/README.md`, `PERFORMANCE.md`, `COMPATIBILITY.md`, `CHANGELOG.md`
- minimal focused test harness changes in media/languages/semantic/core/CLI

**Out of scope**:

- Adding new decoder behavior to make a qualification case pass; route a bug
  back to plans 009–011 or a focused follow-up.
- Private/customer documents, real model calls, credentials, or network tests.
- Runtime dependence on Python, Graphify, LibreOffice, Pandoc, or office apps.
- Benchmark claims against another product without a reproducible, approved,
  license-safe methodology and equivalent configuration.
- Storing opaque generated graph outputs too large for human review.

## Git workflow

- Suggested branch: `advisor/012-document-qualification-gate`
- Commit corpus/contracts, runner, CI/docs/performance evidence separately.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Freeze a reviewable cross-format corpus manifest

Create a versioned machine-readable manifest, for example
`tests/qualification/documents/v1/cases.json`, with one or more cases for:

- Markdown: frontmatter, duplicate headings, nested list, table, reference and
  relative/fragment links, Unicode;
- HTML: title/metadata, sections, table, entities, relative links, skipped
  script/style, malformed recovery;
- DOCX: interleaved paragraph/table/list/link/note;
- XLSX: multiple sheets, sparse cells, typed values/formula evidence, merge/link;
- PPTX: relationship-defined slide order, shapes/table/link/notes/alt text;
- RTF: code page/Unicode, headings/list/table/link/metadata, skipped object;
- current PDF behavior, with page locators and honest structural limitations;
- negative/limit cases for every decoder family.

Each manifest entry must declare source fixture path, format, expected
completeness, expected diagnostic codes, minimum/exact block and edge facts,
locator kind, and modes to run. Do not embed timestamps, absolute paths, or
provider-generated free text.

Keep text/XML fixture sources reviewable. Assemble OOXML ZIPs deterministically
inside Rust tests or a checked repository helper; normalize ZIP timestamps and
member order. If binary PDF bytes are necessary, document their creation and
license in an adjacent README.

**Verify**: a manifest-validation test rejects duplicate IDs, missing fixtures,
unknown schema major, unsupported formats/statuses, and absolute paths.

### Step 2: Add one typed assertion harness

Build a Rust integration harness at the lowest shared layer that reads the
manifest, constructs/loads fixtures, runs normalizers and graph publication,
and asserts typed graph records—not rendered prose. For each case assert:

- ordered block kinds/parents/locators and stable IDs;
- exact selected attributes and link/edge evidence;
- diagnostic codes and completeness;
- no unknown major versions or absolute paths;
- identical normalized graph JSON after stable sorting on two runs and under a
  different temporary workspace root;
- hard negative cases return the declared typed error and publish nothing.

Use a deterministic fake semantic provider for semantic mode. It must return
fixed typed fragments keyed by case/unit ID, assert all sentinels were supplied,
and never contact a network. Compare the structural subgraph against the
structural-only run before checking added semantic facts.

**Verify**: the focused integration test exits 0 and reports case IDs on any
failure. Deliberately perturb one expected fact locally to confirm the harness
fails, then restore it without committing the perturbation.

### Step 3: Create the fixture-only qualification command

Model `scripts/qualify_document_graph_v1.sh` after the code-graph qualification
script's argument parsing, temp-directory safety, output bounding, and cleanup.
It must:

- accept `--fixtures-only` and reject unknown flags;
- require `CARGO_TARGET_DIR` to be an absolute path outside the Compass checkout
  rather than silently choosing a local target; local executions under this
  repository must use `/Volumes/Workspace/crabbuild-target/compass-main`, while
  CI may use a checkout-specific directory under its runner temp volume;
- run manifest validation and the typed cross-format test;
- run product-boundary checks relevant to documents;
- emit a concise per-format pass/fail summary and return nonzero on any skip,
  missing expected format, partial publication, or test failure;
- create temporary output with `mktemp -d` and remove only its own directory.

Do not make the script download tools or fixtures. Add a Make target only if it
matches existing naming, such as `qualify-document-graph-v1`.

**Verify**: the new command exits 0; an unknown flag exits nonzero; unsetting
`CARGO_TARGET_DIR` exits nonzero before Cargo runs; a target path contained by
the checkout also exits nonzero; `sh -n` exits 0.

### Step 4: Gate support claims and CI on the corpus

Add a small checker that compares the documented format matrix's machine-readable
support statuses with manifest coverage. A format may be called `supported`
only when at least one positive, one malformed/limit, determinism, locator, and
structural-plus-semantic case passes. Use `partial` for intentionally incomplete
construct coverage and list the diagnostic behavior.

Wire the fixture-only command into the existing CI workflow near code-graph
qualification. It must not require secrets, external checkouts, network access,
or platform-specific office software. Reuse the checkout-specific external
Cargo target convention in CI where applicable.

**Verify**: the support checker exits 0; changing one documented format to an
unsupported claim/status mismatch makes it fail; workflow syntax remains valid
under the repository's existing CI validation.

### Step 5: Establish performance and boundedness baselines

Add a reproducible local benchmark corpus with small/medium/limit-adjacent cases
and record, per format:

- source and expanded bytes;
- blocks/links/normalized characters;
- cold decode, warm decode/cache, structural publication, and semantic-packing
  time (fake provider only);
- maximum configured allocations/counts, not an unportable guessed RSS value;
- toolchain, hardware, command, and run count.

Run enough iterations to report median and variability. Put measured results
and the exact date/commit in `PERFORMANCE.md`. Do not add a hard wall-clock CI
threshold from one developer machine. CI should enforce count/limit invariants
and optionally a broad regression ceiling only after baseline evidence on its
runner is stable.

**Verify**: the documented command reproduces a machine-readable result; all
limit-adjacent cases remain below configured counts and terminate successfully
or with their declared limit error.

### Step 6: Publish the release contract and run the full baseline

Write `docs/implementation/document-qualification.md` explaining corpus
ownership, adding a case, regenerating deterministic packages, interpreting
completeness, and running the gate. Link it and the format matrix from
`docs/README.md`. Update compatibility and changelog entries without claiming
unqualified parity or superiority.

Run every command in the table, the document support checker, any documented
workflow validation, and `git diff --check`. Inspect `git status --short` for
generated graphs, local `.compass/`, output directories, or credentials; none
may be committed.

## Test plan

- Manifest schema/duplicate/path/coverage validation.
- Positive and hostile cases for Markdown, HTML, PDF, DOCX, XLSX, PPTX, RTF.
- Typed structural, link, locator, provenance, diagnostic, completeness facts.
- Cold/warm/relocated/repeated determinism.
- Structural versus fake-semantic enrichment equivalence.
- Script flag/target-directory/failure/cleanup behavior.
- Support-matrix drift and bounded benchmark corpus.

## Done criteria

- [ ] Every shipped document-format claim has manifest-backed positive and
  negative evidence.
- [ ] The qualification gate is local, deterministic, bounded, and credential-free.
- [ ] Structural facts are invariant when fake semantic enrichment is enabled.
- [ ] Relocated/cached/repeated runs produce equivalent graph contracts.
- [ ] CI and documentation support claims use the same versioned corpus.
- [ ] Performance claims cite reproducible measurements and configured limits.
- [ ] Full baseline, lint, format, product boundary, and diff checks pass.
- [ ] Plan 012 is marked `DONE` in `advisor-plans/README.md`.

## STOP conditions

Stop and report if:

- any prerequisite format plan is incomplete or its support matrix is unclear;
- qualification requires real credentials, network, private documents, or an
  external product checkout;
- a test can pass by comparing only counts/prose instead of typed evidence;
- deterministic fixtures cannot be reproduced from reviewable sources;
- a decoder bug or schema change is needed—open a focused prerequisite instead
  of hiding it in qualification;
- performance results are too noisy to support a published claim;
- CI would build into the repository's local `target/` directory;
- pre-existing user changes cannot be preserved.

## Maintenance notes

The corpus is a product contract, not a snapshot dumping ground. Every format
change must add the smallest reviewable fixture proving the new behavior and a
negative/boundary case. Keep competitor comparisons outside the required gate;
Compass should win on reproducible evidence, not an imported runtime oracle.
