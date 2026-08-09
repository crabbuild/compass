# Compass Query: Phased Technical Design for Performance, Recall, and Intent Accuracy

_Status: Draft — 2026-08-07_

## Objective

Improve natural-language and graph-oriented query quality with measurable gains in:

- recall for relevant nodes/edges/paths,
- top-k ranking quality,
- intent routing accuracy (callers/callees/impact/path/search),
- and bounded latency,
without adding runtime external dependencies, embeddings, or online model calls.

This design is local-first and deterministic and should run with either JSON or Store backends.

## Why this is needed

Current behavior tends to route broad natural-language questions through a single generic traversal-style path and relies on a single relevance model. In practice this can:

- miss relevant candidates (low recall),
- rank useful results below noisy ones,
- execute the wrong operation when intent is clear (e.g., callers vs search),
- and spend unnecessary work on broad token expansions.

The architecture already has the right primitives; the gap is orchestration.

## Existing Compass capabilities we should reuse

- `crates/compass-cli/src/lib.rs::command_natural_query` handles NL entry.
- `crates/compass-query/src/code_query.rs` contains structured operations:
  `search`, `callers`, `callees`, `impact`, `node_trail`, `explore`.
- `crates/compass-query/src/score.rs` contains scoring primitives (`find_node`, `score_nodes`, etc.).
- `crates/compass-query/src/traversal.rs` has existing traversal/search entry points.
- `crates/compass-query/src/text.rs` and `query_terms` helpers already cover normalization + tokenization.
- Relevance infrastructure already exists in `crates/compass-query/src/relevance.rs` and `crates/compass-query/tests/relevance_qualification.rs`.
- Query execution backends are already separated (`CodeQueryEngine::backend`) and testable.

## Hard constraints (must hold)

1. Deterministic output for same input + same graph identity.
2. Explicit ambiguity and bounded truncation.
3. No silent symbol invention; unresolved/multiple matches stay explicit.
4. Preserve existing public contracts unless compatibility bump.
5. No runtime remote ML/embeddings/vector DB.
6. Keep JSON/Store behavior equivalent where parity is a product commitment.
7. Keep local-first behavior and resource usage bounded.

## Canonical quality contracts to optimize

Use `RelevanceMetrics` as first-class acceptance gates:

- `success_at_1`, `mrr_at_10`, `precision_at_10`, `ndcg_at_10`
- `recall_at_5`, `recall_at_20`
- `intent_macro_f1` and per-intent precision/recall/F1
- `edge_precision`, `edge_recall`, `edge_direction_precision`, `edge_direction_recall`
- `path_acceptance_rate`, `mean_accepted_path_rank`
- `accepted_ambiguity_recall`, `no_answer_precision`, `false_positive_rate`
- runtime: p50/p95 latency + structured `WorkCounts`

No phase may regress `no_answer_precision` or `false_positive_rate` without explicit acceptance.

## Proposed target architecture (Google-like retrieval style, local deterministic)

Implement a 4-stage local query IR:

1. Normalization + intent extraction
2. Recall assembly (multi-channel bounded candidate retrieval)
3. Ranking (feature vector + profile)
4. Execution routing (intent-aware operator dispatch)

```
NL input
  -> normalize + token graph extraction
  -> intent plan (query class + slots + confidence)
  -> candidate assembler (exact/id + lexical + relation seeds + optional fuzzy)
  -> ranker profile (deterministic scoring)
  -> execution endpoint dispatch
  -> result envelope + diagnostics
```

## Baseline data model additions

Create/extend in `crates/compass-query`:

- `intent.rs` (new): plan extraction and intent classification.
- `recall.rs` (new): deterministic candidate multiplexing + provenance.
- `query_plan.rs` (optional, or inline): normalized query IR used across CLI and query engine.
- `score_profile.rs` (optional, or extend `score.rs`): ranker versioning and feature definitions.

Minimal core structs:

```rust
pub enum QueryIntent { Search, Callers, Callees, Impact, Path, Explain, Unknown }

pub struct QueryIntentPlan {
    pub intent: QueryIntent,
    pub confidence: u8, // 0..=100
    pub raw_terms: Vec<String>,
    pub symbols: Vec<String>,
    pub direction: Option<RelationDirection>,
    pub depth_hint: Option<u8>,
    pub limit_hint: Option<usize>,
    pub fallback_reason: Option<String>,
}

pub enum CandidateSource {
    ExactId,
    ExactName,
    Alias,
    NameIndex,
    TermIndex,
    RelationSeed,
    Fuzzy,
    HeuristicFallback,
}

pub struct CandidateRecord {
    pub node_id: NodeId,
    pub source_ranks: BTreeMap<CandidateSource, usize>,
    pub score_features: BTreeMap<&'static str, f32>,
    pub evidence_tags: BTreeSet<String>,
}

pub struct CandidateBudget {
    pub total: usize,
    pub per_source: usize,
    pub per_term_postings: usize,
    pub max_fuzzy_terms: usize,
    pub max_fuzzy_hits: usize,
}
```

## Phase plan

## Phase 0 – Stability baseline and parity hardening (Week 1)

