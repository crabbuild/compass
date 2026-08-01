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
- Create `benchmarks/performance/compass/audit.py`: deterministic audit
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

- [x] **Step 1: Implement the production evidence model**

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

- [x] **Step 2: Implement structural and resource validation**

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

- [x] **Step 3: Attach evidence to the extraction cache contract**

Add:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub semantic_evidence: Option<SemanticEvidenceBatch>,
```

to `Extraction`, initialize it to `None`, re-export evidence types from
`compass-languages`, and leave `EXTRACTION_SEMANTICS_VERSION` at its current
value.

- [x] **Step 4: Add post-implementation tests**

Cover round-trip serialization, unknown-field rejection, invalid ranges,
duplicate IDs, dangling references, undeclared capabilities, cross-language
constraints, resource boundaries, and deterministic error ordering in
`crates/compass-languages/tests/universal_evidence.rs`.

- [x] **Step 5: Verify and commit task one**

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

- [x] **Step 1: Implement universal adapter profiles**

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

- [x] **Step 2: Connect file detection to adapter identity**

Add `Registry::universal_adapter(path)` and
`Registry::universal_profile_for_spec(spec)` helpers. Python and Go return
their static profiles. Every other spec returns `None`; this is an explicit
not-yet-cut-over state, not a compatibility projection.

- [x] **Step 3: Add post-implementation registry tests**

Assert unique adapter languages, sorted/deduplicated capabilities, direct
Python and Go registration, and `None` for every not-yet-cut-over language.
Assert that no universal adapter can be configured with an empty capability
set.

- [x] **Step 4: Verify and commit task two**

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

- [x] **Step 1: Implement the production evidence builder**

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

- [x] **Step 2: Hard-cut Python extraction**

Change the Python branch of generic tree extraction to emit declarations,
scopes, imports/re-exports, aliases, calls/construction, decorators,
annotations, bases, members, and ownership through `EvidenceBuilder`.
Remove the replaced Python-specific raw relationship construction and
collection resolver inputs for those capabilities. Retain framework detection
only through its declared framework-fact boundary until task five.

- [x] **Step 3: Hard-cut Go extraction**

Change `go.rs` to emit packages, declarations, scopes, imports/aliases,
receivers, fields, calls/construction, type references, embedding, and
ownership through `EvidenceBuilder`. Remove the replaced Go raw type-reference
and receiver-placeholder algorithm. Preserve qualified package identity in
every constraint.

- [x] **Step 4: Reject stale cached Python and Go facts**

At cache acceptance, resolve the source language through
`AdapterRegistry::universal_profile`. If a universal adapter's cached
`Extraction.semantic_evidence` is absent or invalid, treat the entry as a
normal cache miss and re-extract it. Do not change cache or extraction version
constants.

- [x] **Step 5: Add post-implementation adapter tests**

Use Python and Go Unicode, repeated-call, alias-import, re-export,
decorator/annotation, package import, receiver, embedding, external type,
sourceless stub, and partial-parser fixtures. Assert exact ranges, stable IDs,
bounded diagnostics, capability truthfulness, and cache rejection when
required evidence is absent.

- [x] **Step 6: Prove cold/warm determinism**

Extend `code_graph_v1_determinism.rs` so cold and warm Python/Go extracts
produce byte-identical `graph.json`, evidence batches, and canonical graph
digests after task four materializes resolved evidence.

- [x] **Step 7: Verify and commit task three**

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

- [x] **Step 1: Implement the bounded universal index**

Index declarations by:

- exact ID;
- `(language, qualified_name)`;
- `(language, module_or_package, spelling)`;
- `(language, lexical_scope, spelling)`; and
- import/alias binding identity.

Use `AHashMap`/`AHashSet`, sort candidate IDs before decisions, and enforce
maximum candidates per lookup, declarations, bindings, and occurrences.

- [x] **Step 2: Implement deterministic resolution**

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

- [x] **Step 3: Hard-cut collection resolution**

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

- [x] **Step 4: Add post-implementation resolver tests**

Cover lexical shadowing, explicit aliases, re-export bindings,
same-package definitions, case-distinct owners, standard-library externals,
ambiguous overloads, candidate limits, cross-language `cleanup` collisions,
and input-order determinism.

- [x] **Step 5: Verify and commit task four**

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

- [x] **Step 1: Implement the descriptor contract**

Define a descriptor containing stable ID, source/config/template kind,
supported languages, required capabilities, dependency markers, manifest
policy, accepted semantic roles, emitted relation families, occurrence policy,
and `FrameworkLimits`.

`FrameworkPackRegistry::validate()` rejects duplicate IDs, languages without a
universal adapter profile, undeclared capabilities, an empty accepted-role set,
heuristic packs without rules, and zero limits.

- [x] **Step 2: Add a production registration boundary**

Implement registration and validation APIs used by future hard-cutover packs.
Do not register or translate current `SourcePack`, `ConfigPack`, or
`TemplatePack` values in this increment. Their later cutover removes the old
entry and adds the universal pack atomically.

- [x] **Step 3: Add post-implementation framework conformance tests**

Assert a synthetic valid pack registers for Python and Go, a pack cannot
register for a not-yet-cut-over language, and no pack can bypass capability,
occurrence, activation, or limit requirements. Existing route/domain fixtures
must remain unchanged because no production pack is registered twice.

- [x] **Step 4: Verify and commit task five**

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

- Create: `benchmarks/performance/compass/audit.py`
- Create: `benchmarks/performance/tests/test_audit.py`
- Create: `benchmarks/performance/audits/universal-core.json`
- Modify: `benchmarks/performance/compass/model.py`
- Modify: `benchmarks/performance/harness.py`

**Interfaces:**

- Produces `harness.py audit --manifest PATH --graph PATH --corpus PATH`.
- Produces a deterministic JSON result with strata counts, precision, recall,
  critical violations, and Wilson bounds.

- [x] **Step 1: Implement manifest and result models**

Use Python standard-library dataclasses. Validate schema
`compass.quality-audit`, audit mode (`conformance` or `qualification`), safe
relative paths, 40-character corpus commit, known judgment values, positive
ranges, 64-character lowercase SHA-256 snippet hashes, unique record IDs, and
required source/target/relation fields. A `conformance` manifest may exercise
the engine with a small fixture set but its result is ineligible for production
quality claims. A `qualification` manifest enforces the design's 2,000-record
and stratum minimums without configurable overrides.

- [x] **Step 2: Implement deterministic audit calculations**

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

- [x] **Step 3: Add the CLI subcommand and synthetic manifest**

Add `audit` without changing existing `run`, `compare`, or `report` behavior.
The synthetic conformance manifest uses checked-in performance fixtures and
covers one correct edge, one represented-elsewhere fact, one external
endpoint, one missing fact, and all three critical violation labels. Tests
prove that this small manifest cannot report production qualification
eligibility.

- [x] **Step 4: Add post-implementation harness tests**

Cover exact Wilson values, stale snippets, corpus mismatch, duplicate IDs,
unsafe paths, missing graph facts, undersized strata, critical violations,
deterministic output, and denominator accounting.

- [x] **Step 5: Verify and commit task six**

Run:

```bash
python3 -m unittest benchmarks.performance.tests.test_audit
python3 -m unittest discover -s benchmarks/performance/tests -p 'test_*.py'
```

Commit:

```bash
git add benchmarks/performance/compass/audit.py \
  benchmarks/performance/compass/model.py \
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

