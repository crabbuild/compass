# Query performance, recall, and intent accuracy playbook for Compass

This document is a practical roadmap for improving natural-language query behavior
without adding external model dependencies. It is designed to be implemented under
the existing `compass-query` and `compass-cli` ownership boundaries, with
deterministic outputs and explicit rollout gates.

## 1) Problem and intent

Compass already returns graph-aware answers, but the current natural query path is
heavy on traversal heuristics and light on intent-specific ranking. The goal is to
make recall higher and ranking quality closer to mature search behavior while keeping
determinism, local-first guarantees, and bounded latency.

What we are optimizing:

- intent routing (`search`, `callers`, `callees`, `impact`, `node_trail`, explain-like behavior),
- candidate retrieval for typo/near-miss phrasing,
- ranking quality for relevant nodes, edges, and paths,
- deterministic query performance under bounded budgets,
- and confidence-aware no-answer behavior.

## 2) Constraints (hard requirements)

- no external embeddings services, model credentials, or runtime Python;
- no change to graph semantics or public response contracts unless reviewed;
- deterministic tie handling and stable ordering for equal scores;
- explicit unknown/ambiguous results preferred over invented guesses;
- bounded work counts and hard limits from `CodeQueryLimits`;
- backend parity (`Json` vs `Store`) remains required.

## 3) Current implementation boundaries

Primary owner and entry points:

- CLI dispatch: `crates/compass-cli/src/lib.rs`
  (`command_natural_query` currently calls `query_graph_text_page`)
- Query core and contracts: `crates/compass-query/src/lib.rs`
- Score/seed selection: `crates/compass-query/src/score.rs`
- Text normalization + context inference: `crates/compass-query/src/text.rs`
- Traversal + rendering: `crates/compass-query/src/traversal.rs`
- Backend execution and search/traversal APIs:
  `crates/compass-query/src/code_query.rs`
- Relevance qualification: `crates/compass-query/src/relevance.rs`,
  `crates/compass-query/tests/relevance_qualification.rs`,
  `crates/compass-query/tests/fixtures/relevance/judged.json`

## 4) Target architecture

Adopt a deterministic three-stage pipeline:

1. **Intent parse + plan**
   - convert NL to a structured plan (`intent`, slots, symbol mentions, relation hints, confidence)
2. **Recall-first candidate generation**
   - gather a bounded superset from multiple deterministic sources
3. **Intent-aware ranking + path/edge scoring**
   - produce ordered candidates, then traverse/assemble with the plan

This mirrors modern IR flow while keeping behavior explicit and explainable.

## 5) Measurable targets

- Accuracy and retrieval metrics from existing relevance gate:
  - node: `Success@1`, `MRR@10`, `Recall@5`, `Recall@20`, `nDCG@10`,
    `Precision@10`
  - intent: per-class precision/recall/F1, plus `intent_macro_f1`
  - structure: edge precision/recall, direction precision/recall, path acceptance
  - robustness: no-answer precision, false-positive rate
- Operational: median latency and P95 within budget; no >10% regression compared
  to approved baseline for changed surfaces.
- Behavioral: backend parity remains stable (`json` vs `store`), deterministic ties
  unchanged unless score logic is intentionally versioned.

## 6) Proposed data model for internal ranking

### 6.1 Query intent plan

Introduce an internal `QueryIntentPlan` (in a new `crates/compass-query/src/intent.rs`):

- `intent`: `Search | Callers | Callees | Impact | Path | Explain | Unknown`
- `intent_confidence`: `0..=100`
- `symbols`: normalized symbol candidates extracted from the question
- `relation_hint`: optional normalized relation verb (calls, imports, routes_to, ...)
- `direction`: optional `incoming|outgoing|both`
- `depth` / `limit`: bounded overrides from query if present
- `explicit_contexts`: explicit or inferred context filters
- `parse_reasons`: why the plan was selected (for diagnostics/replays)

