# Compass Query Performance, Accuracy, and Recall Design (Phased Technical Plan)

## 1) Purpose

Implement a local-first, deterministic, ranking-style query pipeline that improves:

- intent understanding for natural-language queries
- candidate recall and typo/noise tolerance
- top-k precision for node/edge/path results
- latency stability at scale

This plan is anchored on existing Compass primitives and does **not** introduce external embedding/vector services.

## 2) Baseline and constraints

### 2.1 Current state (already in-tree)

The query path currently has these relevant components:

- CLI entry: `compass-cli/src/lib.rs::command_natural_query`
- Natural query execution: `crates/compass-query/src/traversal.rs::query_graph_text_page` and helpers
- Search/rank primitives: `crates/compass-query/src/score.rs` (`score_nodes`, `pick_seeds`, `pick_scored_endpoint`)
- Query execution APIs: `crates/compass-query/src/code_query.rs` (`search`, `callers`, `callees`, `impact`, `node_trail`)
- Text parsing/normalization: `crates/compass-query/src/text.rs`
- Qualification harness: `crates/compass-query/src/relevance.rs`, `crates/compass-query/tests/relevance_qualification.rs`, `crates/compass-query/tests/fixtures/relevance/judged.json`

### 2.2 Hard constraints

- local-first: no runtime model calls, no external indexing service
- deterministic: stable output for same input and same graph revision
- bounded: explicit limits for candidates, expansions, and responses
- contract-safe: no silent behavior changes; avoid contract breakage
- parity-first: maintain JSON/store semantic equivalence
- explainability: prefer explicit no-answer/ambiguity over guessed answers

## 3) Desired target architecture

Implement a 5-stage stack:

1. **Intent plan extraction** (parse query into intent/symbols/direction/constraints)
2. **Recall assembly** (bounded, multi-source candidate collection)
3. **Intent-aware ranking** (feature-based scoring + profile-versioned behavior)
4. **Execution routing** (route to dedicated operations when intent is clear)
5. **Validation + observability** (phase-by-phase metrics and guardrails)

Each stage is independent and testable.

## 4) Internal models to introduce

### 4.1 Query intent plan (`compass-query/src/intent.rs`)

Add:

- `QueryIntent` enum: `Search`, `Callers`, `Callees`, `Impact`, `Path`, `Explain`, `Unknown`
- `QueryIntentPlan`:
  - `intent`
  - `confidence: u8 (0..=100)`
  - `symbols: Vec<String>` (canonicalized)
  - `direction: Option<DirectionHint>`
  - `relation_hints: Vec<String>`
  - `limits_hint: Option<QueryLimits>`
  - `raw_tokens: Vec<String>`
  - `parse_trace: Vec<String>` (debug only)

### 4.2 Recall and provenance model

- `CandidateSource` enum:
  - `ExactId`, `ExactName`, `Alias`, `NormalizedName`, `NormalizedAlias`, `Fts`, `RelationSeed`, `PathSeed`, `Fuzzy`
- `CandidateRecord`:
  - `node_id`
  - `sources: BTreeSet<CandidateSource>`
  - `source_rank_by_channel: Vec<(CandidateSource, usize)>`
  - `feature_hits: BTreeMap<String, f64>`
  - `is_source_backed: bool`
- `RecallBudget`:
  - `max_candidates_total`
  - `max_per_source`
  - `max_postings_per_term`
  - `max_relation_seed`
  - `max_fuzzy_total`
  - `max_fuzzy_per_token`

### 4.3 Ranking profile contract

- Keep rank profile version as internal metadata:
  - `query-ranker/1` baseline
  - `query-ranker/2` recall-first with stronger lexical recall
  - `query-ranker/3` intent-aware
- Always emit profile id in relevance artifacts for roll-forward/rollback.

### 4.4 Direction and relation hints

Map parsed direction cues to bounded enums:

- `incoming`, `outgoing`, `both`
- relation families: `calls`, `imports`, `references`, `dependsOn`, etc.

## 5) Phased implementation (8 phases)

### Phase 0 — Baseline hardening and measurement (1 week)

**Goal:** guarantee all future optimization deltas are measurable and unambiguous.

#### Work items

1. Capture current metrics baseline from executable corpus and reviewed fixtures.
2. Fix relevance parity checks to compare semantic fields (operation/ids/truncated/paths), not raw JSON bytes.
3. Add deterministic canonicalization helpers for nodes/edges/results ordering in harness assertions.
4. Add work-counters required for later phases:
   - candidates read
   - postings decoded
   - nodes/edges expanded
   - truncation reasons