- [x] **Step 1: Write the extension guide**

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

- [x] **Step 2: Run focused and full verification**

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

- [x] **Step 3: Verify real graph quality and determinism**

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

Outcome: cold/warm graph bytes and all four query oracles passed, and the final
Django graph has no publication collision or omission. The 10% performance,
strict Graphify-superset, and production 2,000-record precision gates did not
pass or were not qualified; the review records those gaps without promoting a
quality claim.

- [x] **Step 4: Update the review**

Record verification commands, exact corpus revisions, graph counts/digests,
latency, audited precision/recall, every critical-violation count, and an
honest classification of Compass improvements, regressions, representation
changes, and unresolved gaps. Do not run `graphify update .`.

- [x] **Step 5: Commit documentation and push**

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

---

### Task 8: Phase-three exact receiver dispatch and honest residual accounting

**Background:**

The first real-corpus residual analysis found that Django still contained 730
Graphify-only `super()` call hypotheses. The generic Python call extractor
recorded `super()` as an unbound qualifier and correctly refused repository-
wide terminal-name fallback, but it did not preserve enough owner context to
select a proven base method. This was a recall gap, not permission to weaken
resolution.

The same analysis also found Graphify call/reference hypotheses whose
generated endpoint identity differed from Compass even though Compass had one
compatible target at the exact mapped owner, file, and use-site location.
Those cases require semantic comparison, not new product graph edges.

