# Compass Query: Phased Design for Performance, Recall, and Intent Accuracy

## Objective

Increase query recall, intent routing accuracy, ranking quality, and latency stability in Compass without adding external services, while preserving local-first behavior and backend parity.

Non-goal:
- Do not add online embeddings, LLM ranking at runtime, or external vector DB dependencies.

## Hard constraints (must hold across all phases)

- Deterministic outputs for the same inputs and graph digest.
- Explicit bounded truncation and explainable fallbacks.
- No hidden symbol guessing; explicit `NoMatch`/`AmbiguousMatch` remains.
- Local-first and offline-only at query time.
- Keep `query-...` public contracts stable unless version-bumped.
- Keep Store/JSON equivalence where the product currently promises it.

## What “good query quality” means here

Use existing harness metrics in `crates/compass-query/src/relevance.rs` as the source of truth:

- Node retrieval: `success_at_1`, `mrr_at_10`, `recall_at_5`, `recall_at_20`, `precision_at_10`, `ndcg_at_10`
- Intent: `intent_macro_f1` + per-intent precision/recall/F1 (`IntentMetrics`)
- Structure: `edge_precision`, `edge_recall`, `edge_kind_*`, `edge_direction_*`
- Path quality: `path_acceptance_rate`, `mean_accepted_path_rank`
- Robustness: `accepted_ambiguity_recall`, `no_answer_precision`, `false_positive_rate`
- Runtime: `latency_p50_micros`, `latency_p95_micros`, `WorkCounts`

Guardrail: no phase can be accepted if it materially degrades no-answer precision or safety metrics without explicit approval.

## Current implementation boundaries in Compass

- Natural-language command entry: `crates/compass-cli/src/lib.rs::command_natural_query`
- Current default flow: query → `query_graph_text_page` (traversal path)
- Structured operations already exist in `CodeQueryEngine`:
  `search`, `callers`, `callees`, `impact`, `node_trail` in `crates/compass-query/src/code_query.rs`
- Existing scoring primitives:
  `crates/compass-query/src/score.rs`
- Qualification loop and fixtures:
  `crates/compass-query/tests/relevance_qualification.rs`,
  `crates/compass-query/tests/fixtures/relevance/judged.json`
- Relevance harness already supports ranker/planner version fields via `qualification_report`.

The right change set is therefore to add a parser+planner layer and a recall/ranking
pipeline between question and existing execution ops, while retaining all current operations and fallback behavior.

## Target architecture (deterministic, staged, reversible)

### Stage 1 — Intent plan
Parse question into an explicit `QueryIntentPlan`:

- `intent`: `Search | Callers | Callees | Impact | Path | Explain | Unknown`
- `symbols`: extracted concrete symbol candidates (ids, qualified names, method-like tokens)
- `raw_terms`: normalized lexical tokens
- `direction`: incoming/outgoing/both
- `limits`: max rows, max edges/nodes hints
- `confidence`: 0..100
- `reasons`: deterministic parse trace for diagnostics

### Stage 2 — Recall assembly
Build a bounded candidate pool with source provenance before ranking:

1. exact id lookup
2. normalized exact name / alias lookup
3. term index hits (JSON FTS / Store `nodes_for_terms`)
4. intent-seeded relation neighbors (for caller/callee/path)
5. bounded fuzzy fallback only when confidence is low or first pass is insufficient

Every candidate carries provenance (`CandidateSource`) and is deduped deterministically.

### Stage 3 — Ranking
Replace “single-purpose score” with feature-based scoring profile:

- lexical match (exact/prefix/subword/coverage)
- exactness & confidence features (qualified symbol hit, endpoint certainty)
- evidence quality (source-backed vs inferred/heuristic)
- intent fit (callers/callees/path compatibility)
- structure signals (bounded degree signals, hub control)
- ambiguity penalties (test/generated/unresolved slots)

Ranker profile IDs (`query-ranker/1`, `/2`, ...) are emitted in qualification and
all result changes can be rolled back by profile switch.

