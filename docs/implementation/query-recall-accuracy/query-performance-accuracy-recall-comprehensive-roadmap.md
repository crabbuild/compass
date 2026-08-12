# Compass query recall, ranking, and intent accuracy roadmap (phased, actionable, local-first)

## 1) Executive objective

Increase the quality of natural-language and graph-oriented query outcomes in Compass without adding online external services.

Primary outcomes:

- higher recall on both node and intent queries,
- better top-k relevance (`MRR`, `Recall@k`, `nDCG`, precision),
- stronger intent routing (callers/callees/impact/path/search),
- lower ambiguity-driven false positives,
- stable and bounded latency.

This plan is for the current workspace and existing ownership boundaries.

## 2) What Compass already has (and what we should use)

Use existing primitives first. This avoids greenfield redesign:

- `compass-cli/src/lib.rs::command_natural_query`: current NL entrypoint.
- `crates/compass-query/src/traversal.rs`:
  `query_graph_text`, `query_graph_text_page`, `query_terms`, `render_shortest_path`.
- `crates/compass-query/src/score.rs`:
  `score_nodes`, `find_node`, `pick_seeds`, `pick_scored_endpoint`.
- `crates/compass-query/src/code_query.rs`:
  `search`, `callers`, `callees`, `impact`, `node_trail`, `explore`.
- `crates/compass-query/src/text.rs`:
  stopword filtering, tokenization helpers, diacritic normalization helper.
- `crates/compass-query/src/relevance.rs` + `crates/compass-query/tests/relevance_qualification.rs`:
  metric and qualification engine already exists.
- `scripts/qualify_query_relevance.py`: automation for end-to-end score capture.
- Graph backends:
  JSON and Store execution paths (`CodeQueryEngine::backend`) are already shared by design.

Hard constraints:

- local-first execution,
- no runtime model/embedding/vector DB dependencies,
- deterministic output for stable inputs and same graph digest,
- bounded work and bounded truncation,
- explicit ambiguity and no silent guess when intent is unresolved.

## 3) Quality baseline and targets

Compass already emits most required measures in `relevance.rs`; use these as the gating contract:

- `success_at_1`
- `mrr_at_10`
- `recall_at_5`, `recall_at_20`
- `precision_at_10`, `ndcg_at_10`
- `intent_macro_f1` and per-intent precision/recall/F1
- `edge_precision`, `edge_recall`, `edge_kind_precision`, `edge_kind_recall`
- `edge_direction_precision`, `edge_direction_recall`
- `path_acceptance_rate`, `mean_accepted_path_rank`
- `accepted_ambiguity_recall`, `no_answer_precision`, `false_positive_rate`
- runtime: latency p50/p95 plus `WorkCounts`

Baseline and target thresholds should be saved per branch:

- never regress `no_answer_precision`, `false_positive_rate`, and direction precision without explicit exception.
- target recall lift per incremental phase:
  - +10–20% `recall@20` on intent + noisy/typo sets where applicable,
  - +5–15% lift in `mrr@10` / `ndcg@10` on reviewed corpus.
- p95 latency guard: phase-specific non-regression (recommended start: <= +10%, with explicit tuning to recover).

## 4) Target architecture (intent-aware IR stack)

Introduce a four-stage deterministic pipeline while preserving existing query commands:

```text
NL question + explicit context
  -> Normalization
  -> Intent plan (intent + confidence + slots)
  -> Recall assembler (multi-channel, bounded, provenance-tagged candidates)
  -> Ranking profile (intent-aware features + deterministic tie policy)
  -> Execution routing (search/callers/callees/impact/path/fallback)
  -> Response + diagnostics + work counters
```

This is explicitly a staged **local IR pipeline**, not a neural search stack.

## 5) Data model additions (internal only)

Add in `compass-query`:

`intent.rs` (new)

- `QueryIntent` (`Search`, `Callers`, `Callees`, `Impact`, `Path`, `Explain`, `Unknown`)
- `QueryIntentPlan`
  - `intent`
  - `confidence: u8` (0..100)
  - `symbols: Vec<String>`
  - `direction: Option<DirectionHint>`
  - `limits: Option<u32>`
  - `raw_terms: Vec<String>`
  - `parse_trace: Vec<String>` (debug-only)