**Design:**

- Keep declaration context aware of its enclosing type and the exact qualified
  identities of explicit bases.
- For Python `super().method(...)`, emit an exact qualified target only when
  the enclosing type has one explicit base. The shared resolver still requires
  that exact base method to exist as a source declaration.
- Fail closed for multiple inheritance, dynamic bases, explicit-argument
  `super`, absent direct methods, and external-only bases. MRO inference is a
  future evidence capability, not a terminal-name fallback.
- Reuse `ResolutionConstraint.qualified_name`; do not add or change any
  version value.
- Keep the mechanism language-neutral: future adapters may provide an exact
  owner-qualified target from compiler, type, trait, or inheritance evidence
  and use the same resolver rule.
- In the external comparator, count a generated-identity hypothesis as
  dominated only when the mapped source, relation, file, location, and one
  compatible Compass target all agree. Multiple or incompatible targets remain
  ambiguous or missing.

- [x] **Step 1: Implement exact direct-base dispatch**

Add enclosing-type context and a bounded base map to the direct adapter. Emit
an exact `Base::method` constraint for the conservative Python case above.
Implement production behavior before adding tests.

- [x] **Step 2: Add post-implementation fail-closed coverage**

Cover a cross-file imported single base as the positive case and multiple
inheritance as the negative case. Verify that the shared resolver, rather than
a Python-specific resolution branch, materializes the edge.

- [x] **Step 3: Improve source-grounded comparison**

Index exact relationship occurrences by relation, canonical owner, source
file, and source location. Recover only one compatible target at that exact
site. Report ambiguous node counts in the reusable analyzer.

- [x] **Step 4: Verify on pinned Django and Entire**

Use Django commit `1c5927f04a853c79ac9b098eab92fb328ff9e4ad` and Entire
commit `279b988597f1037c14cdd4c46765a5552e067d17`. Retain the
existing Graphify graphs; do not rerun Graphify and do not run
`graphify update .`.

Outcome:

- Django gained 688 exact `super()` call edges and lost three incorrect
  variable-target `copy` call edges, for a net 685 raw edges.
- An independent Python AST audit verified all 688 new edges against the exact
  source class, sole base, call occurrence, target class, and direct method
  declaration: 688/688 correct.
- Graphify edge coverage improved from 7,415 to 5,796 missing hypotheses across
  the comparator and product changes. Strict superset quality still fails.
- Entire graph topology did not change.
- The standardized three-repeat Compass build suite passed, but the retained
  Graphify comparison still misses the 5x cold-build target on Entire and
  Compass still uses substantially more peak memory. Do not describe phase
  three as performance dominance.

- [x] **Step 5: Record evidence and publish**

Record exact counts, coverage deltas, verification commands, performance,
memory, and remaining gaps in
`docs/superpowers/reviews/2026-07-30-semantic-dominance-phase-3.md`.
Commit, push, and open a new PR against current `origin/main`.

---

### Task 9: Universal linearized receiver dispatch

**Background:**

