# Query performance, recall, and accuracy design (phased implementation)

## 0) Objective

Improve Compass query outcomes in the same way users expect from search engines:

- higher recall for relevant nodes/paths/edges,
- better relevance at top ranks,
- stronger intent detection (`callers`, `callees`, `impact`, `path`, `search`),
- bounded latency and deterministic behavior,
- preserved local-first execution and JSON/Store parity.

This document is for implementation, not architecture speculation.

## 1) Current state to build on

- Natural query command currently routes through traversal:
  `crates/compass-cli/src/lib.rs::command_natural_query` -> `query_graph_text_page`.
- Querying primitives already exist in `crates/compass-query/src/code_query.rs`:
  `search`, `callers`, `callees`, `impact`, `node_trail`, `explore`.
- Scoring/pruning today exists in `crates/compass-query/src/score.rs`.
- Relevance harness already measures ranking/intent/edge/path performance:
  `crates/compass-query/src/relevance.rs`,
  `crates/compass-query/tests/relevance_qualification.rs`,
  `crates/compass-query/tests/fixtures/relevance/judged.json`.

Keep these foundations; do not replace them.

## 2) Hard constraints

- deterministic ordering and canonical serialization,
- no external semantic model services,
- no unbounded cache,
- no intent guessing on ambiguous/low-confidence input,
- bounded and explainable truncation,
- backend parity where possible (`Store` vs `Json`).

## 3) Metrics and gates (already available)

Use `RelevanceMetrics` from `relevance.rs` as the authoritative acceptance surface.

- `success_at_1`, `mrr_at_10`, `recall_at_5`, `recall_at_20`, `precision_at_10`, `ndcg_at_10`
- `intent_macro_f1` and class-level precision/recall/F1,
- `edge_precision`, `edge_recall`, `edge_kind_*`, `edge_direction_*`,
- `path_acceptance_rate`, `mean_accepted_path_rank`,
- `accepted_ambiguity_recall`, `no_answer_precision`, `false_positive_rate`,
- latency and work counters (`WorkCounts`: candidates/readings/expanded/nodes/edges/bytes).

Rollout rule: no phase may regress a base metric unless explicitly approved.

## 4) Target architecture (deterministic staged pipeline)

### Stage A: Intent plan
Input question -> deterministic plan with confidence.

Output fields:

- intent (`search`/`callers`/`callees`/`impact`/`path`/`explain`/`unknown`),
- extracted symbols,
- relation hints, direction hints,
- explicit `limit`/`depth` constraints,
- ambiguity/confidence.

### Stage B: Recall assembly
Build a bounded superset from multiple sources:

1. exact id,
2. exact normalized symbol,
3. normalized name index,
4. full-text postings,
5. relation seeded candidates from intent,
6. bounded fuzzy fallback.

Each source records provenance + reason and is merged deterministically.

### Stage C: Ranking
Explicit feature-vector ranking over candidates with intent-aware profiles:

- lexical match strength,
- symbol shape match,
- exactness/evidence confidence,
- intent relation compatibility,
- bounded structure signal (degree/trusted sources),
- ambiguity penalty,
- fuzzy penalty.

Stable sort order is mandatory.

### Stage D: Execution routing
If confidence/ambiguity permit:

- `callers` -> `code_query::callers`,
- `callees` -> `code_query::callees`,
- `impact` -> `code_query::impact`,
- `path` -> `code_query::node_trail`,
- otherwise `search`,
- if low confidence/ambiguous/unresolved, fallback to existing traversal path.

### Stage E: Diagnostics and gating
Emit internal metadata:

- `ranker_version` and `planner_version`,
- source/truncation reasons,
- stage timings and counters (intent/recall/rank/execute).

## 5) Proposed internal models

Add new module `crates/compass-query/src/intent.rs`.

- QueryIntent enum: `Search`, `Callers`, `Callees`, `Impact`, `Path`, `Explain`, `Unknown`.
- `QueryIntentPlan` includes:
  - confidence (0..=100),
  - symbols,
  - relation_hints,
  - direction,
  - limit/depth overrides,
  - parse trace for explainability.
- Candidate model includes:
  - candidate source (`ExactId`, `ExactName`, `NormalizedName`, `TextPostings`, `RelationSeed`, `FuzzyFallback`),
  - per-source rank,
  - provenance reason list,
  - feature values by key.

## 6) Phased plan (actionable, test-linked)

### Phase 0 — Baseline hardening (1 week)

Files:

- `crates/compass-query/tests/relevance_qualification.rs`
- `scripts/qualify_query_relevance.py`

Actions:

- keep reviewed corpus deterministic;
- emit artifact report (JSON) from relevance test;
- add explicit checks for `recall_at_20`, `edge_direction_precision`, `path_acceptance_rate` and latency fields.

Acceptance:

- stable deterministic report output,
- reproducible executable baseline pass.