Goal: make later gains trustworthy.

### Tasks

1. Freeze current metrics per graph fixture and produce reproducible reports.
2. Normalize parity checks between JSON/Store by semantic fields (operation, ordered ids, truncation codes), not brittle full JSON dumps.
3. Add explicit truncation reason reporting and candidate source tags in test artifacts.
4. Add targeted fixtures for:
   - diacritic and case variations,
   - ambiguous symbol names,
   - empty/no-answer cases,
   - short-path/caller direction edge cases.

### Must-pass metrics

- Deterministic `relevance_qualification` runs on both reviewed and executable subsets.
- No backslide in no-answer precision.

### Owner

- `crates/compass-query/tests/relevance_qualification.rs`
- `crates/compass-query/src/relevance.rs`
- `scripts/qualify_query_relevance.py`

## Phase 1 – Deterministic intent parser (Weeks 1–2)

Goal: infer user intent before ranking/execution.

### Mechanism

Rule-based deterministic parser (no model):

- Intent keywords and operators:
  - `callers` / `called by` / `who calls`
  - `callees` / `calls`
  - `impact of` / `depends on` / downstream/upstream
  - `path from` / `path between` / `routes from to`
  - generic `find`/`search`
- Symbol extraction patterns:
  - `A::B`, `pkg.mod.fn`, `Class.method`, camel/pascal snake fragments, file names
- Cue conflicts produce reduced confidence and explicit `Unknown` behavior.

### Scoring

- Add `intent_confidence` in 0..100 based on matched cues and symbol clarity.
- Route only when confidence exceeds threshold (example: 65).
- Preserve fallback to existing flow when confidence low/ambiguous.

### Owner files

- Add `crates/compass-query/src/intent.rs`
- Export from `crates/compass-query/src/lib.rs`
- Integrate in `crates/compass-cli/src/lib.rs`

### Acceptance

- Intent precision improves on curated intent-labelled fixture slice.
- Low-confidence cases remain identical or better than baseline traversal.

## Phase 2 – Multi-channel recall assembly (Weeks 2–3)

Goal: maximize recall in a bounded, explainable way.

### Candidate channels (fixed priority)

1. Exact ID match
2. Exact normalized symbol/name
3. Name index / alias index lookups
4. Term index hits (JSON FTS and Store postings)
5. Relation seeded candidates (for callers/callees/impact/path intents)
6. Fuzzy/typo channel only if coverage is low

### Engine behavior

- All candidates are deduped by canonical node id.
- Every candidate carries `CandidateSource` provenance.
- Global and per-source caps enforced through `CandidateBudget`.
- Always deterministic ordering after normalization and before ranking.

### Important recall correctness fix to include now

Align normalization behavior across all paths (`text.rs`, snapshot term matching in graph store, and index query terms):

- lowercase + unicode normalization + diacritic stripping + whitespace policy.
- Avoid asymmetric handling that causes Store/JSON recall drift.

### Owner files

- New `crates/compass-query/src/recall.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-graph/src/snapshot.rs` (term lookup consistency)
- `crates/compass-query/src/text.rs`

### Acceptance

- Recall@20 improves on noisiest and intent-specific query subset.
- No unbounded candidate growth under long adversarial queries.

## Phase 3 – Ranking profile v2 (Weeks 3–4)

Goal: raise top-k quality without reducing precision drastically.

### Feature model (deterministic)

For each candidate compute a compact vector:

- lexical:
  - exact label hit, exact alias hit, prefix/substring coverage, token coverage ratio,
  - normalized symbol match on qualified names.
- intent-fit:
  - caller/callee direction compatibility,
  - path endpoint plausibility,
  - impact depth coherence.
- evidence:
  - source-confidence (materialized evidence vs heuristic),
  - symbol disambiguation count,
  - unresolved slot penalty.
- structural:
  - capped degree / hub penalty,
  - trusted relationship type weights,
  - path-local constraints.
- fuzzy penalties:
  - distance-based soft penalty and low base score.

### Implementation

- Keep current behavior as `query-ranker/1` for rollback.
- Add `query-ranker/2` and `query-ranker/3` with stricter feature weighting.
- Tie-break order deterministic:
  1. score
  2. evidence confidence
  3. candidate provenance reliability
  4. deterministic secondary hash/id ordering

### Owner files

- `crates/compass-query/src/score.rs`
- optionally `crates/compass-query/src/score_profile.rs`
- regression tests in `crates/compass-query/tests/`

### Acceptance

- measurable improvements in `mrr_at_10`, `ndcg_at_10`, `precision_at_10`.

## Phase 4 – Intent-aware execution routing (Weeks 4–5)

Goal: answer the intent, not just "search".

### Routing matrix

- `Callers` -> `CodeQueryEngine::callers`
- `Callees` -> `CodeQueryEngine::callees`
- `Impact` -> `CodeQueryEngine::impact`
- `Path` -> `CodeQueryEngine::node_trail`
- `Search` or fallback -> existing traversal path

### Guardrails

- Must resolve required endpoints before invoking path/call-style ops.
- Ambiguous symbols, missing anchors, or directional conflicts revert to safe fallback.
- Return explicit diagnostics for non-execution reasons.

