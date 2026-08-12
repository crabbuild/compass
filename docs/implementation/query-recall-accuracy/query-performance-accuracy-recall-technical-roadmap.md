# Query performance, intent accuracy, and recall roadmap (Compass-local IR stack)

This is a practical implementation design for Compass natural-language and graph query quality.
It is written against the current code owners in this repo:

- `compass-cli` handles command routing (`command_natural_query`, `query` flags)
- `compass-query` handles intent parsing/ranking/execution plumbing
- `compass-query` test/qualification stack provides measurable quality gates

## 1) Objective and constraints

Goal:
Increase recall and ranking quality for code-graph queries while keeping deterministic,
local-first execution and bounded latency.

Constraints (hard):

- No Python, network model services, vector DB, or runtime grammar downloads for query ranking.
- No Graphify-style runtime dependency.
- Deterministic ordering for equivalent inputs.
- Preserve explicit ambiguity and “no answer” behavior (no invented guesses).
- Maintain backend parity between Store and JSON execution paths.
- No silent contract changes; versioned behavior changes must be explicit and reversible.

## 2) What “better” means (metrics and acceptance)

Use existing Compass relevance tooling as the scorekeeper:

- Node retrieval
  - `success_at_1`
  - `mrr_at_10`
  - `recall_at_5`
  - `recall_at_20`
  - `precision_at_10`
  - `ndcg_at_10`
- Intent behavior
  - `intent_macro_f1`
  - per-intent precision/recall/F1
- Structural correctness
  - `edge_precision`
  - `edge_recall`
  - `edge_kind_precision`/`edge_kind_recall`
  - `edge_direction_precision`/`edge_direction_recall`
  - `path_acceptance_rate`
- Robustness and quality guardrails
  - `accepted_ambiguity_recall`
  - `no_answer_precision`
  - `false_positive_rate`
- Performance
  - latency p50/p95
  - candidate/posting/node/edge expansion counts (`WorkCounts`)

Acceptance per phase:

- Recall improvements must be measured on the reviewed fixture and executable subset.
- Precision / no-answer metrics must not materially degrade without an explicit tradeoff decision.
- p95 latency budget must not regress beyond approved phase threshold.
- Store/JSON parity must remain stable at semantic level.

## 3) Current baseline architecture snapshot

### Current behavior

- CLI `compass query` always flows through `query_graph_text_page` in `compass-cli`.
- Query execution path:
  - natural question → `query_terms` → `score_nodes` → `pick_seeds` → BFS/DFS render.
- Structured query operations exist but are not automatically used for natural-language intent:
  - `search`, `callers`, `callees`, `impact`, `node_trail` are exposed via
    `CodeQueryEngine` in `crates/compass-query/src/code_query.rs`.
- Relevance tests already exist and can be expanded:
  - `crates/compass-query/tests/relevance_qualification.rs`
  - fixture `crates/compass-query/tests/fixtures/relevance/judged.json`

### Why current stack is suboptimal

- Intent is implicit and traversal-oriented; users asking for intent-specific graph operations can get generic traversal.
- Candidate retrieval is mostly one-channel style (index + strict matching).
- Ranking is effective but mixed with traversal heuristics, and not explicitly intent-aware.
- Backend-parity comparison currently risks false negatives when done at raw JSON bytes instead of semantic fields.

## 4) Target architecture (deterministic IR-style pipeline)

Implement a staged, deterministic pipeline:

1. **Intent Parse**
   - deterministically infer an intent plan from NL
   - extract symbols, relation verbs, direction, and constraints

2. **Recall Assembly**
   - deterministic multi-source candidate collection, each source with provenance tags

3. **Ranking**
   - structured feature scoring with explicit profile IDs and deterministic tie-break

4. **Execution / Materialization**
   - route intent directly to the right graph operation (`search`, `callers`, `callees`, `impact`, `node_trail`)
   - fallback to current traversal only when confidence is weak

5. **Validation**
   - record work counters and diagnostics for tuning/reproducibility
   - evaluate with harness each phase

