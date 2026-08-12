# Compass query quality and recall implementation design

## 1) Executive objective

Build a deterministic, local-first query system that behaves like a real ranking
pipeline without external vector/LLM dependencies:

- higher recall for relevant symbols/edges/paths,
- better ranking at low k (recall@k, precision@k, MRR, NDCG),
- stronger intent routing accuracy,
- bounded latency under existing limits,
- strict backend parity between materialized JSON and Store.

This document is a phased implementation plan with concrete code locations and
validation gates.

## 2) What “good query quality” means in Compass today

Compass currently provides:

- a natural-language entrypoint at `command_natural_query` (CLI),
- graph traversal from heuristic seed selection in `traversal.rs`,
- a simple lexical scoring path in `score.rs`,
- direct execution paths in `code_query.rs` (`search`, `callers`, `callees`,
  `impact`, `node_trail`).

That gives decent baseline behavior but mixes intent handling, recall generation,
and ranking in one flow.

## 3) Non-goals and hard constraints

- No external embedding or network model dependencies.
- No change to public contracts unless explicitly versioned.
- Keep bounded behavior and deterministic ordering.
- No unbounded caches and no hidden state across graph digests.
- Keep evidence directionality, ambiguity, and unknown behavior explicit.
- Preserve JSON/Store result parity where expected by qualification gates.

## 4) Current architecture gap and target architecture

### Current gaps

1. Intent is implicit; intent-specific behavior is not explicit.
2. Candidate retrieval is mostly single-channel (index + exact/normalized matches).
3. Ranking is largely lexical and does not separately optimize recall/precision
   tradeoffs.
4. Edge/path/intent semantics are mixed with generic traversal defaults.
5. Observable work counters for recall and fuzzy fallback are under-detailed.

### Target architecture

Implement explicit stages:

1. **Intent plan** (parse + route)
2. **Recall layer** (bounded multi-channel candidate collection)
3. **Rank layer** (feature-vector scoring + deterministic tie-break)
4. **Materialization layer** (intent-aware traversal/edge/path assembly)

Each stage has deterministic outputs and is independently testable.

## 5) Quality definitions and acceptance baseline

Use existing relevance metrics in `crates/compass-query/src/relevance.rs` and
fixtures in `crates/compass-query/tests/fixtures/relevance/judged.json`:

- `success_at_1`, `mrr_at_10`, `recall_at_5`, `recall_at_20`
- `precision_at_10`, `ndcg_at_10`
- `intent_macro_f1`, per-intent precision/recall/F1
- `entity_slot_exact_match`
- edge quality: `edge_precision`, `edge_recall`, `edge_kind_*`, `edge_direction_*`
- `path_acceptance_rate`, `mean_accepted_path_rank`
- `no_answer_precision`, `false_positive_rate`
- latency `p50/p95`
- backend parity (`store` vs `json`) determinism checks

Use these as hard regression points per phase, not only at the end.

## 6) Concrete design for “Google-like intent + ranking” behavior

Google-like behavior here means: intent classification, multi-channel recall,
feature-based ranking, deterministic tie breaks, and measurable quality
improvement.

### 6.1 Intent plan (query understanding)

Create `crates/compass-query/src/intent.rs` with:

- `QueryIntent` enum:
  - `Search`, `Callers`, `Callees`, `Path`, `Impact`, `Explain`, `Unknown`.
- `QueryIntentPlan`:
  - `intent`, `confidence` (0..=100), `symbols`, `relation_hints`,
    `direction`, `depth`, `limit`, `raw_tokens`, `constraints`, `parse_trace`.
- deterministic parser, no fuzzy ML classifier.

Use weighted cues:

- `who calls`, `callers of`, `X calls` → callers/callees intent.
- `path from A to B`, `connects`, `shortest` → path intent.
- `what does X return`, `arguments`, `parameters`, `imports`, `exports` → context/slot hints.
- explicit identifiers (`CamelCase`, `snake_case`, `sha256:` IDs) should be extracted and tagged.

Only route if confidence threshold is met; otherwise fallback to existing
search path.

### 6.2 Recall-first candidate layer

Create a deterministic candidate aggregator in `compass-query`:

- Source order is fixed and deterministic.
- Every candidate carries provenance tags.

Candidate sources (in strict priority order):

1. exact ID lookup (`node_by_id`)
2. exact normalized symbol/alias lookup
3. name-token buckets (`nodes_by_normalized_name`)
4. text index hits
   - Store: `nodes_for_terms`
   - Materialized: `node_fts` + `nodes`
5. intent-seeded relations (for callers/callees/path)
6. bounded fuzzy expansion from normalized terms

For each source:

- cap reads (`max_source_hits`),
- report truncation reason when cap hit,
- dedupe by canonical graph ID,
- sort by source priority, then stable ID ordering.

