# Query retrieval accuracy, recall, and performance implementation design

This is a concrete engineering plan for improving Compass natural-language and
symbol query quality without adding external ML services, while preserving
deterministic local execution and current Store/JSON parity.

It is written for the current codebase and ownership boundaries in this repository:

- CLI query entry: `crates/compass-cli/src/lib.rs` (`command_natural_query`)
- Query operators and execution: `crates/compass-query/src/code_query.rs`
- Scoring/ranking: `crates/compass-query/src/score.rs`
- Text parsing/tokenization: `crates/compass-query/src/text.rs`
- Traversal and rendering: `crates/compass-query/src/traversal.rs`
- Relevance contracts/gates: `crates/compass-query/src/relevance.rs`,
  `crates/compass-query/tests/relevance_qualification.rs`,
  `crates/compass-query/tests/fixtures/relevance/judged.json`
- Gate runner: `scripts/qualify_query_relevance.py`

---

## 0) Goals and non-goals

### Goals

- Raise recall on intent-like and noisy queries.
- Improve precision at top-k (`precision@10`) and ranking quality
  (`MRR@10`, `nDCG@10`, `recall@20`).
- Make intent-aware routing work (e.g., callers/callees/path/impact) from natural
  queries.
- Keep behavior deterministic, explainable, and backend-stable.

### Non-goals

- No external embedding/vector service.
- No non-deterministic model inference.
- No contract changes to public response fields unless versioned and reviewed.

---

## 1) Existing behavior summary (what we are improving)

### Current behavior today

- `compass query` always executes `query_graph_text_page` and relies on
  lexical token scoring + BFS/DFS traversal (`traversal.rs`).
- Intent is implicit and heuristic (`infer_context_filters`), not explicit.
- `CodeQueryEngine` already has high-signal primitives for `search`, `callers`,
  `callees`, `impact`, and `node_trail`, but natural query does not route to
  these endpoints.
- Ranking is currently embedded in `score_nodes`, with strong lexical fields and
  deterministic tie-breakers but no explicit feature profile per intent.
- Relevance testing already exists, including:
  `Success@1`, `MRR`, recall/precision/nDCG, intent macro-F1,
  edge/path/no-answer metrics, and backend parity checks on compact executable
  subsets.

### What this gives us

We already have strong deterministic foundations; this work is about making query
execution more like a staged IR pipeline while staying local-first.

---

## 2) Target architecture

Implement an explicit, deterministic pipeline with four stages.

1. **Intent plan extraction**
   Parse NL query into a structured plan (`intent`, `slots`, `symbols`,
   confidence, expected graph limits).

2. **Candidate generation (recall-first)**
   Collect bounded candidate nodes from multiple sources:
   exact-id, normalized-name, FTS/store postings, path/neighbor hints, optional
   fuzzy recovery.

3. **Intent-aware ranking**
   Score candidates by explicit feature vector and deterministic tie-breakers.

4. **Materialization/execution**
   Route by intent to the right operation:
   `search`, `callers`, `callees`, `impact`, `node_trail`, fallback traversal.

The output of each stage is observable in a debug/qualification artifact (not
public contract).

---

## 3) Design principles (engineering constraints)

- **Deterministic everywhere:** stable ordering by explicit key order + stable
  graph IDs.
- **Bounded recall expansion:** every channel has a hard cap and truncation reason.
- **Explain-first on ambiguity:** when confidence is low, avoid guessing.
- **Backend neutrality:** JSON and Store must remain equivalent on canonicalized
  fields.
- **Versioned ranking behavior:** keep profile IDs in report diagnostics for easy
  rollback.

---

## 4) Data model additions

These are internal to `compass-query` execution path and do not require public
schema changes.

### 4.1 Intent plan model

`crates/compass-query/src/intent.rs`:

```rust
pub enum QueryIntent {
    Search,
    Callers,
    Callees,
    Impact,
    Path,
    Explain,
    Unknown,
}

pub struct QueryIntentPlan {
    pub intent: QueryIntent,
    pub confidence: u8,           // 0..=100
    pub symbols: Vec<String>,      // normalized symbol mentions
    pub relations: Vec<String>,    // relation hints (calls/imports/etc.)
    pub direction: Option<DirectionHint>, // incoming | outgoing | both
    pub depth_limit: Option<u32>,
    pub result_limit: Option<u32>,
    pub contexts: Vec<String>,     // explicit + inferred context filters
    pub reasons: Vec<String>,      // trace/debug fields
}
```

Rules:
- plan confidence threshold for intent dispatch is high only when both intent cues
  and symbol evidence are present.
- anything below threshold routes through existing traversal flow.

### 4.2 Candidate provenance model

Each candidate includes source provenance for diagnostics:

- `ExactId`, `ExactName`, `Alias`, `NormalizedName`, `Fts`, `TraversalSeed`,
  `PathHint`, `Fuzzy`.
- For each source we store: returned count, truncated flag, per-source latency,
  and whether dedupe collapsed duplicates.

### 4.3 Candidate feature vector (intent-aware)

Add an internal ranked feature representation in `score.rs`:

- lexical exact match score
- prefix/substr score
- token coverage
- id-match score (qualified + raw name)
- source-backed penalty/bonus
- evidence confidence (source-backed vs generated/test)
- intent relation alignment (for callers/callees/path)
- graph utility signal (degree/hub handling)
- ambiguity penalty
- path-readiness score (for path-like plans)

Total score is deterministic float sum + stable tie-breakers.

---

## 5) Intent parser: exact behavior to implement

### 5.1 Cues and weights

- `callers`, `called by`, `who calls` → `Callers` (strong)
- `callees`, `calls`, `invokes` → `Callees` (strong)
- `what does X call`, `impacts`, `used by`, `downstream` → `Impact` (medium)
- `path`, `route`, `connects`, `from A to B`, `between A B` → `Path`
- `search`, `where is`, `find`, `show`, `explain` → `Search`/`Explain`

Confidence policy:
- +30 for explicit verb-class phrase
- +20 for symbol-like token extracted (qualified name, `Class.method`, path-like)
- +20 for explicit directional words (`from`, `of`, `to`)
- +10 for explicit context (`--context` or inferred high-confidence context)
- clamp to 100
- minimum dispatch threshold (example: 65) for route-based execution

### 5.2 Symbol extraction

- Use `search_tokens` as seed and a separate symbol parser for:
  `Qualified.Name`, `::`, `.` method chains, camel/snake forms.
- Keep both strict exact tokens and normalized fallback tokens.

### 5.3 Fallback modes

- high-confidence intent → route to dedicated operation
- low-confidence intent or missing symbol(s) → existing traversal fallback
- if symbol resolution fails, report `NoMatch` diagnostic and return explicit empty set

---

## 6) Candidate generation and recall strategy

### 6.1 Ordered candidate channels (strictly deterministic)

1. exact ID/symbol direct matches
2. exact normalized name + qualified-name bucket (`CodeLookupIndex::nodes_by_normalized_name`)
3. Store/JSON index channel (`nodes_for_terms` / `node_fts`)
4. relation-seeded channel for path/call intent
5. optional fuzzy channel (bounded)

### 6.2 Bounding rules

- per-source caps:
  - `max_source_hits` (channel cap),
  - `max_candidates_total` (global),
  - `max_fuzzy_candidates`,
  - `max_terms_per_query`.
- explicit truncation reasons in work metadata:
  - `candidates_cap`
  - `postings_cap`
  - `fuzzy_cap`
  - `term_cap`

### 6.3 Fuzzy strategy (typo + noise only)

- apply only when top recall is below a small floor (example: < 2 candidates)
  and query is long enough to reduce false positives.
- use bounded edit-distance expansion on token vocabulary:
  - min token length threshold (e.g., >= 4)
  - max edit distance 1 or 2 based on token length
  - max generated variants per token
- never override exact-id and direct-name channels.

---

## 7) Ranking strategy (search vs relation-aware)

### 7.1 Shared base scoring