- deterministic conflict and fallback flags.

`recall.rs` (new)

- `CandidateSource`:
  `ExactId`, `ExactName`, `NameAlias`, `TermIndex`, `RelationSeed`, `PathSeed`, `Fuzzy`.
- `CandidateRecord`:
  node id, source tags, per-source rank, explanation features.
- `RecallBudget`:
  global cap, per-source cap, per-term cap, fuzzy cap, truncation reason.

`rank_profile.rs` (new, optional)
- `RankProfileId`: `query-ranker/1` baseline compatibility, `query-ranker/2` enhanced lexical, `query-ranker/3` intent-aware.
- deterministic feature vector and weight set.

`WorkCounts` extension:

- `intent_candidates`, `term_candidates`, `seed_candidates`,
  `relation_seed_candidates`,
  `fuzzy_candidates`,
  `postings_scanned`,
  `candidates_deduped`,
  `rank_feature_count`.

## 6) Retrieval and ranking strategy (what is changed and why)

### 6.1 Intent planning

Intent parser produces an explicit plan before any ranking.

Signals (deterministic, weighted):

- verb phrases: `callers`, `called by`, `callees`, `what calls`, `depends on`, `impact`, `path from`, `route from/to`, `find`.
- symbol shape: IDs and qualified names, method calls (`Class::m`), file-like tokens.
- directional cues: `from`, `to`, `incoming`, `outgoing`, `upstream`, `downstream`.
- context cues: explicit context and inferred context from existing `text` helpers.

Decision policy:

- confidence >= threshold (e.g., 65): dispatch by intent.
- confidence < threshold: fallback to current traversal behavior with no behavior guess.

### 6.2 Multi-channel recall assembly (recall-first)

All sources are bounded and provenance-tagged before ranking.

Channels, in priority order:

1. Exact id and exact normalized name/alias.
2. Canonical name/label lookup (`nodes_by_normalized_name`).
3. Term index channel:
   - JSON: FTS candidate IDs
   - Store: immutable postings (`nodes_for_terms`).
4. Relation-seeded channel for directed intents (`callers`, `callees`, `impact`, `path`).
5. Optional bounded fuzzy channel (only if insufficient high-confidence coverage).

Budgeting:

- `max_candidates_total`,
- `max_per_source`,
- `max_terms`,
- `max_fuzzy_total`,
- `max_fuzzy_token_len`,
- explicit truncation reason captured in diagnostics/`WorkCounts`.

### 6.3 Ranking profiles

Replace single scoring stack with profile-based feature scoring:

- lexical:
  exact/prefix/substr in normalized label, id, and source-backed context,
- exactness:
  exact qualified-name hit, source-backed exact alias hit,
- intent fit:
  seed alignment with edge direction and operation intent,
- structural penalty/reward:
  degree weighting capped + trusted/heuristic split,
- ambiguity penalty:
  multiple unresolved symbols, conflicting cues.

Tie breaker order must remain deterministic:

1. score
2. source-backed confidence
3. semantic reliability
4. generated/test artifact penalty
5. degree
6. label length
7. stable node-id ordering

### 6.4 Intent-aware execution

Route to explicit operations where plan confidence is sufficient:

- `Callers` -> `CodeQueryEngine::callers`
- `Callees` -> `CodeQueryEngine::callees`
- `Impact` -> `CodeQueryEngine::impact`
- `Path` -> `CodeQueryEngine::node_trail`
- `Search` -> search + ranked result
- `Explain` -> existing explain path if available
- fallback: `query_graph_text_page`

This keeps deterministic behavior and avoids guessing when symbolic resolution fails.

## 7) Phased plan (highly actionable)

Each phase includes owners, files, and explicit acceptance checks.

## Phase 0 — Baseline hardening and safety gates (Week 1)

Objective:
Create trustworthy deltas before changing ranking/recall behavior.

Tasks:

1. Lock and export current baseline with:
   - `scripts/qualify_query_relevance.py`
   - `cargo test -p compass-query --test relevance_qualification --locked`
2. Normalize semantic parity checks:
   avoid byte-by-byte JSON equality for Store/JSON; compare canonical semantic fields (operation/limits/truncation/ordered node-edge-path IDs/diagnostic code/message).
