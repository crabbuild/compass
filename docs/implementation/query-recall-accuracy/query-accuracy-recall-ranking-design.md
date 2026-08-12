# Compass query accuracy, recall, and performance blueprint

## Purpose

This blueprint defines a phased implementation plan to improve natural-language query recall, ranking precision, and intent matching in Compass while preserving local-first execution, deterministic results, and backend parity.

The target behavior is:

- better candidate discovery for node/edge/path queries,
- better ranking quality in top-k,
- high-confidence intent routing (callers/callees/impact/path/search),
- bounded fuzzy recovery,
- stable runtime with measurable regression safeguards.

---

## 1) Current Compass capabilities we can use

Compass already contains core primitives for each part of a modern retrieval stack:

- Query orchestration:
  - `crates/compass-query/src/traversal.rs`: `query_graph_text`, `query_graph_text_page`, `render_shortest_path`
  - `crates/compass-cli/src/lib.rs`: CLI `query` command currently uses `query_graph_text_page`
- Candidate scoring:
  - `crates/compass-query/src/score.rs`: `score_nodes`, `pick_seeds`, `pick_scored_endpoint`
- Graph retrieval/operations:
  - `crates/compass-query/src/code_query.rs` with structured operations:
    `search`, `callers`, `callees`, `impact`, `node_trail`
- Relevance/quality harness:
  - `crates/compass-query/src/relevance.rs`
  - `crates/compass-query/tests/relevance_qualification.rs`
  - `scripts/qualify_query_relevance.py`
  - fixture contract in `fixtures/relevance/judged.json`

These are enough to implement a Google-like “intent + retrieval + ranking” stack without adding external ranking services.

---

## 2) Constraints and invariants (hard)

These must remain true after implementation:

- Local-first, no external vector DB or LLM ranking path at query runtime.
- Deterministic output for identical input and same runtime mode.
- JSON and store backends remain in parity for same query.
- No new Graphify-like runtime dependencies.
- No silent ambiguity resolution by first-match preference.
- Bounded work and bounded output sizes (explicit limits always apply).
- Contracts remain versioned and validated.

---

## 3) Success criteria and baseline metrics

Current harness already supports the critical metrics below. Use these as gating KPIs:

- Node retrieval relevance:
  - `success_at_1`, `mrr_at_10`, `recall_at_5`, `recall_at_20`, `precision_at_10`, `ndcg_at_10`
- Intent quality:
  - `intent_macro_f1`, per-intent precision/recall/F1
- Structural quality:
  - `edge_precision`, `edge_recall`, `edge_kind_precision`, `edge_direction_precision`
  - `path_acceptance_rate`, `mean_accepted_path_rank`
- Robustness and safety:
  - `accepted_ambiguity_recall`, `no_answer_precision`, `false_positive_rate`
- Runtime:
  - `latency_p50_micros`, `latency_p95_micros`, `WorkCounts` (candidates/readers/expanded bytes, etc.)

Recommended definitions:

- `recall@k = relevant_retrieved_in_top_k / relevant_total`
- `precision@k = relevant_retrieved_in_top_k / min(k, returned_total)`
- `ndcg@k = (DCG@k / IDCG@k)` where grade mapping is already in `relevance.rs`
- `mrr@10 = 1 / first_relevant_rank` (only top-10 considered)

Acceptance for each phase:
- No metric regression outside approved tradeoffs.
- Any lift must be verified on fixture corpus and executable subset.
- p95 latency must remain within phase-specific budget.
- Backend parity and determinism remain preserved.

---

## 4) End-state design (target architecture)

Introduce a four-stage pipeline:

1. **Intent planning**
   - Parse question into explicit intent and slots (`intent`, `symbols`, `direction`, `limits`, `fuzzy_needed`).
2. **Recall assembly**
   - Bounded, multi-channel candidate retrieval.
3. **Ranking**
   - Deterministic, feature-based scoring and tie-breaking.
4. **Execution and render routing**
   - Route by confidence to native operations (`search`, `callers`, `callees`, `impact`, `node_trail`) when possible.

This replaces a single-pass lexical seed + graph traversal default for intent-heavy cases, while keeping generic traversal fallback for low-confidence or ambiguous queries.