### Phase 1 — Intent parser and plan object (1–2 weeks)

Files:

- add `crates/compass-query/src/intent.rs`
- `crates/compass-query/src/lib.rs` exports if needed
- `crates/compass-cli/src/lib.rs` integration.

Actions:

- implement deterministic rule-based classifier,
- confidence + ambiguity scoring,
- symbol extraction (id/qualified names/symbol forms),
- parser unit tests.

Acceptance:

- tests for common intent classes,
- low-confidence or unknown input still reaches traversal fallback.

### Phase 2 — Multi-source recall (2 weeks)

Files:

- `crates/compass-query/src/code_query.rs`
- optional new `compass-query/src/recall.rs` module.

Actions:

- add bounded candidate mux in fixed source order,
- enforce per-source caps and total caps,
- preserve deterministic dedupe by `node_id`,
- keep source order and truncation reasons.

Acceptance:

- recall metrics improve on intent and typo slices,
- deterministic output order is unchanged for same plan.

### Phase 3 — Ranking profiles (2 weeks)

Files:

- `crates/compass-query/src/score.rs`
- `crates/compass-query/tests` (scoring regressions).

Actions:

- split score into explicit feature extraction + aggregator,
- add intent/ranker profiles (`query-ranker/1`, `/2`, `/3` in reports),
- deterministic tie-break by id when scores tie.

Acceptance:

- `precision_at_10`, `mrr_at_10`, `ndcg_at_10` improve,
- no precision backslide without explicit sign-off.

### Phase 4 — Intent-aware routing (1–2 weeks)

Files:

- `crates/compass-cli/src/lib.rs`
- `crates/compass-query/src/code_query.rs`

Actions:

- add route planner from plan -> query operation,
- explicit fallback on symbol ambiguity.

Acceptance:

- intent-specific operations improve `intent_macro_f1`,
- unchanged or better outputs for fallback and non-mapped intents.

### Phase 5 — Structural quality (1–2 weeks)

Files:

- `crates/compass-query/src/code_query.rs`

Actions:

- direction/relationship guards on path-like and impact queries,
- reject or down-rank wrong direction candidates,
- stronger path direction diagnostics.

Acceptance:

- `edge_direction_precision`, `edge_direction_recall`, `path_acceptance_rate` hold or improve.

### Phase 6 — Bounded fuzzy and typo recovery (1 week)

Files:

- `crates/compass-query/src/intent.rs`
- `crates/compass-query/src/code_query.rs`

Actions:

- bounded edit-distance expansion by token length and cap,
- fuzzy results only when confidence is low or recall appears missing,
- large penalty for fuzzy provenance.

Acceptance:

- typo slice lift with no severe `false_positive_rate` increase.

### Phase 7 — Runtime and memory control (1–2 weeks)

Files:

- `crates/compass-query/src/relevance.rs`
- `crates/compass-query/src/score.rs`
- `crates/compass-query/src/code_query.rs`

Actions:

- add stage timings and counters,
- bounded memoization for normalized question intent parse and tokenized terms,
- early-stop when rank convergence reached.

Acceptance:

- p95 under agreed budget,
- no unbounded memory growth.

### Phase 8 — Rollout + kill-switch (1 week)

Files:

- `crates/compass-query/src/relevance.rs`
- `crates/compass-cli/src/lib.rs`

Actions:

- profile IDs for planner/ranker,
- shadow comparison against baseline profile,
- phased enablement by intent family.

Acceptance:

- staged rollout with documented rollback path,
- successful parity checks on legacy and new profiles.

## 7) Suggested implementation order and team cadence

Recommended sequence:

1. Phase 0 (measurement lock)
2. Phase 1 + 2 (intent + recall)
3. Phase 3 + 4 (rank + routing)
4. Phase 5 + 6 (quality safety)
5. Phase 7 + 8 (hardening and rollout)

Every phase has concrete test gates and review criteria before moving forward.

## 8) Risk and mitigation

- Recall growth raises ambiguity: control with strict caps and confidence thresholds.
- Wrong route due parser noise: enforce fallback and explicit route reasons.
- Performance drift: add budgets, stage counters, and per-phase caps.
- Structural overmatching: relation and direction checks are mandatory for graph operations.

## 9) Command-level commands for verification

- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test relevance_qualification --locked`
- `CARGO_TARGET_DIR=... COMPASS_RELEVANCE_REPORT=./target/relevance-report.json python3 scripts/qualify_query_relevance.py`
- `cargo test -p compass-query --test relevance_qualification executable_baseline_is_digest_pinned_and_backend_deterministic --locked`
- `cargo test -p compass-cli --test code_query_cli --locked`

## 10) Why this is Google-like within Compass limits

This is query understanding, recall expansion, intent-specific scoring, fuzzy recovery, and ranking reordering—implemented with deterministic local indexes and explicit policy flags instead of external LLM/embedding services.
