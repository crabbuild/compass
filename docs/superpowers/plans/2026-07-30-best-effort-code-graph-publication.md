# Best-effort code graph publication implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task by task. Execute inline in the existing isolated worktree. Do not delegate tasks unless the user explicitly authorizes subagents.

**Goal:** Publish a strictly queryable `compass.graph/1` artifact after deterministically quarantining invalid individual nodes and edges.

**Architecture:** Keep the current strict normalizer and full validator as qualification APIs. Add an explicit best-effort normalizer that catches record-level failures, tracks exact bounded omission diagnostics, sanitizes typed records, and runs the unchanged strict validator before publication. Normal builds use the best-effort path, while queries disclose partial coverage through the existing `incomplete_coverage` diagnostic.

**Tech stack:** Rust 2024, Serde, `compass-model`, `compass-graph`, `compass-core`, `compass-query`, `compass-cli`, Cargo workspace tests, official heavyweight framework repositories

## Global constraints

- Use implementation-first sequencing. Add focused tests after each implementation slice; do not use a red-green TDD sequence.
- Preserve `normalize_v1` and `normalize_document_v1_with_inventory` as strict APIs.
- Every published graph must pass the existing full `validate_code_graph` function.
- Record-level failures may remove records but may not invent node kinds, edge kinds, endpoint identities, provenance, source anchors, or wiring sites.
- Document-level failures remain fatal: invalid envelope, invalid file inventory, unsafe paths, unreadable authoritative input, empty usable graph, serialization failure, and atomic publication failure.
- Best-effort output and diagnostics must be byte-deterministic under reordered raw input.
- Keep at most 100 node examples, 100 edge examples, and 100 identity-collision examples while preserving exact total counts.
- Preserve last-known-good generation behavior.
- Do not add a second durable graph artifact or schema.
- Do not mention retired products in code, documentation, commits, or pull-request text.

## File map

- `crates/compass-model/src/validation.rs`: Structured document, node, and edge validation report plus the unchanged strict aggregate validator
- `crates/compass-model/src/lib.rs`: Public validation report exports
- `crates/compass-model/tests/code_graph_validation.rs`: Structured validation and strict compatibility tests
- `crates/compass-graph/src/quarantine.rs`: Omission counters, bounded diagnostic collection, and publication outcome types
- `crates/compass-graph/src/v1.rs`: Strict and best-effort normalization modes, deterministic raw-record ordering, record quarantine, route repair, and final validation
- `crates/compass-graph/src/lib.rs`: Best-effort publication API exports
- `crates/compass-graph/tests/graph_v1_normalization.rs`: Best-effort normalization, determinism, collision, endpoint, and route tests
- `crates/compass-core/src/pipeline.rs`: Default best-effort publication, build statistics, output statistics, and atomic pipeline integration
- `crates/compass-core/src/build_state.rs`: Backward-compatible persisted omission statistics
- `crates/compass-core/tests/loading_coverage.rs`: Pipeline partial-publication and last-known-good coverage
- `crates/compass-query/src/code_query.rs`: Partial graph detection and query diagnostic injection
- `crates/compass-query/tests/code_search.rs`: Search disclosure coverage
- `crates/compass-query/tests/code_calls.rs`: Callers and callees disclosure coverage
- `crates/compass-cli/src/lib.rs`: Human-readable partial-publication warning
- `docs/concepts/graph-model.md`: Partial graph and quarantine semantics
- `docs/reference/outputs.md`: Durable diagnostic contract
- `docs/reference/commands.md`: Build success warning
- `docs/guides/operations.md`: Operational interpretation
- `docs/cookbook/troubleshooting.md`: Partial graph remediation
- `docs/superpowers/specs/2026-07-27-compass-code-graph-v1-design.md`: Design amendment

## Task 1: Add structured graph validation

**Files:**

- Modify: `crates/compass-model/src/validation.rs`
- Modify: `crates/compass-model/src/lib.rs`
- Modify: `crates/compass-model/tests/code_graph_validation.rs`