Phase three intentionally resolved only zero-argument Python `super()` calls
whose enclosing class had one statically named direct base and whose target
method was declared directly on that base. The retained Graphify comparison
still listed 456 `super()` call hypotheses as missing, but source inspection
showed that many Graphify targets were incorrect: methods on `LazySettings`
were pointed at `UserSettingsHolder`, and unrelated widget constructors were
pointed at `AutocompleteMixin`. Matching those targets would improve the
comparator score while degrading the product graph.

The genuine recall gap is ordered receiver dispatch. Python may select a method
from a later class in a C3 multiple-inheritance order or from an ancestor
beyond the direct base. Per-file extraction cannot prove that target because
the hierarchy crosses files. The shared resolver already sees the merged
repository evidence and is the correct bounded policy boundary.

**Design:**

- Add typed hierarchy constraints to `ResolutionConstraint`: ordered
  direct-base evidence carries whether the full source base set is known, and
  receiver dispatch carries an exact receiver identity plus an explicit
  linearization strategy.
- Advertise a new `HierarchyDispatch` capability only for adapters that emit
  the complete contract. The existing capability equality check makes cached
  Python evidence without it a normal cache miss; no version value changes.
- Hard-cut Python zero-argument `super()` away from the direct-base shortcut.
  It emits receiver-dispatch evidence and cannot simultaneously emit a
  qualified, lexical, local, imported, or external target.
- Build bounded direct-base and directly-owned-member indices in the universal
  resolver. Resolve direct-base publication only by exact qualified identity
  or a qualified external endpoint; never fall through to lexical or
  same-module name matching.
- For `C3AfterReceiver`, recover a unique member declared by the exact first
  base from a complete receiver base list, even when a later ancestor is
  external. Otherwise require every ordered base to resolve to one exact
  source class, require an acyclic and C3-consistent hierarchy within the
  configured bound, and select only one direct member on the first matching
  class in the linearization.
- Fail closed for dynamic or incomplete receiver bases; unresolved ancestors
  outside the proven direct-successor prefix; inconsistent or cyclic
  hierarchies; ambiguous members; explicit-argument `super`; and any
  resource-bound overflow.
- Keep the mechanism extensible: future language adapters add an explicit,
  qualified strategy backed by their compiler/type/trait evidence. They do not
  reinterpret C3 and do not use terminal-name search.
- Do not add a legacy projection, change any version value, rerun Graphify, or
  run `graphify update .`.

- [x] **Step 1: Implement the production hard cutover**

Add the hierarchy model, capability gate, validation rules, exact base
identity emission, bounded C3 resolver, and receiver-member selection.
Remove the phase-three `direct_super_target` path. Implement production code
before tests.

- [x] **Step 2: Add post-implementation conformance coverage**

Cover exact cross-file direct bases, multiple-base source order,
inherited-beyond-direct-base selection, dynamic-base fail-closed behavior,
capability validation, deterministic hierarchy evidence, and prevention of
same-named import fallback.

- [x] **Step 3: Verify real source-grounded graph changes**

Rebuild pinned Django and Entire graphs cold. Require zero validation errors,
byte-identical warm output, and no topology change in Entire. Independently
audit every added or retargeted Django call against Python AST class order,
complete base identities, C3 linearization, direct member ownership, target
file, and target line. Treat every removed edge as a regression unless the old
target is independently proven incorrect or the new exact target represents
the same use.

Result:

- Django publishes 63,892 nodes and 148,710 canonical edges with zero
  validation errors.
- Relative to phase three, Django adds 473 independently verified `calls`
  edges and removes none. The audit covers every added edge: 181 use the exact
  direct-successor proof and 292 use complete C3 linearization.
- Exact base publication rejects same-named local substitutions, including the
  incorrect GIS-local `Transform`, while nested sibling bases resolve to their
  exact enclosing-class declarations.
- Django cold/warm graphs are byte-identical at 201,270,657 bytes.
- Entire remains exactly unchanged at 58,391 nodes, 151,257 canonical edges,
  the same canonical digest, and zero added or removed topology. Its
  cold/warm graphs are byte-identical at 169,597,446 bytes.