### 6.2 Candidate provenance

Use a compact internal provenance enum:

- `ExactId` (exact identifier match)
- `NormalizedName` (node normalized name / qualified name)
- `Alias` (split identifier / alias table)
- `Fts` (index-backed term candidates)
- `Fuzzy` (bounded spelling variant expansion)
- `TraversalSeed` (when plan intentionally starts from one exact anchor)

For each returned candidate keep:

- `node_id`
- provenance rank
- per-source score components
- source-backed evidence marker (exact, resolved, heuristic, generated)
- relation match tags (for caller/callee/path plans)

### 6.3 Ranked feature vector

Replace implicit tuple-only scoring with structured features (phase build-up):

- lexical exact/prefix/substring match strength
- token coverage (`matched_tokens / expected_tokens`)
- evidence quality (source-backed, confidence)
- relation intent alignment (for callers/callees/impact/path)
- alias/fuzzy confidence
- ambiguity risk
- graph utility (degree signal, hub penalty)
- path-readiness score (presence in likely traversal frontier)

Score profiles should be versioned (for safe rollback): e.g. `query-ranker/1`,
`query-ranker/2`.

## 7) Detailed phased implementation

### Phase 0 — Baseline stabilization and instrumentation (1 week)

**Why first:** current qualification has fragile full-serialization parity assertions
in `backend_parity_subset_preserves_normalized_search_ids_and_edges` and the
determinism test. Those should compare canonical fields instead, then enforce strict
ordering and parity explicitly.

Actions:

- Add canonical comparator helpers in relevance tests:
  - `response_signature(store_result, json_result)` that compares:
    - `schema`, `operation`, `limits`, sorted `results`, sorted `nodes`,
      sorted `edges`, sorted diagnostics (code+message), truncation flag.
  - For reports: compare fields used for semantics, not opaque JSON ordering bytes.
- Add `execution_plan` (debug-only) artifact in `QueryObservation::slots` for test
  harness only; keep public response contract unchanged.
- Extend query plan counters in `QueryObservation.work` where implementation can
  safely track:
  - candidates read, postings decoded, nodes/edges expanded, response bytes.
- Gate before rollout:
  - existing relevance test suite green;
  - deterministic baseline report runs with fixed limits and exact serialized order.

Exit criteria:

- no accidental broadening of deterministic equality expectations,
- stable golden baseline remains deterministic,
- test coverage now guards behavior semantics rather than struct layout.

### Phase 1 — Intent parser and route planner (1–2 weeks)

**Scope:** create explicit intent extraction without changing semantics yet.

- Add `crates/compass-query/src/intent.rs` + unit tests:
  - detect verbs: who calls / callers of / callees / what does X call / impact /
    path / from A to B / where is / what uses
  - extract symbols and directional tokens
  - confidence scoring with explicit low-confidence fallback
- Integrate parser into `crates/compass-cli/src/lib.rs::command_natural_query`:
  - low-confidence plan -> existing `query_graph_text_page`
  - high-confidence `Callers/Callees/Path` plans -> execute dedicated paths in
    `CodeQueryEngine`
  - preserve old behavior as fallback.
- Add parser traces to diagnostics (query-level debug, not contract fields):
  detected intent, relation hints, confidence, chosen fallback mode.

Exit criteria:

- intent-labeled synthetic set from fixture reaches first-pass acceptance (>70% on high
  confidence queries),
- no broad behavior regression on unambiguous exact ID and classic search prompts.

### Phase 2 — Recall-first candidate generation (2–3 weeks)

Goal: increase recall before ranking.

- In `crates/compass-query/src/code_query.rs`, add a bounded candidate collector:
  1. exact ID / explicit symbol parse first,
  2. exact normalized name + alias candidates,
  3. FTS/store posting candidates (intersection semantics when feasible),
  4. bounded fuzzy candidate expansion (`query_graph_v1` compatibility intact).
