# Plan 007: Introduce a versioned, provenance-preserving document artifact

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `advisor-plans/README.md` unless a reviewer says they maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat 743a170..HEAD -- crates/compass-media crates/compass-semantic crates/compass-files docs/design COMPATIBILITY.md CHANGELOG.md Cargo.toml Cargo.lock`
> The working tree used to write this plan already contained unrelated changes.
> Preserve them. If an in-scope file changed, compare the symbols below with
> live code and stop if the ownership or cache behavior no longer matches.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `advisor-plans/006-harden-media-ingestion.md`
- **Category**: tech-debt, direction
- **Planned at**: commit `743a170`, 2026-08-01

## Why this matters

Today every rich document is flattened immediately to one Markdown-like
`String`. That destroys block order, hierarchy, link provenance, exact source
locations, parser diagnostics, and the distinction between complete and
partial extraction. It also makes cache correctness depend on undocumented
normalizer behavior. This plan adds one bounded, typed intermediate contract
that all later Markdown, HTML, Office, PDF, semantic, and graph work can share.

The artifact is not a universal AST. It represents only evidence Compass can
prove: ordered blocks, containment, links, metadata, diagnostics, and source
locators. It must never invent byte offsets inside ZIP packages or PDFs.

## Current state

- `crates/compass-media/src/lib.rs` owns bounded PDF/DOCX/XLSX extraction.
  `extract_text(path)` dispatches by extension and returns only `String`.
- `crates/compass-semantic/src/lib.rs:742-838` owns `SemanticUnit` and rereads
  paths through `compass_media::extract_text`.
- `crates/compass-semantic/src/orchestration.rs:1069,1216` namespaces semantic
  cache entries with `compass_files::prompt_fingerprint(prompt)` only.
- `crates/compass-files/src/cache.rs:175-269` already accepts a fingerprint
  string for semantic cache directories. Reuse that interface; do not create a
  second cache implementation.
- Root `Cargo.toml` already provides `serde`, `serde_json`, and
  `serde_yaml_ng` as workspace dependencies.
- Compass contracts require deterministic ordering, explicit unknown-major
  rejection, bounded untrusted input, relative/provenance-safe paths, and
  explicit incomplete results. These rules are in root `AGENTS.md` and
  `docs/design/security-and-privacy.md`.

## Target contract

Add `crates/compass-media/src/document.rs` with public types equivalent to the
following shape. Exact Rust field names may change only if tests and design
documentation use the final names consistently.

```rust
pub const DOCUMENT_SCHEMA: &str = "compass.document/1";
pub const DOCUMENT_NORMALIZER_VERSION: u32 = 1;

pub struct DocumentArtifact {
    pub schema: String,
    pub normalizer_version: u32,
    pub format: DocumentFormat,
    pub blocks: Vec<DocumentBlock>,
    pub links: Vec<DocumentLink>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub diagnostics: Vec<DocumentDiagnostic>,
    pub complete: bool,
}

pub struct DocumentBlock {
    pub ordinal: u32,
    pub parent: Option<u32>,
    pub kind: DocumentBlockKind,
    pub text: String,
    pub locator: DocumentLocator,
}
```

Required enums:

- `DocumentFormat`: plain text, Markdown, HTML, PDF, DOCX, XLSX, PPTX, RTF.
  A variant is vocabulary, not a support claim; unsupported decoders still
  return a typed error.
- `DocumentBlockKind`: document title, heading with level, paragraph, list,
  list item, code, quote, table, row, cell, page, sheet, slide, note, and an
  explicit `Other { role }` for bounded forward compatibility.
- `DocumentLocator`: exact text range; package part plus logical block path;
  PDF page plus item; spreadsheet sheet/row/column; slide/shape. Text ranges
  contain checked byte and line positions. Package/PDF locators are logical
  and must not claim original byte offsets.
- `DocumentLink`: source block ordinal, destination string, optional label,
  relationship kind, and locator.
- `DocumentDiagnostic`: stable code, severity, optional locator, and bounded
  message. `complete=false` means some source content was intentionally skipped
  or could not be represented; a hard limit or corrupt container remains an
  error, not a partial artifact.

Validation must reject unknown schema majors, duplicate/out-of-order ordinals,
missing parents, forward/cyclic parents, invalid ranges, links to missing
blocks, absolute/package-escaping part names, and oversized fields.

## Commands you will need

Set the external target on every Cargo invocation:

| Purpose | Command | Expected on success |
|---|---|---|
| Media | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-media --locked` | exit 0 |
| Semantic | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-semantic --locked` | exit 0 |
| Files | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-files --locked` | exit 0 |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-media -p compass-semantic -p compass-files --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Boundary | `sh scripts/check_product_boundary.sh` | exit 0 |