### 6.3 Ranking layer

Create explicit ranking features in `score.rs` (in addition to existing behavior):

- lexical: exact/prefix/substring
- token coverage fraction
- intent fit (relation/context terms)
- symbol exactness (qualified name vs name match)
- evidence quality
- graph role/degree features (bounded
  penalty/bonus)
- path readiness when traversal is required
- ambiguity penalties for test-generated/no-source IDs

Keep score as a deterministic scalar sum of weighted features.
Persist profile version in internal diagnostics (`query-ranker/1`, `query-ranker/2`) to
support safe roll-forward and roll-back.

Deterministic tiebreaker should include:

1. source-backed flag,
2. semantic rank class,
3. test/generated penalty,
4. source-backed degree,
5. label length,
6. stable node ID.

### 6.4 Materialization and routing stage

In `code_query.rs` and `traversal.rs`:

- `command_natural_query` parses intent and selects the dedicated execution path.
- Search route uses ranked candidate set and traversal seeded from top candidates.
- Callers/callees route should constrain direction/allowed kinds.
- Path route should run seeded shortest path with relation-aware tie-breaking.

## 7) Phase plan (phased and actionable)

## Phase 0 — Baseline hardening (1 week)

Purpose: stabilize gates before algorithm changes.

### Tasks

1. Lock down parity checks in
   `crates/compass-query/tests/relevance_qualification.rs`:
   - compare semantic fields (operation, limits, ordered nodes/edges/results IDs,
     truncated, diagnostics), avoid raw JSON byte-by-byte for Store parity except
     where contract requires canonicalized bytes.
2. Keep/extend diacritic normalization parity assertions:
   `café/cafe`, `résumé/resume`, `ångström` in review subset.
3. Ensure both search paths share normalized term extraction.
4. Enforce deterministic canonical sorting for reviewed outputs in tests.

### Exit criteria

- Reviewed and executable baselines still pass.
- Store and JSON execute deterministically for same query set.
- No unbounded test regressions from output ordering.

## Phase 1 — Intent parser and query dispatch (1–2 weeks)

### Tasks

1. Add `crates/compass-query/src/intent.rs`.
2. Export parsing API in `crates/compass-query/src/lib.rs`.
3. Extend `compass-cli` routing in `command_natural_query`:
   - parse intent first;
   - route high-confidence:
     - `Callers`, `Callees`, `Path`, `Impact` to dedicated ops;
     - fallback to search/traversal on low-confidence.
4. Add `intent` unit tests for ambiguous/weak/noisy queries.

### Exit criteria

- Intent parser precision/recall on curated intent fixtures improves vs baseline.
- `exact ID` and low-signal behavior unchanged under confidence threshold.

## Phase 2 — Recall multiplexer (2–3 weeks)

### Tasks

1. Introduce bounded multi-source candidate assembler.
2. Add provenance tags (`ExactId`, `NormalizedName`, `Fts`, `Fuzzy`,
   `RelationSeed`, `Alias`).
3. Add per-source budget caps:
   - `max_candidate_source_items`,
   - `max_fuzzy_items`,
   - `max_postings_per_term`.
4. Track truncation reasons in debug work counters.
5. Add explicit query-path seeds for path/callers/callees.

### Exit criteria

- Recall metrics improve by target delta on fixture and executable subset.
- Truncation and candidate budget telemetry is visible and bounded.
- Deterministic candidate ordering maintained.

## Phase 3 — Ranking model upgrade (2–3 weeks)

### Tasks

1. Move ranking from implicit tuple logic toward explicit feature vector scoring.
2. Add an internal ranked feature type:
   - lexical, intent, token coverage, relation fit, evidence, ambiguity.
3. Add profile versions and include profile in report/diagnostic metadata.
4. Add per-intent profile tuning defaults (e.g. search vs callers/callees/path).
5. Preserve deterministic tie-break and add regression tests for stable ties.

### Exit criteria

- `nDCG@10`/`precision_at_10` lift.
- no significant `no_answer_precision`/`false_positive_rate` regression.
- tie-break deterministic under equal score.

## Phase 4 — Edge/path quality and direction semantics (2 weeks)

### Tasks

1. Add relation-aware neighbor selection:
   - callers: inbound `calls|routes_to`
   - callees: outbound `calls`
   - impact/path: relation family aware
2. Edge scoring should include direction/kind agreement with intent.
3. Path selection should rank competing paths by endpoint confidence and relation
   coherence before final output.

### Exit criteria

- `edge_direction_precision` and `path_acceptance_rate` improve.
- rejected false direction paths decrease in negative/ambiguous intent sets.

## Phase 5 — Fuzzy + near-match recovery (1–2 weeks)

### Tasks

1. Add bounded typo recovery only when baseline recall is low.
2. Use small edit distance via `strsim`-style metrics (or equivalent) on a
   constrained vocabulary.
