# Rust Universal Phase 2 Hard-Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. This plan
> is implementation-first: production code precedes post-implementation
> conformance tests.

**Goal:** Deliver a quality-gated, version-1 Rust `UniversalCandidate` that
hard-cuts Rust graph production to the source-agnostic universal evidence,
resolution, and projection path.

**Architecture:** Split the current evidence model into a source-agnostic
contract, bounded builder, and local projector in `compass-languages`; add a
language-neutral collection resolver in `compass-resolve`; then make the
existing Rust AST traversal emit only universal evidence. The universal local
projector preserves the single-file `Engine` contract, while collection
resolution adds only uniquely resolved cross-file relationships.

**Tech Stack:** Rust 2024, Tree-sitter, `serde`, `serde_json`, `ahash`,
Compass raw extraction records, Compass normalized graph contracts, Python
3.11+ qualification harness.

## Global Constraints

- Keep `compass.languages.evidence/1`.
- Keep Rust adapter version 1 and profile `UniversalCandidate`.
- Do not dual-run, translate, feature-flag, or retain the replaced Rust graph
  publisher.
- Keep every non-Rust language on its existing extraction algorithm.
- Central resolution and publication must not branch on language names.
- Implement production behavior before adding or updating tests.
- Bevy strict edge coverage must exceed 69.64%.
- Bevy call coverage must remain at least 88.67%.
- Compass must remain faster than Graphify in comparable cold and warm Bevy
  workloads.
- Peak RSS is measured and reported but is not a blocking gate.

---

### Task 1: Make Qualification Runs Reproducible

**Context:** The first Bevy comparison needed two manual workarounds:
`CompassAdapter.prepare` honored `CARGO_TARGET_DIR` during `cargo build` but
looked for the executable under the source worktree, and Graphify wrote
`graphify-out/cache/stat-index.json` into the corpus checkout. The second
side effect made the incremental mutation guard reject an otherwise valid
run. The harness also resolves repository HEAD independently for every run,
which cannot prove that a legacy baseline and a later candidate used the same
corpus and Graphify revisions. Fix these harness boundaries before using
qualification as a cutover gate.

**Files:**

- Modify: `benchmarks/performance/compass_perf/adapters.py`
- Modify: `benchmarks/performance/compass_perf/workloads.py`
- Modify: `benchmarks/performance/compass_perf/workspace.py`
- Modify: `benchmarks/performance/harness.py`
- Modify: `benchmarks/performance/tests/test_adapters.py`
- Modify: `benchmarks/performance/tests/test_workloads.py`
- Modify: `benchmarks/performance/tests/test_workspace.py`
- Modify: `benchmarks/performance/tests/test_harness.py`

**Interfaces:**

- Consumes: `CARGO_TARGET_DIR`, `ToolAdapter`, `run_build_matrix`,
  `guarded_remove`.
- Produces:

```python
class ToolAdapter:
    def cleanup_checkout(self, checkout: Path) -> None:
        """Remove only tool-owned, generated checkout side effects."""

def cargo_target_directory(source_root: Path) -> Path:
    """Resolve CARGO_TARGET_DIR exactly as Cargo does."""

def requested_repository_commits(
    values: Sequence[str],
    selected: Sequence[RepositorySpec],
) -> dict[str, str]:
    """Validate NAME=SHA overrides for exactly the selected corpora."""
```

  `run` and `compare` additionally accept repeatable
  `--repository-commit NAME=SHA`; `compare` accepts
  `--graphify-commit SHA`. Without overrides, behavior remains current remote
  HEAD resolution. With an override, checkout preparation verifies the exact
  object and records the current default branch for provenance, but does not
  require the requested SHA to equal remote HEAD.

- [ ] **Step 1: Implement Cargo target-directory resolution**

  Add `cargo_target_directory(source_root)` to `adapters.py`. Absolute
  `CARGO_TARGET_DIR` values remain absolute; relative values resolve against
  `source_root`; an unset value resolves to `source_root / "target"`.
  `CompassAdapter.prepare` must build and validate
  `<resolved-target>/release/compass`.

- [ ] **Step 2: Implement adapter checkout cleanup**

  Add a no-op `ToolAdapter.cleanup_checkout`. Override it in
  `GraphifyAdapter` to remove only `<checkout>/graphify-out` through
  `guarded_remove`. Invoke cleanup after every measured command and before
  mutation status validation. Cleanup time must remain outside tool timing.

