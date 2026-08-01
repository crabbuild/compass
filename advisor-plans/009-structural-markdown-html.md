# Plan 009: Make Markdown and HTML first-class structural documents

> **Executor instructions**: Follow this plan in order. Run each verification
> command and confirm the stated outcome. Stop rather than improvising when a
> STOP condition occurs. Mark the row in `advisor-plans/README.md` `DONE` after
> completion unless a reviewer maintains the index.
>
> **Drift check (run first)**:
> `git diff --stat 743a170..HEAD -- crates/compass-languages crates/compass-media crates/compass-ingest crates/compass-files crates/compass-core vendor/compass-tree-sitter-language-pack docs CHANGELOG.md COMPATIBILITY.md`
> Plans 007 and 008 must be complete. The original working tree had user edits
> in Markdown/cache/pipeline files; preserve them and stop on an overlap that
> cannot be reconciled.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: `advisor-plans/007-versioned-document-artifact.md`, `advisor-plans/008-lossless-document-fusion.md`
- **Category**: direction
- **Planned at**: commit `743a170`, 2026-08-01

## Why this matters

Markdown is currently recognized with line-oriented regular expressions, HTML
is flattened with tag-stripping regular expressions during URL ingestion, and
semantic-backed Markdown loses its native structure. This misses Setext
headings, nested blocks, reference links, section-local link provenance,
frontmatter, and most HTML semantics. The static parser pack already vendors
Markdown, Markdown-inline, and HTML grammars, so Compass can produce deeper
local graphs without runtime downloads or model dependence.

## Current state

- `crates/compass-languages/src/markdown.rs` extracts headings/links using
  regular expressions and opens the Markdown path itself.
- `crates/compass-languages/src/engine.rs:84-103` already has bounded source
  bytes/text before dispatch, so the Markdown path can read the same file twice.
- `crates/compass-files/src/hash.rs:197-237` strips Markdown frontmatter for
  hashing even though frontmatter can affect document meaning and semantic
  prompts.
- `crates/compass-ingest/src/lib.rs:525-529` implements `html_to_markdown` by
  removing `script`, `style`, and all remaining tags with regexes.
- `crates/compass-files/src/detect.rs` classifies `md`, `mdx`, `qmd`, `html`,
  and other text/document extensions.
- `vendor/compass-tree-sitter-language-pack` statically contains the pinned
  Markdown, Markdown-inline, and HTML grammars. Do not edit vendored code and
  do not add runtime grammar fetching.
- `serde_yaml_ng` is already a workspace dependency and is used by
  `crates/compass-languages/src/package_manifest.rs`; use its bounded parsing
  pattern rather than adding another YAML parser.
- Plans 007–008 provide `DocumentArtifact`, exact text locators, deterministic
  structural projection, and structural/semantic coexistence.

## Required semantics

- Markdown and local HTML use exact byte/line locators into the original bytes.
- A link belongs to its smallest containing section/block; a file-root link is
  used only when no containing block exists.
- A fragment resolves to a heading only when the slug or explicit ID is unique.
  Duplicate headings remain ambiguous; never select the first occurrence.
- YAML frontmatter is metadata, not visible body text. Only JSON-compatible
  scalars and bounded scalar arrays are published. Unsupported YAML values
  produce a diagnostic, not arbitrary nested graph attributes.