## 5) Internal model to add

Add module in `crates/compass-query/src/intent.rs`:

- `enum QueryIntent`:
  - `Search`
  - `Callers`
  - `Callees`
  - `Impact`
  - `Path`
  - `Explain` (optional, if mapped)
  - `Unknown`
- `struct QueryIntentPlan`
  - `intent: QueryIntent`
  - `confidence: u8`
  - `symbols: Vec<String>` (normalized and raw candidates)
  - `relation_hints: Vec<String>`
  - `direction: Option<RelationDirection>`
  - `limits_hint: Option<QueryLimits>`
  - `query_class_hints: Vec<QueryClass>` (for test/analytics)
  - `parse_trace: Vec<String>` (diagnostic only, hidden behind non-contract artifact)

Add candidate provenance model:

- `CandidateSource`: `ExactId`, `ExactName`, `Alias`, `NormalizedName`, `IndexPostings`,
  `Fts`, `RelationSeed`, `PathSeed`, `Fuzzy`.
- `CandidateRecord`:
  - `node_id`
  - `source_priority: u8`
  - `source_tags: BTreeSet<CandidateSource>`
  - `per_source_rank: BTreeMap<CandidateSource, usize>`
  - `feature_hits: BTreeMap<String, f64>`

Add rank profile model:

- `query-ranker/1` (baseline)
- `query-ranker/2` (recall-first + stronger lexical)
- `query-ranker/3` (intent-aware)

## 6) Phase plan (detailed and actionable)

## Phase 0 — Baseline hardening + parity normalization (1 week)

### Why first
Guarantee that future gains are measurable and not hidden by brittle comparisons.

### Scope

1. Replace raw-byte response parity checks for Store vs JSON with semantic comparisons:
   - compare operation, limits, truncation flag, result IDs, and node/edge/path IDs deterministically.
2. Keep existing test outputs deterministic:
   - stable sort and dedupe on all exported node/edge/path collections before assertions.
3. Add canonical ranking diagnostics for each query phase:
   - number of candidates generated, truncated reasons, ranking profile id.
4. Baseline benchmark capture:
   - store current result artifact for a known corpus and executable subset.

### Deliverables

- parity assertions that survive stable canonicalization differences without requiring exact output byte order
- baseline report with explicit `query-ranker/0` profile tag
- no unbounded or ambiguous test expectations

### Exit criteria

- existing quality gates pass after comparison logic fix
- no regression in backend determinism

## Phase 1 — Deterministic intent parser (1–2 weeks)

### Scope

1. Add deterministic parser in `crates/compass-query/src/intent.rs`:
   - verb maps (`who calls`, `called by`, `path from`, `impact`, `what does X use`, etc.)
   - direction tokens (`incoming`, `outgoing`, `to`, `from`)
   - symbol extraction for qualified names (`A::b`, `pkg.mod.fn`, `Class.method`)
2. Add confidence scoring:
   - explicit verb hit: +30
   - symbol confidence: +20
   - directional cues: +15
   - multi-token query intent coherence: +10
   - ambiguous/conflicting signals reduce confidence
3. Route only when confidence threshold reached (example: ≥ 65):
   - else keep existing traversal flow.

### Files

- add `crates/compass-query/src/intent.rs`
- export parser in `crates/compass-query/src/lib.rs`
- integrate call site in `compass-query` execution entry or `compass-cli::command_natural_query`

### Tests

- unit tests for positive/negative intent cases and threshold boundaries
- ambiguity fixture additions in `relevance/judged.json` for mixed-intent statements

### Exit criteria

- measurable lift in `intent_macro_f1` on intent-labeled fixture slices
- no spike in `no_answer_precision` / false positives

## Phase 2 — Recall multiplexer (2–3 weeks)

### Scope

Replace single-source retrieval with bounded, deterministic multi-source recall:

1. **Exact/ID channel**
   - `resolve_symbol` by ID and exact normalized name
