# Plan 005: Add personalized graph ranking and bounded best-first traversal

> **Executor instructions**: Implement only after Plans 001, 003, and 004 are
> DONE. Graph authority is a reranking feature, never a replacement for exact
> or topical evidence. Preserve direction, multiplicity, provenance, and every
> declared work bound. Update `plans/README.md` when complete.
>
> **Drift check (run first)**:
> `git diff --stat 43bceb6e..HEAD -- crates/compass-graph/src/{analyze.rs,cluster.rs,snapshot.rs} crates/compass-query/src/{ranking.rs,traversal.rs,code_query.rs,intent.rs,lib.rs} crates/compass-query/tests PERFORMANCE.md`

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: Plans 001, 003, and 004
- **Category**: performance / correctness / direction
- **Planned at**: commit `43bceb6e`, 2026-08-06

## Why this matters

Compass currently uses graph degree mainly as a tie-breaker and expands natural
results with unweighted BFS/DFS. It cannot propagate topical authority like a
search engine, distinguish a trusted call from weak inferred documentation, or
prefer a meaningful cross-community route over arbitrary short hops. This phase
adds deterministic personalized graph relevance and replaces blind traversal
with a bounded, explainable best-first frontier.

## Current state

- `crates/compass-query/src/score.rs:150-166` uses source-backed degree after
  lexical score, source backing, semantic kind, and test/generated status.
- `crates/compass-query/src/traversal.rs:456-515` expands every successor until
  depth; non-seed hubs above a percentile threshold are stopped entirely.
- `crates/compass-query/src/traversal.rs:548-555` orders non-seed output by
  source backing, raw degree, then ID.
- `crates/compass-query/src/traversal.rs:765-791` finds paths with undirected,
  unweighted BFS.
- `crates/compass-query/src/code_query.rs:943-1013` also uses undirected BFS;
  evidence quality orders adjacent edges but does not affect path cost.
- `crates/compass-graph/src/analyze.rs:140-168` already recognizes high-degree
  nodes, and lines 244-299 calculate betweenness for bridge questions.
- `crates/compass-graph/src/cluster.rs:680-721` builds deterministic weighted
  community graphs, but `set_edge` at lines 769-787 makes adjacency symmetric.
  That representation must not be reused unchanged for directional query
  ranking.

Current traversal excerpt:

```rust
// crates/compass-query/src/traversal.rs:469-477
if !seeds.contains(&node) && graph.degree(node) >= threshold {
    continue;
}
for neighbor in graph.successors(node) {
    if !visited.contains(&neighbor) {
        next.insert(neighbor);
        edges.push((node, neighbor));
    }
}
```

## Design

### Typed directional query graph

Build a query-time or immutable derived projection that preserves:

- source, target, edge ID, and `EdgeKind`;
- parallel occurrence multiplicity;
- evidence origin, confidence, and resolution;
- source-backed versus placeholder endpoints;
- node kind/roles, test/generated status, community, and normalized degree;
- graph/ranking/profile fingerprints.

Do not reuse the undirected Louvain `WeightedGraph`. Store directed outgoing
mass and explicit reverse traversal penalties.

### Intent-specific relation profiles

Define versioned profiles with rational/integer weights and canonical defaults:

```text
CallFlow:      calls, routes_to, handles, triggers, schedules
Impact:        reverse calls/imports/depends_on/references/reads/writes/events
Ownership:     contains, registers, implements, maps_to
DataFlow:      reads, writes, produces, consumes, publishes, subscribes
Architecture:  imports, depends_on, contains, routes_to, cross-community bridges
General:       conservative union with lower structural boost
```

Each profile specifies allowed direction, reverse-edge cost, relation weight,
hop decay, restart probability, provenance multiplier, hub normalization, and
maximum iterations/epsilon. These numbers are heuristic rank features, never
probabilities or graph facts. Tune only through Plan 001 judgments.

### Personalized PageRank / random walk with restart

Use lexical/intent seed scores as normalized restart mass. Required invariants:

- exact identity tier remains fixed above graph contributions;
- no seed means no personalized propagation;
- outgoing mass is normalized, including parallel edges according to explicit
  multiplicity semantics;
- trusted/exact evidence weighs more than inferred, ambiguous, or unresolved;
- generic high-degree nodes receive degree normalization, not a binary ban;
- dangling mass returns to restart seeds deterministically;
- iteration order follows canonical node/edge IDs;
- stop at `max_iterations`, convergence epsilon, node/edge work budget, or
  elapsed deadline and disclose which bound fired;