- [x] **Step 4: Run complete verification and performance qualification**

Run the complete workspace tests, strict Python benchmark tests, formatting,
linting, diff validation, release build, query oracles, and the standardized
three-repeat large-repository build suite. Compare instruction count, wall
time, and peak RSS with phase three. Do not claim performance dominance when
Entire remains below 5x or Compass memory remains higher than Graphify.

Result:

- The complete workspace, strict Python, format, lint, release, and diff gates
  pass.
- The final all-workload qualification passes on current Django and pinned
  Entire, including 10/10 eligible batches for every natural-language and
  CompassQL workload.
- The first all-workload run exposed an unused named-path allocation that
  exceeded the 256 MiB Entire query limit. A conservative compiler
  optimization removes only unreferenced, singly bound paths; the exact failed
  query now passes with output byte-identical to the high-memory reference.
- On the original pinned Django corpus, cold p50 changes by +2.46%, warm by
  -0.44%, and incremental by -1.88% relative to phase three. On unchanged
  Entire, the respective changes are +6.00%, +4.64%, and +4.50%.
- Retained Graphify timing remains approximately 5.49x slower on Django and
  3.23x slower on Entire. Compass peak memory remains higher, so performance
  dominance is not claimed.
- The macOS runner does not expose retired instruction counts; no instruction
  delta is claimed.

- [x] **Step 5: Record evidence and update PR #93**

Add a phase-four review containing exact graph counts, topology deltas,
independent audit results, comparator classifications, latency and memory,
verification commands, and remaining gaps. Commit and push the hard cutover
to the existing PR only after all required gates pass.

---

### Task 10: Remove the legacy Python import projection

**Background:**

The phase-four Django graph contains 13,802 raw V1 edges whose only
producer is the pre-universal `compass.resolve.python-imports` source-text
pass. This violates the hard-cutover invariant even though it is not used as
a fallback for calls. The completed byte-range audit found 12,247 exact
universal replacements, 258 scope-correct ownership replacements, 1,238
corrected symbol exports, 33 corrected identities, 21 redundant module
projections, and five nested runtime imports whose enclosing declarations are
not represented as graph nodes. The typed evidence, not preservation of the
old count or whole-statement anchor, determines whether each transition is
correct.

The direct adapter also stores every Python import in a file-wide lookup,
including imports nested in functions. That can leak a local binding into an
unrelated function. A hard cutover must remove both the second parser and this
scope leak before claiming that universal import evidence is authoritative.

**Design:**

- Stop reparsing Python source in the generic cross-file resolver. Python and
  Go raw calls are already excluded before that resolver; all Python import,
  re-export, alias, decorator, annotation, base, construction, and call
  connectivity must come from the universal batch.
- Delete the legacy parser, definition matcher, re-export walker, edge
  producer, producer name, resolution rules, and compatibility tests. Do not
  retain a hidden feature flag or alternate path.
- Store function-local Python imports only in the owning scope. File imports
  remain module bindings. Duplicate names are ambiguous only within the same
  scope, and a local binding never becomes visible to a sibling function.
- Preserve one exact occurrence per imported item. Package initializer
  `from ... import ...` bindings remain typed symbol re-exports; do not emit a
  repeated edge that incorrectly describes the source module itself as the
  exported value.
- Keep all version values unchanged. Do not run Graphify or
  `graphify update .`.

- [x] **Step 1: Implement the production hard cutover**

Remove legacy Python source reparsing and projection from
`compass-resolve`. Make import binding and target lookup scope-aware in the
direct Python evidence builder. Implement production code before changing
tests.

- [x] **Step 2: Add post-implementation conformance coverage**

Replace legacy-producer tests with universal contracts for exact item spans,
symbol and submodule resolution, multi-hop re-exports, function-local import
ownership, sibling-scope isolation, deterministic input order, and complete
absence of `compass.resolve.python-imports` or its old rules.

- [x] **Step 3: Qualify the real topology transition**