If `/Volumes/Workspace` is unavailable or unwritable, stop rather than using a
local `target/` directory.

## Scope

**In scope**:

- `crates/compass-media/src/lib.rs`
- `crates/compass-media/src/document.rs` (create)
- `crates/compass-media/src/limits.rs` (create if plan 006 did not already)
- `crates/compass-media/tests/document_contract.rs` (create)
- `crates/compass-media/Cargo.toml`
- `crates/compass-semantic/src/orchestration.rs`
- `crates/compass-semantic/src/tests.rs`
- `crates/compass-files/src/hash.rs` and its tests only if the fingerprint
  helper belongs there after ownership review
- `Cargo.toml`, `Cargo.lock` only for workspace dependency wiring
- `docs/design/document-processing.md` (create)
- `docs/README.md`, `COMPATIBILITY.md`, `CHANGELOG.md`

**Out of scope**:

- Rewriting Markdown/HTML/Office decoders; plans 009–011 own that work.
- Changing graph JSON, node kinds, or CompassQL.
- Semantic chunking or structural/semantic fusion; plan 008 owns it.
- Runtime grammar downloads, Python, LibreOffice, Pandoc, or Graphify.
- Stable block IDs based on text content. Content hashes are fingerprints, not
  identity; edits must not silently create a different logical document.

## Git workflow

- Suggested branch: `advisor/007-versioned-document-artifact`
- Use logical commits: contract/tests, decoder migration, cache namespace,
  documentation.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Lock the artifact contract with failing tests

Create `document_contract.rs`. Construct artifacts directly and test:

1. JSON round-trip preserves every enum, locator, metadata value, diagnostic,
   ordering, multiplicity, and `complete` value.
2. an unknown schema major is rejected explicitly;
3. invalid parents, ordinals, ranges, and block references are rejected;
4. package paths containing absolute roots or `..` escape are rejected;
5. diagnostics/messages and every accumulated collection obey named limits;
6. serialization contains no machine-absolute source path;
7. equal logical inputs serialize byte-for-byte identically.

Use `BTreeMap` and explicit vector ordering at serialization boundaries. Do not
use `HashMap` iteration to publish fields.

**Verify**: run media tests before implementation. New tests must fail because
the types do not exist, while existing tests still pass independently.

### Step 2: Implement types, validation, and central limits

Implement the target contract in `document.rs`. Centralize the byte, member,
block, link, metadata, diagnostic, depth, and normalized-text limits in
`limits.rs`; reuse the XLSX and Office limits introduced by plan 006. Expose a
single `DocumentArtifact::validate()` path and call it before returning or
deserializing an artifact.

Define a typed `DocumentError` that distinguishes unsupported format, rejected
limit, corrupt/parse input, invalid artifact, and I/O. Preserve actionable
context without embedding unbounded source content.

**Verify**: media tests exit 0, including every contract rejection test.

### Step 3: Add byte-oriented decoding and a compatibility renderer

Add an API equivalent to:

```rust
pub fn decode_document(
    logical_path: &Path,
    bytes: &[u8],
) -> Result<DocumentArtifact, DocumentError>;
```