- stable arithmetic/tie behavior is qualified across supported platforms.

A small precomputed global authority prior may be added to break non-topical
ties, but it cannot admit a candidate that lacks lexical, intent, path, or
community evidence. Personalized score is the primary graph feature.

### Best-first expansion

Replace natural BFS/DFS internals with a priority frontier:

```text
priority = seed_relevance
         * relation_profile_weight
         * provenance_weight
         * hop_decay
         * personalized_graph_score
         * hub_normalization
```

Use an explicit deterministic priority tuple rather than relying only on a
floating total. Final tie-breaks are hop count, edge evidence quality, relation
kind, target ID, then edge ID.

Enforce aggregate limits before work:

- seeds, candidates, nodes, edges, paths, expansions;
- per-hop expansion and per-hub edge quota;
- frontier size, decoded bytes, response bytes, and elapsed time;
- maximum communities and a small cross-community bridge quota.

When a hub is encountered, retain the best relation-compatible edges under a
quota instead of expanding all or dropping the hub entirely.

### Diversity

After relevance ranking, use deterministic maximal marginal relevance (MMR) or
quotas to avoid redundant results. Diversity dimensions: source file,
community, node kind/role, and qualified-name family. Never diversify the first
exact answer away. Preserve a small bridge budget for architecture questions.

### Weighted paths

Add bounded Dijkstra/A* (zero heuristic is acceptable) and optionally Yen-style
top-k paths after the first implementation is stable. Edge cost includes:

- inverse relation-profile weight;
- reverse-direction penalty;
- inferred/ambiguous/unresolved penalty;
- generic hub penalty;
- hop and cross-community transition cost.

Return the chosen profile, total/component costs, weakest evidence, direction
reversals, and close alternative paths. Path limits must count nodes, edges,
frontier entries, decoded bytes, and time.

### Explanations

Extend internal `RankExplanation` with:

```text
graph_profile
restart_seed_contributions[]
personalized_score
global_prior (if present)
intent_relation_fit
provenance_adjustment
hub_adjustment
community_diversity_adjustment
selected_path_evidence[]
graph_bounds_hit[]
```

Keep source content out of the explanation; IDs, enums, ranges, and short
sanitized labels are sufficient.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Graph tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-graph --locked` | all pass |
| Query tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked` | all pass |
| Scale | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test code_query_scale --locked` | under existing ceiling; work limits pass |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-graph -p compass-query --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Relevance | Plan 001 qualification command | graph/path slices improve or hold |
| Fixtures | `./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |

## Scope

**In scope**:

- `crates/compass-graph/src/query_rank.rs` (create only if immutable derived
  topology statistics belong at graph ownership boundary)
- `crates/compass-graph/src/snapshot.rs`
- `crates/compass-graph/src/lib.rs`
- `crates/compass-query/src/graph_rank.rs` (create)
- `crates/compass-query/src/ranking.rs`
- `crates/compass-query/src/intent.rs`
- `crates/compass-query/src/traversal.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/lib.rs`
- `crates/compass-query/tests/graph_ranking.rs` (create)
- `crates/compass-query/tests/code_traversal.rs`
- `crates/compass-query/tests/code_explore.rs`
- `crates/compass-query/tests/code_query_scale.rs`
- `crates/compass-query/tests/relevance_qualification.rs`
- `PERFORMANCE.md`
- `docs/implementation/query-engine.md`

**Out of scope**:

- changing extraction edge meaning, community construction semantics, public
  v2 serialization, CLI/MCP migration, mandatory embeddings/models, remote
  services, or mutation queries;
- global PageRank as primary retrieval;
- removing legacy BFS/DFS public switches before Plan 006 compatibility work;
- using undirected community adjacency for directional query propagation.

## Git workflow

- Branch: `advisor/005-personalized-graph-ranking`
- Suggested commits: directional projection/profiles; personalized propagation;
  best-first expansion/diversity; weighted paths; qualification/performance.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Add relation-profile and bounded graph projection tests