- Keep deterministic order by:
  1) stable sort by source provenance priority,
  2) canonical ID order within each source.
- Implement bounded fuzzy mode only when:
  - primary candidate sources underfill;
  - token length is adequate;
  - edit budget small and capped.
- Use existing `CodeGraphBackend::nodes_by_normalized_name`, `store_term_candidates`,
  and `search_query` / query-term extraction; introduce helper vocabulary cache for
  single-token typo repair.
- Keep budgets explicit in plan/config:
  - `max_candidate_source`, `max_fuzzy_generated`, `max_fuzzy_per_token`,
    `min_fuzzy_len`, `max_edit_distance`.

Exit criteria:

- recall-at-k improves (target: Recall@20 +10–20% relative to baseline on reviewed
  corpus),
- ambiguity and no-answer precision do not drop materially in controlled runs,
- hard candidate count caps enforced and instrumented.

### Phase 3 — Intent-aware ranking model (2–3 weeks)

- Replace current ad-hoc score accumulation in `score_nodes` with explicit feature
  scoring and deterministic tie policy:
  - stable feature vector + weighted sum (or monotonic profile),
  - deterministic tie-break:
    source-backed flag, semantic confidence, heuristic penalty, degree, label length,
    node ID.
- Add dedicated ranking paths:
  - generic search ranking,
  - caller/callee ranking with direction/intent weighting,
  - path seed scoring with endpoint and edge-kind compatibility.
- Track and emit top-ranking explanation metadata internally for every response in
  non-contract debug mode:
  - top feature weights,
  - which intents/features fired.
- Add tests for:
  - stable ranking on equal scores,
  - deterministic order across equivalent graph seeds,
  - monotonic score behavior under score profile changes.

Exit criteria:

- improved `nDCG@10` and `Precision@10` on intent queries with stable output ordering,
- zero regression in backend-parity signatures from Phase 0.

### Phase 4 — Edge, direction, and path quality upgrades (2 weeks)

- Add operation-specific edge retrieval for relation-like plans:
  - callers uses inbound call edges plus route edge family where plan requires,
  - callees uses outbound call edges first,
  - path queries apply relation and endpoint filters before traversal.
- Add relation mismatch penalty and relation-kind bonus in path ranking:
  - path endpoint similarity,
  - direction consistency,
  - edge-kind match ratio,
  - hop length penalty.
- Return ranked alternatives for path-like queries where multiple alternatives exist and
  include selected explanation IDs in debug slots.

Exit criteria:

- `edge_direction_precision`, `edge_kind_precision`, and `path_acceptance_rate` improve,
- selected debug traces allow easy manual review when path ranking disagrees with
  expectations.

### Phase 5 — Performance and caching (2 weeks)

- Add bounded in-memory process cache (optional, opt-in):
  - key: `(graph_digest, query_digest, normalized plan, limits, intent profile hash)`
  - payload: top-N ranked IDs, parsed intent plan, path candidate ranking.
- Add bounded term/vocabulary cache per open graph:
  - normalized symbol map and hot token map
  - invalidate on graph mismatch only.
- Add query-mode circuit breakers:
  - query-mode budget hard-caps (`max_candidates`, `max_nodes`, `max_edges`)
  - per-phase early exits with explicit truncated diagnostics.
- Keep memory bounded by LRU or exact capacity; no unbounded global caches.

Exit criteria:

- no unbounded allocation regression in `code_query_scale` test class,
- p50 latency and P95 stay within agreed regression band,
- repeated query with same canonicalized input reuses cache deterministically.

### Phase 6 — Rollout gates and product mode (1–2 weeks)

- Ship in stages:
  1. `search` and `callers`/`callees` under shadow execution (compare top IDs only),
  2. `node_trail` and impact-like intent routes,
  3. enable by default behind default-on feature flag.
