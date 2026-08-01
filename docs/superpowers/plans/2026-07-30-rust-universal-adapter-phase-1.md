# Rust Universal Adapter Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Begin Rust's hard cutover to the universal adapter design, improve qualified Rust call resolution beyond Graphify, and produce reproducible quality and performance evidence without changing any legacy language algorithm.

**Architecture:** Keep one Rust tree-sitter traversal in `compass-languages`. That traversal emits typed, versioned universal evidence and projects the currently supported public graph; it must not invoke a second Rust extractor. Register Rust as a universal candidate until the complete conformance matrix passes, and resolve only exact qualified local calls in this increment.

**Tech Stack:** Rust 2024, tree-sitter Rust, `serde`, existing `compass-languages` and `compass-resolve` tests, Python 3.11 performance harness, Graphify Rust extractor.

## Global Constraints

- JavaScript/TypeScript, Ruby, C#, PHP, Swift, C/C++, and all other legacy language algorithms remain unchanged.
- No translation layer or dual-running Rust extractor is introduced.
- Public `compass.graph/1` remains the validated graph output.
- A universal candidate must not advertise complete universal quality.
- Every behavioral edge has an exact source occurrence.
- Qualified calls must never fall back to a same-named method on another type.
- Cold and warm graph output remains deterministic.
- Focused Rust extraction may use `TSLP_LANGUAGES=rust` on constrained development machines.

---

### Task 1: Versioned Universal Adapter Contract and Registry

**Files:**
- Create: `crates/compass-languages/src/universal.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/registry.rs`
- Test: `crates/compass-languages/tests/registry.rs`

**Interfaces:**
- Produces: `UNIVERSAL_EVIDENCE_SCHEMA: &str`
- Produces: `AdapterProfile::{Legacy, UniversalCandidate, UniversalComplete}`
- Produces: `AdapterCapability`
- Produces: `AdapterDescriptor`
- Produces: `Registry::adapter(language: &str) -> AdapterDescriptor`
- Produces: `UniversalEvidence` with typed declarations, scopes, bindings, occurrences, and relationship candidates.

- [ ] **Step 1: Implement the smallest typed contract**

Define serializable, ordered record types with `compass_ir::SourceAnchor`.
`OccurrenceFact` contains `owner`, `role`, `spelling`, optional `qualifier`,
and `anchor`. `RelationshipCandidate` contains the same occurrence plus
`target_kinds` and an explicit `external_identity` flag. Do not add a generic
metadata map or language-specific escape hatch.

- [ ] **Step 2: Register Rust without overstating completion**

Return a static Rust descriptor with profile `UniversalCandidate`. All other
languages return `Legacy`; no existing `LanguageSpec`, matcher, or extractor
dispatch changes.

- [ ] **Step 3: Add registry and schema regression tests**

Assert that `Registry::adapter("rust")` returns the stable ID `compass.rust`,
schema `compass.languages.evidence/1`, profile `UniversalCandidate`, and the
currently emitted Rust capabilities for impl ownership, calls, and qualified
external packages. Assert that `Registry::adapter("typescript")` remains
`Legacy`.

- [ ] **Step 4: Run the focused registry tests**

Run:

```bash
TSLP_LANGUAGES=rust cargo test --locked -p compass-languages --test registry
```

Expected: all registry tests pass.

### Task 2: One-Pass Rust Occurrence and Candidate Evidence

**Files:**
- Modify: `crates/compass-languages/src/facts.rs`
- Modify: `crates/compass-languages/src/rust_lang.rs`
- Test: `crates/compass-languages/tests/semantic_producers.rs`

**Interfaces:**
- `Extraction::universal_evidence: Option<UniversalEvidence>`
- Rust extraction emits one occurrence and one candidate for every supported
  call expression during the same traversal that projects graph facts.

- [ ] **Step 1: Collect typed evidence in `RustState`**

Initialize one `UniversalEvidence` value in `RustState`. During the existing
declaration and call walks, append typed records using the exact tree-sitter
node range. Move that value into `Extraction` at `run()` completion. Do not
parse the file again and do not call the legacy generic extractor.

- [ ] **Step 2: Add occurrence regression coverage**

Use a literal Rust fixture containing `Graph::new()`, `HashMap::new()`, and
`graph.add_edge()`. Assert distinct byte ranges, owner IDs, spellings,
qualifiers, schema, and adapter version.

- [ ] **Step 3: Verify one-pass evidence**

Run:

```bash
TSLP_LANGUAGES=rust cargo test --locked -p compass-languages --test semantic_producers rust_universal_occurrences -- --nocapture
```

### Task 3: Exact Qualified Local Rust Calls

**Files:**
- Modify: `crates/compass-languages/src/rust_lang.rs`
- Test: `crates/compass-languages/tests/semantic_producers.rs`

**Interfaces:**
- Produces exact local `calls` edges for `Type::method()` when one method is
  owned by that exact type.
- Leaves unresolved/external qualified calls as candidates; it never binds by
  terminal method name.

