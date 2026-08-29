# Plan 017: Derive bounded, ranked execution flows from entry points

> **Executor instructions**: Implement the analysis contract before adding
> CLI/MCP presentation. A flow is a ranked structural possibility with stated
> evidence, not proof that code executed. Never enumerate all simple paths;
> every frontier, path, node, edge, byte, and elapsed-time dimension is bounded.
>
> **Drift check (run first)**:
> `git diff --stat 6680842c..HEAD -- crates/compass-analysis crates/compass-model crates/compass-query crates/compass-output crates/compass-core crates/compass-cli crates/compass-mcp docs`
> Stop and reconcile if a versioned execution-flow schema or entry-point
> detector has landed.

## Status

- **Priority**: P2
- **Effort**: L (four phases)
- **Risk**: MED
- **Depends on**: existing universal call graph; notebook/PHP plans are optional evidence enrichments, not prerequisites
- **Category**: direction / analysis / query
- **Planned at**: commit `6680842c`, 2026-08-10

## Why this matters

Compass can trace callers/callees around one caller-supplied root, and its
call-flow renderer groups architecture sections. It cannot yet discover
entry points and return the most important end-to-end call chains. Ranked
flows would give onboarding, debugging, and review workflows a bounded answer
to “what paths enter this system and reach critical effects?” while retaining
uncertain and unresolved hops.

## Current state and constraints

- `crates/compass-analysis/src/call_graph.rs:34-54` requires exactly one root
  and returns a local node/edge neighborhood.
- `crates/compass-analysis/src/call_graph.rs:204-253` performs bounded BFS; it
  does not produce ranked paths.
- `crates/compass-analysis/src/call_graph.rs:258-295` already preserves
  resolved, inferred, ambiguous, and unresolved coverage and continuations.
- `crates/compass-analysis/tests/universal_call_graph.rs` is the closest test
  pattern for graph-only, Program-IR-enriched, high-fanout, ambiguity, and
  latency behavior.
- `crates/compass-output/src/callflow_model.rs` is presentation-only; do not
  move flow semantics into it.
- Route nodes/`routes_to`, registrations, exported handlers, `main`-like
  declarations, Program IR effects, and zero-incoming callables have different
  evidence strength. The response must say which rule nominated each root.

## Target contract

Add strict schema `compass.execution_flows/1` in `compass-analysis`:

```text
ExecutionFlowRequest
  roots? / entry kinds / direction
  max_depth / max_flows / max_paths_per_entry
  max_nodes / max_edges / max_expansions / timeout_ms
  minimum_confidence / include_unresolved

ExecutionFlowResponse
  entries[] with nomination evidence
  flows[] with ordered steps and weakest confidence
  score { criticality, factors[] }
  coverage / omissions / continuations / truncated
```

Initial entry nomination precedence:

1. exact framework routes and registrations targeting a callable;
2. source-backed language/runtime entry declarations already represented in
   graph/Program IR (`main`, CLI command registration, exported handler);
3. explicit caller-supplied roots;
4. zero-incoming callables as heuristic candidates, always labeled inferred.

Criticality is an explainable, versioned tuple rather than a hidden float:
entry evidence rank, weakest edge confidence, reachable affected count,
cross-community boundaries, bridge participation, external I/O/effect sink,
and test reachability. Stable node/edge IDs break ties. Unknown coverage never
earns a positive factor.

## Commands executors will need

