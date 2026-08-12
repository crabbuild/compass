# Compass Query Accuracy, Recall, and Performance Technical Blueprint

This document defines a phased, production-oriented implementation plan to improve
Compass query quality:

- intent understanding (`who calls`, `path from A to B`, `impact of X`),
- recall for node/edge/path discovery,
- ranking quality (top-k precision, MRR, nDCG), and
- query latency stability,

while preserving Compass constraints (local-first, deterministic output, and Store/JSON parity).

## 0) Scope and success criteria

This plan improves the current query flow in:

- `crates/compass-cli/src/lib.rs::command_natural_query`
- `crates/compass-query/src/traversal.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/text.rs`
- `crates/compass-query/src/score.rs`
- `crates/compass-query/src/relevance.rs`
- `crates/compass-query/tests/relevance_qualification.rs`

Hard constraints:

1. No external ML/LLM/embedding dependency at query time.
2. Deterministic output for identical inputs and digests.
3. No hidden guessing; ambiguous/no-match remains explicit.
4. Backend parity between Store and JSON remains a first-class goal.
5. All recall/ranking changes are reversible through explicit profiles.

## 1) Current baseline and why results can be weak

Current path for `compass query` is traversal-first:

1. tokenize via `query_terms`/`score_nodes`,
2. pick seeds,
3. traverse BFS/DFS neighborhood,
4. render text graph output.

The system already has high-quality structured operators in `CodeQueryEngine`:
`search`, `callers`, `callees`, `impact`, `node_trail`, but natural-language
query does not consistently dispatch to them.

Current known mismatch: Store and JSON search paths do not currently apply identical
term normalization for all cases (for example diacritic/normalization behavior), which can reduce recall parity.

## 2) Success metrics (evaluation contract)

Use the existing relevance harness in `crates/compass-query/src/relevance.rs` and
`crates/compass-query/tests/relevance_qualification.rs`.

- `success_at_1`, `mrr_at_10`, `recall_at_5`, `recall_at_20`
- `precision_at_10`, `ndcg_at_10`
- `intent_macro_f1` and per-intent precision/recall/F1
- `edge_precision`, `edge_recall`, `edge_kind_*`, `edge_direction_*`
- `path_acceptance_rate`, `mean_accepted_path_rank`
- `accepted_ambiguity_recall`, `no_answer_precision`, `false_positive_rate`
- `latency_p50_micros`, `latency_p95_micros`, `WorkCounts`

Gate defaults (adjust per release):

- no intent-F1 regression,
- recall-at-20 non-decreasing on intent slices,
- no regression on no-answer precision or false-positive rate,
- p95 stable within phase SLO.

## 3) Target architecture (Google-like local IR stack)

Four deterministic stages:

1. **Intent plan extraction**
   - produce explicit structured intent and slots.
2. **Recall assembly**
   - bounded multi-source candidate discovery with provenance.
3. **Intent-aware ranking**
   - feature vector scoring + deterministic tie-break.
4. **Execution routing**
   - dedicated query operation when intent confidence is sufficient.

Every stage emits provenance for tuning and debugging. No stage can emit unbounded
candidate streams.

## 4) Cross-cutting normalization and consistency layer

Before intent or ranking work, enforce shared normalization across all query
paths:

1. **Symbol/token normalization unification**
   - move diacritic stripping + symbol cleanup into shared helpers used by:
     - `crates/compass-query/src/text.rs` (`query_terms`, `search_query_terms`)
     - `crates/compass-query/src/code_query.rs` (`search_query_terms` / fts query)
     - `crates/compass-graph/src/snapshot.rs` (`build_term_postings` + `nodes_for_terms`)
     - `crates/compass-query/src/score.rs`/`code_query.rs` symbol resolution helpers.

2. **Canonical case-folding policy**
   - lower-case + accent removal consistently where intended.

3. **Deterministic tie rules**
   - sort by explicit tuple keys in all merged lists.

This is required before ranking changes so that recall gains are attributable and
parity gates are stable.

## 5) Phase plan (detailed)

### Phase 0: Measurement lock + parity hardening (1 week)

Goal: prove baseline and avoid false regressions from serialization/parity noise.