---

## 5) Data model to introduce

### 5.1 Intent plan model

New module: `crates/compass-query/src/intent.rs`

- `QueryIntent` enum:
  - `Search`, `Callers`, `Callees`, `Impact`, `Path`, `Explain`, `Unknown`
- `QueryIntentPlan`:
  - `intent`, `confidence: u8` (0–100),
  - `symbols: Vec<String>`,
  - `direction: Option<RelationDirection>`,
  - `include_heuristic: bool`,
  - `relation_hints: Vec<String>`,
  - `limits_hint: Option<usize>`,
  - `raw_terms: Vec<String>`,
  - `rationale: Vec<IntentMatchReason>`
- `IntentMatchReason` for deterministic diagnostics (e.g., matched verb table, symbol parse).

### 5.2 Recall/Ranking intermediate model

- `CandidateSource` (channel tag):
  - `ExactId`, `ExactLabel`, `TermIndex`, `RelationSeed`, `FuzzyAlias`
- `CandidateRecord`
  - node id, source channels, evidence tags, per-feature weights
- `RecallBudget`
  - `max_candidates_total`,
  - `max_relation_seed`,
  - `max_postings_per_term`,
  - `max_fuzzy_total`,
  - `truncate_reason` enum.

### 5.3 Ranking profile metadata

- `ranker_profile_id` and `planner_version` should be reported in qualification reports to make metric change attribution explicit.

---

## 6) Phase 0 – Measurement and safety first (1 week)

**Goal:** ensure all future changes are provable and comparable.

### Work

1. Lock current baseline by running and exporting:
   - `scripts/qualify_query_relevance.py`
   - `cargo test -p compass-query --test relevance_qualification --locked`
2. Canonicalize backend parity checks:
   - In `relevance_qualification.rs`, compare normalized semantic result sets instead of raw JSON bytes.
3. Add explicit regression budget for test and metric outputs in a phase metadata file (if not already present in your process).
4. Confirm fixtures contain reviewed intent/no-answer/noisy/fuzzy classes.

### Deliverables

- Baseline report with stable metrics
- Comparison script/process documented in `PERFORMANCE.md` and/or
  `docs/implementation/query-recall-accuracy/query-accuracy-recall-ranking-design.md`

### Exit criteria

- Stable deterministic qualification output (same input -> same report)
- No hidden JSON-byte parity assumptions
- No fallback regressions introduced while baseline is collected

---

## 7) Phase 1 – Intent parser and planner (2 weeks)

**Goal:** convert implicit query interpretation into explicit deterministic intent plans.

### Work

1. Implement `intent.rs`:
   - deterministic token patterns for verb-intent mapping:
     - `who calls`, `called by`, `callers of`,
     - `what does X call`, `calls from`,
     - `who is impacted by`, `impact of`,
     - `path from A to B`, `what depends on`
   - symbol extraction for qualified names, method-like symbols, and file/module tokens.
2. Parse directional hints:
   - `incoming`, `outgoing`, `to`, `from`, `upstream`, `downstream`.
3. Compute `confidence` with deterministic scoring.
4. Add downgrade paths:
   - confidence below threshold (example: `< 65`) uses legacy traversal flow.
5. Add unit tests:
   - deterministic intent extraction,
   - ambiguous examples,
   - confidence boundary behavior.

### Exit criteria

- Intent coverage for at least 3 classes (e.g., `search/callers/callees`) with measurable precision lift on fixture slice.
- Low-confidence cases still produce safe fallback responses.

---

## 8) Phase 2 – Recall multiplexer (2–3 weeks)

**Goal:** increase recall without adding uncontrolled noise.

### Work

1. Add multi-channel recall builder used before ranking:

   - `ExactId` / `ExactName` from lookup index
   - `TermIndex` from normalized label lookup
   - `RelationSeed` for intent-specific expansion:
     - from `callers/callees/impact` seed symbol
   - `PathSeed` for path requests:
     - source/target seeds, bounded bidirectional expansion seeds
   - `FuzzyAlias` optional fallback (gated by low confidence / low first-pass coverage)