- [ ] **Step 1: Index exact impl ownership**

While adding inherent impl methods, store `(normalized owner, method)` to
candidate node IDs. Preserve multiple values so ambiguity remains unresolved.
Trait implementations use their complete semantic owner and do not silently
masquerade as inherent methods.

- [ ] **Step 2: Resolve scoped identifiers conservatively**

For `scoped_identifier`, read both the path and name fields. Emit a local edge
only when the exact `(qualifier, method)` index contains one target. Otherwise
retain the universal candidate, including the qualifier and exact occurrence.
Do not consult `TRAIT_METHOD_BLOCKLIST` for an explicitly qualified exact
local call.

- [ ] **Step 3: Add qualification regression coverage**

The fixture defines `impl Graph { fn new() {} }` and calls both
`Graph::new()` and `HashMap::new()`. Assert exactly one call targets
`impl Graph::new`, its occurrence equals the `Graph::new()` expression, and no
edge from `HashMap::new()` targets that node.

- [ ] **Step 4: Run the qualification and Rust regression tests**

Run:

```bash
TSLP_LANGUAGES=rust cargo test --locked -p compass-languages --test semantic_producers rust_ -- --nocapture
TSLP_LANGUAGES=rust cargo test --locked -p compass-languages --test program_evidence rust_ -- --nocapture
TSLP_LANGUAGES=rust cargo test --locked -p compass-resolve --test native_routes rust_ -- --nocapture
```

Expected: all selected tests pass.

### Task 4: Rust Conformance and Graphify Quality Comparison

**Files:**
- Create: `crates/compass-languages/tests/rust_universal_conformance.rs`
- Modify: `benchmarks/performance/compass_perf/correctness.py`
- Test: `benchmarks/performance/tests/test_correctness.py`

**Interfaces:**
- Conformance covers namespaces, same-name methods, traits, impl ownership,
  macros, imports, external packages, repeated occurrences, Unicode ranges,
  ambiguity, and deterministic serialization.
- Correctness comparison reports shared Rust occurrence coverage separately
  from raw edge-count differences.

- [ ] **Step 1: Add conformance fixtures after implementation**

Use literal sources and hand-derived ranges. Include two types with `new`,
two trait implementations with the same method, nested modules, aliased
imports, a macro invocation, Unicode before a call, and two repeated calls.

- [ ] **Step 2: Extend correctness comparison with Rust occurrence metrics**

Given indexed Compass and Graphify facts, report Graphify Rust call occurrences
covered by a compatible Compass edge and Compass-only exact Rust occurrences.
The metric must match on normalized source path, line/range, relation family,
and compatible endpoints rather than raw IDs.

- [ ] **Step 3: Run conformance and comparison unit tests**

Run:

```bash
TSLP_LANGUAGES=rust cargo test --locked -p compass-languages --test rust_universal_conformance
python3 -m unittest benchmarks.performance.tests.test_correctness
```

Expected: pass with deterministic literal metrics.

### Task 5: Performance and Real-Corpus Qualification

**Files:**
- Modify only if a measurement defect is found: `benchmarks/performance/`
- Record: `docs/superpowers/reviews/2026-07-30-rust-universal-adapter-phase-1.md`

**Interfaces:**
- Produces fixture-level Compass/Graphify node, edge, occurrence-coverage, and
  elapsed-time evidence.
- Produces Bevy cold/warm/incremental comparison when at least 5 GiB is
  available to the harness; otherwise records the exact preflight blocker.

- [ ] **Step 1: Build release Compass with Rust-only parser selection**

Run:

```bash
TSLP_LANGUAGES=rust cargo build --release --locked -p compass-cli --bin compass
```

- [ ] **Step 2: Compare all checked-in Rust fixtures**

Run both extractors over the same source bytes. Record normalized fact
coverage, false local bindings, exact occurrences, wall time, and peak RSS.
Warm each tool once, then collect at least five fresh-process samples.

- [ ] **Step 3: Run the official Bevy preflight**

Run:

```bash
python3 benchmarks/performance/harness.py doctor --repository bevy
```

If it passes, run the explicit Graphify comparison for Bevy with at least three
build repeats. If it fails, preserve the doctor JSON in the review and do not
claim real-corpus performance qualification.

- [ ] **Step 4: Run formatting and focused workspace gates**

Run:

```bash
cargo fmt --all -- --check
TSLP_LANGUAGES=rust cargo test --locked -p compass-languages
TSLP_LANGUAGES=rust cargo test --locked -p compass-resolve
python3 -m unittest benchmarks.performance.tests.test_correctness
```

- [ ] **Step 5: Refresh the parent graph**

From `/Users/haipingfu/graphify`, run:

```bash
graphify update .
```

- [ ] **Step 6: Write the evidence review**

Separate improved precision, improved recall, representation changes,
remaining Rust gaps, Graphify errors, Compass errors, and performance. Do not
mark Rust `UniversalComplete` until every complete-adapter conformance gate and
the real-corpus qualification gate passes.