#### Exit criteria

- `cargo test -p compass-query --test relevance_qualification --locked` stable
- `python3 scripts/qualify_query_relevance.py` runnable with explicit `CARGO_TARGET_DIR`
- no false parity failures from serialization differences alone

---

### Phase 1 — Intent parser and confidence routing (1–2 weeks)

**Goal:** convert implicit NL into explicit execution-ready plans.

#### Work items

1. Add `intent.rs` with deterministic phrase-matching parser:
   - verbs: callers/callees/path/impact, plus relation-like forms
   - symbol extraction for identifiers: `A::b`, `pkg.mod.fn`, `Class.method`
   - direction cues: `from`, `to`, `incoming`, `outgoing`
2. Add scoring rules with explicit confidence threshold (example: >=65).
3. In `command_natural_query`, branch to intent-backed execution when confidence is high.
4. Add parser unit tests for:
   - positive intent detection
   - ambiguity
   - confidence boundaries

#### Exit criteria

- intent parser has deterministic output for same query
- low-confidence prompts still use legacy/known-safe flow
- no regression in existing exact-ID and generic search cases

---

### Phase 2 — Recall multiplexer (2–3 weeks)

**Goal:** increase recall before ranking.

#### Work items

1. Add bounded multi-source candidate collector:
   - exact id/name channel
   - exact normalized name and alias channel
   - postings channel (`nodes_by_normalized_name`, index/fts-backed)
   - relation/path seed channel for intent-specific queries
   - fuzzy channel gated by starvation/confidence
2. Preserve deterministic ordering:
   - source-priority order
   - canonical node-id sort within each source
   - dedupe by node-id
3. Record truncation events as telemetry.
4. Ensure candidate normalization is canonical across store/materialized paths.

#### Exit criteria

- recall@20 improves on intent/noisy slices
- candidate caps always enforced
- no meaningful precision drop without explicit gate approval

---

### Phase 3 — Ranking v2 (2–4 weeks)

**Goal:** improve ordering quality, especially top ranks.

#### Work items

1. Introduce feature-vector scoring in `score.rs`:
   - exact/prefix/substr matching
   - token coverage ratio
   - qualified-name alignment
   - evidence confidence (source-backed/inferred)
   - intent-fit score
   - degree/hub controls
   - relation/path readiness
   - ambiguity/fuzzy penalty
2. Keep deterministic tie-break chain and emit profile id in non-contract diagnostics.
3. Add ranking stability tests for equal-score ties.

#### Exit criteria

- measured lift in `precision@10`, `ndcg@10`, `mrr@10` on reviewed corpus
- no non-deterministic ranking behavior

---

### Phase 4 — Intent-aware materialization and routing (1–2 weeks)

**Goal:** execute the right graph API for the right intent.

#### Work items

1. Route by parsed intent:
   - `Callers` -> `CodeQueryEngine::callers`
   - `Callees` -> `CodeQueryEngine::callees`
   - `Impact` -> `CodeQueryEngine::impact`
   - `Path` -> `node_trail`
2. Keep fallback to traversal for low-confidence or unresolved symbol cases.
3. Add operation metadata in diagnostics artifacts (internal only).

#### Exit criteria

- improved direction/path behavior with no no-answer precision regression
- fallback behavior remains explicit and safe

---

### Phase 5 — Edge/path quality specialization (2 weeks)

**Goal:** improve structural correctness on relation-heavy queries.

#### Work items

1. Add relation/family-aware edge scoring and direction alignment bonuses.
2. For path queries, rank alternatives by:
   - endpoint confidence
   - direction consistency
   - relation-kind consistency
   - hop penalty
3. Penalize weakly anchored path candidates.

#### Exit criteria

- improved `edge_direction_precision`, `edge_kind_precision`, `path_acceptance_rate`
- reduced wrong-direction path artifacts

---

### Phase 6 — Bounded fuzzy recovery (1 week)

**Goal:** recover from typos/noise while controlling precision risk.

#### Work items

1. Trigger fuzzy only when:
   - base recall is below floor **or**
   - intent confidence is weak
2. Apply bounded edit distance on qualified tokens:
   - min token length threshold (example >=4)
   - low default edit distance
   - hard per-token/per-query caps