2. Add provenance tags per candidate:
   - each candidate stores `source_channels` and `source_rank`.
3. Enforce budget hard limits per stage:
   - `max_candidates_total`, `max_postings_per_term`, etc.
4. Emit truncation reasons into diagnostics/work counters for tuning.

### Exit criteria

- Recall@20 and no-answer precision improved on relevant intent classes without broad precision collapse.
- Traceable candidate count and truncation stats for tuning.

---

## 9) Phase 3 – Ranking v2 (2–4 weeks)

**Goal:** improve top-k order quality deterministically.

### Work

1. Keep `score_nodes` as stable public entry point but refactor internally to:
   - compute structured feature vectors,
   - apply deterministic weights by `ranker_profile`:
     - `query-ranker/1` (baseline),
     - `query-ranker/2` (feature expansion),
     - `query-ranker/3` (intent-aware).
2. Add/expand features:
   - exact/prefix/substring/token coverage,
   - normalized id/qualified-name match,
   - source-backed evidence bonus,
   - intent fit bonus (e.g., caller/callee/path direction),
   - alias confidence penalty,
   - relation-context fit,
   - anti-hub penalty or controlled degree prior.
3. Preserve deterministic tie-breakers:
   - source-backed > inferred,
   - semantic rank,
   - provenance penalty,
   - stable deterministic secondary fields.
4. Emit ranking diagnostics:
   - score components or feature buckets in debug output (never random).

### Exit criteria

- Statistically meaningful lift in `precision_at_10` and `ndcg_at_10` on the reviewed corpus slice.
- No non-deterministic ties.

---

## 10) Phase 4 – Intent-aware execution routing (2 weeks)

**Goal:** avoid generic traversal for explicit graph-intent queries.

### Work

1. In natural query command flow (`query_graph_text_page` / CLI flow), route if intent confidence is strong:
   - `Callers` -> `CodeQueryEngine::callers`
   - `Callees` -> `CodeQueryEngine::callees`
   - `Path` -> `CodeQueryEngine::node_trail` or path operator
   - `Impact` -> `CodeQueryEngine::impact`
2. Return operation metadata in response/diagnostics:
   - intent, routing profile, candidate source mix, truncation reasons.
3. Keep old traversal response path for fallback and for queries with weak intent.

### Exit criteria

- Intent-specific classes show clear gains in direction/edge/path metrics.
- Backend parity unaffected for all routed operations.

---

## 11) Phase 5 – Relation/path quality pass (1–2 weeks)

**Goal:** improve `edge_direction_precision`, `path_acceptance_rate` directly.

### Work

1. Rank edges by:
   - direction fit with intent,
   - relation-kind fit (`calls/imports/references/dependsOn/etc.`),
   - evidence confidence,
   - source anchoring.
2. Rank paths by:
   - endpoint matching,
   - relation-kind sequence plausibility,
   - shortest/bounded depth preference,
   - direction consistency.
3. Penalize path results where endpoints are weakly anchored.

### Exit criteria

- `edge_direction_precision` and `path_acceptance_rate` improve relative to phase baseline.

---

## 12) Phase 6 – Bounded fuzzy recovery (1 week)

**Goal:** recover from misspellings and partial tokenization mismatch without precision collapse.

### Work

1. Fuzzy triggers:
   - weak intent confidence OR recall starvation (`results_found < 3`).
2. Candidate generation constraints:
   - only for tokens length >= 4,
   - bounded Levenshtein horizon (1 by default, conditional 2 only when token is long and corpus density is low),
   - vocabulary from existing symbol names/normalized labels,
   - verify candidate exists before insertion.
3. Strong provenance penalties for fuzzy candidates in ranking.
4. Never allow fuzzy candidates to exceed explicit caps.

### Exit criteria

- Improvement on typo/noisy fixture slices,
- no measurable regression in `no_answer_precision` and negative intent tests.

---

## 13) Phase 7 – Performance hardening and caching (ongoing after phase 5)

**Goal:** avoid latency regressions while adding complexity.

### Work

1. Add stage timing:
   - parse/plan, recall, rank, route, render.
2. Add bounded per-request memoization keyed by:
   - query normalization, graph digest, operation, limits, ranker profile.
