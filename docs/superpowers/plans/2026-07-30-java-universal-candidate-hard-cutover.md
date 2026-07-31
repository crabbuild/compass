# Java Universal Candidate Hard-Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` after the Rust Phase 2 quality gate passes.
> This plan is implementation-first: production code precedes
> post-implementation conformance tests.

**Goal:** Establish Java as a version-1 `UniversalCandidate`, hard-cut Java
graph production and member resolution to the source-agnostic universal path,
and prove the vertical against checked Java/Spring fixtures and the pinned
Spring Framework corpus.

**Architecture:** Implement a Java adapter policy that emits the evidence
contract proven by Rust from one Tree-sitter traversal. Use the existing
universal local projector and collection resolver; remove Java branches from
the generic legacy publisher and language-member resolver; keep Spring pack
activation and domain policy pack-owned while targeting universal Java
declarations.

**Tech Stack:** Rust 2024, Tree-sitter Java, shared universal evidence and
projection APIs, Spring framework pack, Python qualification harness.

## Global Constraints

- Begin only after every Rust Phase 2 blocking gate passes.
- Keep `compass.languages.evidence/1`.
- Keep Java adapter version 1 and set profile `UniversalCandidate`.
- Do not dual-run, translate, feature-flag, or retain the replaced Java graph
  publisher.
- Do not modify central resolver or publisher logic for Java; add only adapter
  policy and evidence.
- Keep every language outside Rust and Java on its existing algorithm.
- Implement production behavior before adding or updating tests.
- Spring strict Graphify coverage must improve over the captured Java legacy
  baseline without a relation-family regression.
- Compass must remain faster than Graphify in comparable cold and warm Spring
  workloads.
- Peak RSS is measured and reported but is not a blocking gate.

---

### Task 1: Capture the Java Legacy Baseline

**Context:** Java needs a pre-cutover comparison distinct from the final
candidate result. The Rust cutover does not change Java extraction, so capture
the baseline after Rust qualifies and before editing Java production code.

**Files:**

- Create: `docs/superpowers/reviews/2026-07-30-java-universal-candidate.md`
- Generated, ignored:
  `target/performance/runs/spring-java-legacy/run.json`
- Generated, ignored:
  `target/performance/runs/spring-java-legacy/summary.md`

**Interfaces:**

- Consumes: existing checked Java fixtures, pinned Spring Framework checkout,
  current Graphify revision.
- Produces: legacy node/edge counts, per-relation strict coverage,
  deterministic digests, cold/warm/incremental timing, and RSS.

- [ ] **Step 1: Record checked fixture output**

  Extract `jpa.java`, Spring jobs and messaging fixtures,
  `NearMatches.java`, `PlayController.java`, and `SpringController.java`
  three times. Store canonical digests and normalized relation counts in the
  new review.

- [ ] **Step 2: Run Spring preflight and prepare**

```bash
python3 benchmarks/performance/harness.py doctor --repository spring
python3 benchmarks/performance/harness.py prepare --repository spring
```

  Read and retain the exact Spring SHA from `preparation.json`. Resolve and
  retain the exact Graphify SHA once. These become immutable inputs to both
  the legacy and candidate comparisons.

- [ ] **Step 3: Run the three-sample legacy comparison**

```bash
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
python3 benchmarks/performance/harness.py compare \
  --repository spring \
  --repository-commit spring=<SPRING_SHA> \
  --graphify-commit <GRAPHIFY_SHA> \
  --workload build \
  --build-repeats 3 \
  --output target/performance/runs/spring-java-legacy
```

  A nonzero harness exit caused solely by a reported Graphify incremental
  determinism failure does not discard an otherwise complete report. Record
  the failure as baseline evidence and require all Compass samples to remain
  eligible.

- [ ] **Step 4: Write and commit the baseline review**

  Include pinned commits, eligible samples, correctness failures, relation
  coverage, medians, p95, and RSS. A failing legacy gate is baseline evidence,
  not a reason to weaken the final candidate thresholds.

```bash
git add docs/superpowers/reviews/2026-07-30-java-universal-candidate.md
git commit -m "docs(quality): record Java legacy baseline"
```

---

### Task 2: Implement the Java Universal Evidence Adapter

**Context:** Java currently shares `ExtractState` with legacy languages and
publishes nodes, edges, and `RawCall` records directly. Implement one
Java-owned AST traversal that emits the already-proven universal evidence
contract, then invokes the shared local projector. This is an evidence
adapter, not a second graph extractor.

**Files:**

