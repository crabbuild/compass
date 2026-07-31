# Universal Semantic Evidence Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the universal semantic evidence core and hard-cut Python and
Go extraction/resolution to it, with a capability registry, shared constrained
resolver, framework-pack contract, and conformance harness.

**Architecture:** `compass-languages` owns language-neutral declarations,
scopes, bindings, occurrences, candidates, adapter profiles, and validation.
`compass-resolve` owns a bounded resolver over those facts.
Python and Go adapters emit the new evidence directly and their replaced
language-specific resolution paths are removed in the same increment. Other
languages continue their current algorithms untouched until their own hard
cutovers. Missing evidence on a cached Python or Go extraction forces
re-extraction without changing any existing version value.

**Tech Stack:** Rust 2024, serde/serde_json, tree-sitter source ranges, ahash,
the existing Compass extraction cache, Python 3 standard library, SQLite, and
the existing performance harness.

## Global Constraints

- This is implementation-first, not red-green TDD: implement and review
  production behavior before adding focused tests.
- Keep all existing extraction, cache, producer, graph, adapter, and framework
  version values unchanged.
- Python and Go graph topology may change only through source-grounded hard
  cutover improvements measured by the quality audit.
- No runtime Graphify dependency and no network access in production paths.
- No repository-wide terminal-label fallback.
- Fabricated occurrences, cross-language matches, and unsafe local-target
  substitutions are forbidden.
- Every collection and lookup has an explicit bound.
- Existing user changes and unrelated worktree changes must be preserved.
- Do not run `graphify update .` for this plan.

## Background

The current extraction contract is flexible:

- `RawNodeRecord` and `RawEdgeRecord` store most semantics in JSON maps;
- `RawCall` carries a separate language-specific extension map;
- framework detectors emit separate `RawFrameworkFact` records;
- collection resolution is an ordered series of language-specific passes; and
- adapter capability is inferred from language names and dispatch branches.

That flexibility enabled broad language coverage, but it makes occurrence,
scope, qualification, and resolution rules inconsistent. The universal core
provides typed evidence without a translation layer. Python and Go cut over
directly; adapters not selected for this increment keep their current code
paths and do not claim universal capabilities.

## File and responsibility map

- Create `crates/compass-languages/src/evidence/model.rs`: typed universal
  evidence data model.
- Create `crates/compass-languages/src/evidence/validate.rs`: structural and
  resource validation for evidence batches.
- Create `crates/compass-languages/src/evidence/build.rs`: direct evidence
  builders used by cut-over adapters.
- Create `crates/compass-languages/src/evidence/mod.rs`: public internal API and
  limits.
- Create `crates/compass-languages/src/adapters.rs`: adapter profiles,
  capability registry, and maturity states.
- Modify `crates/compass-languages/src/facts.rs`: optional typed evidence batch
  on `Extraction`.
- Modify `crates/compass-languages/src/engine.rs`: require and validate direct
  Python and Go evidence.
- Modify `crates/compass-languages/src/registry.rs`: expose the adapter profile
  associated with every resolved language.
- Modify `crates/compass-languages/src/frameworks/mod.rs`: typed framework-pack
  descriptors and registry validation while retaining current detectors.
- Create `crates/compass-resolve/src/evidence.rs`: bounded universal candidate
  index and deterministic resolution decisions.
- Modify `crates/compass-resolve/src/lib.rs`: resolve Python and Go through the
  universal evidence path and remove replaced resolver calls.
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
Stable evidence identities use the unchanged
`EXTRACTION_SEMANTICS_VERSION` value as one hash component; this plan does not
modify that value.

- [ ] **Step 2: Implement structural and resource validation**

`validate_evidence(batch, limits)` must reject:

- empty adapter language or producer;
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
`compass-languages`, and leave `EXTRACTION_SEMANTICS_VERSION` at its current
value.

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

### Task 2: Hard-cutover adapter capability registry

**Files:**

- Create: `crates/compass-languages/src/adapters.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/registry.rs`
- Modify: `crates/compass-languages/tests/universal_evidence.rs`

**Interfaces:**

- Produces:
  `AdapterRegistry::universal_profile(language) ->
  Option<&'static AdapterProfile>`.
- A returned profile means that the language must use universal evidence; no
  fallback to its replaced algorithm is allowed.

- [ ] **Step 1: Implement universal adapter profiles**

