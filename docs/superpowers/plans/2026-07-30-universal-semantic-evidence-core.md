# Universal Semantic Evidence Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver increment one of the universal graph-quality design: a
versioned semantic evidence model, capability registry, legacy compatibility
projection, shared resolution-policy boundary, framework-pack contract, and
conformance harness without changing published graph topology.

**Architecture:** `compass-languages` owns language-neutral declarations,
scopes, bindings, occurrences, candidates, adapter profiles, and validation.
`compass-resolve` owns a bounded resolver over those facts.
Existing extractors and framework detectors continue to publish their current
raw graph facts while a deterministic compatibility projection populates the
new evidence batch in shadow mode. This gives later Python, Go, Java, Rust, and
framework migrations one contract without a flag day.

**Tech Stack:** Rust 2024, serde/serde_json, tree-sitter source ranges, ahash,
the existing Compass extraction cache, Python 3 standard library, SQLite, and
the existing performance harness.

## Global Constraints

- This is implementation-first, not red-green TDD: implement and review
  production behavior before adding focused tests.
- Public `compass.graph/1` bytes and topology must remain unchanged in this
  increment.
- Bump the internal extraction-semantics version because cached `Extraction`
  gains a typed evidence batch.
- No runtime Graphify dependency and no network access in production paths.
- No repository-wide terminal-label fallback.
- Fabricated occurrences, cross-language matches, and unsafe local-target
  substitutions are forbidden.
- Every collection and lookup has an explicit bound.
- Existing user changes and unrelated worktree changes must be preserved.
- After code changes, run `graphify update .` from `/Users/haipingfu/graphify`.

## Background

The current extraction contract is flexible:

- `RawNodeRecord` and `RawEdgeRecord` store most semantics in JSON maps;
- `RawCall` carries a separate language-specific extension map;
- framework detectors emit separate `RawFrameworkFact` records;
- collection resolution is an ordered series of language-specific passes; and
- adapter capability is inferred from language names and dispatch branches.

That flexibility enabled broad language coverage, but it makes occurrence,
scope, qualification, and resolution rules inconsistent. The universal core
must provide typed evidence without forcing all existing adapters to migrate
at once. Increment one therefore runs typed projection and validation in
shadow mode. Later increments switch complete adapters and framework packs to
produce the typed facts directly.

## File and responsibility map

- Create `crates/compass-languages/src/evidence/model.rs`: versioned universal
  evidence data model.
- Create `crates/compass-languages/src/evidence/validate.rs`: structural and
  resource validation for evidence batches.
- Create `crates/compass-languages/src/evidence/legacy.rs`: conservative
  projection from existing `Extraction` facts.
- Create `crates/compass-languages/src/evidence/mod.rs`: public internal API and
  limits.
- Create `crates/compass-languages/src/adapters.rs`: adapter profiles,
  capability registry, and maturity states.
- Modify `crates/compass-languages/src/facts.rs`: optional typed evidence batch
  on `Extraction`.
- Modify `crates/compass-languages/src/engine.rs`: populate and validate shadow
  evidence after extraction.
- Modify `crates/compass-languages/src/registry.rs`: expose the adapter profile
  associated with every resolved language.
- Modify `crates/compass-languages/src/frameworks/mod.rs`: typed framework-pack
  descriptors and registry validation while retaining current detectors.
- Create `crates/compass-resolve/src/evidence.rs`: bounded universal candidate
  index and deterministic resolution decisions.
- Modify `crates/compass-resolve/src/lib.rs`: validate/index the shadow evidence
  without changing raw graph resolution output.
- Create `crates/compass-languages/tests/universal_evidence.rs`: evidence,
  adapter, projection, and cache contracts.
- Create `crates/compass-resolve/tests/universal_resolution.rs`: shared
  resolution and isolation contracts.
- Create `benchmarks/performance/compass_perf/audit.py`: deterministic audit
  manifest validation, strata accounting, and Wilson precision interval.
