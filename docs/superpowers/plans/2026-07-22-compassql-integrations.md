# CompassQL Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the completed CompassQL engine and architecture-policy evaluator reusable through Compass CLI and MCP, then add exact historical queries and new-violation checks without duplicating graph loading, compilation, execution, or output semantics.

**Architecture:** A shared service in compass-core owns graph selection, immutable snapshots, compiled-plan caching, budgets, execution, and structured result envelopes. compass-cli and compass-mcp are adapters over that service. History is consumed by extending the existing GraphSelection contract behind a new SnapshotProvider boundary, and differential checks evaluate the same policy suite against two exact snapshots before comparing stable violation fingerprints.

**Tech Stack:** Rust 2024, Rust 1.97.1, rmcp 2.2, serde/serde_json, existing compass-core/cypher/query/policy/history/model/output/CLI/MCP crates.

## Global Constraints

- Complete docs/superpowers/plans/2026-07-22-compassql-engine.md and docs/superpowers/plans/2026-07-22-compassql-policy.md first.
- Tasks 5 and 6 additionally require the Versioned Graph Prolly Tree plan in the parent Graphify repository through its Task 9 GraphSelection and LoadedGraph::from_document contracts.
- CLI and MCP call one shared compile/execute service; adapters may parse transport arguments and render responses but may not implement query semantics.
- Every request captures one immutable Arc-backed graph snapshot. A reload affects only later requests.
- Compiled plans contain no graph data and are reusable only when language, planner, query, parameter types, and schema fingerprint all match.
- MCP query/check tools are Compass-native additions. Existing Graphify-compatible tools, resources, strings, and error behavior remain unchanged.
- MCP returns machine-readable structuredContent for new tools plus a concise text fallback generated from the same typed result.
- Historical selectors resolve an exact commit and never mix uncommitted worktree state into that snapshot.
- Differential checks evaluate identical policy source, parameters, limits, language version, and fingerprint version on both snapshots; either-side failure fails the operation.
- Enforce the engine's row, traversal, memory, deadline, cancellation, source, and parameter limits at every adapter boundary.
- Preserve workspace lints, unsafe_code = forbid, cross-platform standalone binaries, and Graphify parity.
- Use red-green-refactor, task-sized commits, and run graphify update . after code changes are complete.

---

## Plan sequence

This is plan 3 of 3. Tasks 1-4 may begin after the engine and policy completion gates. Tasks 5-6 wait for compass-history and exact snapshot selection. Task 7 is the release gate for the complete CompassQL core.

## File and crate map

Extend:

- crates/compass-core/src/query_service.rs: shared requests, graph selection, snapshot ownership, query/check orchestration, error taxonomy.
- crates/compass-core/src/lib.rs: stable re-exports.
- crates/compass-query/src/cache.rs: bounded concurrent PlanCache keyed by PlanCacheKey.
- crates/compass-mcp/src/lib.rs and tests/: CompassQL tool schemas, typed invocation, structured results, hot reload.
- crates/compass-cli/src/query_commands.rs and check_commands.rs: use the service and accept history/differential selectors.
- crates/compass-policy/src/differential.rs: compare active cmpv1 fingerprints across two successful suites.
- crates/compass-output/src/differential.rs: text, JSON, JSONL, and SARIF differential rendering.
- tests/fixtures/cql/: shared current/historical query and policy fixtures.
- docs/COMPASSQL.md and docs/COMPASSQL_POLICIES.md: CLI, MCP, history, caching, and differential contracts.

## Shared public interfaces

Use the GraphSelection type delivered by the history plan; do not introduce a second selector enum. This plan adds SnapshotProvider around it. The command parser resolves the default current graph to GraphSelection::File(default_graph_path).

~~~rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphSelection {
    File(std::path::PathBuf),
    Commit(String),
}

pub trait SnapshotProvider: Send + Sync {
    fn load(
        &self,
        selection: &GraphSelection,
    ) -> Result<std::sync::Arc<GraphSnapshot>, SnapshotError>;
}

pub struct GraphSnapshot {
    pub graph: compass_model::Graph,
    pub overlay: std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>>,
    pub schema_fingerprint: compass_model::SchemaFingerprint,
    pub identity: SnapshotIdentity,
}

pub struct CqlServiceRequest<'a> {
    pub selection: &'a GraphSelection,
    pub source_name: &'a str,
    pub source: &'a str,
    pub parameters: &'a compass_cypher::Parameters,
    pub limits: compass_query::QueryLimits,
    pub mode: compass_cypher::QueryProfileMode,
    pub cancellation: &'a std::sync::atomic::AtomicBool,
}