- Keep rollback path:
  - static compatibility flags for old rank profile and old intent parser route,
  - kill-switch if deterministic parity or no-answer precision drops.

Acceptance gates before GA:

- revised reviewed-queries relevance suite above thresholds,
- backend parity (`json` vs `store`) stable,
- repeated execution deterministic once timing removed,
- performance gate pass (latency median and tail within threshold).

## 8) Test and qualification plan

### Harness updates

- `crates/compass-query/tests/relevance_qualification.rs`
  - add phased suites:
    - parser-only coverage (intent and slot extraction),
    - recall uplift probes,
    - ranking stability and no-answer safeguards,
    - backend parity/determinism checks against canonical fields.
- `crates/compass-query/tests/fixtures/relevance/judged.json`
  - add intent/negative fuzz/typo and relation-direction cases,
  - avoid auto-derived IDs; keep judgments reviewer-authored.
- `scripts/qualify_query_relevance.py`
  - continue to be the gate, now also exporting phase-specific JSON summaries.

### Coverage targets

- unit tests: intent parser, query planner, ranking features, fuzzy edit gating,
- integration: deterministic parity, no-answer, candidate cap behavior,
- performance: `code_query_scale`, cold/warm process probes, query profile snapshots.

## 9) Suggested execution timeline (example)

- Week 1: Phase 0 + baseline comparator hardening.
- Week 2: Phase 1 intent parser + CLI routing.
- Week 3–4: Phase 2 candidate recall expansion.
- Week 5–6: Phase 3 ranking profiles + Phase 4 path/edge quality.
- Week 7: Phase 5 caching/perf hardening.
- Week 8: Phase 6 rollout, gate review, and GA decision.

## 10) Risk register

- **Over-recall -> noise increase**
  - mitigated by ranking + strict confidence thresholds + `no_answer` class coverage.
- **Latency regression**
  - mitigated with hard caps and phase-by-phase rollout.
- **Ambiguous short/low-signal prompts**
  - mitigated with explicit `Unknown` intent and conservative fallback path.
- **Backend divergence after new candidate sources**
  - mitigated with canonical backend parity signature tests in both directions.
- **Regression in explainability**
  - mitigated by preserving `QueryDiagnosticCode::BoundedTruncation` and
    adding internal plan traces for diagnostics.

## 11) Concrete file-level work queue

- `crates/compass-query/src/intent.rs` (new): intent grammar + confidence planner.
- `crates/compass-query/src/score.rs`: feature vector and profile-aware ranking.
- `crates/compass-query/src/code_query.rs`: candidate collector, ranked retrieval, path
  routing hooks.
- `crates/compass-query/src/text.rs`: reusable token/symbol/context helpers for parser.
- `crates/compass-query/src/traversal.rs`: plan-driven seed selection and traversal
  bounds.
- `crates/compass-query/tests/relevance_qualification.rs`: parity comparator and new
  phase gates.
- `crates/compass-query/tests/fixtures/relevance/judged.json`: intent/fuzzy/typo corpus
  updates.
- `crates/compass-cli/src/lib.rs`: dispatch by query plan with explicit fallback.

## 12) What “good” looks like after implementation

- query answers are more stable by intent (callers/callees/impact/path no longer
  rely on broad search fallbacks),
- typo and near-match prompts return expected entities with bounded, explainable
  candidate correction,
- higher Recall@k without sacrificing no-answer precision,
- path and relation answers are cleaner due to explicit direction/relation scoring,
- all public behavior remains deterministic and backend-parity is maintained.

## 13) Suggested immediate next action

If you want this moved into an actionable engineering backlog, start with:

1. Phase 0 parity/comparator refactor in relevance harness.
2. Add a minimal intent parser (`callers`/`callees`/`path`/`search`).
3. Route `command_natural_query` to parser-backed execution with fallback.
4. Measure before/after on `python3 scripts/qualify_query_relevance.py` using your
   selected external `CARGO_TARGET_DIR`.