- [ ] **Step 3: Implement exact revision pinning**

  Parse and validate repository commit overrides centrally in `harness.py`.
  Reject malformed SHAs, duplicate names, unselected repositories, and
  missing requested objects. Pass an optional exact commit to
  `GraphifyAdapter.prepare`. Extend `prepare_checkout` with an explicit pinned
  mode that retains exact-checkout validation while skipping only the
  remote-HEAD-equality assertion. Persist every effective corpus and tool
  commit in `run.json`.

- [ ] **Step 4: Add post-implementation harness tests**

  Add tests covering unset, absolute, and relative Cargo target directories,
  Compass executable selection, Graphify-only cleanup, and a build matrix
  where `graphify-out/cache/stat-index.json` is created on every run. Add
  parser and checkout tests proving exact corpus and Graphify commits are
  honored and invalid or mismatched overrides fail closed.

- [ ] **Step 5: Verify the harness**

  Run:

```bash
python3 -m unittest \
  benchmarks.performance.tests.test_adapters \
  benchmarks.performance.tests.test_workloads \
  benchmarks.performance.tests.test_harness
```

  Expected: all selected harness tests pass and the checkout remains clean.

- [ ] **Step 6: Commit**

```bash
git add benchmarks/performance
git commit -m "fix(perf): isolate qualification tool artifacts"
```

---

### Task 2: Build the Source-Agnostic Evidence Contract

**Context:** `crates/compass-languages/src/universal.rs` currently contains
only unbounded data structures. Rust Phase 2 and every later adapter need the
same builder, diagnostics, sorting, and resource limits. Preserve serialized
compatibility by adding fields with Serde defaults; adapter version remains 1.
The extraction semantics version changes because cached Rust graph facts are
no longer produced by the old path.

**Files:**