- Replace byte-for-byte Store/JSON equality checks with semantic comparison in
  qualification harness (operation, limits, ordered IDs, truncation, and payload
  identity).
- Capture a frozen baseline report artifact for reviewed and executable subsets.
- Add canonical sort/dedupe in both test paths before compare.
- Add truncation reasons and candidate counters to relevance observations (where
  available).

Exit criteria:

- `relevance_qualification` tests are stable on current mainline.
- Store/JSON parity still explicit and debuggable.

### Phase 1: Intent understanding parser (1–2 weeks)

Goal: make intent explicit and bounded.

Files:
- new `crates/compass-query/src/intent.rs`
- export API via `crates/compass-query/src/lib.rs`

Implement:

- deterministic rule set (not ML) for:
  - `callers`/`called by`
  - `callees`/`calls`
  - `impact`/`downstream`/`upstream`
  - `path from A to B`
  - generic search/lookup
- symbol extractor for ID-like patterns:
  - camel/pascal/snake identifiers,
  - `A::B`, `pkg.mod.fn`, `Class.method`.
- confidence scoring and hard threshold (e.g., `>= 65`) for routing.
- explicit fallback mode: if confidence below threshold, keep current traversal path.

Exit criteria:

- deterministic parser unit tests and threshold-boundary tests.
- no increase in false positives on low-confidence classes.

### Phase 2: Multi-channel recall assembly (2–3 weeks)

Goal: improve recall without introducing noisy floods.

Files:
- new `crates/compass-query/src/recall.rs`
- extend `crates/compass-query/src/code_query.rs` candidate builders
- reuse `crates/compass-query/src/text.rs` normalization utilities.

Candidate channels, fixed order:

1. exact ID lookup
2. exact normalized symbol/name lookup
3. name-token and metadata-token index (`nodes_by_normalized_name`, Store term postings)
4. intent-seeded relation candidates
5. bounded fuzzy/near-match fallback (phase-gated)

Per-query budgets (hard):

- `max_candidates_total`
- `max_per_source`
- `max_terms`
- `max_postings_per_term`
- `max_fuzzy_candidates`

Every candidate carries `CandidateSource` provenance and per-source rank.

Exit criteria:

- improved recall@20 for intent/noisy slices,
- no uncontrolled precision drift,
- deterministic output order remains reproducible.

### Phase 3: Ranking profile v2 (2–3 weeks)

Goal: improve top-k ordering.

Files:
- `crates/compass-query/src/score.rs`

Refactor into explicit feature scoring:

- lexical exact/prefix/subword/token-coverage
- exact-id and qualified-name matches
- evidence quality (exact/inferred/ambiguous)
- intent compatibility (callers/callees/path direction)
- hub control/degree penalty
- ambiguity penalty
- relation/path readiness for path intent

Keep deterministic tie-breakers:

1. score,
2. source-backed flag,
3. confidence,
4. unresolved/test-derived penalty,
5. stable id.

Introduce rank profile IDs in internal diagnostics: `query-ranker/1`, `/2`, `/3`.

Exit criteria:

- measurable lift in `mrr_at_10`, `precision_at_10`, `ndcg_at_10`,
- no nondeterministic ordering changes.

### Phase 4: Intent-aware execution routing (1–2 weeks)

Goal: execute what user asked, not a generic traversal.

Files:
- `crates/compass-cli/src/lib.rs::command_natural_query`
- `crates/compass-query/src/code_query.rs`

Routing matrix:

- `Callers` → `CodeQueryEngine::callers`
- `Callees` → `CodeQueryEngine::callees`
- `Impact` → `CodeQueryEngine::impact`
- `Path` → `CodeQueryEngine::node_trail` or path-capable operator
- `Search` / Unknown / low confidence → current traversal path

Exit criteria:

- improved intent macro-F1 on intent-labeled fixture,
- fallback remains explicit and safe.

### Phase 5: Structural quality and directionality (2 weeks)

Goal: stronger edge/path correctness.