### Owner files

- `crates/compass-cli/src/lib.rs`
- `crates/compass-query/src/code_query.rs`

### Acceptance

- `intent_macro_f1` improves with no no-answer precision collapse.

## Phase 5 – Structural precision hardening (Weeks 5–6)

Goal: improve edge/path correctness, especially direction.

### Tasks

- Add explicit direction checks before final ranking for caller/callee/path intents.
- Add path endpoint validation and explicit mismatch reason tags.
- Penalize candidates where endpoint role does not match requested relation.

### Owner files

- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/traversal.rs`
- `crates/compass-query/src/relevance.rs`

### Acceptance

- edge direction precision/recall and path acceptance improve or remain within tolerance.

## Phase 6 – Controlled fuzzy/typo recovery (Week 6)

Goal: match noisy queries without turning recall gains into noise.

### Policy

Enable fuzzy only when:

- coverage remains below `min_coverage_ratio` after exact+relation channels,
- query tokens are long enough (e.g., >=4),
- and there is remaining candidate budget.

### Algorithm

- Generate 1-edit and selected 2-edit candidates (guarded by max edit suggestions per token).
- Use bounded Damerau/Levenshtein or transposition-aware distance.
- Only index terms that pass prefix entropy checks.
- Candidate score penalty for fuzzy source to avoid dominating exact matches.

### Owner files

- `crates/compass-query/src/recall.rs`
- `crates/compass-query/src/text.rs`

### Acceptance

- Recall improvement on typo set with non-degradation on `false_positive_rate` and `no_answer_precision`.

## Phase 7 – Performance hardening and diagnostics (Weeks 6–7)

Goal: keep query latency stable while adding quality features.

### Add

- per-stage timing: normalize, intent, recall, ranking, execution,
- per-stage counters in `WorkCounts`.
- bounded caches:
  - parsed intent cache (query+graph-digest key),
  - normalized token cache (query token + profile key),
  - optionally candidate source hints cache.
- early-stop rules for low-marginal gain ranking expansion.

### Optimizations

- avoid repeated string allocations in critical loops,
- avoid repeated graph scans on same symbol sets,
- preallocate candidate/result buffers by budget.

### Owner files

- `crates/compass-query/src/relevance.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/lib.rs`

### Acceptance

- p95 does not exceed phase budget threshold (e.g., +10% over baseline during canary; can tighten later).

## Phase 8 – Rollout and governance (Week 8)

Goal: reduce risk and keep rollback easy.

### Actions

- Add ranker/planner ids in internal diagnostics output.
- Run shadow mode: baseline profile + new profile in sampled queries.
- Enable by intent families progressively: search -> callers/callees -> path/impact.
- Keep feature flags to route full corpus by profile if needed.

### Owner files

- `crates/compass-query/src/lib.rs`
- `crates/compass-query/src/relevance.rs`
- `crates/compass-cli/src/lib.rs`

## Scoring/Ranking details (closer to Google-like behavior)

Compass cannot use external PageRank at runtime, but can approximate search-engine behavior with local, deterministic signals:

1. **Term matching depth (IR-style)**
   - exact > prefix > substring > token overlap.
2. **Query intent fitness**
   - relation and direction compatibility with requested operation.
3. **Evidence quality**
   - prefer source-backed structural evidence over heuristic matches.
4. **Canonicality/ambiguity penalty**
   - penalize names with multiple unresolved senses.
5. **Hub control**
   - cap over-influence of highly connected utility hubs.
6. **Path consistency**
   - only boost candidates aligned with valid path semantics.

A practical rank formula:

`score = 0.35*lex + 0.25*intent + 0.20*evidence + 0.10*structure + 0.10*path_fit - 0.10*fuzzy_penalty - 0.05*ambiguity`

Tune weights via relevance corpus; keep profile IDs for quick A/B.

## Test and verification matrix by phase

For each phase, run at least:

- `CARGO_TARGET_DIR=<qualification-corpus-root>/crubuild-target/compass-main cargo test -p compass-query --test relevance_qualification --locked`
- `python3 scripts/qualify_query_relevance.py`
- phase-specific targeted tests:
  - intent parser unit tests,
  - recall fixtures (noisy/typo/no-answer),
  - path/edge direction tests,
  - store vs json parity subset.

Before merge to wider scope, run mandated checks from AGENTS for affected surface.

## Risks and mitigations

- **Recall gain adds noise** -> enforce strict budgets and confidence gates.
- **Incorrect routing on edge cases** -> explicit fallback and clear diagnostics.
- **Parity drift (Store/JSON)** -> shared normalization helpers and semantic parity checks.
- **Latency drift** -> stage budgets + profiling + incremental caching with hard TTL/cap.

## Deliverables per gate

- **Gate A (end of Phase 2):** normalized recall pipeline + intent parser + richer candidates.
- **Gate B (end of Phase 4):** intent routing enabled with safe fallback.
- **Gate C (end of Phase 6):** fuzzy recovery deployed with no safety metric regression.
- **Gate D (final):** profiling, rollout controls, and rollback playbook complete.