### Stage 4 — Execution routing
- `Callers` plan -> `CodeQueryEngine::callers`
- `Callees` plan -> `CodeQueryEngine::callees`
- `Impact` plan -> `CodeQueryEngine::impact`
- `Path` plan -> `CodeQueryEngine::node_trail` when both endpoints resolve
- `Search`/fallback -> search + ranked candidate pipeline
- low-confidence/ambiguous/invalid -> legacy traversal fallback

### Stage 5 — Observability
- Extend internal telemetry for diagnostics (not public contract changes):
  `intent`, `operation`, `planner_version`, `ranker_version`, truncation reasons,
  per-stage counters and timings.
- Keep telemetry bounded and deterministic.

## Data models to add

`crates/compass-query/src/intent.rs`

```rust
pub enum QueryIntent {
    Search, Callers, Callees, Impact, Path, Explain, Unknown
}

pub struct QueryIntentPlan {
    pub intent: QueryIntent,
    pub confidence: u8,             // 0..=100
    pub raw_terms: Vec<String>,
    pub symbols: Vec<String>,
    pub direction: Option<RelationDirection>, // incoming|outgoing|both
    pub result_limit_hint: Option<usize>,
    pub endpoint_limit_hint: Option<usize>,
    pub reasons: Vec<String>,
    pub parse_trace: Vec<String>,
}
```

`crates/compass-query/src/recall.rs` (new)

```rust
pub enum CandidateSource {
    ExactId, ExactName, Alias, NameIndex, TermIndex, RelationSeed, Fuzzy, HeuristicFallback
}

pub struct CandidateRecord {
    pub node_id: String,
    pub source_priority: u8,
    pub source_ranks: BTreeMap<CandidateSource, usize>,
    pub feature_hits: BTreeMap<String, f64>,
}

pub struct RecallBudget {
    pub max_total_candidates: usize,
    pub max_per_source: usize,
    pub max_candidates_per_term: usize,
    pub max_fuzzy_candidates: usize,
}
```

`crates/compass-query/src/score.rs`

- Move current heuristics into explicit feature extraction and weighted aggregation.
- Keep deterministic tiebreak chain stable and explicit.

## Phased implementation plan

### Phase 0 — Baseline hardening (1 week)

Goal: lock a trustworthy comparison point before model changes.

Tasks:
1. Make backend parity checks semantic (IDs/order/diagnostics) instead of raw JSON-byte equality for queries.
2. Freeze reviewed corpus baseline and report it as artifact.
3. Add explicit assertions for edge-direction, recall@20, no-answer precision, path acceptance, and latency p50/p95.
4. Record benchmark scripts in `PERFORMANCE.md` for reproducibility.

Acceptance:
- `cargo test -p compass-query --test relevance_qualification --locked` deterministic.
- Executable/reviewed fixture checks pass with deterministic report outputs.

### Phase 1 — Intent parser + planner (1–2 weeks)

Goal: move from implicit intent to explicit query plans.

Tasks:
1. Add `crates/compass-query/src/intent.rs`.
2. Implement deterministic parsing for:
   - callers/callees phrases,
   - path/from-to phrasing,
   - impact/downstream/upstream,
   - plain search.
3. Extract symbol tokens robustly (sha256 IDs, `CamelCase`, `snake_case`, `pkg.mod.fn`, `A::B`).
4. Add confidence scoring and `Unknown` fallback.
5. Add parser unit tests (ambiguous cases, false triggers, edge cases).

Acceptance:
- intent macro-F1 improves on curated intent slices.
- fallback behavior unchanged for low-confidence or non-actionable input.

### Phase 2 — Multi-channel recall layer (2–3 weeks)

Goal: lift recall before ranking decisions.

Tasks:
1. Add `recall.rs` that merges sources in fixed priority order.
2. Add per-source and global budgets + truncation reasons.
3. Dedupe by canonical node ID deterministically.
4. Keep source tags on every candidate for auditability.
5. Share term normalization semantics between Store and Materialized paths.

Acceptance:
- measurable lift in `recall_at_20` on fuzzy/intent/noise sets.
- no unbounded expansion and no unstable ordering.

