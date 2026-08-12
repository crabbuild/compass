# Plan 015: Add bounded Jupyter and Databricks notebook extraction

> **Executor instructions**: Implement one phase at a time. A notebook is
> untrusted structured data, never an executable. Do not invoke kernels,
> notebook servers, Python, package managers, or cell magics. Preserve the
> previous coherent graph when a notebook violates a hard limit.
>
> **Drift check (run first)**:
> `git diff --stat 6680842c..HEAD -- crates/compass-files crates/compass-languages crates/compass-resolve crates/compass-graph crates/compass-model crates/compass-output crates/compass-cli fixtures docs`
> If `.ipynb` support or a container-aware source-location contract has landed,
> stop and reconcile instead of adding a second representation.

## Status

- **Priority**: P1
- **Effort**: L (four phases)
- **Risk**: MED
- **Depends on**: none
- **Category**: direction / language support
- **Planned at**: commit `6680842c`, 2026-08-10

## Why this matters

Compass recognizes dozens of source languages, but `.ipynb` is absent from the
static registry and public document-format contract. Data and ML repositories
therefore lose code declarations, imports, calls, Markdown context, and exact
cell provenance at a common container boundary. The existing language engines
make this implementable without executing notebooks or adding a new runtime.

## Current state and constraints

- `crates/compass-languages/src/registry.rs:5-22` has no notebook extractor
  kind, and `REGISTRY_CASES` has no `.ipynb` matcher.
- `crates/compass-languages/src/lib.rs:1-80` exposes a statically linked engine;
  runtime grammar downloads and dynamic loading are prohibited.
- `docs/reference/document-formats.md` distinguishes discoverability from real
  structural extraction and currently makes no notebook support claim.
- `compass-languages::Engine::extract_source` is the reusable per-cell parsing
  boundary. A notebook adapter should delegate code bytes to this engine rather
  than duplicate language parsers.
- Graph identities, anchors, ordering, limits, and cache keys must include the
  notebook container and cell coordinate. Outputs from cells are not source
  and must not enter the graph by default.
- Notebook JSON, Markdown, outputs, attachments, and metadata are untrusted.
  Inputs are bounded before allocation and strings are validated before being
  rendered.

## Target design

Add `ExtractorKind::Notebook` and a dedicated `notebook` module in
`compass-languages`. It reads a strict, bounded subset of nbformat 4:

- top-level `nbformat`, `nbformat_minor`, `metadata`, and `cells`;
- cell `id`, `cell_type`, `source`, and bounded language metadata;
- code and Markdown cells only; raw cells are retained as container inventory
  but not semantically parsed;
- code language precedence: cell metadata, notebook `kernelspec.language`,
  notebook `language_info.name`, then explicit magic recognition;
- only languages already present in the static Compass registry are parsed;
- line/cell magics are stripped or diagnosed by a small allowlist; they never
  execute or select arbitrary parsers.

Publish a notebook container node, ordered cell nodes, `contains` edges, and
the delegated code/Markdown facts. Every delegated fact retains a
`NotebookCoordinate { notebook, cell_id, cell_index, cell_line }` extension.
For nbformat cell IDs, use the validated ID. For old notebooks without IDs,
derive `cell-<source-digest>-<duplicate-ordinal>` so inserting a different cell
does not rename every later cell. The source file remains the real notebook
path; cell coordinates are separate typed provenance, not fragments smuggled
into filesystem paths.

## Limits

Define named constants and typed limit errors for at least:

- notebook bytes;
- cell count;
- source bytes per cell and total source bytes;
- metadata depth/bytes;
- output and attachment bytes inspected (normally zero content retained);
- delegated nodes, edges, diagnostics, and nested source locations.

A limit error is not an empty notebook. Structural errors may publish a partial
notebook only when the normal publication diagnostics explicitly report the
omission and at least one safe cell remains.

## Commands executors will need

