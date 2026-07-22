# CompassQL Architecture Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build compass check on top of the completed CompassQL engine so portable read-only Cypher queries enforce architecture rules with witness evidence, accountable exemptions, baselines, SARIF, and stable exit behavior.

**Architecture:** A new compass-policy crate discovers isolated policy directories, validates strict TOML metadata, executes each query through compass-cypher and compass-query against one immutable snapshot, fingerprints returned rows, and evaluates exemptions/baselines. compass-output renders structured policy results; compass-cli exposes Compass-only check after all policy gates pass.

**Tech Stack:** Rust 2024, Rust 1.97.1, serde/serde_json, toml, sha2, time, existing compass-cypher/query/model/files/output/CLI crates.

## Global Constraints

- Complete docs/superpowers/plans/2026-07-22-compassql-engine.md first.
- Policies are ordinary read-only CompassQL; zero rows pass and every returned row is one violation.
- Keep query.cypher portable; policy metadata remains in policy.toml.
- Every policy requires schema, unique ID, severity, message, at least one owner, and limits that only lower global defaults.
- Never treat invalid metadata, query errors, graph errors, timeouts, cancellation, or resource limits as a pass.
- Every exemption requires exact fingerprint, reason, owner, and non-expired ISO-8601 date.
- Baselines suppress exact existing fingerprints only; they never suppress execution errors.
- Discovery never follows symlinks and every canonical metadata/query/baseline path remains within the selected worktree policy root.
- compass check is Compass-only and remains hidden until this plan is complete.
- Preserve existing Graphify compatibility and all workspace lint, platform, safety, and performance gates.
- Use red-green-refactor, task-sized commits, and graphify update . after code changes are complete.

---

## File and crate map

~~~text
crates/compass-policy/
├── Cargo.toml
├── src/
│   ├── lib.rs          public policy request/result interface
│   ├── error.rs        CPL1xxx discovery/config/baseline diagnostics
│   ├── config.rs       strict schema-1 TOML model
│   ├── discover.rs     root-confined deterministic discovery
│   ├── evaluate.rs     compile/execute zero-row policy contract
│   ├── evidence.rs     typed returned-row evidence
│   ├── fingerprint.rs  cmpv1 canonical SHA-256 identity
│   ├── exemption.rs    owner/reason/expiry evaluation
│   └── baseline.rs     canonical baseline reads/writes
└── tests/
    ├── discovery.rs
    ├── evaluation.rs
    ├── fingerprint.rs
    ├── exemption.rs
    └── baseline.rs
~~~

Extend:

- crates/compass-output/src/policy.rs and sarif.rs.
- crates/compass-cli/src/check_commands.rs and CLI tests.
- docs/COMPASSQL_POLICIES.md and examples/compass-policies/.
- hardening fuzz, mutation, coverage, and performance matrices.

## Shared public interfaces

~~~rust
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

pub struct PolicyRequest<'a> {
    pub graph: &'a compass_model::Graph,
    pub policy_roots: &'a [std::path::PathBuf],
    pub baseline: Option<&'a Baseline>,
    pub fail_on: Severity,
    pub cancellation: &'a std::sync::atomic::AtomicBool,
}

pub struct PolicySuiteResult {
    pub policies: Vec<PolicyResult>,
    pub violations: Vec<Violation>,
    pub warnings: Vec<PolicyWarning>,
    pub failure: Option<PolicyFailure>,
}

pub struct Violation {
    pub policy_id: String,
    pub severity: Severity,
    pub message: String,
    pub owners: Vec<String>,
    pub fingerprint: ViolationFingerprint,
    pub evidence: EvidenceRow,
    pub exemption: Option<AppliedExemption>,
}