Define:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterProfile {
    pub language: &'static str,
    pub capabilities: &'static [LanguageCapability],
}
```

Register Python and Go as universal adapters with the capabilities implemented
by task three. Do not register Java, Rust, pseudo-languages, or any other
adapter in this increment. They continue their current algorithms unchanged
and cannot claim universal capabilities.

- [ ] **Step 2: Connect file detection to adapter identity**

Add `Registry::universal_adapter(path)` and
`Registry::universal_profile_for_spec(spec)` helpers. Python and Go return
their static profiles. Every other spec returns `None`; this is an explicit
not-yet-cut-over state, not a compatibility projection.

- [ ] **Step 3: Add post-implementation registry tests**

Assert unique adapter languages, sorted/deduplicated capabilities, direct
Python and Go registration, and `None` for every not-yet-cut-over language.
Assert that no universal adapter can be configured with an empty capability
set.

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

### Task 3: Direct Python and Go evidence extraction

**Files:**

- Create: `crates/compass-languages/src/evidence/build.rs`
- Modify: `crates/compass-languages/src/evidence/mod.rs`
- Modify: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-languages/src/go.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-languages/tests/universal_evidence.rs`
- Modify: `crates/compass-core/tests/code_graph_v1_determinism.rs`

**Interfaces:**

- Produces:
  `EvidenceBuilder::finish() -> Result<SemanticEvidenceBatch, EvidenceError>`.
- Python and Go extraction must return a populated, valid evidence batch.
- Cached Python or Go extraction without evidence is a cache miss.

- [ ] **Step 1: Implement the production evidence builder**

Implement bounded methods that accept typed parser facts directly:

- `declare(kind, identity, scope, range)`;
- `open_scope(owner, parent, range)`;
- `bind(kind, spelling, qualified_target, scope, range)`;
- `occur(role, owner, spelling, qualifier, scope, range)`;
- `relate(relation, source_fact, occurrence, constraints)`; and
- `diagnose(code, range, message)`.

Stable evidence IDs use length-prefixed SHA-256 inputs containing the unchanged
extraction-semantics identity, adapter language, source path, role, range, and
producer identity. Sort and deduplicate by typed ID before validation.

- [ ] **Step 2: Hard-cut Python extraction**

Change the Python branch of generic tree extraction to emit declarations,
scopes, imports/re-exports, aliases, calls/construction, decorators,
annotations, bases, members, and ownership through `EvidenceBuilder`.
Remove the replaced Python-specific raw relationship construction and
collection resolver inputs for those capabilities. Retain framework detection
only through its declared framework-fact boundary until task five.

- [ ] **Step 3: Hard-cut Go extraction**

Change `go.rs` to emit packages, declarations, scopes, imports/aliases,
receivers, fields, calls/construction, type references, embedding, and
ownership through `EvidenceBuilder`. Remove the replaced Go raw type-reference
and receiver-placeholder algorithm. Preserve qualified package identity in
every constraint.

- [ ] **Step 4: Reject stale cached Python and Go facts**

At cache acceptance, resolve the source language through
`AdapterRegistry::universal_profile`. If a universal adapter's cached
`Extraction.semantic_evidence` is absent or invalid, treat the entry as a
normal cache miss and re-extract it. Do not change cache or extraction version
constants.

- [ ] **Step 5: Add post-implementation adapter tests**

Use Python and Go Unicode, repeated-call, alias-import, re-export,
decorator/annotation, package import, receiver, embedding, external type,
sourceless stub, and partial-parser fixtures. Assert exact ranges, stable IDs,
bounded diagnostics, capability truthfulness, and cache rejection when
required evidence is absent.

- [ ] **Step 6: Prove cold/warm determinism**

Extend `code_graph_v1_determinism.rs` so cold and warm Python/Go extracts
produce byte-identical `graph.json`, evidence batches, and canonical graph
digests after task four materializes resolved evidence.

- [ ] **Step 7: Verify and commit task three**

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
  crates/compass-languages/src/go.rs \
  crates/compass-core/src/pipeline.rs \
  crates/compass-languages/tests/universal_evidence.rs \
  crates/compass-core/tests/code_graph_v1_determinism.rs
git commit -m "feat(languages): hard-cut Python and Go semantic evidence"
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
  resolved declarations and decisions materialize the Python and Go raw graph
  records consumed by strict publication.

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

- [ ] **Step 3: Hard-cut collection resolution**

Merge Python and Go evidence batches by adapter language after per-file
extractions merge. Validate, construct the universal index, resolve every
candidate, and materialize declarations, qualified external nodes, exact
edges, ambiguity diagnostics, and unresolved diagnostics into the existing
raw publication boundary.

Remove calls to the replaced Python imported-type/import-guided and Go
receiver/imported-type resolver passes. Delete their dead helper functions
once no non-universal adapter calls them. A missing or invalid universal batch
sets `Extraction.error` and blocks complete publication; there is no fallback
to the prior resolver.

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

- Create: `crates/compass-languages/src/frameworks/pack.rs`
- Modify: `crates/compass-languages/src/frameworks/mod.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/tests/engine_edge_coverage.rs`

**Interfaces:**

- Produces `FrameworkPackDescriptor`, `FrameworkPackKind`,
  `FrameworkManifestPolicy`, and `FrameworkPackRegistry::descriptors()`.
- Existing framework packs remain on their current execution until their
  dedicated hard cutover; no compatibility adapter connects them to this
  contract.