- Create `benchmarks/performance/tests/test_audit.py`: audit harness contracts.
- Create `benchmarks/performance/audits/universal-core.json`: checked-in
  synthetic conformance manifest.
- Create `docs/reference/universal-semantic-evidence.md`: extension guide for
  future language and framework authors.

---

### Task 1: Universal evidence model and validator

**Files:**

- Create: `crates/compass-languages/src/evidence/model.rs`
- Create: `crates/compass-languages/src/evidence/validate.rs`
- Create: `crates/compass-languages/src/evidence/mod.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/facts.rs`

**Interfaces:**

- Produces:
  `SemanticEvidenceBatch`, `DeclarationFact`, `ScopeFact`, `BindingFact`,
  `OccurrenceFact`, `RelationshipCandidate`, `ResolutionConstraint`,
  `SemanticRole`, `LanguageCapability`, `EvidenceLimits`, and
  `validate_evidence`.
- `Extraction.semantic_evidence` is
  `Option<SemanticEvidenceBatch>` with serde default and empty omission.

- [ ] **Step 1: Implement the production evidence model**

Create closed serde enums for `SemanticRole`, `LanguageCapability`,
`BindingKind`, and `CandidateRelation`. Use `camelCase` fields and
`snake_case` enum values. Define the exact core shapes:

```rust
pub const UNIVERSAL_EVIDENCE_VERSION: &str =
    "compass.semantic-evidence/1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceRange {
    pub source_file: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticEvidenceBatch {
    pub schema: String,
    pub adapter: AdapterIdentity,
    pub declarations: Vec<DeclarationFact>,
    pub scopes: Vec<ScopeFact>,
    pub bindings: Vec<BindingFact>,
    pub occurrences: Vec<OccurrenceFact>,
    pub candidates: Vec<RelationshipCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<EvidenceDiagnostic>,
}
```

Every fact receives a stable string ID, language, and typed occurrence or
scope reference. `ResolutionConstraint` contains optional exact language,
module/package, scope, qualified name, allowed target kinds, and
`allow_external`; it has no terminal-label fallback flag.

- [ ] **Step 2: Implement structural and resource validation**

`validate_evidence(batch, limits)` must reject:

- a schema other than `compass.semantic-evidence/1`;
- empty adapter language/version;
- duplicate fact IDs;
- unsafe or absolute source paths;
- zero-width or reversed ranges;
- missing scope, declaration, binding, or occurrence references;
- a candidate whose source occurrence language differs from its constraint;
- behavioral candidates without an occurrence;
- undeclared adapter capabilities;
- more facts than the corresponding `EvidenceLimits` field; and
- diagnostics beyond the bounded maximum.

Return one stable `EvidenceError` containing a code and bounded message. Sort
validation traversal by fact ID so input order cannot change the first error.

- [ ] **Step 3: Attach evidence to the extraction cache contract**

Add:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub semantic_evidence: Option<SemanticEvidenceBatch>,
```

to `Extraction`, initialize it to `None`, re-export evidence types from
`compass-languages`, and bump
`EXTRACTION_SEMANTICS_VERSION` from `compass.languages.extraction/2` to
`compass.languages.extraction/3`.

- [ ] **Step 4: Add post-implementation tests**

Cover round-trip serialization, unknown-field rejection, invalid ranges,
duplicate IDs, dangling references, undeclared capabilities, cross-language
constraints, resource boundaries, and deterministic error ordering in
`crates/compass-languages/tests/universal_evidence.rs`.

- [ ] **Step 5: Verify and commit task one**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-languages --test universal_evidence
cargo check -p compass-languages -p compass-resolve -p compass-core
```

Commit:

```bash
git add crates/compass-languages/src/evidence \
  crates/compass-languages/src/lib.rs \
  crates/compass-languages/src/facts.rs \
  crates/compass-languages/tests/universal_evidence.rs
git commit -m "feat(languages): add universal semantic evidence model"
```

---

### Task 2: Adapter capability registry

**Files:**

