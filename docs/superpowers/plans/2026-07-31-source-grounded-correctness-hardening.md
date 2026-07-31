# Source-Grounded Correctness Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. This is an implementation-first plan: production code is written before regression checks, and TDD is explicitly out of scope.

**Goal:** Remove the confirmed Python receiver-resolution false edge, make Graphify comparison reflect source-statement evidence instead of exact-line artifacts, and reduce peak build retention without weakening deterministic graph publication.

**Architecture:** Receiver intent is encoded as a typed hierarchy strategy by the language adapter and resolved only through the exact receiver, a source-proven C3 prefix, or a complete C3 hierarchy. Benchmark comparison gains an optional, repository-rooted occurrence oracle with extension-based providers; its first provider uses Python AST statement spans for imports and calls. The build pipeline shares source bytes and prepares cache payloads without cloning the complete fresh extraction inventory.

**Tech Stack:** Rust, Tree-sitter evidence extraction, shared universal resolver, Python `ast`, SQLite comparison harness, MessagePack AST cache, Cargo and `unittest` verification.

## Background

PR #93 established direct Python and Go semantic evidence, recovered runtime declarations, and improved Django Graphify-hypothesis coverage. Its retained Django artifact contains 68,761 Compass nodes and 156,092 canonical edges. A post-merge source audit found that `AdminEmailHandler.emit()` incorrectly calls `ServerFormatter.format()` for `self.format(...)`, because `self` and `cls` currently fall through to a unique same-module terminal-name lookup.

The same audit found that 3,208 reported missing Graphify relationships already have identical mapped endpoints and source files; Graphify and Compass anchor different lines within the same multiline import or call statement. Exact line equality therefore inflates the missing count. The current checked-in nine-record audit remains a conformance fixture, not production qualification.

Peak RSS is also material: the standardized Django run measured about 4.76 GiB cold and 5.18 GiB incremental. The pipeline clones source bytes, prepared syntax input, and every fresh `Extraction` while resolution and cache publication overlap.

## Global Constraints

- Correctness is non-negotiable: unresolved or ambiguous evidence must fail closed.
- Implement production behavior first, then add regression and qualification checks; do not use TDD.
- Do not bump crate, schema, producer, extraction-semantics, cache, or document versions.
- Do not preserve a legacy receiver fallback or add a compatibility projection.
- Use hard cutovers to the new algorithms.
- Do not run Graphify or `graphify update .`.
- Do not count Graphify output as ground truth; every accepted comparison relaxation needs repository source evidence.
- Preserve deterministic cold/warm graph bytes and atomic cache publication.

---

### Task 1: Hard-cut source-grounded receiver dispatch

**Files:**
- Modify: `crates/compass-languages/src/evidence/model.rs`
- Modify: `crates/compass-languages/src/evidence/build.rs`
- Modify: `crates/compass-resolve/src/evidence.rs`
- Modify: `crates/compass-languages/tests/universal_evidence.rs`
- Modify: `crates/compass-resolve/tests/universal_resolution.rs`

**Interfaces:**
- Produces: `ReceiverDispatchStrategy::C3FromReceiver` for `self.member()` and `cls.member()`.
- Consumes: exact `DeclarationContext::enclosing_type_qualified_name` and ordered direct-base evidence.
- Invariant: receiver dispatch returns from the hierarchy-specific resolver path and never enters lexical, module, or imported terminal-name fallback.

- [x] Add `C3FromReceiver` and emit it for Python `self`/`cls` calls with an exact enclosing type.
- [x] Implement shared resolver traversal that checks the receiver’s exact member, a source-proven first-base/single-inheritance prefix, and then its complete C3 linearization.
- [x] Keep `super()` on `C3AfterReceiver`; dynamic or unprovable base order remains unresolved.
- [x] Add post-implementation extraction and resolution regressions covering own member, inherited member, external base, unrelated same-module member, subclass-only override, nested construction, and `cls` dispatch.
- [x] Run the focused language and resolver suites and inspect changed Django `self`/`cls` facts.