Rebuild pinned Django and Entire graphs. Require zero validation errors,
byte-identical cold/warm output, and no legacy producer occurrence. A shared
resolver correction may change Entire topology only when every added or
retargeted fact has independent source proof; an unchanged count is not a
quality gate. Classify every removed Django edge into exact universal
replacement, more precise scoped ownership, corrected symbol-export
semantics, corrected identity, redundant module projection, or an explicit
remaining graph-model gap. Restore any genuine source-proven fact through
universal evidence before proceeding.

Result:

- Pinned Django publishes 63,797 nodes, 145,219 raw edges, and 144,295
  canonical edges. Pinned Entire publishes 58,391 nodes, 152,161 raw edges,
  and 151,267 canonical edges. Both validate with zero errors and have
  byte-identical cold/warm graphs.
- All 13,802 retired legacy edges are classified: 12,247 exact replacements,
  258 scope-correct ownership replacements, 1,238 corrected symbol exports,
  33 corrected identities, 21 redundant module projections, and five
  unrepresented nested-declaration imports. None is restored through the old
  algorithm.
- Exact import binding precedence also corrects 96 Django and 16 Entire
  added or retargeted relationships. Independent Python AST/import and Go
  import-path audits verified 96/96 and 16/16 respectively, with no failures.
- Entire therefore changes by ten canonical edges. This is an audited shared
  resolver improvement, not an assertion that a topology freeze is more
  important than source-correct targets.

- [x] **Step 4: Run complete correctness and performance verification**

Run focused resolver/language tests, the full workspace, strict Python
benchmark tests, format, lint, release build, query oracles, comparator, and
the standardized large-repository performance suite. Record wall time and
peak RSS honestly; do not infer performance dominance from edge removal.

Result:

- Focused resolver, language, core determinism, Python benchmark, release,
  format, lint, and diff checks pass. The full locked workspace all-target
  suite passes, including scale tests.
- Standardized run `phase5-python-import-hard-cut-final` passes every gate on
  current Django and Entire heads with 3/3 eligible cold, warm, and
  incremental builds and 10/10 eligible samples for every query workload.
- Django p50 is 11.635 seconds cold, 1.547 seconds warm, and 19.175 seconds
  incremental. Entire p50 is 7.611, 0.891, and 15.146 seconds respectively.
  Peak memory remains high, so this is not a performance-dominance claim.

- [x] **Step 5: Record evidence and update PR #93**

Add a phase-five review with exact topology classifications, scope-isolation
evidence, graph counts and digests, Graphify comparison deltas, performance,
verification commands, and remaining gaps. Commit and push only after all
required gates pass.

---

### Task 11: Source-grounded runtime declaration ownership

**Background:**

Both the generic Python graph extractor and the direct universal evidence
collector currently stop declaration discovery when they enter a function.
That behavior omits source declarations created at runtime inside functions
and methods. On pinned Django the omitted source population contains 1,068
nested functions and 2,345 classes declared inside functions. It also causes
the five phase-five runtime-import gaps: the import use is real, but its exact
lexical owner has no graph node, so restoring the old file-owned edge would be
incorrect.

This is a graph-model recall gap, not permission to attach nested facts to an
outer file or method. The source contains an exact declaration range, lexical
parent, name, and declaration kind, so the universal evidence path can model
the owner without guessing a runtime call target.

**Design:**

- Generalize Python declaration traversal from a class-only parent to an exact
  lexical declaration parent. A nested function is owned by the enclosing
  function or method; a runtime class is owned by the enclosing function; and
  methods remain owned by their exact class.
- Give every nested declaration a deterministic identity derived from its
  lexical owner's graph identity and source name. Repeated same-named
  declarations under one owner use the existing source-line discriminator.
- Emit the raw source node and universal `DeclarationFact`, scope, and
  containment candidate from the same parser declaration. Their graph IDs
  must agree exactly so resolution never needs a compatibility projection.
- Recurse into Python function bodies only. Other language adapters retain
  their current behavior until their own atomic hard cutovers.
- Imports, calls, decorators, annotations, bases, and further nested
  declarations inside the new owner use that owner's scope. Never project a
  nested fact to a file or sibling declaration.