- [ ] **Step 1: Implement the descriptor contract**

Define a descriptor containing stable ID, source/config/template kind,
supported languages, required capabilities, dependency markers, manifest
policy, accepted semantic roles, emitted relation families, occurrence policy,
and `FrameworkLimits`.

`FrameworkPackRegistry::validate()` rejects duplicate IDs, languages without a
universal adapter profile, undeclared capabilities, an empty accepted-role set,
heuristic packs without rules, and zero limits.

- [ ] **Step 2: Add a production registration boundary**

Implement registration and validation APIs used by future hard-cutover packs.
Do not register or translate current `SourcePack`, `ConfigPack`, or
`TemplatePack` values in this increment. Their later cutover removes the old
entry and adds the universal pack atomically.

- [ ] **Step 3: Add post-implementation framework conformance tests**

Assert a synthetic valid pack registers for Python and Go, a pack cannot
register for a not-yet-cut-over language, and no pack can bypass capability,
occurrence, activation, or limit requirements. Existing route/domain fixtures
must remain unchanged because no production pack is registered twice.

- [ ] **Step 4: Verify and commit task five**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-languages --test engine_edge_coverage
cargo test -p compass-resolve --test framework_routes
```

Commit:

```bash
git add crates/compass-languages/src/frameworks/pack.rs \
  crates/compass-languages/src/frameworks/mod.rs \
  crates/compass-languages/src/lib.rs \
  crates/compass-languages/tests/engine_edge_coverage.rs
git commit -m "feat(frameworks): add universal evidence pack contract"
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
- Produces a deterministic JSON result with strata counts, precision, recall,
  critical violations, and Wilson bounds.

- [ ] **Step 1: Implement manifest and result models**

Use Python standard-library dataclasses. Validate schema
`compass.quality-audit`, audit mode (`conformance` or `qualification`), safe
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

### Task 7: Extension guide and Python/Go hard-cutover qualification

**Files:**

- Create: `docs/reference/universal-semantic-evidence.md`
- Modify:
  `docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md`
- Modify:
  `docs/superpowers/plans/2026-07-30-universal-semantic-evidence-core.md`

**Interfaces:**

- Documents the exact adapter, capability, hard-cutover, framework-pack,
  resolution, and conformance extension process used by later increments.

- [ ] **Step 1: Write the extension guide**

Document:

- every evidence record and invariant;
- how to register an adapter and capability profile;
- how tree-sitter and source-driven adapters emit identical contracts;
- how to register a framework pack;
- resolver evidence order and forbidden fallbacks;
- hard-cutover requirements and cache re-extraction;
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

- [ ] **Step 3: Verify real graph quality and determinism**

Build release Compass and extract fresh Django and Entire graphs using the
same pinned corpora and harness command as phase two. Verify:

- zero validation errors;
- byte-identical cold/warm `graph.json`;
- all four existing query oracles pass;
- cold/warm latency remains within 10%;
- every topology transition is classified by relation and source occurrence;
- the Python/Go capability audit meets the plan's precision, recall, and zero
  critical-violation gates; and
- fewer Django edges are called an improvement only when invalid evidence is
  removed and real later uses remain represented.

- [ ] **Step 4: Update the review**

Record verification commands, exact corpus revisions, graph counts/digests,
latency, audited precision/recall, every critical-violation count, and an
honest classification of Compass improvements, regressions, representation
changes, and unresolved gaps. Do not run `graphify update .`.

- [ ] **Step 5: Commit documentation and push**

```bash
git add docs/reference/universal-semantic-evidence.md \
  docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md \
  docs/superpowers/plans/2026-07-30-universal-semantic-evidence-core.md
git commit -m "docs(quality): document universal evidence extension"
git push
```

Update PR #88 if it remains open; otherwise create a new PR targeting current
`origin/main`. State explicitly that Python and Go hard-cut in this increment;
Java/Rust and framework packs follow through separate hard-cutover increments.

## Plan self-review

- **Design coverage:** Tasks 1–6 cover the evidence model, adapter registry,
  direct Python/Go extraction, production resolver, framework-pack contract,
  and conformance harness. Task 7 covers extension documentation and
  real-corpus hard-cutover qualification.
- **Scope:** This plan delivers the core plus Python/Go hard cutover. It claims
  quality only for capabilities that pass the audit; Java/Rust and framework
  hard cutovers remain separate increments.
- **Type consistency:** `SemanticEvidenceBatch`, `AdapterProfile`,
  `UniversalResolutionIndex`, and `FrameworkPackDescriptor` are defined before
  consumers use them.
- **Execution order:** Every task implements production code before adding
  tests. No red-green TDD step appears.
- **Safety:** Every changed graph fact must have typed source evidence, old
  Python/Go resolver paths are removed, stale caches re-extract, and there is
  no compatibility projection.