| Purpose | Command | Expected result |
| --- | --- | --- |
| Target preflight | `test -d /Volumes/Workspace && mkdir -p /Volumes/Workspace/crabbuild-target/compass-main && test -w /Volumes/Workspace/crabbuild-target/compass-main` | exit 0 |
| Analysis tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-analysis --locked` | pass |
| Query/output tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query -p compass-output --locked` | pass |
| CLI/MCP tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-cli -p compass-mcp --locked` | pass |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-analysis -p compass-query -p compass-output -p compass-cli -p compass-mcp --all-targets --locked -- -D warnings` | exit 0 |
| Format/qualification | `cargo fmt --all -- --check && ./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |

## Scope

**In scope**:

- entry-point candidates, bounded path search, criticality factors, typed
  response, renderers, CLI and MCP adapters, and qualification fixtures;
- graph-only operation with optional Program IR enrichment;
- exact anchors, direction, confidence, omissions, and continuation metadata.

**Out of scope**:

- dynamic tracing, profiling, coverage collection, or claims of runtime order;
- model/provider scoring;
- all-simple-path enumeration or an unbounded “complete flows” mode;
- changing call-edge identity or silently dropping unresolved calls;
- language-specific entry heuristics without direct fixtures and activation.

## Phase 1: Add typed entry-point candidates

**Context**: Root discovery must be useful on graph-only builds and improve
when Program IR is available. It cannot infer “entry” from a familiar name
alone without recording that heuristic.

**Deliverables**:

1. Add `EntryPointCandidate`, `EntryKind`, `EntryEvidence`, and
   `EntryDiscoveryCoverage` to `compass-analysis` under an internal versioned
   profile.
2. Build deterministic indexes for incoming `calls`, exact `routes_to`,
   registrations, declarations, communities, and available Program IR
   functions/effects.
3. Implement the precedence above and deduplicate candidates by stable callable
   identity while retaining every nomination reason.
4. Add explicit candidate/node/edge/byte/time bounds and typed diagnostics.
5. Test web routes, CLI registrations, `main`, duplicate labels, ambiguous
   route target, zero-incoming fallback, cycles, partial graphs, no entries,
   and 100k-node bounded behavior.

**Acceptance criteria**:

- exact route/registration targets rank above name-based candidates;
- ambiguity creates candidate evidence but never an exact entry;
- graph-only and enriched results disclose their evidence layer;
- repeated input returns identical candidate order and diagnostics;
- no-entry, partial, and limit results are distinguishable;
- analysis tests and Clippy pass.

## Phase 2: Enumerate bounded flows and explain criticality

**Context**: Flow search must stop early and retain the best candidates without
materializing the combinatorial path set.

**Deliverables**:

1. Implement a bounded best-first frontier over directed call edges. Frontier
   state includes path IDs, weakest confidence, visited-set digest, score tuple,
   depth, and stable tie key.
2. Emit a flow at a known effect/external sink, a leaf, a cycle boundary, an
   unresolved call, or the requested depth. Record why it terminated.
3. Reject repeated nodes within one path except to emit one explicit cycle
   terminator. Retain multiedge occurrence/provenance without creating duplicate
   semantic paths.
4. Calculate versioned factors from facts already indexed. Sampled bridge
   metrics must disclose sampling; if not available, omit the factor.
5. Return deterministic continuations for pruned frontier states and exact
   omission counts when knowable.

**Acceptance criteria**:

- output never exceeds max flows, paths per entry, nodes, edges, expansions,
  response bytes, or timeout;
- exact high-criticality paths rank before equally shaped inferred paths;
- stable IDs resolve every tie; hash/filesystem iteration never affects order;
- cycles, multiedges, ambiguous hops, unresolved sinks, and partial Program IR
  have explicit fixtures;
- a high-fanout/deep graph stays within the checked latency/memory ceiling;
- analysis tests and Clippy pass.

## Phase 3: Expose one CLI/MCP/query operation and renderers

**Context**: Adapters consume the analysis result. They do not rediscover
entries or recalculate scores.

**Deliverables**:

1. Add `compass flow [ROOT] [--entry-points] [--depth N] [--max-flows N]
   [--format text|json] [--graph PATH|--at REV]` with conservative defaults.
2. Add text/JSON renderers in `compass-output` showing entry evidence, each
   ordered hop, source/relationship anchors, weakest confidence, factors,
   termination, coverage, and truncation.
3. Add an MCP `execution_flows` tool with the same domain request and existing
   structured response/transport bounds.
4. If the query engine supplies graph loading/index caches, reuse its public
   backend-neutral seam; JSON/store engines must produce equivalent flow
   semantics and ordering.
5. Add CLI and MCP contract tests for roots, automatic entries, historical
   selection, ambiguity, invalid limits, pagination/continuation, JSON schema,
   text budgets, missing Program IR, and transport overflow.

**Acceptance criteria**:

- CLI JSON and MCP structured content serialize the same domain result;
- text output never hides weakest confidence or truncation;
- `--at` binds the flow to the selected immutable realization;
- JSON/store backends are semantically/order equivalent;
- no adapter adds an unbounded option or recalculates criticality;
- targeted tests, format, and Clippy pass.

## Phase 4: Qualify entry and flow accuracy

**Context**: A ranked path feature needs a reviewed oracle, not only synthetic
unit assertions, before it can support “critical” product claims.

**Deliverables**:

1. Create a small reviewed corpus spanning route-heavy web apps, CLIs,
   workers/jobs, libraries with no true entry, cycles, async/event boundaries,
   multiple languages, and partial Program IR.
2. Record acceptable entry identities and ordered/graded flow judgments with
   direction, required hops, forbidden hops, confidence, and termination.
3. Add metrics: entry Success@k, path precision/recall for required hops,
   deterministic repeatability, bounded work, backend parity, and latency.
4. Gate public “critical execution flow” wording on reviewed thresholds.
   Otherwise ship as “ranked structural flows” with limitations.
5. Update command/concept/cookbook docs, roadmap, compatibility, changelog,
   performance evidence, and MCP reference.

**Acceptance criteria**:

- exact route/registration entries reach 100% Success@1 on the reviewed corpus;
- no forbidden-direction hop appears;
- every judged result is byte-stable across repeated runs and backend-equivalent;
- thresholds, corpus provenance, and failures are checked in and reviewable;
- applicable baseline and qualification gates pass or are reported.

## Done criteria

- [ ] All four phases meet acceptance criteria.
- [ ] Entry evidence, flow confidence, factors, limits, and omissions are typed.
- [ ] Search is bounded best-first, not all-path enumeration.
- [ ] Graph-only behavior is useful and Program IR enrichment remains optional.
- [ ] CLI/MCP/backends return equivalent domain semantics.
- [ ] Qualification justifies the exact public wording.
- [ ] `advisor-plans/README.md` marks this plan DONE.

## STOP conditions

Stop if ranking requires a provider/model, if entry discovery can only work by
unqualified label matching, if a proposed score treats unknown test/Program IR
coverage as negative evidence, or if continuation cannot preserve bounded
deterministic semantics. Stop if implementing the public command would require
duplicating graph loading or query indexes in the CLI.

## Maintenance notes

Entry nomination and criticality profiles are compatibility-sensitive even if
the graph schema is unchanged. Version them and include them in qualification
digests. When new edge families are admitted, review direction, confidence,
sink semantics, and path explosion before enabling them by default.