**Interfaces:**

- Produces: `CodeGraphValidationReport`
- Produces: `RecordValidationErrors`
- Produces: `validate_code_graph_records(&CodeGraphDocument) -> CodeGraphValidationReport`
- Preserves: `validate_code_graph(&CodeGraphDocument) -> Result<(), CodeGraphValidationError>`

- [ ] **Step 1: Implement typed validation reporting**

Add ordered public report types:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordValidationErrors {
    pub id: String,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodeGraphValidationReport {
    pub document_errors: Vec<String>,
    pub node_errors: Vec<RecordValidationErrors>,
    pub edge_errors: Vec<RecordValidationErrors>,
}

impl CodeGraphValidationReport {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.document_errors.is_empty()
            && self.node_errors.is_empty()
            && self.edge_errors.is_empty()
    }
}
```

Refactor the current validation loop so document and file-inventory failures enter `document_errors`, node failures enter the matching node record, and edge failures enter the matching edge record. Keep traversal order stable.

- [ ] **Step 2: Preserve strict error text and ordering**

Implement `validate_code_graph` by flattening the structured report in the current order. Keep `CodeGraphValidationError.errors` and its display format unchanged so existing strict tests and callers retain compatible behavior.

- [ ] **Step 3: Export the report API**

Export the new types and `validate_code_graph_records` from `crates/compass-model/src/lib.rs`.

- [ ] **Step 4: Format and compile the model crate**

Run:

```bash
cargo fmt --all
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo check -p compass-model --locked
```

Expected: both commands exit 0.

- [ ] **Step 5: Add focused tests**

Add tests proving:

- invalid metadata appears only in `document_errors`
- an invalid source anchor appears under its node ID
- an invalid endpoint-kind pair appears under its edge ID
- the strict aggregate error list remains identical to the expected ordering
- a valid document produces an empty report

- [ ] **Step 6: Run model validation tests**

Run:

```bash
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo test -p compass-model --test code_graph_validation --locked
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/compass-model/src/validation.rs crates/compass-model/src/lib.rs crates/compass-model/tests/code_graph_validation.rs
git commit -m "feat(graph): expose structured validation results"
```

## Task 2: Implement bounded quarantine accounting

**Files:**

- Create: `crates/compass-graph/src/quarantine.rs`
- Modify: `crates/compass-graph/src/lib.rs`
- Modify: `crates/compass-graph/src/v1.rs`
- Modify: `crates/compass-graph/tests/graph_v1_normalization.rs`

**Interfaces:**

- Consumes: `CodeGraphValidationReport`
- Produces: `PublicationOutcome`
- Produces: `PublicationOmissions`
- Produces: `normalize_v1_best_effort(Extraction, BuildEvidence) -> Result<PublicationOutcome, GraphError>`

- [ ] **Step 1: Add publication outcome types**

Create `quarantine.rs` with:

```rust
pub const MAX_QUARANTINE_EXAMPLES: usize = 100;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PublicationOmissions {
    pub nodes: usize,
    pub edges: usize,
    pub identity_collisions: usize,
    pub examples_omitted: usize,
}

#[derive(Clone, Debug)]
pub struct PublicationOutcome {
    pub document: GraphDocument,
    pub omissions: PublicationOmissions,
}

impl PublicationOmissions {
    #[must_use]
    pub const fn is_partial(self) -> bool {
        self.nodes > 0 || self.edges > 0 || self.identity_collisions > 0
    }
}
```

Add a crate-private collector that records exact counters and at most 100 diagnostics per record category. Use warning codes `publication_omitted_node`, `publication_omitted_edge`, and `publication_identity_collision`. Add one `publication_omission_summary` diagnostic when any counter is nonzero.

- [ ] **Step 2: Add explicit strict and best-effort modes**

Refactor `normalize_v1` through:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationMode {
    Strict,
    BestEffort,
}

fn normalize_v1_with_mode(
    extraction: Extraction,
    evidence: BuildEvidence,
    mode: PublicationMode,
) -> Result<PublicationOutcome, GraphError>
```