- Create: `crates/compass-languages/src/adapters.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/registry.rs`
- Modify: `crates/compass-languages/tests/universal_evidence.rs`

**Interfaces:**

- Produces:
  `AdapterRegistry::profile(language) -> Option<&'static AdapterProfile>`.
- Every `LanguageSpec` resolved by the file registry has exactly one adapter
  profile.

- [ ] **Step 1: Implement adapter profiles and maturity**

Define:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterMaturity {
    Legacy,
    Shadow,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterProfile {
    pub language: &'static str,
    pub version: &'static str,
    pub maturity: AdapterMaturity,
    pub capabilities: &'static [LanguageCapability],
}
```

Register every language returned by `Registry::cases()`. Python, Go, Java, and
Rust start as `Shadow`; all other entries start as `Legacy`. A shadow profile
claims only capabilities conservatively projected in task three. Config,
template, document, and manifest pseudo-languages receive explicit profiles
rather than disappearing from capability diagnostics.

- [ ] **Step 2: Connect file detection to adapter identity**

Add `Registry::adapter(path)` and `Registry::profile_for_spec(spec)` helpers.
They must return the same static profile for aliases such as TypeScript
extensions and must fail closed when a spec lacks a profile.

- [ ] **Step 3: Add post-implementation registry tests**

Assert unique adapter languages, non-empty versions, sorted/deduplicated
capabilities, complete coverage of `Registry::cases()`, correct maturity for
the four shadow adapters, and explicit legacy maturity for every other
registered language.

- [ ] **Step 4: Verify and commit task two**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-languages --test universal_evidence
cargo test -p compass-languages --test registry
```

Commit:

```bash
git add crates/compass-languages/src/adapters.rs \
  crates/compass-languages/src/lib.rs \
  crates/compass-languages/src/registry.rs \
  crates/compass-languages/tests/universal_evidence.rs
git commit -m "feat(languages): register semantic adapter capabilities"
```

---

### Task 3: Conservative legacy compatibility projection

**Files:**

- Create: `crates/compass-languages/src/evidence/legacy.rs`
- Modify: `crates/compass-languages/src/evidence/mod.rs`
- Modify: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-languages/tests/universal_evidence.rs`
- Modify: `crates/compass-core/tests/code_graph_v1_determinism.rs`

**Interfaces:**

- Produces:
  `project_legacy_evidence(extraction, profile, limits) ->
  Result<SemanticEvidenceBatch, EvidenceError>`.
- Projection runs after producer/project metadata stamping and before cache
  serialization.

- [ ] **Step 1: Implement typed projection from current facts**

Project only information proven by existing records:

- anchored raw nodes become declarations and lexical scopes;
- exact `contains`/`method`/`defines` edges become ownership candidates at
  their real declaration ranges;
- import/export edges become bindings at their edge ranges;
- `RawCall` entries with complete source locations become call occurrences;
- anchored exact raw edges become relationship candidates;
- missing ranges, unknown relations, inconsistent languages, and unbounded
  metadata become bounded diagnostics rather than invented facts.

Stable evidence IDs use length-prefixed SHA-256 inputs containing schema,
adapter identity, source path, role, range, and producer identity. Sort and
deduplicate by typed ID before validation.

- [ ] **Step 2: Populate shadow evidence in the engine**

After `stamp_producer_metadata`, look up the adapter profile. For `Shadow` and
`Legacy`, project and validate evidence. Store the batch on successful
projection. On failure, keep the original raw graph facts, store one bounded
`_compass_universal_evidence_error` value in `Extraction.extensions`, and do
not publish a partial evidence batch. Do not use the existing partial
extraction-quality marker because shadow-evidence failure must not alter graph
publication.

Do not read source files again and do not change `nodes`, `edges`,
`raw_calls`, or `framework_facts`.

- [ ] **Step 3: Add post-implementation projection tests**

Use Python, Go, Java, Rust, Unicode, repeated-call, alias-import, sourceless
stub, and partial-parser fixtures. Assert exact ranges, stable IDs, bounded
diagnostics, capability truthfulness, and identical raw graph facts before
and after projection.

- [ ] **Step 4: Prove graph output remains byte-stable**

Extend `code_graph_v1_determinism.rs` so a cold and warm extract with shadow
evidence produces identical `graph.json` bytes and the same canonical graph
digest as an equivalent extraction with the evidence batch removed before
publication.

- [ ] **Step 5: Verify and commit task three**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-languages --test universal_evidence
cargo test -p compass-core --test code_graph_v1_determinism
cargo check --workspace --all-targets
```

Commit:

```bash
git add crates/compass-languages/src/evidence \
  crates/compass-languages/src/engine.rs \
  crates/compass-languages/tests/universal_evidence.rs \
  crates/compass-core/tests/code_graph_v1_determinism.rs
git commit -m "feat(languages): project legacy facts into universal evidence"
```

---

### Task 4: Shared constrained resolution-policy boundary

**Files:**

- Create: `crates/compass-resolve/src/evidence.rs`
- Modify: `crates/compass-resolve/src/lib.rs`
- Create: `crates/compass-resolve/tests/universal_resolution.rs`

**Interfaces:**

- Produces:
  `UniversalResolutionIndex::new(batch, limits)` and
  `resolve(candidate_id) -> ResolutionDecision`.
- Decisions are `Resolved`, `QualifiedExternal`, `Ambiguous`, or `Unresolved`;
  no decision mutates raw graph facts in this increment.

- [ ] **Step 1: Implement the bounded universal index**

Index declarations by:

- exact ID;
- `(language, qualified_name)`;
- `(language, module_or_package, spelling)`;
- `(language, lexical_scope, spelling)`; and
- import/alias binding identity.

Use `AHashMap`/`AHashSet`, sort candidate IDs before decisions, and enforce
maximum candidates per lookup, declarations, bindings, and occurrences.

- [ ] **Step 2: Implement deterministic resolution**

Resolve in this exact order:

1. exact lexical declaration;
2. explicit binding or alias;
3. unique module/package definition;
4. qualified external endpoint when allowed;
5. ambiguous or unresolved.

Filter language and allowed target kinds before uniqueness checks. Multiple
case-folded candidates never select a winner unless one exact case-sensitive
identity remains and all other constraints match. Emit a typed
`ResolutionEvidence` describing the rule and candidate count.

- [ ] **Step 3: Wire shadow indexing into collection resolution**

Merge per-file evidence batches by schema and adapter identity after raw
extractions merge. Validate and construct the index under internal profiling,
but do not rewrite nodes or edges. Record bounded projection/index failures in
`Extraction.error` only when the evidence batch itself claims `Complete`;
legacy and shadow failures remain evidence diagnostics.

- [ ] **Step 4: Add post-implementation resolver tests**

Cover lexical shadowing, explicit aliases, re-export bindings,
same-package definitions, case-distinct owners, standard-library externals,
ambiguous overloads, candidate limits, cross-language `cleanup` collisions,
and input-order determinism.

- [ ] **Step 5: Verify and commit task four**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-resolve --test universal_resolution
cargo test -p compass-resolve --tests
cargo check --workspace --all-targets
```

Commit:

```bash
git add crates/compass-resolve/src/evidence.rs \
  crates/compass-resolve/src/lib.rs \
  crates/compass-resolve/tests/universal_resolution.rs
git commit -m "feat(resolve): add universal constrained resolution core"
```

---

### Task 5: Universal framework-pack descriptor

**Files:**

- Modify: `crates/compass-languages/src/frameworks/model.rs`
- Modify: `crates/compass-languages/src/frameworks/mod.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/tests/engine_edge_coverage.rs`
- Modify: `crates/compass-resolve/tests/framework_routes.rs`

**Interfaces:**

- Produces `FrameworkPackDescriptor`, `FrameworkPackKind`,
  `FrameworkManifestPolicy`, and `FrameworkPackRegistry::descriptors()`.
- Existing detectors remain function pointers associated with validated
  descriptors.

- [ ] **Step 1: Implement the descriptor contract**

Define a descriptor containing stable ID, pack version, source/config/template
kind, supported languages, required capabilities, dependency markers,
manifest policy, accepted semantic roles, emitted relation families,
occurrence policy, and `FrameworkLimits`.

`FrameworkPackRegistry::validate()` rejects duplicate IDs, empty versions,
unknown language profiles, undeclared capabilities, an empty accepted-role
set, heuristic packs without rules, and zero limits.

- [ ] **Step 2: Convert existing pack registration**

Replace separate metadata fields on `SourcePack`, `ConfigPack`, and
`TemplatePack` with one descriptor reference plus the existing typed detector
function. Preserve current pack order, activation semantics, detector calls,
and published facts exactly.

- [ ] **Step 3: Add post-implementation framework conformance tests**

Assert every pack has a valid unique descriptor, every supported language has
an adapter profile, activation still requires the same manifests, current
route/domain fixtures publish byte-identical facts, and a synthetic pack
cannot bypass occurrence or limit requirements.

- [ ] **Step 4: Verify and commit task five**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-languages --test engine_edge_coverage
cargo test -p compass-resolve --test framework_routes
cargo test -p compass-resolve --test domain_resolution
```

Commit:

```bash
git add crates/compass-languages/src/frameworks \
  crates/compass-languages/src/lib.rs \
  crates/compass-languages/tests/engine_edge_coverage.rs \
  crates/compass-resolve/tests/framework_routes.rs
git commit -m "feat(frameworks): register universal evidence pack contracts"
```

---

### Task 6: Deterministic conformance and audit harness

**Files:**

- Create: `benchmarks/performance/compass_perf/audit.py`
- Create: `benchmarks/performance/tests/test_audit.py`
- Create: `benchmarks/performance/audits/universal-core.json`
- Modify: `benchmarks/performance/compass_perf/model.py`
- Modify: `benchmarks/performance/harness.py`

**Interfaces:**

- Produces `harness.py audit --manifest PATH --graph PATH --corpus PATH`.
- Produces a versioned JSON result with strata counts, precision, recall,
  critical violations, and Wilson bounds.

- [ ] **Step 1: Implement manifest and result models**

Use Python standard-library dataclasses. Validate schema
`compass.quality-audit/1`, audit mode (`conformance` or `qualification`), safe
relative paths, 40-character corpus commit, known judgment values, positive
ranges, 64-character lowercase SHA-256 snippet hashes, unique record IDs, and
required source/target/relation fields. A `conformance` manifest may exercise
the engine with a small fixture set but its result is ineligible for production
quality claims. A `qualification` manifest enforces the design's 2,000-record
and stratum minimums without configurable overrides.

- [ ] **Step 2: Implement deterministic audit calculations**

Compute:

- observed precision as correct accepted edges divided by audited accepted
  edges;
- two-sided 95% Wilson lower/upper bounds using `statistics`/`math`;
- recall per advertised capability;
- per-corpus/language/relation/capability strata;
- zero-tolerance critical violation counts; and
- explicit invalid, ambiguous, external, represented-elsewhere, and missing
  classifications.

Fail when records are stale, graph facts are absent, a required qualification
stratum is undersized, or a critical violation exists. Never exclude an
invalid record from a denominator. Include `eligibleForQualityClaim: false`
for every conformance-mode result.

- [ ] **Step 3: Add the CLI subcommand and synthetic manifest**

Add `audit` without changing existing `run`, `compare`, or `report` behavior.
The synthetic conformance manifest uses checked-in performance fixtures and
covers one correct edge, one represented-elsewhere fact, one external
endpoint, one missing fact, and all three critical violation labels. Tests
prove that this small manifest cannot report production qualification
eligibility.

- [ ] **Step 4: Add post-implementation harness tests**

Cover exact Wilson values, stale snippets, corpus mismatch, duplicate IDs,
unsafe paths, missing graph facts, undersized strata, critical violations,
deterministic output, and denominator accounting.

- [ ] **Step 5: Verify and commit task six**

Run:

```bash
python3 -m unittest benchmarks.performance.tests.test_audit
python3 -m unittest discover -s benchmarks/performance/tests -p 'test_*.py'
```

Commit:

```bash
git add benchmarks/performance/compass_perf/audit.py \
  benchmarks/performance/compass_perf/model.py \
  benchmarks/performance/harness.py \
  benchmarks/performance/audits/universal-core.json \
  benchmarks/performance/tests/test_audit.py
git commit -m "feat(quality): add universal graph conformance audit"
```

---

### Task 7: Extension guide and increment-one qualification

**Files:**

- Create: `docs/reference/universal-semantic-evidence.md`
- Modify:
  `docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md`
- Modify:
  `docs/superpowers/plans/2026-07-30-universal-semantic-evidence-core.md`

**Interfaces:**

- Documents the exact adapter, capability, framework-pack, resolution, and
  conformance extension process used by later increments.

- [ ] **Step 1: Write the extension guide**

Document:

- every evidence record and invariant;
- how to register an adapter and capability profile;
- how tree-sitter and source-driven adapters emit identical contracts;
- how to register a framework pack;
- resolver evidence order and forbidden fallbacks;
- legacy, shadow, and complete maturity;
- required conformance fixtures and numerical gates; and
- one complete minimal language/profile example plus one minimal framework
  pack example using the actual APIs implemented above.

- [ ] **Step 2: Run focused and full verification**

Run fresh:

```bash
cargo fmt --all -- --check
cargo test -p compass-languages --tests
cargo test -p compass-resolve --tests
cargo test -p compass-core --test code_graph_v1_determinism
python3 -m unittest discover -s benchmarks/performance/tests -p 'test_*.py'
cargo check --workspace --all-targets
cargo test --workspace --all-targets
```

- [ ] **Step 3: Verify real graph non-regression**

Build release Compass and extract fresh Django and Entire graphs using the
same pinned corpora and harness command as phase two. Verify:

- zero validation errors;
- identical node/edge counts and canonical digests to phase two;
- byte-identical cold/warm `graph.json`;
- all four existing query oracles pass; and
- cold/warm latency remains within 10%.

This increment is shadow-only, so any topology difference is a blocker rather
than an expected quality change.

- [ ] **Step 4: Refresh parent graph and update review**

Run:

```bash
cd /Users/haipingfu/graphify
graphify update .
```

Record verification commands, exact corpus revisions, graph counts/digests,
latency, and the fact that this increment makes no production quality claim
yet.

- [ ] **Step 5: Commit documentation and push**

```bash
git add docs/reference/universal-semantic-evidence.md \
  docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md \
  docs/superpowers/plans/2026-07-30-universal-semantic-evidence-core.md
git commit -m "docs(quality): document universal evidence extension"
git push
```

Update PR #88 if it remains open; otherwise create a new PR targeting current
`origin/main`. State explicitly that later increments migrate Python/Go,
Java/Rust, and framework packs from shadow evidence to production resolution.

## Plan self-review

- **Design coverage:** Tasks 1–6 cover the evidence model, adapter registry,
  compatibility projection, resolver policy, framework-pack contract, and
  conformance harness. Task 7 covers extension documentation and shadow-mode
  real-corpus qualification.
- **Scope:** This plan delivers only increment one. It does not claim the
  99.5% universal production quality target; it creates the enforceable
  substrate for later adapter migrations.
- **Type consistency:** `SemanticEvidenceBatch`, `AdapterProfile`,
  `UniversalResolutionIndex`, and `FrameworkPackDescriptor` are defined before
  consumers use them.
- **Execution order:** Every task implements production code before adding
  tests. No red-green TDD step appears.
- **Safety:** Public graph topology must remain byte-identical, and all new
  behavior is shadow-only.