- Create: `crates/compass-languages/src/java_lang.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-languages/src/registry.rs`
- Create: `crates/compass-languages/tests/java_universal_conformance.rs`

**Interfaces:**

- Consumes: `EvidenceBuilder`, `project_local`, Tree-sitter Java nodes, and
  adapter descriptor `compass.java`.
- Produces:

```rust
pub(crate) fn extract(
    path: &Path,
    source: &[u8],
    root: tree_sitter::Node<'_>,
) -> Extraction;
```

  Registry descriptor:

```rust
AdapterDescriptor {
    id: "compass.java".to_owned(),
    language: "java".to_owned(),
    version: 1,
    evidence_schema: UNIVERSAL_EVIDENCE_SCHEMA.to_owned(),
    profile: AdapterProfile::UniversalCandidate,
    capabilities: vec![
        AdapterCapability::Declarations,
        AdapterCapability::LexicalScopes,
        AdapterCapability::Namespaces,
        AdapterCapability::Overloads,
        AdapterCapability::Annotations,
        AdapterCapability::Inheritance,
        AdapterCapability::Interfaces,
        AdapterCapability::Imports,
        AdapterCapability::Calls,
        AdapterCapability::ExternalPackages,
    ],
}
```

- [ ] **Step 1: Emit Java packages and scopes**

  Parse `package_declaration` into a package declaration and root scope.
  Emit nested scopes for classes, interfaces, enums, records,
  annotation-type declarations, constructors, methods, and initializer
  blocks.

- [ ] **Step 2: Emit declarations and overload identities**

  Emit class, interface, enum, record, annotation type, constructor, method,
  field, and enum-member declarations. Normalize parameter types without
  parameter names or whitespace. Construct callable identity from package,
  lexical owner, callable role, name, and normalized parameter signature.

  Example identity:

```text
org.example.Service::find(java.lang.String,int)
org.example.Service::<init>(Repository)
```

- [ ] **Step 3: Emit Java imports and external identities**

  Parse normal, static, and wildcard imports. Preserve imported spelling,
  full identity, static member identity, wildcard package/type identity, and
  exact anchors. Mark package roots external when no same-repository package
  declaration proves them local.

- [ ] **Step 4: Emit Java relationships**

  Emit annotations, superclass, implemented interfaces, parameter/return/
  field type references, constructor calls, and method calls. Preserve
  receiver spelling, qualifier, argument count, and exact occurrence.
  Candidate kinds and relationship kinds must be explicit.

- [ ] **Step 5: Project one local Java graph**

  Finish one evidence batch, invoke `project_local`, retain the batch, and
  return no direct legacy nodes, edges, or raw calls.

- [ ] **Step 6: Register and dispatch Java**

  Add `mod java_lang`; dispatch `"java"` to `java_lang::extract` in
  `extract_generic_from_tree`; retain the Tree-sitter grammar registry and
  framework detection.

- [ ] **Step 7: Add post-implementation Java conformance tests**

  Cover packages, nested types, records, annotation types, fields, enum
  members, constructors, same-name overloads, generics, normal/static/
  wildcard imports, annotations, extends, implements, constructor calls,
  instance/static calls, ambiguity, external packages, Unicode anchors,
  repetition, malformed syntax, and evidence limits.

```rust
let overloads = batch
    .declarations
    .iter()
    .filter(|fact| fact.name == "find")
    .collect::<Vec<_>>();
assert_eq!(overloads.len(), 2);
assert_ne!(overloads[0].symbol, overloads[1].symbol);
assert!(overloads.iter().all(|fact| fact.signature.is_some()));
```

- [ ] **Step 8: Verify the Java adapter**

```bash
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-languages \
  --test java_universal_conformance \
  --test callable_builtin_semantics \
  --test engine_edge_coverage
```

- [ ] **Step 9: Commit**

```bash
git add crates/compass-languages
git commit -m "feat(languages): add Java universal evidence adapter"
```

---

### Task 3: Hard-Cut Java and Remove Legacy Resolution

**Context:** Java is not hard-cut while Java-specific branches in
`ExtractState` or `members::resolve_typed_members` can publish additional
facts. Remove those branches only after Task 2 produces equivalent graph
facts.

**Files:**