- HTML excludes `script`, `style`, `template`, and `noscript` content, decodes
  entities, preserves visible structural order, and never executes/fetches a
  resource discovered in markup.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Languages | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-languages --locked` | exit 0 |
| Markdown coverage | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-languages --test markdown_coverage --locked` | exit 0 |
| Media | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-media --locked` | exit 0 |
| Ingest | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-ingest --locked` | exit 0 |
| Core | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-core --locked` | exit 0 |
| Qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main ./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-languages -p compass-media -p compass-ingest -p compass-core --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |

## Scope

**In scope**:

- `crates/compass-languages/src/markdown.rs`
- `crates/compass-languages/src/html.rs` (create)
- `crates/compass-languages/src/engine.rs`, `lib.rs`, registry wiring/tests
- `crates/compass-languages/Cargo.toml`
- `crates/compass-media/src/document.rs` and a focused HTML normalization
  module if shared normalization belongs in media
- `crates/compass-media/Cargo.toml`
- `crates/compass-ingest/src/lib.rs`, `Cargo.toml`, and tests
- `crates/compass-files/src/hash.rs` and hash/cache tests
- focused fixtures/tests in the affected crates
- `docs/design/document-processing.md`, `docs/reference/`, `docs/README.md`
- `COMPATIBILITY.md`, `CHANGELOG.md`

**Out of scope**:

- Editing `vendor/` or downloading grammars at runtime.
- Network fetching from links, images, stylesheets, frames, or base URLs.
- Full MDX/JSX execution; opaque JSX must remain bounded source evidence.
- Treating frontmatter as trusted configuration or secrets.
- Office/RTF work from plans 010–011.

## Git workflow

- Suggested branch: `advisor/009-structural-markdown-html`
- Commit parser wiring/tests, Markdown, HTML/shared ingest, then cache/docs.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Change extraction to source-driven parser APIs

Before editing, read `crates/compass-languages/src/lib.rs`, `Cargo.toml`,
`engine.rs`, `markdown.rs`, registry tests, and
`docs/design/language-architecture.md`. Change Markdown extraction to accept
the already-loaded source and source-file facts supplied by the engine. Remove
the extractor's direct filesystem read. Add a test runner or counter fixture
showing one source read per Markdown file.

Wire the existing statically linked Markdown, Markdown-inline, and HTML parser
factories through the language pack's normal ownership boundary. Do not create
direct raw FFI or a second parser registry.

**Verify**: language tests exit 0 and `rg 'read_to_string|File::open' crates/compass-languages/src/markdown.rs`
returns no matches.

### Step 2: Parse bounded Markdown frontmatter into artifact metadata

Recognize frontmatter only when the source begins with a whole-line `---` and
has a whole-line closing `---` within 64 KiB. Parse with `serde_yaml_ng`, reject
aliases/tags or values outside JSON-compatible scalar/scalar-array policy, cap
published keys at 256, cap individual strings at the document field limit, and
preserve deterministic key order.

Publish approved general metadata without application behavior: `title`,
`tags`, `type`, `source_url`, `status`, and `date`. Preserve other valid scalar
keys in the artifact metadata map if the plan-007 contract permits it, but do
not make them control execution. Add diagnostics for invalid/unclosed/oversized
frontmatter and continue parsing the body only when bounds permit.

Change Markdown cache hashing to include raw frontmatter because metadata now
changes graph output. Remove body-only hashing for Markdown or introduce an
explicit, versioned graph hash that includes metadata. Invalidate old entries;
do not probe both meanings under one key.

**Verify**: tests show a title/tag-only edit changes the graph/cache hash, body
offsets remain exact, and malformed YAML is bounded and diagnosed.

### Step 3: Replace line regexes with Markdown syntax evidence

Walk the Markdown and inline trees to emit ordered artifact blocks/facts for:

- ATX and Setext headings with hierarchy;
- paragraphs, nested lists/items, tasks, block quotes, thematic breaks;
- fenced/indented code with language/info string;
- pipe tables, rows, and cells;
- inline links/images, reference definitions/usages, autolinks, footnotes;
- Quarto/MDX constructs as explicit bounded `Other` blocks when the grammar
  recognizes them but Compass does not assign deeper meaning.

Preserve occurrence, exact byte ranges, heading levels, and parent section.
Slug headings deterministically using one documented algorithm. Build a
multimap from slug/explicit ID to heading ordinals; resolve only unique targets.
Retain unresolved/ambiguous evidence instead of inventing an edge target.

Keep the existing public graph vocabulary where it is expressive enough. If a
new node/edge kind is unavoidable, stop for schema/compatibility review rather
than publishing an undocumented string.

**Verify**: expand `markdown_coverage` with nested, duplicate-heading,
reference-link, footnote, table, code-fence, Unicode, malformed, and CRLF cases.
Assert identities, relationship direction, multiplicity, ranges, provenance,
and deterministic order.

### Step 4: Add structural HTML normalization and extraction

Create a shared HTML-to-`DocumentArtifact` decoder using the pinned HTML
grammar. In document order, support:

- document title, headings, `main`/`article`/`section`/`nav` landmarks;
- paragraphs, lists/items, block quotes, `pre`/`code`, tables/rows/cells;
- anchors with resolved entity text, `id`, `href`, optional `rel`;
- bounded `<meta name|property content>` values and canonical/base URL as
  metadata/link evidence;
- visible text with HTML entity decoding and whitespace normalization that
  does not merge separate blocks.

Skip script/style/template/noscript subtrees entirely. Treat malformed markup
according to parser recovery while recording a diagnostic when content is
incomplete. Resolve relative URLs against a validated source/base URL string,
but never fetch them.

Add an HTML language adapter that projects the artifact structurally. Change
URL ingestion to call the same deterministic renderer instead of private
`html_to_markdown` regexes. Preserve the current ingestion return contract.

**Verify**: ingest and language tests prove scripts/styles are absent, entities
decode once, nested structure remains ordered, links use containing sections,
malformed HTML is deterministic, and no fixture fetch occurs beyond the
original requested page.

### Step 5: Verify structural and semantic coexistence

Add core integration fixtures for the same Markdown and HTML documents with
semantic mode off and with a fake semantic provider. Both modes must retain
identical deterministic document/block/link subgraphs. Semantic mode adds
provider concepts/edges without replacing, duplicating, or changing stable
structural identities.

Run the normalizer twice and under a relocated temporary root; graph records
must be byte-identical and contain no absolute paths.

**Verify**: core tests and fixture-only qualification exit 0.

### Step 6: Publish the supported-feature contract

Update document-processing design and create/update a document-format reference
matrix covering shipped Markdown and HTML constructs, unsupported constructs,
locator guarantees, frontmatter policy, and local/network boundary. Add
compatibility and changelog notes for the new graph facts and cache cut.

Run every command in the table, product-boundary check, and `git diff --check`.

## Test plan

- Markdown parser coverage for every construct and ambiguity listed above.
- Frontmatter bounds, YAML types, hash invalidation, and secret-safe diagnostics.
- HTML structural/order/entity/script/link/base/malformed cases.
- URL-ingestion parity through shared normalization.
- Cold/cached, semantic/non-semantic, and relocated-root graph determinism.

## Done criteria

- [ ] Markdown and HTML extraction use pinned syntax trees and already-read bytes.
- [ ] Frontmatter changes invalidate graph/semantic cache inputs.
- [ ] Links carry containing-block provenance and ambiguous fragments remain unresolved.
- [ ] HTML ingestion no longer relies on regex tag stripping.
- [ ] Structural graphs survive semantic enrichment unchanged.
- [ ] Docs state exactly what is and is not supported.
- [ ] All targeted tests, lint, format, qualification, boundary, and diff checks pass.
- [ ] Plan 009 is marked `DONE` in `advisor-plans/README.md`.

## STOP conditions

Stop and report if:

- the language pack does not expose the pinned Markdown/inline/HTML grammars;
- implementation would require editing vendored parser sources or runtime
  grammar downloads;
- a proposed graph kind is not covered by a versioned public contract;
- exact Markdown/HTML byte ranges cannot be preserved after normalization;
- cache invalidation would reuse an old key for new frontmatter semantics;
- an in-scope user edit cannot be preserved.

## Maintenance notes

Parser recovery is evidence, not permission to invent hierarchy. Add grammar
corpus fixtures before extending syntax mapping. Keep HTML normalization shared
between local files and fetched content so security, semantics, and cache
behavior cannot drift.