3. Add deterministic query-plan diagnostics in test-only artifacts:
   intent label, candidate source split, truncation codes, rank profile id.
4. Add targeted fixtures for:
   - diacritic variants,
   - no-answer and ambiguous queries,
   - typo/noisy variants,
   - path-direction edge cases.

Acceptance:

- all current qualification tests are deterministic,
- no unintentional parity failures from serialization formatting changes,
- no phase 0 behavior regression.

Owner files:

- `crates/compass-query/tests/relevance_qualification.rs`
- `crates/compass-query/src/relevance.rs`
- `scripts/qualify_query_relevance.py`

## Phase 1 — Deterministic intent parser (Week 1–2)

Objective:
Turn implicit intent into explicit executable plan.

Tasks:

1. Add `crates/compass-query/src/intent.rs`.
2. Parse query into:
   - intent,
   - symbols,
   - direction/depth/limit clues,
   - confidence + rationale.
3. Add unit tests:
   - strong intent positives,
   - ambiguous/contradictory intents,
   - confidence boundary behavior.
4. Add parser integration point in `command_natural_query` for high-confidence paths.

Acceptance:

- phase-1 intent precision improves on intent-labeled fixture slice,
- low-confidence path behavior remains fallback-only and stable.

Owner files:

- `crates/compass-query/src/intent.rs`
- `crates/compass-query/src/lib.rs` or CLI dispatch point
- `crates/compass-cli/src/lib.rs`

## Phase 2 — Multi-channel recall multiplexer (Week 2–3)

Objective:
Improve recall before ranking decisions.

Tasks:

1. Add `crates/compass-query/src/recall.rs`:
   collect candidates from source channels and dedupe deterministically by node-id.
2. Implement bounded per-source and global caps.
3. Keep relation-seeded candidates for intent operations:
   caller/callee/impact/path.
4. Track source tags and truncation reasons in debug artifacts.

Acceptance:

- `recall@20` lifts on noisy and intent classes,
- no broad false-positive explosion,
- hard caps remain enforced under adversarial queries.

Owner files:

- `crates/compass-query/src/recall.rs` (new)
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/traversal.rs` (seed handoff)

## Phase 3 — Ranking profiles and feature model (Week 3–4)

Objective:
Improve top-k quality while preserving deterministic ordering.

Tasks:

1. Split baseline ranking into profile-based scoring.
2. Add feature extraction pipeline used by all profiles.
3. Keep legacy ranking as profile `query-ranker/1` and implement `query-ranker/2/3`.
4. Add profile selection based on intent confidence + query class.
5. Keep explainability via compact internal score-vector summary for tests.

Acceptance:

- measurable lift in `mrr@10` and `precision@10`,
- deterministic tie behavior on equal score cases proven by tests.

Owner files:

- `crates/compass-query/src/score.rs`
- `crates/compass-query/src/rank_profile.rs` (or merged with `score.rs`)
- ranking tests in `crates/compass-query/tests/`

## Phase 4 — Intent-aware routing and structured operation execution (Week 4–5)

Objective:
Map intent to the appropriate high-signal endpoint.

Tasks:

1. CLI/command dispatch:
   connect intent plan -> endpoint in `code_query`.
2. Preserve existing behavior for:
   low-confidence,
   unresolved symbols,
   contradictory cues.
3. Ensure same response envelopes and truncation semantics.

Acceptance:

- intent macro-F1 and per-intent recall improve,
- no-answer precision holds.

Owner files:

- `crates/compass-cli/src/lib.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/cql.rs` (if any cross-impacts on API path)

## Phase 5 — Structural precision hardening (Week 5–6)

Objective:
Improve edge-direction and path correctness so precision gains do not hide wrong orientation.

Tasks:

1. Add directional intent validation for callers/callees/path queries.
2. Validate endpoint compatibility before expensive path traversal.
3. Add explicit diagnostics for direction mismatch / unresolved endpoint / budgeted truncation.
4. Add tests for directed/undirected and reverse rendering cases.

Acceptance:

- `edge_direction_precision` and `path_acceptance_rate` improve or remain stable,
- no regression in `edge_direction_recall`.

Owner files:

- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/traversal.rs`
- `crates/compass-query/tests/` path and traversal tests.

## Phase 6 — Bounded typo/fuzzy recall channel (Week 6)