Define versioned profiles and a tiny fixture for every relation family,
direction, evidence quality, parallel edge, hub, dangling node, community, and
test/generated role. Assert canonical projection ordering and exact work-limit
accounting.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query graph_rank::projection --locked`
→ projection and profile goldens pass.

### Step 2: Implement personalized propagation

Implement random walk with restart over a bounded directed subgraph retrieved
from top hybrid seeds. Add convergence, iteration, frontier, node, edge, byte,
and deadline limits. Return partial results only where the typed contract marks
them truncated; otherwise return a typed limit error.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query graph_rank::pagerank --locked`
→ hand-computed chains, hubs, cycles, parallel edges, dangling nodes, evidence
weights, and repeated runs match expected ranks.

### Step 3: Integrate graph reranking behind a profile flag

Add graph score only within the non-exact relevance tier. Select the profile
from Plan 004 intent. Initially keep a feature/config switch internal so
qualification can compare lexical-only and hybrid ranks. Record full component
explanations and work counts.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test graph_ranking rerank --locked`
→ exact answers stay first, topical authorities rise, and unrelated global hubs
cannot enter the candidate set.

### Step 4: Replace traversal internals with bounded best-first expansion

Keep legacy BFS/DFS adapter names temporarily, but execute the new frontier
when an intent profile is present. Add hub quotas, hop decay, community/file
diversity, bridge quota, and typed bound diagnostics. Pagination must operate on
the deterministic selected result, not change traversal semantics page by page.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test code_traversal --test code_explore --locked`
→ selection is deterministic, bounded, direction-aware, and pagination pages
cover one stable result set without duplication/loss.

### Step 5: Implement weighted semantic paths

Add relation-profile path costs and bounded weighted search. Preserve stored
arrows in rendering. Return the old shortest-hop result through a compatibility
adapter until Plan 006. Include a case where a longer trusted call path beats a
short containment/documentation shortcut.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query weighted_path --locked`
→ kind, direction, evidence, hub, alternate-path, and limit fixtures pass.

### Step 6: Tune only against the judged corpus

Evaluate lexical-only versus graph hybrid by intent slice. Require exact
Success@1 unchanged, overall nDCG@10 improved/held, architecture/path slices
improved, and no unwaived slice regression over two points. Record the selected
profile version and rationale in `PERFORMANCE.md`.

**Verify**:
Plan 001 qualification command plus `code_query_scale` → relevance thresholds,
operation-count bounds, and existing elapsed ceiling pass.

## Test plan

- Unit tests for profile validation, normalization, convergence, dangling mass,
  cycles, parallel edges, direction, evidence, hubs, and deterministic ties.
- Integration tests for callers, impact, data flow, architecture, general
  search, best-first expansion, community diversity, and weighted paths.
- Regression proving global hubs do not outrank exact/topical nodes.
- Pagination regression proving page selection does not rerun with a different
  frontier.
- Scale tests for candidate subgraph size, iterations, expansions, frontier,
  decoded bytes, response bytes, and elapsed ceiling.
- Cross-platform repeated-run score/output fixtures where floating arithmetic
  is used.

## Done criteria

- [ ] Personalized graph relevance uses typed direction, relation, and
  provenance profiles seeded from topical candidates.
- [ ] Exact tiers cannot be displaced by graph scores.
- [ ] Best-first traversal is bounded before expansion and replaces binary hub
  suppression with deterministic quotas.
- [ ] Result diversity covers files/communities/kinds without displacing exact
  answers.
- [ ] Weighted paths prefer intent-compatible trusted evidence and explain
  costs/direction.
- [ ] Relevance and scale gates pass with full JSON/store determinism.
- [ ] All targeted tests, Clippy, format, fixtures, and performance checks pass.

## STOP conditions

Stop and report if:

- PageRank must scan/materialize the complete graph per request to meet the
  proposed behavior;
- global authority admits candidates with no topical or intent evidence;
- the implementation loses edge direction, multiplicity, or occurrence sites;
- cross-platform numeric drift changes ordered results;
- a hub limit silently drops all valid paths rather than reporting truncation;
- pagination user changes are overwritten or page number changes ranking;
- `/Volumes/Workspace` is unavailable.

## Maintenance notes

- Profile weights, convergence, hub normalization, and diversity policy are
  versioned ranking semantics. Change them only with corpus evidence.
- Reassess profiles when new `EdgeKind` values or framework evidence are
  introduced; missing relations should fail validation, not receive implicit
  defaults.
- Precomputed priors are derived artifacts keyed by immutable graph digest and
  profile version. They are safe to rebuild and never historical graph facts.
