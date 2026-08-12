# Plan 004: Compile natural language into typed node and edge intents

> **Executor instructions**: Implement only after Plans 001–003 are DONE. The
> planner must be inspectable, bounded, deterministic, and credential-free.
> Never execute raw model text or silently select an ambiguous entity. Run each
> gate and update `plans/README.md` when complete.
>
> **Drift check (run first)**:
> `git diff --stat 43bceb6e..HEAD -- crates/compass-model/src/{query_contract.rs,code_graph.rs} crates/compass-query/src/{text.rs,traversal.rs,code_query.rs,lib.rs} crates/compass-query/tests crates/compass-mcp crates/compass-cli docs`

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: Plans 001 and 003
- **Category**: correctness / architecture / direction
- **Planned at**: commit `43bceb6e`, 2026-08-06

## Why this matters

Natural questions currently select lexical seeds and always run the same
outgoing BFS/DFS neighborhood. Keyword inference only filters six edge-context
groups. As a result, “who calls authorize?” can walk toward callees, while
impact, path, ownership, architecture, and relationship questions are not
routed to the typed operations Compass already implements. This phase adds a
typed planning boundary and makes relationships searchable without first
guessing one node.

## Current state

- `crates/compass-query/src/traversal.rs:70-106` analyzes terms, selects seeds,
  infers/normalizes context, then invokes BFS or DFS for every natural question.
- `crates/compass-query/src/text.rs:257-305` recognizes only call, import,
  field, parameter, return, and generic context hints.
- `crates/compass-query/src/traversal.rs:456-515` traverses successors only for
  natural BFS/DFS.
- `crates/compass-model/src/query_contract.rs:10-19` already distinguishes
  `Search`, `Callers`, `Callees`, `Impact`, `Explore`, and `NodeTrail`.
- `crates/compass-model/src/code_graph.rs:194-227` defines a closed typed edge
  vocabulary including calls, imports, routes, reads, writes, handles,
  publishes, subscribes, schedules, triggers, depends-on, and maps-to.
- `crates/compass-mcp/src/lib.rs:383-423` exposes typed tools beside a separate
  text-only `query_graph` tool.
- The public `--context` documentation describes subsystem anchoring, while
  `Graph::with_edge_contexts` applies exact edge-context filtering. Do not
  silently reinterpret the existing option.

Current operation vocabulary:

```rust
// crates/compass-model/src/query_contract.rs:12-19
pub enum CodeQueryOperation {
    Search, Callers, Callees, Impact, Explore, NodeTrail,
}
```

## Design

### Internal typed plan

Add an internal `QueryPlan` now; Plan 006 publishes its v2 representation:

```text
QueryIntent
  FindNodes
  FindEdges
  IncomingRelations
  OutgoingRelations
  Impact
  PathBetween
  Explain
  ExploreNeighborhood
  ExploreArchitecture

QueryPlan
  planner_version
  intent
  entity_slots[]
    role: subject | source | target | scope
    original_text
    candidate IDs with rank evidence
    resolution: exact | ranked | ambiguous | unresolved
  concepts[]
  node_kinds[]
  node_roles[]
  edge_kinds[]
  direction: incoming | outgoing | both
  scope[]
    path | qualified_prefix | community | framework | language
  limits
  confidence: high | medium | low
  reasons[]
  alternatives[]
  fallback: execute | broad_search | require_disambiguation
```

`confidence` is a rule category, not a calibrated probability. Sort all enums,
IDs, and alternatives canonically. Bound question bytes, patterns tried,
entities, concepts, alternatives, and plan bytes.

### Deterministic planner rules

Use an ordered rule table, not scattered `if` statements. Higher-specificity
patterns win; ties produce alternatives. Initial rules:

| Wording | Intent | Direction/relations |
| --- | --- | --- |
| “who/what calls X”, “callers of X” | IncomingRelations | incoming calls/routes/triggers |
| “what does X call/use/invoke” | OutgoingRelations | outgoing calls/uses-family |
| “what breaks/changes/is affected if X” | Impact | incoming impact profile |
| “path/how does X reach/connect to Y” | PathBetween | source to target, typed profile |
| “explain/show details/evidence for X” | Explain | incident trusted evidence |
| “what reads/writes/publishes/subscribes Y” | FindEdges | explicit kind and direction |
| “where does X enter/start/handle requests” | ExploreArchitecture | routes/handles/registers |
| simple noun/symbol phrase | FindNodes | hybrid search |