`normalize_v1` calls strict mode and returns `outcome.document`. `normalize_v1_best_effort` returns the complete outcome.

- [ ] **Step 3: Deterministically normalize or quarantine raw nodes**

In best-effort mode, sort raw nodes with a canonical key derived from raw ID, declared kind, qualified name, portable source data, and serialized attributes.

For each node:

- quarantine normalization failures
- quarantine duplicate raw IDs after the deterministic first record
- merge compatible stable identities
- select a deterministic survivor for incompatible stable-identity collisions
- leave quarantined raw IDs absent from `id_remap`

Use the design ranking: exact AST evidence, exact source-backed evidence, inferred source-backed evidence, exact wiring-site evidence, then canonical serialized order.

- [ ] **Step 4: Deterministically normalize or quarantine raw edges**

In best-effort mode, sort raw edges by raw source, relation, raw target, source site, occurrence rule, and serialized attributes.

For each edge:

- quarantine it if either endpoint lacks an `id_remap` entry
- quarantine `normalize_edge` or `normalize_trusted_edge` failures
- merge compatible duplicates
- quarantine conflicting duplicate edge identities

Do not alias an unknown relation to `references`.

- [ ] **Step 5: Handle unwired generic external placeholders**

Change `resolve_or_drop_generic_symbols` to accept publication mode and a quarantine collector. Strict mode keeps the current error. Best-effort mode omits an unwired placeholder and its incident edges with `publication_omitted_node`.

- [ ] **Step 6: Format and compile the graph crate**

Run:

```bash
cargo fmt --all
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo check -p compass-graph --locked
```

Expected: both commands exit 0.

- [ ] **Step 7: Add raw-record quarantine tests**

Add tests for:

- unknown relation omits one edge and retains both nodes
- missing wiring site omits the placeholder and its incident edge
- invalid raw node kind omits its node and incident edge
- duplicate raw ID retains one deterministic record
- Razor-style method collision retains one method
- Rust-style repeated module collision retains one module
- reversing node and edge input produces identical serialized output
- strict `normalize_v1` still returns the original errors

- [ ] **Step 8: Run graph normalization tests**

Run:

```bash
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo test -p compass-graph --test graph_v1_normalization --locked
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/compass-graph/src/quarantine.rs crates/compass-graph/src/lib.rs crates/compass-graph/src/v1.rs crates/compass-graph/tests/graph_v1_normalization.rs
git commit -m "feat(graph): quarantine invalid raw records"
```

## Task 3: Sanitize typed records and repair framework topology

**Files:**

- Modify: `crates/compass-graph/src/v1.rs`
- Modify: `crates/compass-graph/tests/graph_v1_normalization.rs`

**Interfaces:**

- Consumes: `validate_code_graph_records`
- Produces: `sanitize_document(&mut GraphDocument, &mut QuarantineCollector) -> Result<(), GraphError>`
- Preserves: final `validate_code_graph(&document)` gate

- [ ] **Step 1: Implement typed sanitization**

After constructing the typed document:

1. Call `validate_code_graph_records`.
2. Return `InvalidCodeGraph` when `document_errors` is nonempty.
3. Quarantine every node in `node_errors`.
4. Remove every edge incident to a quarantined node.
5. Quarantine every remaining edge in `edge_errors`.
6. Append bounded diagnostics and the exact summary.
7. Run the existing strict validator.

Use ID sets for linear filtering. Do not repeatedly clone the complete graph.

- [ ] **Step 2: Repair route stages after edge removal**

Build the set of retained `routes_to` stage tuples `(route_id, stage, position, target_id)`. For every route:

- clear a stage target when no retained tuple matches it
- remove candidates that reference quarantined nodes
- set the stage to `unresolved` when no target remains
- call the current `recompute_route_resolution`

The route node and its framework evidence remain when only its handler edge fails.