2. **Lexical/name channel**
   - normalized-name lookup and label token matches
3. **Postings channel**
   - Store: `nodes_for_terms`
   - JSON path: materialized query index / FTS match
4. **Relation seed channel**
   - for callers/callees/path plans, include relation-specific seeds
5. **Fuzzy/typo channel**
   - gated by recall starvation threshold and confidence boundary

### Recall budget policy (all hard limits)

- `max_candidates_total`
- `max_per_source`
- `max_relation_seed`
- `max_postings_per_term`
- `max_fuzzy_candidates`
- explicit truncation reasons emitted to diagnostics/counters

### Deterministic dedupe and ordering

1. dedupe by canonical node ID immediately
2. stable source-priority ordering
3. deterministic tie-break inside each source by stable node ID

### Exit criteria

- `recall_at_20` improved without uncontrolled precision loss
- clear truncation telemetry per query
- no hidden fallback behavior

## Phase 3 — Ranking model upgrade (2–3 weeks)

### Scope

1. Refactor ranking to explicit feature vector (instead of only implicit tuple/weighted ad-hoc behavior):
   - exact / prefix / substring lexical score
   - token coverage
   - exact-id and qualified-name alignment
   - evidence strength (`confidence`: exact vs inferred)
   - intent fit (path/caller/callee/direction)
   - structural utility (`source_backed_degree`) with bounded anti-hub handling
   - path-readiness features
2. Deterministic tie-break policy in this order:
   - score
   - source-backed flag
   - semantic rank class
   - test/placeholder penalty
   - degree
   - label length
   - stable node ID
3. Tag each result evaluation with active `ranker_profile_id`.

### Files

- `crates/compass-query/src/score.rs` (feature vector + profile switch)
- optional helper adapters in `crates/compass-query/src/code_query.rs` for operation-specific scoring

### Exit criteria

- measurable lift in `precision_at_10`, `nDCG@10`
- deterministic output across runs for same query and plan

## Phase 4 — Intent-aware execution routing (1–2 weeks)

### Scope

1. In natural-language flow (`command_natural_query`), route by high-confidence plan:
   - `Callers` → `CodeQueryEngine::callers`
   - `Callees` → `CodeQueryEngine::callees`
   - `Impact` → `CodeQueryEngine::impact`
   - `Path` → `node_trail` or path-capable operation
   - `Search`/fallback → current ranked/traversal flow
2. Add operation metadata in diagnostics only (no contract break).
3. Preserve explicit fallback for ambiguous/no-match cases.

### Exit criteria

- directioned edge recall and path acceptance improve in fixture slices
- no regression in generic search intent coverage

## Phase 5 — Edge and path quality pass (2 weeks)

### Scope

1. Add operation-specific edge scoring:
   - direction match bonus
   - relation kind match bonus
   - evidence confidence
2. Add path scoring policy:
   - endpoint alignment strength
   - relation-sequence plausibility
   - path length and hop penalties
3. For path-like queries, rank competing paths before materialization truncation.

### Exit criteria

- `edge_direction_precision` improves
- `path_acceptance_rate` and mean accepted path rank improve

## Phase 6 — Fuzzy and typo recovery (1–2 weeks)

### Scope

1. Trigger only when:
   - plan confidence is low or recall count below a threshold, and
   - query token length conditions are met.
2. Generate spelling variants from bounded symbol vocabulary:
   - edit-distance caps (default 1, optionally 2 for long tokens)
   - max variants per token
   - global candidate cap for fuzzy channel
3. Apply strong source penalty for fuzzy candidates in ranking.
4. Keep fuzzy behavior explicit and reproducible.

### Guardrails

- No one/two-character tokens
- no unbounded generation
- no ranking domination by fuzzy matches

### Exit criteria

- typo/noise recall lift on synthetic noisy-class fixtures
- stable/noise metrics not materially worse

## Phase 7 — Performance hardening and caching (2 weeks)

### Scope