- Keep all version values unchanged. Do not run Graphify or
  `graphify update .`.

- [x] **Step 1: Implement lexical runtime declarations**

Refactor the production generic Python declaration walk and direct evidence
collector to emit source-backed nested function/class nodes, exact ownership,
and aligned identities. Implement production behavior before tests.

- [x] **Step 2: Add post-implementation conformance coverage**

Cover nested functions, classes declared inside functions, methods of runtime
classes, repeated same-named nested declarations, exact nested imports, and
sibling-scope isolation. Assert raw graph IDs and universal declaration IDs
materialize to the same nodes without dangling or file-owned fallback edges.

- [x] **Step 3: Qualify the real topology transition**

Rebuild pinned Django and Entire cold and warm. Require zero validation
errors, byte-identical output, no topology change in Entire, and no remaining
unrepresented phase-five runtime import. Independently audit every added node
and relationship against Python AST declaration nesting, source range,
lexical owner, import binding, and target source. Any relationship without
exact source proof fails the phase.

Result:

- Pinned Django publishes 68,761 nodes, 157,056 raw edges, and 156,092
  canonical edges with zero validation errors and byte-identical cold/warm
  output. Pinned Entire remains byte-identical to phase five at 58,391 nodes,
  152,161 raw edges, and 151,267 canonical edges.
- All five phase-five runtime-import gaps are recovered under their exact
  nested declaration owners. None is projected to a file or outer method.
- The complete changed population is source-grounded: 4,662 added declaration
  nodes match Python AST file, line, name, kind, and lexical ownership; 307
  placeholders retain bounded source anchors; and all 11,941 added
  relationships match exact declaration nesting or exact use sites with
  bounded binding/target evidence.
- Target confidence remains explicit: inferred external identities are not
  described as exact internal declarations, and this changed-population audit
  is not a repository-wide precision claim.

- [x] **Step 4: Run complete verification and performance qualification**

Run focused language/resolver tests, core determinism, the strict Python
benchmark suite, the full locked workspace, format, lint, release build,
query oracles, comparator, and the standardized large-repository suite. Record
wall time and peak RSS honestly; added source declarations are a quality gain
only when their ownership and downstream facts pass the complete audit.

Result:

- Focused language/resolver tests, core determinism, the strict 81-test Python
  benchmark suite, full locked workspace all-target suite, format, production
  and test lint, release build, diff check, comparator, and source audits pass.
- Standardized run `phase6-runtime-declarations-final` passes every internal
  correctness, determinism, natural-query, and CompassQL gate with 3/3 build
  and 10/10 query samples eligible on current Django and Entire heads.
- Performance is mixed and is not claimed as an improvement. In a
  non-controlled comparison to phase five's different remote corpus commits,
  Django incremental p50 rises 15.5%; peak build memory remains high.

- [x] **Step 5: Record evidence and update PR #93**

Add a phase-six review with exact topology deltas, complete changed-population
audit, graph counts and digests, Graphify classifications, performance, and
remaining gaps. Commit and push only after all required gates pass.

Result: phase-six evidence is recorded in
`docs/superpowers/reviews/2026-07-31-semantic-dominance-phase-6.md`.
Implementation and qualification commits are pushed, and PR #93 contains the
phase-six scope, results, performance caveats, and remaining gaps.

## Plan self-review

- **Design coverage:** Tasks 1–6 cover the evidence model, adapter registry,
  direct Python/Go extraction, production resolver, framework-pack contract,
  and conformance harness. Task 7 covers extension documentation and
  real-corpus hard-cutover qualification. Task 8 adds conservative
  owner-qualified receiver dispatch and source-grounded residual accounting.
  Task 9 replaces that conservative shortcut with typed, bounded linearized
  dispatch in the shared resolver. Task 10 removes the residual pre-universal
  Python import projection and makes direct import bindings scope-correct.
  Task 11 closes the known runtime-declaration ownership gap without reviving
  file-level projection.
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