- [ ] **Step 3: Repair provenance candidates and diagnostics**

Remove candidate IDs that do not correspond to retained nodes from node evidence, edge evidence, and route stages. Remove related IDs that reference quarantined records from new quarantine diagnostics.

- [ ] **Step 4: Add typed sanitization tests**

Add tests proving:

- `calls` from method to module is omitted
- invalid `contains` from class to module is omitted
- all incident edges disappear when a node fails typed validation
- a route becomes unresolved when its `routes_to` edge is omitted
- every best-effort result passes `validate_code_graph`
- diagnostic caps retain exact summary counts

- [ ] **Step 5: Run graph tests and Clippy**

Run:

```bash
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo test -p compass-graph --locked
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo clippy -p compass-graph --all-targets --locked -- -D warnings
```

Expected: both commands exit 0.

- [ ] **Step 6: Commit**

```bash
git add crates/compass-graph/src/v1.rs crates/compass-graph/tests/graph_v1_normalization.rs
git commit -m "feat(graph): sanitize partial graph topology"
```

## Task 4: Use best-effort publication in the build pipeline

**Files:**

- Modify: `crates/compass-graph/src/v1.rs`
- Modify: `crates/compass-graph/src/lib.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/build_state.rs`
- Modify: `crates/compass-core/tests/loading_coverage.rs`
- Modify: `crates/compass-cli/src/lib.rs`

**Interfaces:**

- Produces: `normalize_document_v1_with_inventory_best_effort(...) -> Result<PublicationOutcome, GraphError>`
- Extends: `BuildResult` with `omitted_nodes`, `omitted_edges`, `identity_collisions`, and `partial_graph`
- Extends persisted stats with backward-compatible `#[serde(default)]` fields

- [ ] **Step 1: Add the document-level best-effort adapter**

Add `normalize_document_v1_with_inventory_best_effort` beside the strict adapter. It constructs `Extraction` and `BuildEvidence` through the same code path, then calls `normalize_v1_best_effort`.

- [ ] **Step 2: Switch normal pipeline publication**

Use the best-effort adapter in no-cluster preflight, clustered preflight, and final publication. Write `outcome.document` only after successful final strict validation.

Keep semantic raw guards, generation staging, artifact sealing, and pointer swap unchanged.

- [ ] **Step 3: Extend build and persisted statistics**

Add these fields with zero defaults:

```rust
pub omitted_nodes: usize,
pub omitted_edges: usize,
pub identity_collisions: usize,
pub partial_graph: bool,
```

Persist them in `OutputStats` and `SavedStats`. Populate unchanged-build results from the saved statistics so cached builds report the same partial status.

- [ ] **Step 4: Report partial success in the CLI**

When `result.partial_graph` is true, append to stderr:

```text
Compass published a partial graph: N nodes and M edges omitted; C identity collisions quarantined.
```

Do not mention `compass diagnose publication` until that command exists.

- [ ] **Step 5: Add pipeline and CLI tests**

Add coverage for:

- extract succeeds with one invalid relation
- update succeeds with invalid endpoint-kind edges
- output `graph.json` passes strict load validation
- omission stats persist through an unchanged build
- a document-level failure preserves the active generation
- CLI stdout reports indexing success and stderr reports partial publication
- complete builds leave stderr unchanged

- [ ] **Step 6: Run affected tests**

Run:

```bash
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo test -p compass-core --test loading_coverage --locked
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo test -p compass-cli --lib --locked
```

Expected: both commands exit 0.

- [ ] **Step 7: Commit**

```bash
git add crates/compass-graph/src/v1.rs crates/compass-graph/src/lib.rs crates/compass-core/src/pipeline.rs crates/compass-core/src/build_state.rs crates/compass-core/tests/loading_coverage.rs crates/compass-cli/src/lib.rs
git commit -m "feat(core): publish usable partial graphs"
```

## Task 5: Disclose partial coverage in typed queries

**Files:**