1. Add bounded in-process caches:
   - normalized-plan cache keyed by `(graph_digest, intent_plan_signature, operation, limits)`
   - optional small candidate cache keyed by normalized query + profile
2. Add per-stage timing counters:
   - parse_ms, recall_ms, rank_ms, execute_ms
   - candidates_read, postings_decoded, nodes_expanded, edges_expanded
3. Add early-stop caps:
   - if budget hit, stop expansion deterministically and record truncation reason

### Exit criteria

- p50/p95 within agreed release budget after cache warm-up
- no growth beyond cap thresholds

### Phase 8 — Rollout and governance (1 week)

1. Introduce shadow mode:
   - run old + new pipeline in parallel for a controlled query set
   - compare top-k IDs, not raw JSON bytes
2. Roll out with feature profile:
   - default profile remains previous unless tests approve
   - bump profile config only after acceptance
3. Maintain rollback:
   - pin profile in one env or internal setting

## 7) Validation matrix by phase

Run these every phase where behavior changes:

- `cargo test -p compass-query --test relevance_qualification --locked`
- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked`
- `python3 scripts/qualify_query_relevance.py`
- if CLI behavior changes: `cargo test -p compass-cli --test compass_product --locked`

For ranking/path-only changes:

- run executable subset (small reviewed graph) nightly
- compare report deltas:
  - recall/precision/intent metrics
  - p95 latency
  - parity mismatch count (store/json)

## 8) Data/implementation ownership map

### `compass-query`

- `src/intent.rs` (new): intent parser and plan model
- `src/score.rs`: feature vectors + ranker profiles
- `src/code_query.rs`: structured execution dispatch + candidate provenance hooks
- `src/text.rs`: symbol/token utility and diacritic normalization reuse
- `src/traversal.rs`: fallback behavior and seed behavior unchanged unless explicitly needed
- `tests/relevance_qualification.rs`: phase-specific assertions and canonical diff helpers
- `tests/fixtures/relevance/judged.json`: add intent-only, typo/near-miss, no-answer negative sets

### `compass-cli`

- `src/lib.rs`: decide natural-query route by parsed intent and preserve current fallback behavior
- `src/query_commands.rs`: keep command compatibility unchanged (`--cql`, `--help`, flags)

## 9) Deliverables at each release gate

### Gate A (after Phase 2)

- intent parser enabled
- recall assembly implemented
- canonical parity checks fixed
- metrics: non-decreasing recall@20 for reviewed intent subset

### Gate B (after Phase 3)

- ranker profile v2 in effect
- measurable lift in nDCG@10 and precision@10
- deterministic tie-break tests added

### Gate C (after Phase 4)

- intent-aware routing for callers/callees/impact/path
- edge-direction/path acceptance improved

### Gate D (final)

- fuzzy recovery enabled with caps and provenance penalties
- performance tuning with bounded caches
- rollout profile and rollback procedure documented

## 10) Risks and mitigations

- **Over-recall adds noise**: use strict confidence gates, caps, and ambiguity-aware ranking penalties.
- **Intent misclassification**: keep deterministic parser and explicit confidence thresholds; route only above threshold.
- **Precision regression from fuzzy**: isolate to low-confidence paths and penalize heavily.
- **Parity drift**: always compare semantic fields first and maintain explicit truncation semantics.
- **Latency spikes**: enforce hard budgets first, then add caching only after stable baseline.

## 11) Example rollout timeline

- Week 1–2: Phase 0 + 1
- Week 3–4: Phase 2
- Week 5–6: Phase 3
- Week 7: Phase 4 + 5
- Week 8: Phase 6 + 7
- Week 9: Phase 8 and release notes

## 12) Final recommendation

Prioritize rollout sequence by risk:

1. parity hardening,
2. intent parsing,
3. bounded recall expansion,
4. feature-ranked reranking,
5. intent routing + path/edge refinements,
6. optional fuzzy mode,
7. cache/timing optimization.

This keeps every change measurable, reversible, and compatible with existing local-first
constraints while moving Compass closer to intent-aware, ranked graph retrieval.