- Modify: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-languages/src/config.rs`
- Modify: `crates/compass-resolve/src/members.rs`
- Modify: `crates/compass-resolve/tests/builtin_resolution.rs`
- Modify: `crates/compass-resolve/tests/universal_resolution.rs`

**Interfaces:**

- Consumes: profile-based legacy bypass introduced by the Rust cutover.
- Produces: Java graph facts exclusively from `compass.java` evidence and
  universal projection.

- [ ] **Step 1: Remove Java branches from `ExtractState`**

  Delete Java parent-edge, enum-constant, function-reference, call-name,
  member deferral, Java `RawCall`, and built-in-type helper paths. Keep Groovy
  on the generic configuration by splitting the current `"java" | "groovy"`
  configuration arm.

- [ ] **Step 2: Remove Java member-family resolution**

  Delete `MemberFamily::Java`, its receiver/type rules, strict method branch,
  and tests. Java member calls must arrive only through universal candidates.
  Preserve Swift, TypeScript, C++, C#, Objective-C, Python, Ruby, and Pascal
  behavior.

- [ ] **Step 3: Add post-cutover assertions**

  Assert Java extraction contains one `compass.java` batch, every Java
  declaration node carries `_universal_adapter = "compass.java"`, raw calls
  are absent, and no legacy semantic-work metadata exists. Assert Groovy
  fixture output is unchanged.

- [ ] **Step 4: Verify removal and regression safety**

```bash
rg -n 'self\\.language == "java"|MemberFamily::Java|lang:.*java' \
  crates/compass-languages/src/engine.rs \
  crates/compass-resolve/src/members.rs
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-languages
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-resolve
```

- [ ] **Step 5: Commit**

```bash
git add crates/compass-languages crates/compass-resolve
git commit -m "refactor(languages): hard cut Java to universal projection"
```

---

### Task 4: Cut Spring Targeting to Universal Java Facts

**Context:** Spring activation and route/domain policy remain pack-owned, but
their targets must be universal Java declarations rather than assumptions
about legacy Java IDs or terminal labels. The generic framework target index
already accepts normalized nodes; adapt only the Java-facing evidence and
constraints.

**Files:**

- Modify: `crates/compass-languages/src/frameworks/java.rs`
- Modify: `crates/compass-languages/src/frameworks/enterprise.rs`
- Modify: `crates/compass-resolve/src/frameworks/target_index.rs`
- Modify: `crates/compass-resolve/tests/php_ruby_jvm_routes.rs`
- Modify: `crates/compass-resolve/tests/domain_resolution.rs`

**Interfaces:**

- Consumes: universal Java node attributes `qualified_name`, `signature`,
  `lexical_owner`, `symbol_kind`, and exact anchors.
- Produces: Spring route, controller, scheduled-job, messaging, JPA, and
  inheritance facts targeting exact universal Java node IDs.

- [ ] **Step 1: Replace legacy Java target assumptions**

  Resolve Spring handlers by package-qualified owner and method signature
  where available. Use exact annotation occurrence evidence for framework
  origin. Do not add terminal-name fallback.

- [ ] **Step 2: Preserve uniform pack limits**

  Keep activation evidence, target constraints, occurrence policy, candidate
  budgets, and resource limits in the existing framework interfaces. No
  Spring-specific branch may be added to the universal resolver or projector.

- [ ] **Step 3: Add post-implementation Spring tests**

  Cover overloaded controller methods, same-named controllers in different
  packages, inherited mappings, annotations with aliases, static imports,
  scheduled methods, message listeners, JPA entities, exact origins, and
  ambiguous near matches.

- [ ] **Step 4: Verify framework integration**

```bash
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-resolve \
  --test php_ruby_jvm_routes \
  --test domain_resolution \
  --test framework_resolution_scale
```

- [ ] **Step 5: Commit**

```bash
git add crates/compass-languages/src/frameworks \
  crates/compass-resolve/src/frameworks \
  crates/compass-resolve/tests
git commit -m "feat(frameworks): target Spring through universal Java facts"
```

---

### Task 5: Qualify Java Fixtures and Full Regression Suites

**Context:** Confirm the hard cut preserves checked Java output and does not
change other registered languages before the full Spring comparison.

**Files:**

- Modify: `docs/superpowers/reviews/2026-07-30-java-universal-candidate.md`
- Modify: `benchmarks/performance/tools/compare_language_fixtures.py`
- Modify: `benchmarks/performance/tests/test_language_fixture_compare.py`

**Interfaces:**

- Consumes: the reusable fixture comparator from the Rust plan.
- Produces: final Java fixture relation coverage and deterministic digests.

- [ ] **Step 1: Extend the fixture comparator manifest**

  Add `.java` fixture selection without Java-specific comparison logic.

- [ ] **Step 2: Extract every checked Java fixture three times**

  Require identical canonical graph digests and exact occurrence counts.
  Compare against both the captured Java legacy baseline and Graphify.

- [ ] **Step 3: Enforce fixture gates**

  Require no previously covered normalized fact to become missing or
  ambiguous. Require packages, overloads, annotations, inheritance,
  interfaces, imports, calls, and external identities to appear in the
  conformance summary.

- [ ] **Step 4: Run full post-implementation suites**

```bash
cargo fmt --all -- --check
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-languages
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-resolve
python3 -m unittest discover -s benchmarks/performance/tests -p 'test_*.py'
git diff --check
```

- [ ] **Step 5: Record and commit fixture evidence**

```bash
git add benchmarks/performance \
  docs/superpowers/reviews/2026-07-30-java-universal-candidate.md