pub struct CqlServiceResponse {
    pub snapshot: SnapshotIdentity,
    pub result: compass_query::QueryResult,
    pub cache: PlanCacheDisposition,
}

pub struct CheckServiceRequest<'a> {
    pub selection: &'a GraphSelection,
    pub policy_roots: &'a [std::path::PathBuf],
    pub baseline: Option<&'a compass_policy::Baseline>,
    pub fail_on: compass_policy::Severity,
    pub cancellation: &'a std::sync::atomic::AtomicBool,
}

pub trait CompassQueryService: Send + Sync {
    fn query(&self, request: CqlServiceRequest<'_>) -> Result<CqlServiceResponse, ServiceError>;
    fn check(&self, request: CheckServiceRequest<'_>) -> Result<compass_policy::PolicySuiteResult, ServiceError>;
}
~~~

## Test fixture contract

Create concrete helpers rather than redefining them per test:

- tests/support/cql_fixture.rs exports CqlFixture::current(), write_graph, write_history_snapshot, write_policy, run_compass, run_graphify, and service.
- crates/compass-mcp/tests/support/mod.rs exports McpFixture::new, call, replace_graph_atomically, hold_snapshot, structured_content, and text_content.
- Every subprocess result helper exposes code(), stdout(), and stderr(); every service helper returns the public typed result.
- Fixtures use tempfile::TempDir, explicit graph.json documents, deterministic node/edge IDs, and no network or system Git configuration.

### Task 1: Add the shared snapshot-aware Compass query service

**Files:**
- Create: crates/compass-core/src/query_service.rs
- Modify: crates/compass-core/src/lib.rs
- Modify: crates/compass-core/Cargo.toml
- Modify: Cargo.lock
- Create: crates/compass-core/tests/query_service.rs
- Create: tests/support/cql_fixture.rs

**Interfaces:**
- Consumes: GraphSelection, SnapshotProvider, compass_cypher::compile, compass_query::execute, compass_policy::evaluate.
- Produces: CompassQueryService, LocalQueryService, requests/responses, SnapshotIdentity, ServiceError.

- [ ] **Step 1: Write failing query/check service tests**

~~~rust
#[test]
fn query_and_check_capture_one_snapshot_and_share_error_codes() {
    let fixture = CqlFixture::current();
    let service = fixture.service();
    let query = service.query(fixture.query("MATCH (n) RETURN n.id ORDER BY n.id"))
        .expect("query");
    assert_eq!(query.snapshot, fixture.snapshot_identity());
    assert_eq!(query.result.rows.len(), 3);

    let check = service.check(fixture.check()).expect("check");
    assert_eq!(check.policies.len(), 1);

    let error = service.query(fixture.query("CREATE (n)"))
        .expect_err("writes are forbidden");
    assert_eq!(error.code(), "CQL2001");
}
~~~

- [ ] **Step 2: Run and verify the service is absent**

Run: cargo test -p compass-core --test query_service

Expected: FAIL because query_service and LocalQueryService do not exist.

- [ ] **Step 3: Implement one orchestration boundary**

LocalQueryService owns Arc<dyn SnapshotProvider> and Arc<PlanCache>. query loads one Arc<GraphSnapshot>, derives ParameterTypes, compiles or obtains an eligible plan, executes against that exact snapshot, and returns its identity. check loads one snapshot and passes its graph to compass-policy. Convert lexer/parser/semantic/runtime/policy/snapshot errors to ServiceError without losing their stable public code, span, or causal message.

- [ ] **Step 4: Remove adapter-level query orchestration**

Refactor the Compass CLI CQL/check paths from plans 1-2 to construct service requests. Natural-language Graphify-compatible query stays on its existing path until a separate parity-approved migration.

- [ ] **Step 5: Run focused tests and clippy**

Run: cargo test -p compass-core --test query_service && cargo test -p compass-cli --test cql_cli --test check_cli && cargo clippy -p compass-core -p compass-cli --all-targets -- -D warnings

Expected: PASS with unchanged Graphify parity fixtures.

- [ ] **Step 6: Commit**

~~~bash
git add Cargo.toml Cargo.lock crates/compass-core crates/compass-cli tests/support
git commit -m "refactor(cql): centralize query service"
~~~

### Task 2: Harden plan caching for concurrent shared services

**Files:**
- Modify: crates/compass-query/src/cql/cache.rs
- Modify: crates/compass-core/src/query_service.rs
- Create: crates/compass-query/tests/plan_cache.rs
- Modify: crates/compass-core/tests/query_service.rs

**Interfaces:**
- Consumes: PlanCacheKey, CompiledQuery, SchemaFingerprint.
- Produces: production PlanCacheConfig, PlanCacheDisposition, CacheStats, concurrent duplicate suppression, and exact heap accounting on the engine plan's PlanCache.

- [ ] **Step 1: Write hit, miss, eviction, and schema-isolation tests**

~~~rust
#[test]
fn cache_key_includes_schema_and_parameter_types() {
    let cache = PlanCache::new(PlanCacheConfig { max_entries: 2, max_bytes: 64 * 1024 });
    let first = key("MATCH (n:Function) RETURN n", schema("a"), types(&[]));
    let changed_schema = key("MATCH (n:Function) RETURN n", schema("b"), types(&[]));
    let changed_types = key("MATCH (n:Function) RETURN n", schema("a"), types(&[("x", "Integer")]));
    cache.insert(first.clone(), compiled("first")).expect("insert");
    assert!(cache.get(&first).is_some());
    assert!(cache.get(&changed_schema).is_none());
    assert!(cache.get(&changed_types).is_none());
}
~~~

- [ ] **Step 2: Run and verify shared-service behavior is absent**

Run: cargo test -p compass-query --test plan_cache

Expected: FAIL because the engine cache does not yet expose bounded concurrent service behavior or stats.

- [ ] **Step 3: Harden the engine cache for bounded concurrency**

Key SHA-256 over language version, planner version, exact UTF-8 source, ordered parameter type signature, compile limits that affect plans, and graph schema fingerprint. Store Arc<CompiledQuery> and exact measured heap weight. Default to 1,024 entries and 64 MiB; evict least-recently-used entries under one mutex, compile outside the lock, then insert with duplicate suppression. Never cache diagnostics, execution results, graph values, deadlines, or cancellation state.

- [ ] **Step 4: Expose deterministic observability**

Return Hit, Miss, or Bypassed in CqlServiceResponse. CacheStats exposes entries, bytes, hits, misses, evictions, and duplicate_compilations using atomics. Do not place timings or cache disposition in normal query rows.

- [ ] **Step 5: Run focused and race-oriented tests**

Run: cargo test -p compass-query --test plan_cache && cargo test -p compass-core --test query_service && cargo clippy -p compass-query -p compass-core --all-targets -- -D warnings

Expected: PASS, including 32 concurrent requests for one key and deterministic capacity eviction.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-query crates/compass-core
git commit -m "perf(cql): cache schema-safe query plans"
~~~

### Task 3: Expose structured CompassQL MCP tools

**Files:**
- Modify: crates/compass-mcp/src/lib.rs
- Create: crates/compass-mcp/src/cql_tools.rs
- Create: crates/compass-mcp/tests/cql_tools.rs
- Create: crates/compass-mcp/tests/support/mod.rs
- Modify: crates/compass-mcp/Cargo.toml

**Interfaces:**
- Consumes: CompassQueryService and typed results.
- Produces: query_cql, explain_cql, and check_architecture MCP tools.

- [ ] **Step 1: Write tool-schema and structured-result tests**

~~~rust
#[test]
fn cql_tools_publish_closed_schemas_and_structured_results() {
    let fixture = McpFixture::new();
    let tools = fixture.tools();
    assert!(tools.iter().any(|tool| tool.name == "query_cql"));
    assert!(tools.iter().any(|tool| tool.name == "explain_cql"));
    assert!(tools.iter().any(|tool| tool.name == "check_architecture"));
    let result = fixture.call("query_cql", json!({
        "query": "MATCH (n) RETURN n.id AS id ORDER BY id",
        "parameters": {},
        "max_rows": 100
    })).expect("call");
    let structured = fixture.structured_content(&result);
    assert_eq!(structured["schema"], "compass.cql.result/1");
    assert_eq!(structured["columns"][0]["name"], "id");
    assert!(fixture.text_content(&result).contains("3 rows"));
}
~~~

- [ ] **Step 2: Run and verify tools are absent**

Run: cargo test -p compass-mcp --test cql_tools

Expected: FAIL because the three tools are not listed.

- [ ] **Step 3: Define exact closed input schemas**

query_cql accepts query, optional JSON parameters, project_path, max_rows, timeout_ms, max_path_depth, max_expanded_relationships, and max_memory_bytes. explain_cql accepts query, parameter_types, project_path, and compile limits and never executes. check_architecture accepts project_path, policy_roots or policy_ids, fail_on, baseline, and the lower-only limits. Set additionalProperties false and bounded lengths/counts in every JSON Schema; MCP has no presentation-format input because rows, plans, and policy results are always structured.

- [ ] **Step 4: Return versioned structured envelopes**

Use rmcp CallToolResult::structured for success and structured_error for tool-domain failures, then replace their generated JSON text content with one concise ContentBlock::text summary derived from the same typed object. Result schemas are compass.cql.result/1, compass.cql.plan/1, and compass.policy.result/1 and contain typed columns/rows or plan/policy objects, snapshot identity, truncation=false, and stable diagnostic objects. Protocol/invalid-schema failures remain MCP ErrorData.

- [ ] **Step 5: Preserve the compatibility surface**

Do not change existing tool names, input schemas, invoke string behavior, resources, server name, or Graphify parity snapshots. Add a typed invoke_structured test helper; keep invoke for legacy tests.

- [ ] **Step 6: Run MCP and parity gates**

Run: cargo test -p compass-mcp --test cql_tools --test hot_reload && cargo test -p compass-parity && cargo clippy -p compass-mcp --all-targets -- -D warnings

Expected: PASS; new tools return structuredContent and old tools remain byte-compatible.

- [ ] **Step 7: Commit**

~~~bash
git add crates/compass-mcp
git commit -m "feat(mcp): expose structured CompassQL tools"
~~~

### Task 4: Make MCP snapshot reload and caching atomic

**Files:**
- Modify: crates/compass-mcp/src/lib.rs
- Modify: crates/compass-mcp/src/cql_tools.rs
- Modify: crates/compass-mcp/tests/hot_reload.rs
- Modify: crates/compass-mcp/tests/cql_tools.rs

**Interfaces:**
- Consumes: Arc<GraphSnapshot>, SnapshotProvider, PlanCache.
- Produces: atomic per-request snapshot capture and reload-safe MCP execution.

- [ ] **Step 1: Write in-flight reload and cache-reuse tests**

~~~rust
#[test]
fn in_flight_request_keeps_old_snapshot_and_next_request_gets_new_snapshot() {
    let fixture = McpFixture::new();
    let held = fixture.hold_snapshot();
    fixture.replace_graph_atomically(graph_with_node("new"));
    assert!(held.graph.node_by_id("new").is_none());
    let next = fixture.call("query_cql", json!({"query": "MATCH (n {id:'new'}) RETURN n"}))
        .expect("next request");
    assert_eq!(fixture.structured_content(&next)["rows"].as_array().expect("rows").len(), 1);
}
~~~

- [ ] **Step 2: Run and observe the missing snapshot API**

Run: cargo test -p compass-mcp --test hot_reload

Expected: FAIL at the new hold_snapshot assertion/API.

- [ ] **Step 3: Refactor GraphStore to provide snapshots**

Replace GraphContext graph ownership with Arc<GraphSnapshot> while keeping overlay/community derived views associated with that same identity. Continue metadata-key reload detection, but load and validate the replacement fully before swapping the cache entry. Never hold the cache mutex during file reads, parsing, query compilation, or execution.

- [ ] **Step 4: Share one process-level plan cache**

GraphifyMcp clones share LocalQueryService and PlanCache. A graph content change with the same schema may reuse plans; a schema-fingerprint change misses. Add a test-only stats accessor behind cfg(test), not a production MCP tool.

- [ ] **Step 5: Run reload, concurrency, and memory gates**

Run: cargo test -p compass-mcp --test hot_reload --test cql_tools && cargo test -p compass-mcp --release --test cql_tools

Expected: PASS with no partial reload, deadlock, mixed-snapshot row, or unbounded cache growth.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-mcp
git commit -m "perf(mcp): share immutable query snapshots"
~~~

### Task 5: Add exact historical CompassQL selection

**Precondition:** compass-history, LoadedGraph::from_document, GraphSelection::Commit, and existing query/path/explain --at tests from the Versioned Graph Prolly Tree plan are complete.

**Files:**
- Modify: crates/compass-cli/src/query_commands.rs
- Modify: crates/compass-cli/src/check_commands.rs
- Modify: crates/compass-core/src/query_service.rs
- Modify: crates/compass-history/src/lib.rs
- Create: crates/compass-cli/tests/cql_history.rs
- Modify: crates/compass-cli/tests/coverage_paths.rs

**Interfaces:**
- Consumes: GraphSelection::Commit and the history realization loader.
- Produces: compass query --cql ... --at REV and compass check --at REV.

- [ ] **Step 1: Write exact-commit query and policy tests**

~~~rust
#[test]
fn cql_and_policy_use_only_the_resolved_commit_snapshot() {
    let fixture = CqlFixture::with_history();
    fixture.modify_worktree_without_commit("worktree-only");
    let old = fixture.run_compass(&[
        "query", "--cql", "MATCH (n) RETURN n.id ORDER BY n.id", "--at", "HEAD~1",
    ]);
    assert_eq!(old.code(), 0);
    assert!(!old.stdout().contains("worktree-only"));
    let check = fixture.run_compass(&["check", "--at", "HEAD~1"]);
    assert_eq!(check.code(), 0);
}
~~~

- [ ] **Step 2: Run and verify CQL/check reject --at**

Run: cargo test -p compass-cli --test cql_history

Expected: FAIL because these command paths do not consume GraphSelection::Commit.

- [ ] **Step 3: Reuse the shared selector parser**

Accept --at REV and --at=REV for query --cql and check. Reject --at with --graph, duplicate selectors, empty/missing revisions, unresolved/ambiguous revisions, and Graphify frontend before graph loading. Keep query source positional/--file/--stdin exclusivity unchanged.

- [ ] **Step 4: Resolve once and retain snapshot identity**

Extend LocalQueryService's SnapshotProvider to resolve the revision to a full commit OID, load the committed graph document through the history realization loader, construct GraphSnapshot with identity commit:<40-hex-oid>:<root-hash>, and retain its activity guard through output completion. Never consult current graph.json after Commit selection succeeds.

- [ ] **Step 5: Run history, CLI, and parity gates**

Run: cargo test -p compass-cli --test cql_history --test coverage_paths && cargo test -p compass-history && cargo test -p compass-parity

Expected: PASS for current/file/commit selectors and unchanged Graphify behavior.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-cli crates/compass-core crates/compass-history
git commit -m "feat(cql): query exact historical graphs"
~~~

### Task 6: Add differential architecture enforcement

**Files:**
- Create: crates/compass-policy/src/differential.rs
- Modify: crates/compass-policy/src/lib.rs
- Create: crates/compass-policy/tests/differential.rs
- Modify: crates/compass-cli/src/check_commands.rs
- Create: crates/compass-cli/tests/check_differential.rs
- Create: crates/compass-output/src/differential.rs
- Modify: crates/compass-output/src/lib.rs

**Interfaces:**
- Consumes: two successful PolicySuiteResult values using cmpv1 fingerprints.
- Produces: DifferentialPolicyResult and compass check --against REV [--new-only].

- [ ] **Step 1: Write new/resolved/unchanged and failure tests**

~~~rust
#[test]
fn differential_result_partitions_active_fingerprints() {
    let comparison = suite(&["cmpv1:old", "cmpv1:same"]);
    let current = suite(&["cmpv1:same", "cmpv1:new"]);
    let diff = compare_suites(&comparison, &current).expect("compare");
    assert_eq!(diff.new_fingerprints(), ["cmpv1:new"]);
    assert_eq!(diff.resolved_fingerprints(), ["cmpv1:old"]);
    assert_eq!(diff.unchanged_fingerprints(), ["cmpv1:same"]);
}

#[test]
fn either_snapshot_failure_is_not_a_clean_diff() {
    let fixture = CqlFixture::with_history();
    fixture.corrupt_historical_graph("HEAD~1");
    let result = fixture.run_compass(&["check", "--against", "HEAD~1", "--new-only"]);
    assert_eq!(result.code(), 3);
}
~~~

- [ ] **Step 2: Run and verify differential APIs are absent**

Run: cargo test -p compass-policy --test differential && cargo test -p compass-cli --test check_differential

Expected: FAIL.

- [ ] **Step 3: Implement typed set comparison**

Require the same policy IDs, language version, policy source digests, typed parameter digests, effective limits, and fingerprint version on both suites. Partition non-exempt, non-baselined active violations by (policy_id, fingerprint) into new, resolved, and unchanged BTreeMaps. Preserve full current evidence for new/unchanged and comparison evidence for resolved. A mismatch returns CPL4001 rather than comparing unlike evaluations.

- [ ] **Step 4: Add the exact CLI contract**

compass check --against REV evaluates current selection versus REV. --new-only requires --against and exits 1 only for new active violations at/above fail-on; without --new-only, normal current violations determine exit 1 while output includes all partitions. Reject --against with --at for the comparison side ambiguity, duplicate revisions, and Graphify frontend. Baselines and exemptions apply independently but identically before set comparison.

- [ ] **Step 5: Render every supported format**

Text groups New, Resolved, and Unchanged with counts. JSON schema compass.policy.diff/1 contains both snapshot identities and all partitions. JSONL emits one header followed by typed violation records with status. SARIF emits current new/unchanged results and records resolved fingerprints in run properties; --new-only emits only new results.

- [ ] **Step 6: Run differential gates**

Run: cargo test -p compass-policy --test differential && cargo test -p compass-cli --test check_differential && cargo test -p compass-output && cargo clippy -p compass-policy -p compass-cli -p compass-output --all-targets -- -D warnings

Expected: PASS with stable ordering and exit codes 0/1/2/3/4.

- [ ] **Step 7: Commit**

~~~bash
git add crates/compass-policy crates/compass-cli crates/compass-output
git commit -m "feat(policy): detect new architecture violations"
~~~

### Task 7: Qualify the integrated CompassQL product surface

**Files:**
- Create: scripts/benchmark_compassql_integrations.sh
- Modify: .github/workflows/compass-hardening.yml
- Modify: scripts/check_critical_coverage.sh
- Modify: docs/COMPASSQL.md
- Modify: docs/COMPASSQL_POLICIES.md
- Modify: docs/COMPASSQL_SUPPORT.md
- Modify: README.md
- Modify: PERFORMANCE.md
- Modify: COMPATIBILITY.md

**Interfaces:**
- Consumes: complete engine, policy, CLI, MCP, cache, and history integration.
- Produces: release evidence and one authoritative public contract.

- [ ] **Step 1: Add end-to-end equivalence tests**

For the same graph, source, parameters, and limits, assert CLI JSON, MCP structuredContent, and direct service results normalize to identical columns/typed rows/diagnostics. Repeat for explain, policy suites, current snapshots, and historical snapshots. Assert old MCP and Graphify CLI golden fixtures remain byte-identical.

- [ ] **Step 2: Add integrated performance gates**

scripts/benchmark_compassql_integrations.sh measures cold CLI, warm in-process service, warm MCP, plan-cache contention, atomic graph reload, exact historical snapshot load, 100-policy differential checks, peak RSS, and cancellation latency. Fail if warm MCP adds more than 15% over the service, cache hits fail the engine warm target, reload mixes identities, peak RSS exceeds Python, or existing Compass gates regress by more than 10%.

- [ ] **Step 3: Extend hardening and platform CI**

Run integration tests on Linux/macOS/Windows x86-64 and ARM64 where hosted/self-hosted runners exist. Add critical coverage for service error mapping, MCP schemas, cache keys/eviction, selector conflicts, and differential mismatch/exit branches. Include sanitizers or Miri for cache/snapshot ownership where supported.

- [ ] **Step 4: Publish the complete contract**

Document compass query --cql, compass check, --at, --against, --new-only, the three MCP tools, versioned structured schemas, limits, cache eligibility, snapshot identities, diagnostics, exit codes, portability boundaries, and examples. The support matrix links each accepted clause/function/tool/flag to tests and labels all Neo4j/openCypher differences.

- [ ] **Step 5: Run the complete product gate**

Run:

~~~bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
python3 scripts/check_compassql_support.py
scripts/benchmark_compassql.sh
scripts/benchmark_compass_policies.sh
scripts/benchmark_compassql_integrations.sh
~~~

Expected: all commands pass and the documented CompassQL surface is complete on every supported target.

- [ ] **Step 6: Refresh the graph and commit**

Run: graphify update .

~~~bash
git add scripts .github/workflows docs README.md PERFORMANCE.md COMPATIBILITY.md graphify-out
git commit -m "docs(cql): qualify CompassQL integrations"
~~~

## Integration completion gate

CompassQL core is complete only when CLI, MCP, policies, cache, current/history snapshots, and differential checks share one service contract; all structured schemas and compatibility surfaces are documented; no partial commands are exposed; and conformance, parity, fuzz, mutation, coverage, cross-platform, memory, latency, and cancellation gates pass.