The vocabulary maps user verbs to the closed `EdgeKind` enum. Never invent an
unknown edge kind. Preserve direction and allow an explicit typed request to
override natural inference.

### Entity resolution

Resolve slot text with Plan 003's shared retriever. Policy:

- exact ID or unique exact qualified name executes directly;
- close same-tier matches remain an ambiguous set;
- low-confidence fuzzy entities may support broad exploration but cannot drive
  destructive or externally mutating behavior (queries are read-only today);
- path/impact/caller operations that require unique endpoints return
  `AmbiguousMatch` or `NoMatch`, with retry candidates;
- architecture/concept exploration may retain several diversified seeds.

### Scope versus relation context

Create separate internal fields:

- `scope`: subsystem/module/path/community/framework/language constraints or
  boosts over nodes;
- `edge_context`: exact relationship occurrence-context filtering.

Keep existing `--context` behavior unchanged until Plan 006 defines an explicit
compatibility migration, likely retaining it as `--edge-context` alias while
adding `--scope`.

### First-class edge retrieval

Add `EdgeSearchRequest` and internal `EdgeHit` candidates. Index a bounded
projection for each edge:

```text
edge ID, kind, source ID/name/qualified name, target ID/name/qualified name,
directional role, relationship source path/range, context, framework,
evidence origin/confidence/resolution, source and target community
```

Exact endpoint and kind evidence precede descriptive matching. Preserve
parallel edges and occurrence sites. Return an occurrence-level edge when the
question names a concrete write/call/route; do not collapse multiplicity.
Unresolved or ambiguous evidence remains visible and lower-ranked unless
explicitly requested.

### Optional planner extension point

Define a provider-neutral trait only after the deterministic plan is stable:

```text
IntentPlanner::plan(question, bounded_catalog) -> UntrustedPlanProposal
```

This phase does **not** implement a network/model provider. A future provider
may propose only the same enums and bounded slots. Compass validates every
field, resolves entities itself, records planner provenance, and never executes
raw generated CompassQL or treats repository text as instructions.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Model tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-model --locked` | all pass |
| Query tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked` | all pass |
| MCP tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-mcp --locked` | all pass |
| Query lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-model -p compass-query -p compass-mcp --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Relevance | Plan 001 qualification command | intent/edge thresholds pass |
| Fixtures | `./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |

## Scope

**In scope**:

- `crates/compass-model/src/query_contract.rs` for internal/additive types that
  do not mutate v1 serialization
- `crates/compass-query/src/intent.rs` (create)
- `crates/compass-query/src/edge_search.rs` (create)
- `crates/compass-query/src/text.rs`
- `crates/compass-query/src/retrieval.rs`
- `crates/compass-query/src/ranking.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/traversal.rs`
- `crates/compass-query/src/lib.rs`
- `crates/compass-query/tests/intent_planner.rs` (create)
- `crates/compass-query/tests/edge_search.rs` (create)
- `crates/compass-query/tests/relevance_qualification.rs`
- `crates/compass-query/tests/store_engine.rs`
- MCP internals/tests only as required to exercise the planner without changing
  the public tool schema yet
- `docs/implementation/query-engine.md`

**Out of scope**:

- public `compass.query/2` fields, CLI flags, MCP tool schema migration, global
  PageRank, best-first traversal, embeddings, network/model providers, graph
  extraction, or CompassQL grammar changes;
- silently changing current `--context`, pagination, v1 requests, or v1
  responses;
- executing arbitrary query text supplied by a provider.

## Git workflow

- Branch: `advisor/004-intent-edge-query`
- Suggested commits: intent types/rules; entity resolution; edge index/search;
  internal natural-query routing; qualification.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Characterize natural-intent failures