- Delete: `crates/compass-languages/src/universal.rs`
- Create: `crates/compass-languages/src/universal/mod.rs`
- Create: `crates/compass-languages/src/universal/evidence.rs`
- Create: `crates/compass-languages/src/universal/builder.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Create: `crates/compass-languages/tests/universal_contract.rs`

**Interfaces:**

- Consumes: `AdapterDescriptor`, `SourceAnchor`, the existing evidence
  records, and source line numbers supplied by adapters.
- Produces:

```rust
pub const UNIVERSAL_EVIDENCE_SCHEMA: &str = "compass.languages.evidence/1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvidenceLimits {
    pub declarations: usize,             // default 16_384
    pub scopes: usize,                   // default 16_384
    pub bindings: usize,                 // default 4_096
    pub occurrences: usize,              // default 65_536
    pub relationship_candidates: usize,  // default 65_536
    pub scope_depth: usize,               // default 256
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd,
         Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    Contains,
    References,
    Calls,
    Imports,
    Extends,
    Implements,
    AnnotatedBy,
    InvokesMacro,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd,
         Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceRole {
    Call,
    ConstructorCall,
    Import,
    TypeReference,
    TraitBound,
    Annotation,
    Inheritance,
    Implementation,
    MacroInvocation,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd,
         Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingKind {
    Import,
    StaticImport,
    WildcardImport,
    Alias,
    Module,
    Package,
}

pub struct EvidenceBuilder {
    // Private bounded collections and diagnostics.
}

impl EvidenceBuilder {
    pub fn new(
        adapter: &AdapterDescriptor,
        source_file: &Path,
        limits: EvidenceLimits,
    ) -> Self;

    pub fn declare(&mut self, fact: DeclarationFact);
    pub fn scope(&mut self, fact: ScopeFact);
    pub fn bind(&mut self, fact: BindingFact);
    pub fn occur(&mut self, fact: OccurrenceFact);
    pub fn candidate(&mut self, fact: RelationshipCandidate);
    pub fn finish(self) -> UniversalEvidence;
}
```

  Extend `DeclarationKind` with `Package`, `Namespace`, `Class`,
  `Interface`, `Record`, `AnnotationType`, `Constructor`, and `EnumMember`.
  `DeclarationFact` carries `symbol`, `name`, `qualified_name`, `kind`,
  optional `owner`, optional normalized `signature`, `scope`, exact `anchor`,
  and one-based `line`. `ScopeFact`, `BindingFact`, `OccurrenceFact`, and
  `RelationshipCandidate` also carry a stable scope and one-based line where
  applicable. A relationship candidate explicitly carries
  `relationship: RelationshipKind`, occurrence role, spelling, qualifier,
  target kinds, optional normalized signature, optional argument count,
  anchor, scope, and optional external identity. Add `language`, `complete`,
  and bounded diagnostics to `UniversalEvidence`. Existing serialized batches
  must deserialize through defaults.

- [ ] **Step 1: Split and extend the production evidence model**

  Move the existing public types into `universal/evidence.rs`, add the fields
  and enums above, and re-export them from `universal/mod.rs` and `lib.rs`.
  Keep schema and adapter version values unchanged.

- [ ] **Step 2: Implement the bounded builder**

  Implement deterministic collection limits, one diagnostic per exhausted
  collection, scope-depth validation, stable sorting, and exact
  deduplication. `finish()` sets `complete = false` after any limit or
  structural violation.

- [ ] **Step 3: Invalidate old extraction caches**

  Change `EXTRACTION_SEMANTICS_VERSION` from
  `compass.languages.extraction/2` to `compass.languages.extraction/3`.
  Do not change `UNIVERSAL_EVIDENCE_SCHEMA` or adapter versions.

- [ ] **Step 4: Add post-implementation contract tests**

  Cover old-batch deserialization, deterministic sort/deduplication, each
  collection limit, invalid scope parentage, exact UTF-8 byte anchors, and
  source-agnostic use with synthetic Rust, Java, and config adapter
  descriptors.

  Representative assertion:

```rust
let first = build_evidence(input_order_a);
let second = build_evidence(input_order_b);
assert_eq!(first, second);
assert_eq!(first.schema, "compass.languages.evidence/1");
assert_eq!(first.adapter_version, 1);
```

- [ ] **Step 5: Verify the contract**

```bash
cargo fmt --all
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-languages --test universal_contract
```

- [ ] **Step 6: Commit**

```bash
git add crates/compass-languages
git commit -m "feat(languages): add bounded universal evidence builder"
```

---

### Task 3: Add the Universal Projector and Collection Resolver

**Context:** A hard-cut adapter must still return a useful graph from
`Engine::extract_source`, while the corpus pipeline needs cross-file package
and import resolution. The local projector therefore creates declaration,
containment, and uniquely local relationship facts from one batch. The
collection resolver consumes all batches after merge and adds only uniquely
resolved cross-file edges. Both components use evidence fields rather than
language-name branches.

**Files:**

- Create: `crates/compass-languages/src/universal/projector.rs`
- Modify: `crates/compass-languages/src/universal/mod.rs`
- Create: `crates/compass-resolve/src/universal.rs`
- Modify: `crates/compass-resolve/src/lib.rs`
- Create: `crates/compass-resolve/tests/universal_resolution.rs`

**Interfaces:**

- Consumes: completed `UniversalEvidence` batches and existing raw graph
  records from legacy languages.
- Produces:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UniversalProjectionLimits {
    pub max_candidates_per_lookup: usize, // default 256
    pub max_import_expansion: usize,      // default 4_096
    pub max_scope_hops: usize,            // default 256
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UniversalProjectionReport {
    pub declarations: usize,
    pub relationships: usize,
    pub unresolved: usize,
    pub ambiguous: usize,
    pub diagnostics: Vec<String>,
}

pub fn project_local(
    extraction: &mut Extraction,
    evidence: &UniversalEvidence,
    limits: UniversalProjectionLimits,
) -> UniversalProjectionReport;

pub fn resolve_and_project(
    extraction: &mut Extraction,
    limits: UniversalProjectionLimits,
) -> UniversalProjectionReport;
```

- [ ] **Step 1: Implement local projection**

  Map declaration kinds to normalized `symbol_kind`, use
  `DeclarationFact.symbol` as the stable node ID, stamp exact source file,
  one-based line, start byte, end byte, qualified name, signature, and
  `_universal_adapter`. Derive `contains` from declaration ownership and
  scope parentage. Emit local relationship edges only when exactly one
  compatible declaration survives scope and kind filtering.

- [ ] **Step 2: Implement collection indexes**

  In `compass-resolve/src/universal.rs`, index declarations by adapter,
  repository scope, qualified identity, lexical scope, imported spelling,
  declaration kind, callable owner, and signature. Enforce the projection
  limits before allocating candidate result vectors.

- [ ] **Step 3: Implement fail-closed resolution**

  Resolve in this order: exact owner, exact lexical scope, explicit import,
  same package/module, explicit external identity. Multiple candidates remain
  unresolved. Preserve occurrence anchors on every projected edge and never
  use terminal-name-only fallback.

- [ ] **Step 4: Integrate collection resolution once**

  Call `universal::resolve_and_project` near the start of
  `finish_resolution`, before generic collision disambiguation. Process only
  non-legacy evidence batches. Do not alter nodes or edges belonging solely
  to legacy adapters.

- [ ] **Step 5: Add post-implementation resolution tests**

  Add synthetic multi-file tests for exact local resolution, explicit
  imports, same-package lookup, nested scopes, overload ambiguity, wildcard
  expansion limits, external placeholders, duplicate batches, deterministic
  ordering, and a legacy-language graph that remains byte-for-byte equal.

```rust
let resolved = resolve(&[caller, target], &sources);
let calls = resolved
    .edges
    .iter()
    .filter(|edge| edge.string("relation") == "calls")
    .collect::<Vec<_>>();
assert_eq!(calls.len(), 1);
assert_eq!(calls[0].string("start_byte"), expected_start.to_string());
```

- [ ] **Step 6: Verify projector and resolver**

```bash
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-languages --test universal_contract
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-resolve --test universal_resolution
```

- [ ] **Step 7: Commit**

```bash
git add crates/compass-languages crates/compass-resolve
git commit -m "feat(resolve): add universal evidence projection"
```

---

### Task 4: Emit Complete Rust Phase 2 Evidence

**Context:** `rust_lang.rs` currently emits direct legacy nodes and edges plus
only call occurrences/candidates. Convert its existing single Tree-sitter
walk to populate `EvidenceBuilder`. Do not add a second traversal for graph
publication; call `project_local` once after the evidence batch is finished.

**Files:**

- Modify: `crates/compass-languages/src/rust_lang.rs`
- Modify: `crates/compass-languages/src/registry.rs`
- Modify: `crates/compass-languages/src/facts.rs`
- Modify: `crates/compass-languages/tests/rust_universal_conformance.rs`
- Create: `crates/compass-languages/tests/rust_universal_phase2.rs`

**Interfaces:**

- Consumes: `EvidenceBuilder`, `project_local`, the existing Rust syntax tree,
  adapter descriptor `compass.rust` version 1.
- Produces a single version-1 batch advertising:

```rust
vec![
    AdapterCapability::Declarations,
    AdapterCapability::LexicalScopes,
    AdapterCapability::Namespaces,
    AdapterCapability::Traits,
    AdapterCapability::ImplOwnership,
    AdapterCapability::Macros,
    AdapterCapability::Imports,
    AdapterCapability::Calls,
    AdapterCapability::ExternalPackages,
]
```

- [ ] **Step 1: Convert Rust declarations and scopes**

  Emit module, trait, struct, enum, type-alias, function, method, field,
  constant, and macro declarations. Create explicit module, trait, impl, and
  callable scopes. Preserve inherent and trait impl owner identities without
  treating impl blocks as declarations with user-visible names.

- [ ] **Step 2: Parse Rust bindings**

  Recursively flatten `use` trees, including grouped paths, aliases, globs,
  `crate`, `self`, and `super`. Store imported spelling separately from
  canonical identity. Mark the first path component as external only when it
  is not local, `crate`, `self`, or `super`.

- [ ] **Step 3: Emit exact occurrences and candidates**

  Record calls, type references, trait bounds, macro invocations, and imports
  with exact byte ranges and one-based lines. Candidate target kinds must be
  explicit. Preserve repeated occurrences instead of collapsing by endpoint.

- [ ] **Step 4: Project one local graph**

  Finish the builder, invoke `project_local`, store the same evidence batch in
  `Extraction.universal_evidence`, and return the projected extraction.
  Remove direct Rust `add_node`, `add_edge`, and raw-call publication paths as
  their replacements become active.

- [ ] **Step 5: Update Rust registry capabilities**

  Keep ID `compass.rust`, schema `compass.languages.evidence/1`, version 1,
  and profile `UniversalCandidate`. Advertise exactly the capabilities listed
  above.

- [ ] **Step 6: Add post-implementation Rust tests**

  Add cases for nested modules, grouped and aliased uses, globs, inherent and
  trait impl ownership, same-named methods on different owners, trait bounds,
  macro definitions/invocations, repeated occurrences, Unicode byte ranges,
  external package identities, malformed syntax, and evidence limits.

```rust
assert_eq!(batch.adapter_version, 1);
assert_eq!(batch.profile, AdapterProfile::UniversalCandidate);
assert!(batch.declarations.iter().any(|fact| {
    fact.kind == DeclarationKind::Trait && fact.qualified_name == "api::Render"
}));
assert!(graph.edges.iter().all(|edge| {
    edge.string("relation") != "calls"
        || edge.attributes.contains_key("start_byte")
}));
```

- [ ] **Step 7: Verify Rust adapter tests**

```bash
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-languages \
  --test rust_universal_conformance \
  --test rust_universal_phase2 \
  --test semantic_producers \
  --test engine_edge_coverage
```

- [ ] **Step 8: Commit**

```bash
git add crates/compass-languages
git commit -m "feat(languages): emit Rust phase 2 universal evidence"
```

---

### Task 5: Hard-Cut Rust and Remove the Replaced Path

**Context:** A universal evidence batch is not a hard cut while
`semantic::enrich` still runs the Rust-specific semantic publisher. Remove the
replaced route atomically after Task 4 produces the complete candidate graph.
Program IR extraction and Rust framework detection remain because they are
separate consumers, not competing graph publishers.

**Files:**

- Modify: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-languages/src/semantic.rs`
- Modify: `crates/compass-resolve/src/members.rs`
- Modify: `crates/compass-languages/tests/program_evidence.rs`
- Modify: `crates/compass-resolve/tests/universal_resolution.rs`

**Interfaces:**

- Consumes: `Registry::adapter(spec.name).profile`.
- Produces this dispatch boundary:

```rust
let descriptor = Registry::adapter(spec.name);
if descriptor.profile == AdapterProfile::Legacy {
    attach_definition_metadata(&mut extraction, source, root, &config, spec.name);
    semantic::enrich(path, source, root, spec.name, &mut extraction);
}
```

  Universal adapters supply equivalent exact metadata through
  `project_local`; framework detection still runs after this boundary.

- [ ] **Step 1: Bypass legacy post-processing for universal adapters**

  Implement the profile-based dispatch above. Keep Python, TypeScript,
  C#, and all other legacy behavior unchanged.

- [ ] **Step 2: Delete Rust semantic publisher code**

  Remove the `"rust" => state.rust()` branch and Rust-only declaration,
  alias, override, test, impl-owner, callable, and helper code from
  `semantic.rs`. Retain shared inventory machinery used by TypeScript and C#.

- [ ] **Step 3: Remove remaining legacy Rust graph resolution**

  Ensure the Rust adapter emits no `RawCall` facts for the language member
  resolver. Remove any Rust-specific graph compatibility branch that becomes
  unreachable, while preserving Rust Program IR and framework resolution.

- [ ] **Step 4: Add post-cutover assertions**

  Assert Rust extraction has exactly one universal evidence batch, every
  projected Rust declaration carries `_universal_adapter = "compass.rust"`,
  and no `_semantic_work` extension or legacy raw-call buffer remains.
  Assert TypeScript and C# still retain their existing semantic work metadata.

- [ ] **Step 5: Verify the hard-cut boundary**

```bash
rg -n '"rust" => state\\.rust|fn rust\\(&mut self\\)|RawCall' \
  crates/compass-languages/src/semantic.rs \
  crates/compass-languages/src/rust_lang.rs
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-languages
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-resolve
```

  Expected: no replaced Rust semantic publisher match, no Rust raw-call
  output, and both full suites pass.

- [ ] **Step 6: Commit**

```bash
git add crates/compass-languages crates/compass-resolve
git commit -m "refactor(languages): hard cut Rust to universal projection"
```

---

### Task 6: Qualify Rust Fixtures and Determinism

**Context:** Tests prove contracts, not graph parity. Compare final Rust output
against the nine checked fixtures and repeat extraction to catch ordering or
identity instability before spending time on Bevy.

**Files:**

- Create: `benchmarks/performance/tools/compare_language_fixtures.py`
- Create: `benchmarks/performance/tests/test_language_fixture_compare.py`
- Modify: `docs/superpowers/reviews/2026-07-30-rust-universal-adapter-phase-1.md`

**Interfaces:**

- Consumes: Compass and Graphify graph JSON plus a language-filtered fixture
  manifest.
- Produces normalized counts by relation and status:

```text
language, fixture, relation, graphify_total, exact, dominated,
rejected, ambiguous, missing
```

- [ ] **Step 1: Implement reusable fixture comparison**

  Reuse `compass_perf.correctness` indexing and classification instead of
  introducing a second normalization algorithm. Accept repeated
  `--fixture <path>` arguments and emit JSON plus Markdown.

- [ ] **Step 2: Add post-implementation comparator tests**

  Cover relation normalization, exact occurrence preservation, rejected
  qualified-external rebinding, missing endpoints, deterministic ordering,
  and invalid graph input.

- [ ] **Step 3: Run Rust fixture extraction three times**

  Use the Graphify `sample.rs` fixture plus the eight checked Compass Rust
  fixtures. Require identical canonical graph digests across repetitions.

- [ ] **Step 4: Enforce the fixture gate**

  Require all previously covered Graphify fixture facts to remain covered,
  calls to remain at least 11, and no relation family to gain missing or
  ambiguous facts.

- [ ] **Step 5: Record the Phase 2 fixture result**

  Add a distinct Phase 2 section to the Rust evidence review. Do not overwrite
  the Phase 1 Bevy data.

- [ ] **Step 6: Verify and commit**

```bash
python3 -m unittest \
  benchmarks.performance.tests.test_language_fixture_compare
git add benchmarks/performance docs/superpowers/reviews
git commit -m "test(quality): qualify Rust universal fixtures"
```

---

### Task 7: Run the Pinned Bevy Quality Gate

**Context:** Rust Phase 2 cannot hand off to Java until the approved Bevy
gates pass. Preserve the existing run summary before cleaning generated
artifacts. Use the repaired harness rather than symlinks or process-scoped Git
excludes.

**Files:**

- Modify: `docs/superpowers/reviews/2026-07-30-rust-universal-adapter-phase-1.md`
- Generated, ignored:
  `target/performance/runs/bevy-rust-phase2/run.json`
- Generated, ignored:
  `target/performance/runs/bevy-rust-phase2/summary.md`

**Interfaces:**

- Consumes: pinned Bevy repository definition and the final clean Rust commit.
- Produces: three-sample cold, warm, incremental, and restore results, strict
  graph coverage, speed ratios, peak RSS, and deterministic digests.

- [ ] **Step 1: Run preflight and prepare**

```bash
python3 benchmarks/performance/harness.py doctor --repository bevy
python3 benchmarks/performance/harness.py prepare --repository bevy
```

  Read the exact Bevy commit from `preparation.json` and the exact Graphify
  commit from the Phase 1 review. Do not allow either revision to float.

- [ ] **Step 2: Run the official comparison**

```bash
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
python3 benchmarks/performance/harness.py compare \
  --repository bevy \
  --repository-commit bevy=<BEVY_SHA> \
  --graphify-commit <GRAPHIFY_SHA> \
  --workload build \
  --build-repeats 3 \
  --output target/performance/runs/bevy-rust-phase2
```

  The harness may return nonzero when Graphify's own incremental
  determinism gate fails. If both tools produced complete eligible samples
  and the report was written, preserve and evaluate the report; do not
  misclassify that known Graphify failure as a missing Compass result.

- [ ] **Step 3: Evaluate blocking gates**

  Continue only if strict edge coverage is greater than 69.64%, call coverage
  is at least 88.67%, all Compass workload digests are stable, all Compass
  samples are eligible, and Compass cold/warm medians are faster than
  Graphify. If one fails, return to the responsible production task, make one
  scoped correction, rerun its post-implementation tests, and repeat this
  gate before beginning Java.

- [ ] **Step 4: Record quality and performance evidence**

  Record node/edge counts, per-relation coverage, cold/warm/incremental
  medians, p95, RSS, Graphify correctness failures, and any non-blocking
  memory regression in the Rust review.

- [ ] **Step 5: Final Rust verification**

```bash
cargo fmt --all -- --check
git diff --check
git status --short
```

- [ ] **Step 6: Commit the Rust Phase 2 evidence**

```bash
git add docs/superpowers/reviews/2026-07-30-rust-universal-adapter-phase-1.md
git commit -m "docs(quality): record Rust phase 2 qualification"
```