| Purpose | Command | Expected result |
| --- | --- | --- |
| Target preflight | `test -d /Volumes/Workspace && mkdir -p /Volumes/Workspace/crabbuild-target/compass-main && test -w /Volumes/Workspace/crabbuild-target/compass-main` | exit 0 |
| Language tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-languages --locked` | pass |
| Resolution/graph tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-resolve -p compass-graph --locked` | pass |
| CLI contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-cli --test compass_product --locked` | pass |
| Qualification | `./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-languages -p compass-resolve -p compass-graph --all-targets --locked -- -D warnings` | exit 0 |
| Format/boundary | `cargo fmt --all -- --check && sh scripts/check_product_boundary.sh` | exit 0 |

## Scope

**In scope**:

- classification/registry, notebook decoding, cell delegation, resolution and
  graph publication;
- optional source-coordinate additions in `compass-model`;
- output/viewer projection needed to display and navigate cell locations;
- small, synthetic fixtures for Jupyter and Databricks metadata;
- docs, compatibility, performance evidence, and snapshots directly affected.

**Out of scope**:

- executing notebooks, kernels, widgets, or arbitrary magics;
- importing cell outputs, secrets, binary attachments, or widget state;
- `.dbc` archives or hosted Databricks API ingestion;
- runtime grammar downloads, Python dependencies, or notebook conversion tools;
- inferring dataflow from execution counts or output values.

## Phase 1: Version the notebook and cell-location contract

**Context**: Code-cell facts need a real filesystem source plus a stable cell
coordinate. Reusing `path#fragment` would break path containment and editor
navigation, so location must be typed.

**Deliverables**:

1. Add `NotebookCoordinate` as an additive, optional source/evidence field in
   `compass-model`, with strict validation, canonical serialization, and
   preservation through graph normalization, history, query, and export.
2. Add `ExtractorKind::Notebook`, `.ipynb` registry classification, producer
   version, and cache-fingerprint contribution.
3. Define `NotebookLimits`, `NotebookDiagnosticCode`, and a decoded internal
   model containing only bounded fields needed by later phases.
4. Add parsing tests for array/string `source`, CRLF, Unicode, valid and invalid
   cell IDs, duplicate IDs, missing IDs, malformed JSON, unknown major
   nbformat, deep metadata, oversized source/output, and deterministic order.

**Acceptance criteria**:

- `.ipynb` is classified as Notebook and no other `.json` file changes owner;
- unknown nbformat major and every hard limit return a distinct error;
- no output/attachment body is retained in the decoded model;
- identical notebooks serialize identically across repeated runs;
- old notebooks get stable digest-based cell identities with deterministic
  duplicate ordinals;
- `compass-model` and `compass-languages` tests/Clippy pass.

## Phase 2: Publish container, Markdown, and code-cell facts

**Context**: The notebook adapter composes existing extractors. It does not
become a second implementation of Python, Markdown, SQL, or Scala semantics.

**Deliverables**:

1. Publish one notebook node and one ordered node per safe cell, with
   `contains` edges and exact notebook/cell provenance.
2. Feed Markdown cell source to the existing Markdown structural extractor and
   code source to `Engine::extract_source` using the resolved static language.
3. Remap delegated IDs and anchors through the notebook/cell identity so equal
   declarations in different cells cannot collide.
4. Recognize Databricks nbformat metadata and a bounded allowlist of `%python`,
   `%sql`, `%scala`, `%r`, and `%sh` first-line magics. Record the magic and
   stripped line offset; unsupported/dynamic magics yield a diagnostic and no
   fabricated language.
5. Preserve import/call candidates between cells for the resolver, ordered by
   notebook cell order only where the language semantics justify it. Execution
   counts are descriptive metadata, never proof of order.

**Acceptance criteria**:

- Python, SQL, Markdown, shell, and one unsupported-language fixture publish
  the expected container/cell facts and diagnostics;