Base candidate scoring (all intents):
- lexical exact/prefix/substring + token coverage
- source-backed + evidence-quality signals
- test/generated penalty
- stability tie-break: evidence, semantic role, ambiguity, source-backed degree,
  label length, id

### 7.2 Intent-specific modifiers

- Callers/Callees:
  - boost source nodes with matching direction/edge family overlap.
- Path:
  - boost endpoint confidence and expected relation sequence alignment.
- Impact:
  - boost nodes with high outbound/inbound fanout in impact edge families.

### 7.3 Rank profile versioning

Set explicit rank profile IDs (for qualification and rollback), e.g.:
- `query-ranker/1` (current baseline)
- `query-ranker/2` (search improvements)
- `query-ranker/3` (path/intent-aware)

---

## 8) Materialization and routing mapping

### Query routing table

| Intent       | Preferred operation                           | Fallback |
|--------------|----------------------------------------------|----------|
| Search       | enhanced candidate ranking + existing search path | traversal |
| Callers      | `CodeQueryEngine::callers` (inbound)         | traversal |
| Callees      | `CodeQueryEngine::callees` (outbound)        | traversal |
| Impact       | `CodeQueryEngine::impact`                    | traversal |
| Path         | `CodeQueryEngine::node_trail` if 2 symbols   | traversal |
| Explain      | `render_explanation` when explicit ID/label   | traversal |
| Unknown      | existing traversal path                        | traversal |

### Multi-step path queries

- If natural path query supplies ≥2 symbols:
  execute `node_trail` directly.
- If only one resolved endpoint:
  fallback to seeded traversal with explicit relation hints.

---

## 9) Directionality and relation correctness

Current edge model is directed in `compass.graph/1` (source->target). Query quality
improves with explicit direction scoring.

Implement:
- query intent direction inference for callers/callees/path
- edge kind filtering by relation families for relation-like intent
- edge direction precision metrics tied to expected direction in fixture

---

## 10) Observability, diagnostics, and budgets

### 10.1 Work counters

Extend internal query execution observations with:
- `candidates_read`
- `postings_decoded`
- `nodes_expanded`
- `edges_expanded`
- `response_bytes`

`response_bytes` already exists; the others should be tracked where
implementation has stable meaning.

### 10.2 Stage timings

Capture per-stage durations for internal artifacts:
- parse_ms
- candidate_ms
- rank_ms
- materialize_ms

Keep them in non-contract qualification metadata.

### 10.3 Truncation taxonomy

Keep explicit reasons and make them queryable in review:
- `candidate_cap`, `postings_cap`, `relation_seed_cap`, `fuzzy_cap`,
  `path_cap`, `traversal_cap`

---

## 11) Phased rollout plan (actionable)

Each phase has explicit owners and acceptance gates.

### Phase 0 — Measurement hardening (1 week)

Owner: query team (core)

Tasks:
- baseline all existing relevance gates; document current metric set and baseline scores.
- add strict canonical parity helpers in relevance tests:
  compare semantic fields (`operation`, `limits`, `results`, `nodes`, `edges`,
  `truncated`, diagnostics) instead of broad raw JSON equivalence where brittle.
- stabilize fixture execution seeds for deterministic replay.

Acceptance:
- all existing tests in `relevance_qualification` pass.
- deterministic re-run of executable subset remains stable.
- `scripts/qualify_query_relevance.py` runs with external `CARGO_TARGET_DIR`.

### Phase 1 — Intent parser & planner (2 weeks)

Owner: `compass-query`

Tasks:
- add `intent.rs` with rule-based parser and confidence scoring
- wire parser into `command_natural_query`
- add unit tests for 200+ intent phrases (callers/callees/path/noisy/unknown)
- capture plan in non-contract test metadata

Exit:
- intent precision/recall lift on curated intent subset.
- no regression for exact ID explainability cases.

### Phase 2 — Recall multiplexer (2 weeks)

Owner: `compass-query`