### Task 2: Source-statement occurrence oracle

**Files:**
- Create: `benchmarks/performance/compass/occurrences.py`
- Modify: `benchmarks/performance/compass/correctness.py`
- Modify: `benchmarks/performance/harness.py`
- Modify: `benchmarks/performance/analyze.py`
- Modify: `benchmarks/performance/tests/test_correctness.py`

**Interfaces:**
- Produces: `SourceOccurrenceOracle.same_statement(relation, source_file, left_location, right_location) -> bool`.
- Consumes: an optional pinned corpus root passed to `compare_graphs`.
- Invariant: exact file/line matching remains the default; relaxed matching requires both locations to fall within the same supported AST node under a validated in-root source path.

- [x] Implement a provider registry keyed by source suffix and a Python AST provider for import statements and call expressions.
- [x] Pass the pinned corpus checkout into the shared graph gate and expose the same option in offline analysis.
- [x] Replace exact-line-only occurrence checks with exact-or-proven-statement checks while retaining a distinct coverage reason.
- [x] Add post-implementation checks for multiline imports, different statements, traversal rejection, parse failure, unsupported languages, and false Graphify call locations.
- [x] Reclassify the retained Django artifact and report exact, statement-equivalent, ambiguous, rejected, and genuinely missing populations separately.

### Task 3: Production audit population support

**Files:**
- Modify: `benchmarks/performance/compass/audit.py`
- Modify: `benchmarks/performance/harness.py`
- Modify: `benchmarks/performance/tests/test_audit.py`
- Modify: `benchmarks/performance/README.md`

**Interfaces:**
- Produces: deterministic candidate export from comparison SQLite into source-bounded audit records.
- Invariant: generated candidates are unjudged inputs; only explicit adjudication can enter precision or recall gates.

- [x] Add deterministic export of accepted, rejected, ambiguous, and missing relationship hypotheses with endpoint and occurrence evidence.
- [x] Require pinned commit, graph digest, snippet digest, target cluster, and explicit judgment before qualification consumes a record.
- [x] Add post-implementation validation checks and document how to grow from conformance to the 2,000-record production gate.

### Task 4: Eliminate avoidable peak-retention clones

**Files:**
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/program.rs`
- Modify: `crates/compass-files/src/cache.rs`
- Modify: `crates/compass-files/tests/contracts.rs`
- Modify: `crates/compass-core/tests/code_graph_v1_publication_resilience.rs`

**Interfaces:**
- Produces: prepared owned AST-cache writes and shared immutable source bytes.
- Invariant: cache keys, MessagePack bytes, graph bytes, and atomic publication semantics remain unchanged.

- [x] Store fresh source bytes in shared immutable buffers through extraction and Program preparation.
- [x] Split portable AST cache preparation from publication so the pipeline encodes borrowed extractions, then moves the originals into owned resolution without cloning them.
- [x] Avoid constructing a second source-text byte map when no external artifact requires it.
- [x] Add post-implementation cache-contract and byte-stability checks.
- [ ] Run a controlled same-commit Django cold/warm/incremental comparison and report both elapsed time and peak RSS; retain the change only if correctness is unchanged and the tradeoff is justified.

### Task 5: Verification and delivery

**Files:**
- Create: `docs/superpowers/reviews/2026-07-31-source-grounded-correctness-hardening.md`

- [ ] Run formatting, focused tests, Python benchmark tests, clippy, locked workspace tests, and deterministic publication checks.
- [ ] Run the retained Django comparison with the pinned source root and audit every changed classification population.
- [ ] Confirm the known `AdminEmailHandler.emit -> ServerFormatter.format` edge is absent without losing source-proven receiver edges.
- [ ] Record controlled performance evidence and clearly distinguish measured results from inference.
- [ ] Commit cohesive changes, push the branch, and create a PR with exact commands, results, residual risks, and no perfection claim unsupported by evidence.