3. Only allow edits for terms meeting length thresholds.
4. Cap fuzzy candidates by token and per-query totals.

### Exit criteria

- typo/noisy query recall improves in fixture (especially near-match intent class).
- false positives remain under baseline tolerance.

## Phase 6 — Performance and observability hardening (2 weeks)

### Tasks

1. Add bounded caches:
   - query-plan cache by normalized query + graph digest + profile + locale
   - optional candidate cache with fixed capacity and TTL.
2. Add stage timing/work counters:
   - parse_ms, recall_ms, rank_ms, materialize_ms,
   - candidates_read, postings_decoded, nodes_expanded, edges_expanded, response_bytes.
3. Add early stop controls:
   - if truncated or max work is reached, stop expansion cleanly.

### Exit criteria

- no unbounded memory growth,
- repeated canonical queries materially improve p95,
- p95 remains within agreed release budget.

## Phase 7 — Rollout and governance (1–2 weeks)

### Execution plan

1. Add shadow mode in CLI/tests:
   - old path + new path in parallel,
   - compare top-N IDs and score deltas.
2. Introduce feature gate:
   - `query-ranker/1` fallback and explicit profile pinning.
3. Roll out in steps: search-only → callers/callees → path/impact.
4. Keep rollback immediate through profile pinning.

## 8) File-by-file implementation map

- `crates/compass-query/src/lib.rs`
  - export intent parsing APIs and profile IDs.
- `crates/compass-query/src/intent.rs` (new)
  - deterministic parser + confidence.
- `crates/compass-query/src/code_query.rs`
  - query entrypoints for intent-specific routing,
  - candidate aggregation and provenance.
- `crates/compass-query/src/score.rs`
  - feature-score vectors and deterministic ranking profiles.
- `crates/compass-query/src/text.rs`
  - reuse/shared normalization, context inference, token extraction.
- `crates/compass-query/src/traversal.rs`
  - seed selection and path/traversal behavior adjusted for intent.
- `crates/compass-cli/src/lib.rs`
  - update `command_natural_query` dispatch path.
- `crates/compass-query/tests/relevance_qualification.rs`
  - phase-gated thresholds + per-phase assertions.
- `crates/compass-query/tests/fixtures/relevance/judged.json`
  - add intentional intent typo/path/negative coverage.

## 9) Validation matrix

### Before and after each phase

- `cargo test -p compass-query --test relevance_qualification --locked`
- targeted relevance tests (`code_search`, `code_query` as needed)
- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main python3
  scripts/qualify_query_relevance.py`

### After each phase that touches ranking/candidates

- `cargo test -p compass-query --locked`
- if query contracts changed:
  - `cargo test -p compass-cli --test compass_product --locked`

### Final phase completion gate

- metric improvement target achieved per intent class,
- no backend parity regression,
- deterministic output behavior stable,
- no unbounded latency or memory growth in stress fixtures.

## 10) Risk register and mitigations

- **Over-recall creates noise**: keep conservative recall thresholds and require
  intent confidence for aggressive channels.
- **Intent misclassification**: use deterministic parse with confidence gates,
  keep fallback behavior unchanged and visible.
- **JSON/Store mismatch**: canonicalize candidate normalization before ranking.
- **Performance regressions**: hard caps, early exits, explicit per-phase timing.
- **Ranking churn**: profile-version roll-forward and explicit rollback profile.

## 11) Minimal acceptance targets (first milestone)

For the first milestone (Phase 1 + Phase 2), the following minimums are
recommended:

- `success_at_1`, `recall_at_20`, `precision_at_10` non-decreasing on reviewed
  corpus,
- `intent_macro_f1` + per-intent recall improve on intent-tagged queries,
- edge direction precision not worse than baseline for callers/callees,
- p95 latency within configured service budget,
- Store/JSON parity tests still pass.

## 12) “Done in code” summary

When implemented through all phases, Compass should answer:

- *“who calls X?”* using incoming call-path intent rather than generic BFS,
- *typo/near-typo* symbol lookups with bounded fallback,
- *path* queries with direction and relation-aware candidate ranking,
- with deterministic behavior and measurable improvement in recall and ranking quality.

## 13) Suggested rollout timeline

Use this as a planning template:

- Week 1: Phase 0 + Phase 1 scaffolding (intent parser + routing integration)
- Week 2: Phase 1 hardening + start Phase 2 candidate multiplexer
- Week 3–4: Finish Phase 2 + baseline recall-focused gates
- Week 5–6: Phase 3 ranking upgrade + feature-guard regression
- Week 7: Phase 4 edge/path direction semantics + path scoring
- Week 8: Phase 5 fuzzy + Phase 6 observability + roll-forward dry-run
- Week 9: Phase 7 staged rollout + rollback validation