Files:
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/relevance.rs` (intent/path/edge assertions)

Implement:

- direction-aware neighbor family constraints,
- path edge-kind and direction checks before final ranking,
- rejected-path diagnostics (direction mismatch, missing endpoint, truncated).

Exit criteria:

- edge-direction precision/recall improvement on edge/path fixtures,
- higher path acceptance rate.

### Phase 6: Bounded fuzzy + typo recovery (1 week)

Goal: recover from noisy/misspelled queries without quality collapse.

Files:
- `crates/compass-query/src/intent.rs`
- `crates/compass-query/src/recall.rs`

Activation:

- only when recall is low or confidence is weak,
- only for tokens meeting length and complexity thresholds,
- strict caps on generated variants and total fuzzy results.

Ranking policy:

- fuzzy results receive explicit penalty and lower profile weight.

Exit criteria:

- recall gain on typo/noise slice,
- no severe no-answer precision/fp-rate regression.

### Phase 7: Performance controls and observability (1–2 weeks)

Goal: hold latency while raising quality.

Files:
- `crates/compass-query/src/relevance.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/score.rs`

Actions:

- add per-stage timings: parse/recall/rank/execute
- add per-query counters: candidates, postings, traversals, response bytes
- bounded in-memory cache for normalized intent plan (digest+query+profile key),
  optional for production rollout,
- early-stop when hard budgets are hit with explicit truncation reason.

Exit criteria:

- p95 within phase budget,
- bounded memory behavior under stress.

### Phase 8: Rollout and governance (1 week)

Goal: low-risk production deployment.

- shadow mode: baseline profile + new profile on a sampled query subset,
- compare top-k IDs and metrics, not raw JSON bytes,
- keep profile pinning for immediate rollback,
- stage by intent family (search first, then callers/callees, then path/impact).

## 6) Implementation sequencing and ownership

Recommended sequence:

1. Phase 0
2. Phase 1 + Phase 2
3. Phase 3
4. Phase 4 + Phase 5
5. Phase 6 + Phase 7
6. Phase 8

Primary ownership:

- `compass-query`: intent model, recall assembly, ranking, execution logic,
  quality harness.
- `compass-cli`: route selection and compatibility handling.
- `compass-graph`: index/build normalization consistency.

## 7) Concrete test and verification matrix

Phase-gated checks:

- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test relevance_qualification --locked`
- `python3 scripts/qualify_query_relevance.py`
- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test code_graph_assembly --locked` (if graph assembly touched)
- `cargo test -p compass-cli --test compass_product --locked`

For any change in `score.rs`, rerun:

- full `relevance_qualification` with reviewed and executable fixtures,
- both Store and JSON execution subsets,
- deterministic summary diffs only (no raw byte comparison for ranking-only
  semantic changes).

## 8) Milestone targets (example)

Week 1: Phase 0, Phase 1 scaffolding

Week 2-3: Phase 2 recall channel expansion + gating

Week 4-5: Phase 3 ranking v2 + regression baselines

Week 6: Phase 4 routing and Phase 5 edge/path improvements

Week 7: Phase 6 fuzzy + Phase 7 performance hardening

Week 8: Phase 8 rollout and rollback validation

## 9) Risks and mitigations

- **Recall gain introduces noise**: enforce source caps and intent confidence gates.
- **Intent parser misses edge cases**: deterministic fallback + explicit confidence.
- **Parity drift appears**: semantic compare for Store/JSON and identical
  normalization helpers.
- **Latency regressions**: hard budgets first, then optional short-lived caches.

## 10) Deliverables at phase gates

Phase A (after 2):

- normalized term behavior baseline fixed,
- intent parser enabled under threshold,
- stable parity semantic comparator.

Phase B (after 3):

- recall and ranking profile improvements,
- measurable top-k gains.

Phase C (after 5):

- intent routing, edge-direction/path behavior improvements,
- quality gates still green.

Phase D (final):

- optional fuzzy + caches,
- rollout profile docs and rollback strategy.

## 11) Final expected behavior after completion

Compass can answer intent-like questions with Google-like mechanics in a local,
deterministic way:

- understands query intent instead of treating all NL the same,
- retrieves a bounded high-recall candidate set from multiple channels,
- ranks candidates with explicit intent-aware features,
- routes to the correct graph operator,
- controls fuzziness under explicit budgets,
- emits reliable telemetry and versioned behavior so changes are reversible.