Add tests for at least callers, callees, impact, path, edge write/read, route
entry, explain, architecture, ambiguous endpoint, unknown intent, and no-match.
Assert the intended typed plan, not only output substrings. Include the known
directional regression: “who calls authorize?” must plan incoming calls.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test intent_planner characterization --locked`
→ tests document current gaps before planner routing and pass after Step 4.

### Step 2: Implement versioned rule planning

Create ordered rules, bounded token/phrase matching, slot extraction, enum
mapping, reasons, alternatives, and fallback. Keep rule data in one auditable
table. Add property-style tests for punctuation/case invariance, rule order,
bounded alternatives, and deterministic output.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query intent::tests --locked`
→ all intent and bound tests pass.

### Step 3: Resolve slots and scope through shared retrieval

Connect entity slots to Plan 003 candidates. Implement per-intent uniqueness
requirements and calibrated ambiguity based on exact tier and score gaps from
the judged corpus. Separate node scope from edge context internally. Do not
change public CLI arguments.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test intent_planner resolution --locked`
→ exact, fuzzy, ambiguous, scoped, and unresolved slot cases pass.

### Step 4: Execute plans through typed operations

Add a `PlannedQueryExecutor` that dispatches to `search`, callers, callees,
impact, trail, explain/explore, or edge search. Reuse existing bounds and typed
diagnostics. Unknown/low-confidence plans fall back to broad hybrid search with
alternatives; they do not silently use outgoing BFS.

Keep the legacy natural text wrapper and pagination as an adapter so current
CLI behavior can be qualified before public migration in Plan 006.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test intent_planner --locked`
→ each question executes the expected typed operation and preserves pagination.

### Step 5: Build and query the edge projection

Add JSON/store-compatible edge postings and common edge ranking. Search must
filter/score kind, direction, endpoints, context, source site, and provenance;
return parallel occurrences; and honor candidate/edge/response bounds. Add
exact direction tests and full backend differential tests.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test edge_search --test store_engine --locked`
→ full ordered edge results and diagnostics match across engines.

### Step 6: Meet intent and edge qualification gates

Expand Plan 001 to at least ten reviewed questions per supported intent plus
negative and ambiguity cases. Require intent macro-F1 ≥0.90 and judged edge
kind/direction precision ≥0.95. Report per-intent metrics so a strong `FindNodes`
slice cannot hide a broken `Impact` slice.

**Verify**:
Plan 001 qualification command → thresholds pass with no unwaived slice
regression.

## Test plan

- Rule unit tests for synonyms, direction, negation, punctuation, case, missing
  entities, two-entity paths, conflicting cues, and bounds.
- Slot tests for exact/fuzzy/ambiguous/unresolved identities and scope.
- Executor integration tests for every `QueryIntent` and fallback.
- Edge tests for relation kind, direction, occurrence multiplicity, provenance,
  source anchors, heuristic inclusion, ambiguity, and truncation.
- JSON/store differential tests compare complete ordered plans/results.
- Security tests treat repository-authored imperative text as query evidence,
  never planner instructions.

## Done criteria

- [ ] Every natural question produces a bounded typed plan with reasons and
  alternatives before execution.
- [ ] Supported caller/callee/impact/path/edge questions execute with correct
  relation direction.
- [ ] Edge occurrences are searchable as first-class results with provenance
  and multiplicity.
- [ ] Scope and edge-context are distinct internally; public compatibility is
  unchanged in this phase.
- [ ] Low-confidence or ambiguous plans are explicit and deterministic.
- [ ] Intent macro-F1 and edge precision thresholds pass.
- [ ] No model/network credential is required or called.
- [ ] All targeted tests, lint, format, relevance, and fixture gates pass.

## STOP conditions

Stop and report if:

- a requested intent cannot map to the closed graph relationship vocabulary
  without inventing meaning;
- path/impact execution would need silently selecting one ambiguous entity;
- edge indexing cannot preserve parallel occurrences and source anchors;
- adding store edge search requires changing published historical snapshots in
  place;
- current pagination or `--context` user changes would be overwritten;
- a model provider becomes necessary for the baseline planner;
- `/Volumes/Workspace` is unavailable.

## Maintenance notes

- Add a planner rule only with positive, negative, ambiguity, and metric-corpus
  coverage. Rule order is observable semantics and requires a planner-version
  bump.
- New `EdgeKind` variants must update intent vocabulary and edge qualification
  in the same change.
- Keep future provider proposals untrusted, bounded, typed, and provenance
  labeled; direct model-generated CompassQL execution remains prohibited.