pub fn evaluate(request: PolicyRequest<'_>) -> Result<PolicySuiteResult, PolicyError>;
~~~

## Test fixture contract

Create concrete reusable helpers with Task 1:

- crates/compass-policy/tests/support/mod.rs exports PolicyFixture::new, with_graph, wide_graph, root, write_policy, write_raw, write_query, evaluate, evaluate_with_timeout_ms, and add_violation.
- The same module exports valid_policy, fixture_result, evidence_with_node, fixture_exemptions, and suite; all produce public policy/query/model types with deterministic IDs and fixed clocks.
- crates/compass-cli/tests/support/mod.rs extends the engine plan's CliFixture with add_policy, add_violating_policy, corrupt_policy, and run_compass/run_graphify CommandResult values.
- Output fixtures compare canonical serde_json::Value or normalized SARIF; terminal golden files normalize only platform path separators, never semantic fields.

### Task 1: Scaffold compass-policy and strict schema-1 discovery

**Files:**
- Modify: Cargo.toml
- Modify: Cargo.lock
- Create: crates/compass-policy/Cargo.toml
- Create: crates/compass-policy/src/lib.rs
- Create: crates/compass-policy/src/error.rs
- Create: crates/compass-policy/src/config.rs
- Create: crates/compass-policy/src/discover.rs
- Create: crates/compass-policy/tests/discovery.rs

**Interfaces:**
- Consumes: compass-files bounded reads and canonical path helpers.
- Produces: PolicyConfig, PolicyLimits, Severity, DiscoveredPolicy, discover.

- [ ] **Step 1: Write strict discovery tests**

~~~rust
#[test]
fn discovery_is_sorted_strict_and_root_confined() {
    let fixture = PolicyFixture::new();
    fixture.write_policy("z-last", valid_policy("z.policy"));
    fixture.write_policy("a-first", valid_policy("a.policy"));
    let policies = discover(fixture.root()).expect("discover");
    assert_eq!(
        policies.iter().map(|policy| policy.config.id.as_str()).collect::<Vec<_>>(),
        vec!["a.policy", "z.policy"]
    );
    fixture.write_raw("escape/policy.toml", "schema=1\nid='escape'\nquery='../outside.cypher'\nseverity='error'\nmessage='x'\nowners=['team']");
    let error = discover(fixture.root()).expect_err("escape");
    assert_eq!(error.code(), "CPL1004");
}
~~~

- [ ] **Step 2: Run and verify package absence**

Run: cargo test -p compass-policy --test discovery

Expected: FAIL because compass-policy does not exist.

- [ ] **Step 3: Add the crate and exact metadata model**

PolicyConfig uses serde deny_unknown_fields and contains schema u16, validated dotted ID, query relative path, Severity, non-empty message/owners, stable tags, PolicyLimits, typed parameters, and exemptions. Reject schema other than 1, duplicate IDs, empty owners, invalid severity, absolute paths, symlinks, unsupported TOML values, and limits above global defaults with stable CPL1xxx errors.

- [ ] **Step 4: Implement deterministic root-confined discovery**

Recursively inspect directories beneath each selected policy root without following symlinks. A policy directory contains exactly policy.toml and its referenced query file; ignore unrelated regular files but reject duplicate metadata. Canonicalize the root and every existing file, verify prefix containment, read under 1 MiB caps, and sort by policy ID then path.

- [ ] **Step 5: Run discovery and clippy**

Run: cargo test -p compass-policy --test discovery && cargo clippy -p compass-policy --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add Cargo.toml Cargo.lock crates/compass-policy
git commit -m "feat(policy): discover strict Compass policies"
~~~

### Task 2: Evaluate zero-row policies and typed evidence

**Files:**
- Create: crates/compass-policy/src/evidence.rs
- Create: crates/compass-policy/src/evaluate.rs
- Modify: crates/compass-policy/src/lib.rs
- Create: crates/compass-policy/tests/evaluation.rs

**Interfaces:**
- Consumes: compass_cypher::compile and compass_query::execute.
- Produces: EvidenceRow, PolicyResult, Violation, PolicySuiteResult, evaluate.

- [ ] **Step 1: Write pass, violation, witness, and failure tests**

~~~rust
#[test]
fn zero_rows_pass_and_each_row_is_one_violation() {
    let fixture = PolicyFixture::with_graph();
    fixture.write_query("pass", "MATCH (n) WHERE false RETURN n");
    fixture.write_query(
        "fail",
        "MATCH p=(a {id:'domain'})-[:IMPORTS_FROM]->(b {id:'database'}) RETURN p AS witness,a,b",
    );
    let result = fixture.evaluate().expect("suite");
    assert!(result.policy("pass").expect("pass").passed);
    assert_eq!(result.policy("fail").expect("fail").violation_count, 1);
    assert!(result.violations[0].evidence.witness().is_some());
}

#[test]
fn timeout_is_suite_failure_not_pass() {
    let error = PolicyFixture::wide_graph().evaluate_with_timeout_ms(0).expect_err("timeout");
    assert_eq!(error.exit_code(), 4);
}
~~~

- [ ] **Step 2: Run and verify evaluator absence**

Run: cargo test -p compass-policy --test evaluation

Expected: FAIL.

- [ ] **Step 3: Compile and execute every policy against one snapshot**

Build ParameterTypes from metadata, compile query source with policy source name, lower QueryLimits from metadata, and execute with the shared Graph and cancellation token. Convert every Row into EvidenceRow retaining column names and typed CompassValue data. A Path column named witness is primary; other columns remain structured evidence.

- [ ] **Step 4: Define failure precedence and deterministic ordering**

Discovery/config/compile errors map to exit 2, graph errors to 3 at the caller boundary, and runtime/limit/cancel/internal errors to 4. Stop evaluating after a non-query-local infrastructure failure; collect independent metadata/compile errors only when no query executed. Sort policy results by ID and violations by policy ID, then by the cmpv1 fingerprint introduced in Task 3; until Task 3, use the canonical evidence bytes that become that fingerprint as the identical ordering key.

- [ ] **Step 5: Run evaluation gates**

Run: cargo test -p compass-policy --test evaluation && cargo clippy -p compass-policy --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-policy/src crates/compass-policy/tests/evaluation.rs
git commit -m "feat(policy): evaluate architecture queries"
~~~

### Task 3: Add canonical fingerprints and accountable exemptions

**Files:**
- Create: crates/compass-policy/src/fingerprint.rs
- Create: crates/compass-policy/src/exemption.rs
- Modify: crates/compass-policy/src/config.rs
- Modify: crates/compass-policy/src/evaluate.rs
- Create: crates/compass-policy/tests/fingerprint.rs
- Create: crates/compass-policy/tests/exemption.rs

**Interfaces:**
- Consumes: EvidenceRow and time::Date.
- Produces: ViolationFingerprint, fingerprint, AppliedExemption, PolicyWarning.

- [ ] **Step 1: Write stability, expiry, and stale-exemption tests**

~~~rust
#[test]
fn fingerprint_ignores_locations_but_tracks_graph_identity() {
    let first = evidence_with_node("node-1", "L10");
    let moved = evidence_with_node("node-1", "L99");
    let different = evidence_with_node("node-2", "L10");
    assert_eq!(fingerprint("policy.id", &first), fingerprint("policy.id", &moved));
    assert_ne!(fingerprint("policy.id", &first), fingerprint("policy.id", &different));
}

#[test]
fn expired_exemption_fails_and_unknown_exemption_warns() {
    let today = time::Date::from_calendar_date(2026, time::Month::July, 22)
        .expect("valid fixed date");
    let result = evaluate_exemptions(today, fixture_exemptions());
    assert!(result.errors.iter().any(|error| error.code() == "CPL1012"));
    assert!(result.warnings.iter().any(|warning| warning.code == "CPLW1002"));
}
~~~

- [ ] **Step 2: Run and verify failures**

Run: cargo test -p compass-policy --test fingerprint --test exemption

Expected: FAIL.

- [ ] **Step 3: Implement cmpv1 canonical encoding**

Encode fingerprint schema, policy ID, returned column names, stable node IDs, relationship source/type/target plus parallel identity, ordered path node/relationship IDs, and recursively typed scalar/list/map values into length-prefixed bytes; exclude timestamps, source lines, display formatting, and graph indices. Hash with SHA-256 and format cmpv1 plus lowercase hex separated by a colon.

- [ ] **Step 4: Apply strict exemptions**

Parse exact cmpv1 fingerprint, non-empty reason/owner, and ISO date. An expired exemption is CPL1012 and exit 2. A matching active exemption marks but does not remove the violation. An unknown active fingerprint emits CPLW1002. Duplicate exemption fingerprints are CPL1013.

- [ ] **Step 5: Run fingerprint and exemption tests**

Run: cargo test -p compass-policy --test fingerprint --test exemption && cargo clippy -p compass-policy --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-policy
git commit -m "feat(policy): fingerprint and exempt violations"
~~~

### Task 4: Add explicit canonical baselines

**Files:**
- Create: crates/compass-policy/src/baseline.rs
- Modify: crates/compass-policy/src/lib.rs
- Modify: crates/compass-policy/src/evaluate.rs
- Create: crates/compass-policy/tests/baseline.rs

**Interfaces:**
- Consumes: ViolationFingerprint and compass-files atomic writer.
- Produces: Baseline, BaselineEntry, load_baseline, write_baseline.

- [ ] **Step 1: Write round-trip, new-only, stale, and corruption tests**

~~~rust
#[test]
fn baseline_suppresses_exact_existing_fingerprints_only() {
    let baseline = Baseline::new(["cmpv1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);
    assert!(baseline.contains("cmpv1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    assert!(!baseline.contains("cmpv1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
    let bytes = baseline.to_canonical_json().expect("json");
    assert_eq!(Baseline::from_slice(&bytes).expect("round trip"), baseline);
}
~~~

- [ ] **Step 2: Run and verify baseline API absence**

Run: cargo test -p compass-policy --test baseline

Expected: FAIL.

- [ ] **Step 3: Implement versioned canonical JSON**

Baseline schema 1 stores generated_at only as informational metadata plus sorted unique entries containing fingerprint and policy ID. Identity comparisons ignore generated_at. Reject unknown schema/fields, duplicate fingerprints, invalid fingerprints, files above 16 MiB, symlinks, and paths outside the worktree root.

- [ ] **Step 4: Implement explicit atomic writes and evaluation**

write_baseline requires the caller to pass confirmed=true, writes with compass_files::write_bytes_atomic, and never overwrites through a symlink. Evaluation marks matching violations baseline_existing, reports unmatched entries as stale warnings, and leaves new fingerprints active. No baseline state affects errors.

- [ ] **Step 5: Run baseline tests**

Run: cargo test -p compass-policy --test baseline && cargo clippy -p compass-policy --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-policy
git commit -m "feat(policy): add explicit violation baselines"
~~~

### Task 5: Render terminal, JSON, JSONL, and SARIF policy results

**Files:**
- Create: crates/compass-output/src/policy.rs
- Create: crates/compass-output/src/sarif.rs
- Modify: crates/compass-output/src/lib.rs
- Modify: crates/compass-output/Cargo.toml
- Create: crates/compass-output/tests/policy.rs
- Create: crates/compass-output/tests/sarif.rs

**Interfaces:**
- Consumes: PolicySuiteResult.
- Produces: render_policy_text/json/jsonl/sarif.

- [ ] **Step 1: Write witness and SARIF schema tests**

~~~rust
#[test]
fn text_witness_preserves_direction_relation_confidence_and_location() {
    let text = render_policy_text(&fixture_result());
    assert!(text.contains("src/domain/order.rs:18"));
    assert!(text.contains("--CALLS [EXTRACTED]-->"));
    assert!(text.contains("Owner: platform-architecture"));
}

#[test]
fn sarif_has_rule_result_fingerprint_and_locations() {
    let value: serde_json::Value = serde_json::from_str(&render_policy_sarif(&fixture_result()).expect("sarif")).expect("json");
    assert_eq!(value["version"], "2.1.0");
    assert_eq!(value["runs"][0]["results"][0]["ruleId"], "architecture.domain-isolation");
    assert!(value["runs"][0]["results"][0]["partialFingerprints"]["compassViolation"].is_string());
}
~~~

- [ ] **Step 2: Run and verify renderers are absent**

Run: cargo test -p compass-output --test policy --test sarif

Expected: FAIL.

- [ ] **Step 3: Implement terminal and structured renderers**

Text groups by severity/policy and prints witness path plus evidence, owners, policy path, fingerprint, exemption/baseline state, and summary. JSON emits one stable suite object. JSONL emits one violation or failure object per line followed by one summary object. Escape controls and never include ANSI in non-terminal formats.

- [ ] **Step 4: Implement SARIF 2.1.0**

Emit one rule per policy, one result per active or exempted violation, source artifact locations from evidence, related locations for witness hops, level from severity, partial fingerprint, owners/tags properties, and suppression only for active exact exemptions or baseline entries. Execution failures become SARIF invocation notifications and never a clean run.

- [ ] **Step 5: Run output gates**

Run: cargo test -p compass-output --test policy --test sarif && cargo clippy -p compass-output --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-output
git commit -m "feat(policy): render architecture findings"
~~~

### Task 6: Expose Compass-only compass check with stable exits

**Files:**
- Create: crates/compass-cli/src/check_commands.rs
- Modify: crates/compass-cli/src/lib.rs
- Modify: crates/compass-cli/Cargo.toml
- Create: crates/compass-cli/tests/check_cli.rs
- Modify: crates/compass-cli/tests/coverage_paths.rs

**Interfaces:**
- Consumes: compass_policy::evaluate and policy output renderers.
- Produces: compass check command with exit codes 0-4.

- [ ] **Step 1: Write command visibility and exit-code tests**

~~~rust
#[test]
fn check_is_compass_only_and_uses_stable_exit_categories() {
    let fixture = CliFixture::new();
    assert_eq!(fixture.run_compass(&["check"]).code(), 0);
    fixture.add_violating_policy();
    assert_eq!(fixture.run_compass(&["check"]).code(), 1);
    fixture.corrupt_policy();
    assert_eq!(fixture.run_compass(&["check"]).code(), 2);
    assert_ne!(fixture.run_graphify(&["check"]).code(), 0);
}
~~~

- [ ] **Step 2: Run and verify command absence**

Run: cargo test -p compass-cli --test check_cli

Expected: FAIL because check is unknown.

- [ ] **Step 3: Parse the exact CLI contract**

Support zero or more confined policy roots, --graph, --format text|json|jsonl|sarif, --output, --fail-on error|warning|info, --baseline, and --write-baseline PATH --confirm. Reject --confirm without a write, baseline write without confirm, unsupported format, duplicate singular flags, unknown flags, and Graphify frontend before loading a graph.

- [ ] **Step 4: Evaluate once and render atomically**

Load one directed immutable graph, evaluate all policies with one cancellation token, choose exit 0/1/2/3/4 from suite state and threshold, render complete output in memory under budget, and atomically write --output. Default fail_on is Error; warning/info may lower but never raise above Error.

- [ ] **Step 5: Expose help only after Tasks 1-5 pass**

Add compass check to Compass help and README command list. Graphify help and behavior stay unchanged.

- [ ] **Step 6: Run CLI and parity regression**

Run: cargo test -p compass-cli --test check_cli --test coverage_paths && cargo test -p compass-parity && cargo clippy -p compass-cli --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 7: Commit**

~~~bash
git add crates/compass-cli
git commit -m "feat(policy): expose compass check"
~~~

### Task 7: Add policy hardening, performance, and documentation gates

**Files:**
- Create: fuzz/fuzz_targets/policy_toml.rs
- Create: fuzz/fuzz_targets/policy_baseline.rs
- Modify: fuzz/Cargo.toml
- Modify: .github/workflows/compass-hardening.yml
- Modify: scripts/check_critical_coverage.sh
- Create: scripts/benchmark_compass_policies.sh
- Create: docs/COMPASSQL_POLICIES.md
- Create: examples/compass-policies/domain-isolation/policy.toml
- Create: examples/compass-policies/domain-isolation/query.cypher
- Modify: README.md
- Modify: PERFORMANCE.md

**Interfaces:**
- Consumes: complete policy engine.
- Produces: release gates and policy authoring guide.

- [ ] **Step 1: Add hostile metadata and baseline fuzz targets**

policy_toml feeds arbitrary bytes through the bounded strict config parser under a temp root. policy_baseline feeds arbitrary bytes through Baseline::from_slice. Seed corpora include symlink markers, traversal paths, duplicate IDs, expired exemptions, invalid UTF-8 bytes, huge numbers, unknown fields, and malformed cmpv1 values.

- [ ] **Step 2: Add policy performance qualification**

scripts/benchmark_compass_policies.sh builds suites of 1, 10, 100, and 1,000 policies over medium/large/adversarial graphs, measures shared graph/index load, compile/cache, execution, fingerprinting, SARIF, peak RSS, and cancellation. It fails on memory-budget violation, non-linear repeated graph loading, cancellation beyond 100 ms checkpoint latency, or a greater than 10% regression from the approved baseline.

- [ ] **Step 3: Extend hardening**

Add policy fuzz targets, 95% critical coverage for config/discovery/fingerprint/exemption/baseline/evaluation, mutation targets for pass/fail and expiry comparisons, and cross-platform check_cli execution.

- [ ] **Step 4: Write the authoring guide and examples**

Document directory layout, metadata schema, zero-row convention, witness alias, limits, parameters, severity, fail thresholds, fingerprints, exemptions, baseline workflow, SARIF, CI snippets, and failure exits. The example passes against an allowed graph and returns an exact witness against a forbidden graph.

- [ ] **Step 5: Run the complete policy gate**

Run:

~~~bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
scripts/benchmark_compass_policies.sh
~~~

Expected: PASS.

- [ ] **Step 6: Refresh the graph and commit**

Run: graphify update .

~~~bash
git add fuzz .github/workflows scripts docs examples README.md PERFORMANCE.md graphify-out
git commit -m "docs(policy): qualify architecture enforcement"
~~~

## Policy completion gate

Do not begin the integration plan until compass check is complete, Compass-only, resource bounded, deterministic, fully documented, and green under policy unit/integration/fuzz/mutation/coverage/cross-platform/performance gates.