- Modify: `crates/compass-query/src/code_query.rs`
- Modify: `crates/compass-query/tests/code_search.rs`
- Modify: `crates/compass-query/tests/code_calls.rs`

**Interfaces:**

- Consumes: `publication_omission_summary` graph diagnostic
- Produces: one `QueryDiagnosticCode::IncompleteCoverage` per typed response

- [ ] **Step 1: Parse the omission summary once**

When `CodeQueryEngine` opens the graph, derive an optional concise message from `graph.graph.diagnostics`. Store it on the engine:

```rust
partial_graph_message: Option<String>,
```

Use the summary diagnostic rather than scanning every omission example per query.

- [ ] **Step 2: Inject query disclosure**

At the beginning of `finish_response`, append one diagnostic when `partial_graph_message` is present:

```rust
QueryDiagnostic {
    code: QueryDiagnosticCode::IncompleteCoverage,
    message: message.clone(),
    node_id: None,
    path: None,
}
```

Also call the same helper on early successful returns caused by no match or ambiguity so every typed operation discloses partial coverage.

- [ ] **Step 3: Add search and call tests**

Test complete and partial graphs across search, callers, callees, impact, explore, and node trail. Assert one disclosure diagnostic, stable response ordering, and unchanged query nodes and edges.

- [ ] **Step 4: Run query tests and Clippy**

Run:

```bash
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo test -p compass-query --locked
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo clippy -p compass-query --all-targets --locked -- -D warnings
```