3. Implement early-stop:
   - stop recall channels once top-k confidence and diversity thresholds are met.
4. Add explicit counters for truncation reasons:
   - `candidates_cap`, `postings_cap`, `relation_seed_cap`, `fuzzy_cap`, `traversal_cap`.

### Exit criteria

- p95 latency not worse than baseline envelope for benchmark queries.
- Throughput stable for repeated workload after warm cache.

---

## 14) Phase 8 – Rollout and kill-switch strategy (2–3 weeks)

### Rollout sequence

1. Internal profile shadowing:
   - baseline profile as control, profile-2/3 as treatment.
2. Evaluate on relevance corpus with top-k deltas and per-metric deltas.
3. Production rollout by intent class:
   - Stage A: `search` + `callers`
   - Stage B: `callees`
   - Stage C: `impact` + `path`
4. Add explicit kill switch through CLI/flag/config:
   - force `query-ranker/1` and `intent-planner/1` fallback.

### Exit criteria

- All targeted gates pass on both fixtures and executable parity subset.
- No regression in negative/no-answer classes.
- New behavior documented with migration notes if user-facing UX changes.

---

## 15) Concrete ownership mapping

- `crates/compass-query/src/intent.rs` (new): intent parser/planner.
- `crates/compass-query/src/score.rs`: structured features + ranking profiles.
- `crates/compass-query/src/traversal.rs`: intent-aware path from planning -> execution.
- `crates/compass-query/src/code_query.rs`: operation-specific seed/ranking hooks.
- `crates/compass-query/src/relevance.rs`: expand diagnostics and profile metadata as needed.
- `crates/compass-query/tests/relevance_qualification.rs`: new/updated phase tests and parity checks.
- `crates/compass-query/tests/fixtures/relevance/judged.json`: expand intent/fuzzy/path fixture corpus.
- `crates/compass-cli/src/lib.rs` and `crates/compass-query/src/lib.rs`: wire profile-aware execution.
- `scripts/qualify_query_relevance.py`: keep as hard gate.

---

## 16) Validation matrix per phase

Run at least these commands at phase transitions:

- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test relevance_qualification --locked`
- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked`
- `python3 scripts/qualify_query_relevance.py` (with env var checked in script)
- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test relevance_qualification -- --nocapture` for triage when deltas are unexpected
- `cargo clippy -p compass-query --all-targets --all-features --locked -- -D warnings`
- `./scripts/qualify_code_graph_v1.sh --fixtures-only` if graph projection behavior changes.

For production-visible CLI changes, add:

- `cargo test -p compass-cli --test compass_product --locked`
- `sh scripts/check_product_boundary.sh`

---

## 17) Risks and mitigations

- **Over-recall increases noise**
  - Mitigation: strict caps + fuzzy penalties + route thresholds.
- **Wrong direction interpretation**
  - Mitigation: explicit direction parsing + direction-labeled fixtures.
- **Ranking instability**
  - Mitigation: stable deterministic tie-break chain + profile IDs.
- **Latency regression**
  - Mitigation: stage timings + early-stop + bounded channel caps.
- **Contract drift**
  - Mitigation: versioned profiles and explicit qualification gating.

---

## 18) Suggested 6–12 week execution plan

- **Weeks 1–2:** Phase 0 + Phase 1
- **Weeks 3–5:** Phase 2 + Phase 3 (partial)
- **Weeks 6–7:** Phase 3 complete + Phase 4
- **Weeks 8–9:** Phase 5 + Phase 6
- **Weeks 10–12:** Phase 7 + Phase 8 rollout

---

## 19) Fast 2-week MVP pilot (lowest risk)

1. Canonical parity hardening and stable metrics baseline.
2. Implement intent parser only for:
   - `callers of X`
   - `callees of X`
3. Add relation seed channel for these two intents.
4. Add candidate provenance penalties.
5. Add 20–40 targeted fixture queries for intent and typo classes.

If this pilot is successful, extend to path/impact direction and path ranking in the next phase.

---

This blueprint is designed for incremental execution: each phase is independently releasable, backward-compatible, and measurable against strict quality and parity constraints.