- code in two cells with the same symbol name has distinct source identities;
- cross-cell imports/calls resolve only when existing language rules produce a
  unique target; ambiguity remains explicit;
- output text containing code-like or secret-like strings produces no facts;
- anchors round-trip notebook path, cell ID/index, and cell-relative line;
- targeted language/resolution tests pass.

## Phase 3: Integrate discovery, incremental builds, queries, and navigation

**Context**: A one-cell edit should invalidate one notebook extraction unit,
not bypass publication coherence. Per-cell cache reuse is desirable but must
not create partial current output.

**Deliverables**:

1. Extend discovery/inventory and manifest tests so `.ipynb` participates in
   deterministic scope and hash changes.
2. Add a notebook cell cache keyed by notebook producer version, cell identity,
   language/profile version, normalized source digest, and meaning-affecting
   metadata. Publish the notebook atomically only after all selected cells and
   diagnostics are validated.
3. Preserve `NotebookCoordinate` through FTS/search, impact, traversal,
   CompassQL, HTML/VS Code source cards, GraphML/SVG/Wiki/Obsidian, and history
   diff. Unsupported editor jumps must display a cell locator rather than open
   a fake path.
4. Add incremental tests: unchanged notebook reuses every cell; editing one
   cell reuses the others; reordering cells preserves cell IDs but updates
   order; deleting a cell removes only its owned facts.

**Acceptance criteria**:

- cold and incremental notebook graphs are semantically equivalent;
- a two-cell edit reports exact analyzed/reused cell counts;
- graph, manifest, analysis, and store sidecar still publish as one coherent
  generation after a simulated failure;
- query results expose real notebook path plus cell locator and never a
  fragment path accepted by filesystem APIs;
- affected package tests and fixture qualification pass.

## Phase 4: Qualify and document notebook support

**Context**: Broad-language claims require evidence across positive, negative,
ambiguity, limits, and determinism cases.

**Deliverables**:

1. Add a reviewable notebook qualification corpus with synthetic Python,
   Markdown, SQL, Databricks magic, duplicate/missing IDs, malformed notebooks,
   large outputs, and cross-cell ambiguity. Do not check in private notebooks
   or generated output artifacts.
2. Add expected graph fixtures asserting identity, direction, multiplicity,
   anchors, language, provenance, diagnostics, and order.
3. Record cold/incremental latency and memory under `PERFORMANCE.md`; do not
   publish a performance claim unless the documented threshold passes.
4. Update README, language/document references, roadmap, compatibility,
   security/privacy, changelog, and migration notes if users must act.

**Acceptance criteria**:

- fixture qualification passes twice with byte-identical expected semantics;
- limit, corruption, and unsupported-language cases are non-empty diagnostics,
  not successful empty graphs;
- no test uses a real kernel, network, credentials, Python, or Databricks;
- documentation states exactly which notebook/magic forms and editor jumps are
  supported and which remain unresolved;
- applicable repository baseline/gates pass or are reported as not run.

## Done criteria

- [ ] All four phases meet their acceptance criteria.
- [ ] Notebook extraction is non-executing, bounded, deterministic, and atomic.
- [ ] Cell coordinates survive graph/query/history/output contracts.
- [ ] Incremental reuse is proven without weakening coherent publication.
- [ ] Qualification covers ambiguity, negative cases, and maliciously large data.
- [ ] `advisor-plans/README.md` marks this plan DONE.

## STOP conditions

Stop if cell provenance requires encoding a fragment as a filesystem path, if
the implementation needs Python or a runtime grammar download, if outputs must
be inspected beyond a bounded metadata count, or if per-cell reuse cannot be
published atomically. Also stop if a language cannot be selected from direct
metadata/magic evidence without guessing.

## Maintenance notes

Every adapter-version or cell-normalization change must invalidate only the
affected cell cache keys. Reviewers should scrutinize output/attachment
handling, source remapping, duplicate cell IDs, and the distinction between
cell order and execution order.