git commit -m "test(quality): qualify Java universal fixtures"
```

---

### Task 6: Run the Pinned Spring Quality Gate

**Context:** Compare the final Java candidate against the same pinned Spring
and Graphify revisions used for the legacy baseline. A hard cut passes only
when strict coverage improves without a relation-family regression and
latency remains better.

**Files:**

- Modify: `docs/superpowers/reviews/2026-07-30-java-universal-candidate.md`
- Generated, ignored:
  `target/performance/runs/spring-java-universal/run.json`
- Generated, ignored:
  `target/performance/runs/spring-java-universal/summary.md`

**Interfaces:**

- Consumes: clean final Java commit and prepared Spring corpus.
- Produces: comparable three-sample candidate qualification.

- [ ] **Step 1: Re-run doctor**

```bash
python3 benchmarks/performance/harness.py doctor --repository spring
```

- [ ] **Step 2: Run the official comparison**

```bash
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
python3 benchmarks/performance/harness.py compare \
  --repository spring \
  --repository-commit spring=<SPRING_SHA> \
  --graphify-commit <GRAPHIFY_SHA> \
  --workload build \
  --build-repeats 3 \
  --output target/performance/runs/spring-java-universal
```

  Use the exact SHAs captured in Task 1. The harness may return nonzero for a
  Graphify-owned incremental determinism failure; if the report and all
  eligible samples exist, evaluate it and record that failure explicitly.

- [ ] **Step 3: Evaluate blocking gates**

  Require eligible deterministic Compass cold, warm, incremental, and restore
  samples; higher overall strict coverage than the Java legacy baseline; no
  lower per-relation coverage; and lower Compass cold/warm median latency than
  Graphify. If a gate fails, return to the responsible production task and
  repeat its post-implementation tests before rerunning Spring.

- [ ] **Step 4: Record the comparison**

  Add pinned revisions, legacy-versus-candidate deltas, Graphify coverage,
  medians, p95, RSS, diagnostics, and remaining gaps. State explicitly that
  RSS is non-blocking and Java remains `UniversalCandidate`.

- [ ] **Step 5: Commit qualification evidence**

```bash
git add docs/superpowers/reviews/2026-07-30-java-universal-candidate.md
git commit -m "docs(quality): record Java universal qualification"
```

---

### Task 7: Refresh the Repository Graph and Audit the Cutovers

**Context:** The final audit proves Rust and Java are hard-cut while every
other language remains registered on its prior path.

**Files:**

- Modify only if generated by command:
  `/Users/haipingfu/graphify/graphify-out/`
- Inspect: all files changed by both plans.

**Interfaces:**

- Consumes: final clean Rust/Java candidate branch.
- Produces: refreshed Graphify knowledge graph and a clean audit.

- [ ] **Step 1: Audit registry profiles**

  Assert Rust and Java are version-1 `UniversalCandidate`; every other
  registered language is `Legacy`; the universal resolver/projector contains
  no language-name match; and the generic Java/Rust legacy publisher branches
  are absent.

- [ ] **Step 2: Refresh Graphify**

```bash
cd /Users/haipingfu/graphify
graphify update .
```

- [ ] **Step 3: Run final verification**

```bash
cd /Users/haipingfu/graphify/compass/.worktrees/rust-universal-adapter
cargo fmt --all -- --check
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-languages
CARGO_TARGET_DIR=/Users/haipingfu/graphify/compass/target \
  cargo test --locked -p compass-resolve
python3 -m unittest discover -s benchmarks/performance/tests -p 'test_*.py'
git diff --check
git status --short
```

- [ ] **Step 4: Commit any required graph metadata**

  Commit only graph outputs actually changed by `graphify update .`; do not
  commit ignored benchmark corpora, caches, binaries, or generated run
  artifacts.