Expected: both commands exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-query/src/code_query.rs crates/compass-query/tests/code_search.rs crates/compass-query/tests/code_calls.rs
git commit -m "feat(query): disclose partial graph coverage"
```

## Task 6: Update the product documentation

**Files:**

- Modify: `docs/concepts/graph-model.md`
- Modify: `docs/reference/outputs.md`
- Modify: `docs/reference/commands.md`
- Modify: `docs/guides/operations.md`
- Modify: `docs/cookbook/troubleshooting.md`
- Modify: `docs/superpowers/specs/2026-07-27-compass-code-graph-v1-design.md`

**Interfaces:**

- Documents: default best-effort publication
- Documents: strict durable graph validation
- Documents: omission diagnostics and query disclosure

- [ ] **Step 1: Amend the original v1 design**

Add a dated amendment under validation and failure handling. State that record-level failures are quarantined before final validation, while document-level failures remain fatal.

- [ ] **Step 2: Update graph and output references**

Define partial graph, quarantine, diagnostic codes, exact summary counts, and the 100-example cap. Explain that unknown relations never enter `links`.

- [ ] **Step 3: Update command and operations guidance**

Document the CLI partial-success warning and tell operators to inspect graph diagnostics before interpreting absent topology.

- [ ] **Step 4: Update troubleshooting guidance**

Add remedies for high omission counts, repeated identity collisions, unknown producer relations, and unresolved framework handlers.

- [ ] **Step 5: Check documentation**

Run:

```bash
rg -n 'T[B]D|TO[D]O|FIX[M]E|compass diagnose publication' docs/concepts/graph-model.md docs/reference/outputs.md docs/reference/commands.md docs/guides/operations.md docs/cookbook/troubleshooting.md docs/superpowers/specs/2026-07-27-compass-code-graph-v1-design.md
git diff --check
```

Expected: no placeholders, no nonexistent diagnostic command, and no whitespace errors.

- [ ] **Step 6: Commit**

```bash
git add docs/concepts/graph-model.md docs/reference/outputs.md docs/reference/commands.md docs/guides/operations.md docs/cookbook/troubleshooting.md docs/superpowers/specs/2026-07-27-compass-code-graph-v1-design.md
git commit -m "docs(graph): explain best-effort publication"
```

## Task 7: Run workspace qualification

**Files:**

- Modify only files required to fix failures directly caused by Tasks 1 through 6

**Interfaces:**

- Verifies: strict API compatibility
- Verifies: best-effort publication behavior
- Verifies: query disclosure

- [ ] **Step 1: Run formatting and the full workspace test suite**

Run:

```bash
cargo fmt --all --check
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo test --workspace --lib --bins --tests --locked
```

Expected: formatting passes and all non-ignored tests pass.

- [ ] **Step 2: Run affected workspace Clippy**

Run:

```bash
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo clippy -p compass-model -p compass-graph -p compass-core -p compass-query -p compass-cli --all-targets --locked -- -D warnings
```

Expected: exit 0 with no warnings.

- [ ] **Step 3: Run semantic qualification**

Run:

```bash
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target bash scripts/qualify_code_graph_v1.sh --fixtures-only
```

Expected: the production binary builds and every fixture, invariant, framework flow, and deterministic comparison passes. Record the complete qualification summary in the handoff.

- [ ] **Step 4: Verify deterministic fixtures**

Run clean, warm-cache, forced, and reordered fixture builds. Hash each `graph.json` and assert byte identity when source and configuration are equal.

- [ ] **Step 5: Inspect the final diff**

Run:

```bash
git diff --check origin/main...HEAD
git status --short
git log --oneline origin/main..HEAD
```

Expected: no whitespace errors, only intended files, and scoped commits.

## Task 8: Requalify heavyweight frameworks

**Files:**

- No repository source modifications
- Write benchmark logs under `<qualification-corpus-root>/compass-heavy-framework-best-effort-20260730/`

**Interfaces:**

- Verifies: real framework publication
- Verifies: query usability
- Verifies: strict validity after quarantine

- [ ] **Step 1: Build the exact release binary**

Run:

```bash
CARGO_TARGET_DIR=<qualification-corpus-root>/compass-v1-remediation-target cargo build --release --locked -p compass-cli --bin compass
shasum -a 256 <qualification-corpus-root>/compass-v1-remediation-target/release/compass
```

Record the commit and binary hash.

- [ ] **Step 2: Run pinned cold builds sequentially**

Use the pinned checkouts from the design:

- Django `274a1d494d11d87a1b767340d1f398f197810f93`
- Spring Framework `317eae88d0746534974bf75487042e007b53f681`
- Angular `1a2bcb2295c8b4e3a398c8ecbd92cce0affaff1d`
- ASP.NET Core `2e05e269f599be5615cea4fcd2d27f7080f6e54f`
- Rails `0f36bbf72cc8b814bf1ad05c896c9c427b18217f`
- Laravel Framework `7e5b3aff7dcc0843758cc0ad83c383aab84596b8`
- Bevy `25368b78ce5e9b15dc770cdf2af4595602cc8a7b`

Run each with:

```bash
/usr/bin/time -l compass extract repository_path --code-only --out output_path --no-cluster --no-viz --max-workers 8
```

Apply a 10-minute observation ceiling per repository and run sequentially to avoid resource contention.

- [ ] **Step 3: Validate every published graph**

For every successful build, record:

- schema
- retained nodes and edges
- omitted nodes and edges
- identity collisions
- validation errors
- graph size
- wall time
- peak resident memory
- graph SHA-256
- route and `routes_to` counts

Spring Framework, ASP.NET Core, Rails, Laravel Framework, and Bevy must publish a graph with zero strict validation errors.

- [ ] **Step 4: Run real queries**

Run at least one symbol search and one callers or callees query against every published graph. Assert a typed `compass.query/1` response, bounded results, and `incomplete_coverage` when the graph is partial.

- [ ] **Step 5: Recheck determinism on a failing corpus**

Choose the corpus with the largest omission count. Run cold and forced builds and compare `graph.json` SHA-256 hashes.

- [ ] **Step 6: Record remaining performance blockers**

Report Angular separately if it exceeds 10 minutes. Do not classify a performance timeout as successful validation remediation.

- [ ] **Step 7: Commit any qualification fixtures or scripts**

Commit only reusable repository-owned qualification fixtures or scripts. Keep downloaded repositories, generated graphs, and raw benchmark logs outside Git.