### Phase 3 — Ranking profile v2 (2 weeks)

Goal: improve top-k quality without changing recall shape unexpectedly.

Tasks:
1. Introduce feature vectors and deterministic feature weights.
2. Add rank profile IDs and keep previous profile behavior intact.
3. Add regression tests for tie-break behavior and deterministic ordering.

Acceptance:
- improvements in `mrr_at_10`, `precision_at_10`, `ndcg_at_10` with stable/no-regression guardrails.

### Phase 4 — Intent-aware execution routing (1–2 weeks)

Goal: execute the intended operation when intent is clear.

Tasks:
1. Integrate planner in `command_natural_query`.
2. Route to `CodeQueryEngine::callers`, `::callees`, `::impact`, `::node_trail` when confident.
3. Keep traversal fallback when confidence/ambiguity boundary fails.

Acceptance:
- no-input or ambiguous cases still return deterministic fallback.
- `intent_macro_f1` and per-intent routing precision improve.

### Phase 5 — Structural quality hardening (2 weeks)

Goal: reduce direction/path/edge mismatch.

Tasks:
1. Add direction-aware edge kind ranking for callers/callees.
2. Path endpoint compatibility checks (A→B direction/semantics) before expensive path search.
3. Add rejected-path diagnostics (`relation mismatch`, `endpoint unresolved`, budget-limited).

Acceptance:
- edge direction precision/recall and path acceptance improve or hold.
- ambiguity and false positives remain controlled.

### Phase 6 — Fuzzy + typo resilience (1 week)

Goal: recover from noisy input while limiting over-match.

Tasks:
1. Add bounded edit-distance recovery only for symbols/terms not already covered by exact channels.
2. Restrict by token length and source reliability.
3. Add per-query caps: max fuzzy suggestions and max tokens fed to fuzzy.

Acceptance:
- recall lift for misspellings/noise queries.
- no-answer precision and path correctness remain within acceptance bands.

### Phase 7 — Performance hardening and caching (1 week)

Goal: make gains sustainable.

Tasks:
1. Add per-query stage timing (intent/recall/rank/execute) and candidate counters.
2. Optional digest-scoped caches for normalization and parsed intent.
3. Ensure memory and lock contention profiles are bounded.

Acceptance:
- p95 remains in agreed budget.
- no growth in latency tail under bounded query sets.

### Phase 8 — Controlled rollout and rollback (continuous)

Goal: gradual production-like activation.

Tasks:
1. Default to profile versions `query-planner/1`, `query-ranker/1`.
2. Gate `query-planner/2` and `query-ranker/2` via environment or benchmark profile in a controlled run.
3. Track each phase delta in qualification report artifacts.

Acceptance:
- can revert by profile switch with no behavioral drift in public contracts.

## Command/test command list

- `python3 scripts/qualify_query_relevance.py`
- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test relevance_qualification --locked`
- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-cli --test code_query_cli --locked`
- Re-run specific phase tests for parity and behavior checks.

## Key implementation sequencing

1. **First pass:** finish Phase 0 + 1 (planner/inference).
2. **Second pass:** recall multiplexer + ranking profile.
3. **Third pass:** routing + structural quality.
4. **Final:** fuzzy + performance.

This sequencing keeps risk low because intent routing is controlled by confidence and is reversible.

## Risks and explicit mitigations

- Over-recall from fuzzy channels
  → keep strict caps, evidence penalties, and ambiguity guardrails.
- Wrong intent routing
  → keep confidence threshold + always preserve fallback.
- Backend parity divergence
→ canonicalize source normalization and compare semantic fields as primary parity oracle.
- Latency regression
→ per-stage timing and profile rollout with p95 budgets.

## Why this is feasible in Compass today

Compass already owns:
- deterministic graph operations and traversal,
- existing execution ops for intent-specific actions,
- a complete evaluation harness,
- and deterministic storage engines (JSON + Store).

This plan adds structure around what already exists instead of replacing the engine, which is exactly what gives good “Google-like retrieval” behavior while staying local-first and auditable.