Objective:
Recover from misspellings and near-miss user phrasing without opening recall noise.

Tasks:

1. Add fuzz candidate source only when:
   - top-stage coverage is low,
   - query includes long enough tokens,
   - candidate cap headroom exists.
2. Restrict fuzzy operations:
   symbol length >= 4,
   edit distance 1 or 2 only where measured beneficial,
   max suggestions per token.
3. Add strict penalties and optional source tags to avoid over-ranking fuzzy hits.

Acceptance:

- recall gains on noisy variants,
- no degradation in no-answer precision and path false positives.

Owner files:

- `crates/compass-query/src/recall.rs`
- `crates/compass-query/src/intent.rs`
- `crates/compass-query/tests/relevance_qualification.rs`

## Phase 7 — Performance and production hardening (Week 7)

Objective:
Keep gains without p95 latency drift and memory growth.

Tasks:

1. Add stage timing:
   normalization, intent, recall, ranking, execution.
2. Profile and cap expensive allocations:
   candidate vectors, maps, token expansions.
3. Add per-graph/version caches where safe:
   normalized query token cache with bounded capacity and invalidation by graph identity.
4. Run stress and fuzz tests for adversarial input length and symbol count.

Acceptance:

- p50/p95 within phase budget,
- memory/time remains stable over repeated query bursts.

Owner files:

- `crates/compass-query/src/lib.rs`
- `crates/compass-query/src/text.rs`
- `crates/compass-cli/src/lib.rs`

## 8) Validation and gating (mandatory at each phase)

After each phase:

1. Run targeted tests first:
   - `cargo test -p compass-query --test relevance_qualification --locked`
   - `cargo test -p compass-query --locked`
2. Run qualification runner:
   - `python3 scripts/qualify_query_relevance.py` (external benchmark graph + fixed seeds)
3. Run compatibility checks:
   - JSON/Store parity test subset,
   - executable subset repeated run,
   - no semantic regression in backend comparison.
4. Only after passing query gates run broader workspace checks required by AGENTS:
   - `cargo test --workspace --lib --bins --locked`
   - `cargo clippy --workspace --lib --bins -- -D warnings`
   - `cargo fmt --all -- --check`
5. Record phase metric deltas in `PERFORMANCE.md` and PR summary.

Note:
When running Cargo checks in this repository, use a per-checkout `CARGO_TARGET_DIR`
(example: `/Volumes/Workspace/crabbuild-target/compass-main`) as required by repo
operating policy.

## 9) Instrumentation and observability

Add debug-only fields to `QueryObservation`/relevance artifacts only:

- planner version + rank profile id,
- final intent + confidence,
- candidate channel counts,
- truncation reasons,
- stage timing buckets.

Do **not** change public response contracts unless explicitly versioned.

Also persist a canonical query-level "explain snippet" for quality triage:

- matched verb intent,
- resolved symbols,
- source channels that contributed to top candidates,
- why fallback happened.

## 10) Rollout and rollback strategy

Recommended rollout:

1. Dark run profile `query-ranker/2` under tests only.
2. Shadow mode:
   execute legacy + new flow for same corpus, compare metrics only.
3. Partial rollout:
   intent routing enabled only for high-confidence intent classes.
4. General rollout:
   broader intent classes + conservative fuzzy.
5. Default profile switch only after two green cycles.

Rollback:

- keep legacy `query-ranker/1` + traversal fallback wiring available behind planner/ranker flags in code;
- revert output routing from intent endpoints to traversal if safety or precision budgets fail.

## 11) Non-negotiable failure criteria

Stop and rebaseline if any phase causes:

- ambiguity-to-answer conversion increases,
- explicit `NoMatch` cases become non-empty without intent evidence,
- deterministic output order changes without profile id and contract decision,
- unbounded candidate growth beyond caps.

## 12) Concrete next deliverables (this cycle)

- Finalize baseline metrics snapshot in `scripts/qualify_query_relevance.py` outputs.
- Land `intent.rs`, `recall.rs`, and rank-profile wiring in `compass-query`.
- Integrate intent routing in `command_natural_query`.
- Add parity-safe canonical assertion checks in relevance tests.
- Deliver phase 1 + phase 2 behind feature gate, gather metrics, then proceed with phase 3.