Tasks:
- add bounded multi-source candidate aggregator in search/resolve path
- preserve deterministic dedupe/order.
- add `fuzzy` as opt-in channel with explicit thresholds.

Exit:
- `recall@20` improved vs phase baseline.
- no-answer precision stays within tolerance.
- truncation telemetry present for every multi-source run.

### Phase 3 — Ranking model upgrade (2–3 weeks)

Owner: `compass-query` + tests

Tasks:
- implement explicit feature-vector scoring.
- add intent-specific ranking modifiers.
- version rank profiles and include in qualification report metadata.
- add regression tests for tie stability.

Exit:
- `nDCG@10` and `precision@10` improve.
- no deterministic order regressions without profile version change.

### Phase 4 — Relation and path quality (2 weeks)

Owner: `compass-query`

Tasks:
- apply direction + relation family constraints in path/call intent flows.
- add ranking for path alternatives before materialization.
- improve edge-direction and path-acceptance metrics.

Exit:
- `edge_direction_precision` and `path_acceptance_rate` up.
- wrong-direction false positives reduced.

### Phase 5 — Performance and hardening (2 weeks)

Owner: shared infra + core

Tasks:
- add bounded caches (optional): plan cache key on `(graph_digest, plan hash, limits, profile)`.
- add early cutoffs with explicit truncated reasons.
- enforce budgets and validate no unbounded growth.

Exit:
- p95 latency budget unchanged or improved under same fixture + realistic query set.
- no scale test regressions (`code_query_scale`, large synthetic graph behavior).

### Phase 6 — Controlled rollout + rollback

Owner: platform + QA

Tasks:
- shadow mode: run old path and new path side-by-side for a fixed query corpus;
  compare top-N IDs and ranking deltas.
- rollout by intent class:
  search + search+callers, then callees, then path/impact.
- keep kill-switch by rank profile/version pin.

Exit:
- phase gates signed off with metrics plus backend parity.
- docs + changelog/MIGRATION updates if user-visible behavior changes.

---

## 12) Qualification and test playbook

### Required recurring gates

- `cargo test -p compass-query --test relevance_qualification --locked`
- `python3 scripts/qualify_query_relevance.py`
- periodic focused correctness:
  `cargo test -p compass-query --test code_query_scale`
- any touched contract paths:
  `cargo test -p compass-cli --test compass_product --locked`

### Metric gates to track by phase

- `success@1`, `mrr@10`, `recall@20`, `precision@10`, `ndcg@10`
- `intent_macro_f1` and per-intent precision/recall
- `edge_precision`, `edge_recall`, `edge_kind_precision`, `edge_direction_precision`
- `path_acceptance_rate`, `mean_accepted_path_rank`
- `no_answer_precision`, `false_positive_rate`
- `latency_p50_micros`, `latency_p95_micros`
- `truncation` + work counters

### Fixture expansion checklist

- Intent confusion set (who calls/does X call/called by).
- Directional path set (`incoming` vs `outgoing` expected direction).
- Fuzzy/noisy set (`calcl`, `retun`, diacritic variants).
- Negative set (should return no answer).

---

## 13) Delivery risks and mitigation

- **Over-recall causing precision drop**
  - mitigate with intent gates + tight caps + early stop.
- **Path direction mismatch**
  - mitigate by explicit direction features and direction-aware tests.
- **Performance regression**
  - mitigate by strict budgets and early-stop counters.
- **Ranking churn**
  - mitigate via profile IDs and gated rollout.

---

## 14) Concrete first 2-week implementation checklist

If you want to start immediately, this is the minimal first milestone:

1. Add `intent.rs` + parser tests.
2. Wire parser into `command_natural_query`.
3. Route `callers` and `callees` intents to `CodeQueryEngine` operations.
4. Add 20–30 intent-labeled queries to fixture/relevance tests.
5. Add explicit intent-related assertions in qualification baseline.
6. Run phase 0 gates and capture baseline metrics.
7. Add phase 1-2 rollback guardrails (`operation` mismatch assertions + limits).

This creates measurable intent routing gains quickly with low architectural risk.