3. Mark fuzzy candidates as lower-priority in ranking.

#### Exit criteria

- typo/noisy recall improves
- no significant `false_positive_rate` or `no_answer_precision` regression

---

### Phase 7 — Performance hardening (2 weeks)

**Goal:** preserve or improve latency while adding recall features.

#### Work items

1. Add internal stage timings:
   - parse/plan ms, recall ms, rank ms, execute ms
2. Optional bounded caches:
   - parsed intent cache
   - top-k candidate cache keyed by normalized query + graph digest + profile
3. Early-stop controls and hard ceilings across stages.
4. Add cold/warm perf checks for repeatable workloads.

#### Exit criteria

- p95 stays within phase budget
- no unbounded memory growth
- repeatable warm-run latency after cache warm-up

---

### Phase 8 — Rollout and governance (1–2 weeks)

**Goal:** low-risk release and rollback plan.

#### Work items

1. Shadow mode: old + new execution compared by top-k IDs.
2. Add profile pinning and kill-switch.
3. Roll out intent classes progressively:
   - `search/callers` then `callees` then `impact/path`
4. Update docs/changelog if user-visible behavior changes.

#### Exit criteria

- all phase gates pass
- backend parity remains stable and deterministic
- rollback path validated

## 6) Metrics, gates, and acceptance thresholds

### 6.1 Core quality metrics

Use existing harness metrics in `relevance.rs`:

- `success_at_1`
- `mrr_at_10`
- `recall_at_5`
- `recall_at_20`
- `precision_at_10`
- `ndcg_at_10`
- `intent_macro_f1` and per-intent precision/recall/F1
- `edge_precision`, `edge_recall`, `edge_kind_*`, `edge_direction_*`
- `path_acceptance_rate`, `mean_accepted_path_rank`
- `no_answer_precision`, `false_positive_rate`
- latency `p50/p95`
- work counters and truncation reasons

### 6.2 Suggested gates by phase

- Phase 0: baseline reproducibility and parity
- Phase 2: recall@20 non-decreasing on reviewed sets
- Phase 3: `precision@10` and `ndcg@10` lift
- Phase 4–5: intent and edge/path quality lift
- Phase 7: no >10% p95 regression (baseline-approved corpus)

### 6.3 Command matrix

- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test relevance_qualification --locked`
- `python3 scripts/qualify_query_relevance.py`
- `cargo test -p compass-query --locked`
- If behavior changes user-facing: `cargo test -p compass-cli --test compass_product --locked`
- `cargo clippy -p compass-query --all-targets --all-features --locked -- -D warnings`

## 7) Risks and mitigations

- Over-recall noise: keep strict caps, intent thresholds, and no-answer class monitoring.
- Wrong intent routing: deterministic parser + fallback path.
- Ranking instability: versioned profiles + deterministic tie chains.
- Backend parity drift: canonical semantic comparisons in tests, not raw JSON bytes.
- Latency regression: stage budgets and early-stop controls before optional caches.

## 8) Ownership map

- `crates/compass-query/src/intent.rs` (new): parser/planner
- `crates/compass-query/src/score.rs`: feature ranking and profile switch
- `crates/compass-query/src/code_query.rs`: routing + relation-seed recall
- `crates/compass-query/src/traversal.rs`: fallback and seed-traversal behavior
- `crates/compass-query/src/text.rs`: token/symbol normalization reuse
- `crates/compass-cli/src/lib.rs`: intent-aware dispatch
- `crates/compass-query/tests/relevance_qualification.rs`: phase gates and canonical comparisons
- `crates/compass-query/tests/fixtures/relevance/judged.json`: expand intent/noisy/negative sets
- `scripts/qualify_query_relevance.py`: gate runner and report persistence

## 9) 12-week suggested cadence

- Weeks 1–2: Phase 0 + Phase 1
- Weeks 3–5: Phase 2 + start Phase 3
- Weeks 6–7: Phase 3 complete + Phase 4
- Weeks 8–9: Phase 5 + Phase 6
- Weeks 10–11: Phase 7
- Week 12: Phase 8 and release review

## 10) Immediate 2-week starter (minimal risk)

1. Fix semantic parity checks in phase 0 harness.
2. Implement minimal intent parser for `callers` and `callees`.
3. Add relation-seed recall channel for those intents.
4. Route these two intents in CLI with fallback.
5. Expand fixtures and run full relevance gate.

This delivers measurable gains with low surface risk before full rollout.