The path supplies a relative logical name/extension only; all source bytes are
already present. Decoders must use `Cursor<&[u8]>` for archives so callers do
not need a second disk read. Migrate current PDF/DOCX/XLSX functions behind
this API without intentionally changing their extraction output yet.

Keep `extract_text(path)` as a compatibility wrapper: perform one bounded read,
call `decode_document`, validate, then call a deterministic
`render_document_markdown(&artifact)`. Escape table pipes, backslashes, and
newlines at rendering time rather than mutating IR text. Keep
`extract_text_compat` only for documented legacy callers.

**Verify**: existing media fixtures render identically except for intentional,
tested Markdown escaping. `rg 'read\(|File::open' crates/compass-media/src`
must show one top-level source read per compatibility call; ZIP member reads
are allowed.

### Step 4: Version the semantic cache namespace

Add one helper, preferably owned by `compass-semantic`, that hashes these exact
inputs with unambiguous separators:

- semantic prompt and deep/standard mode;
- `DOCUMENT_SCHEMA`;
- `DOCUMENT_NORMALIZER_VERSION`.

Replace direct `prompt_fingerprint` use at orchestration cache read/write sites
with the combined fingerprint. Old entries must be cache misses; do not probe
the old prompt-only directory as a fallback. Add tests proving that a prompt
change, mode change, schema change, or normalizer version change changes the
namespace, while repeated identical inputs do not.

Document the hard cache cut in `COMPATIBILITY.md`; users do not need to delete
old cache entries manually, so `MIGRATION.md` is unnecessary unless live code
shows otherwise.

**Verify**: semantic and files tests exit 0; `rg 'prompt_fingerprint\(' crates/compass-semantic/src/orchestration.rs`
finds use only inside the combined helper or none.

### Step 5: Document ownership and land the compatibility boundary

Create `docs/design/document-processing.md` describing:

- source bytes → validated `DocumentArtifact` → structural graph and semantic
  chunk consumers;
- exact versus logical locators;
- completeness and diagnostic semantics;
- format-support claims versus enum vocabulary;
- cache invalidation rules;
- why no subprocess/runtime-model dependency is permitted.

Link it from `docs/README.md`. Update `CHANGELOG.md` and `COMPATIBILITY.md` for
the new public crate contract and cache namespace.

Run all commands in the table plus `git diff --check`.

## Test plan

- Artifact construction, validation, serde round-trip, deterministic encoding,
  unknown-major rejection, limits, path containment, and locator semantics.
- Compatibility rendering for plain text and current PDF/DOCX/XLSX fixtures.
- Semantic cache namespace tests covering all four inputs.
- Existing media, semantic, and files suites remain green.

## Done criteria

- [ ] All supported document decoders return validated `DocumentArtifact`.
- [ ] Exact byte locators are used only when exact byte evidence exists.
- [ ] `extract_text` is a compatibility renderer over the artifact, not a
  parallel extraction implementation.
- [ ] Semantic caches include schema and normalizer versions and never fall
  back to prompt-only entries.
- [ ] The design and compatibility docs define the contract.
- [ ] Targeted tests, Clippy, format, product boundary, and `git diff --check` pass.
- [ ] Plan 007 is marked `DONE` in `advisor-plans/README.md`.

## STOP conditions

Stop and report if:

- plan 006 is incomplete or live media errors still become successful empty
  semantic sources;
- implementing serde requires an unknown-major value to be accepted silently;
- an existing public consumer relies on absolute paths in serialized media;
- a decoder can only provide approximate package byte offsets—use logical
  locators, and stop if a reviewer requires fabricated offsets;
- the change requires a graph schema migration before plan 008;
- an in-scope user modification cannot be preserved.

## Maintenance notes

Increment `DOCUMENT_NORMALIZER_VERSION` for any output-affecting normalization
change. Increment the schema major only for incompatible field semantics. New
format decoders must validate before publication and must expose unsupported or
partial constructs through typed errors/diagnostics rather than silent loss.
