# Query accuracy, intent routing, and recall roadmap for Compass (local-first)

## 1) Objective and scope

This roadmap targets all natural-language interactions with Compass and structured
symbol queries where recall, precision, and ranking behavior matter:

- `compass query` (full-text-like traversal)
- `search`, `callers`, `callees`, `impact`, `node_trail`, `explore` in
  `compass-query`
- edge/path rendering in traversal and explain commands
- relevance/rewrite gates in `crates/compass-query/tests/relevance_qualification.rs`

The goal is an explicit, deterministic, local-first pipeline that behaves like
an information retrieval system with an explicit intent plan and bounded recall,
not a black-box semantic vector engine.

Hard constraints:

- no Python, model credentials, external embeddings service, or untrusted network
  dependencies at query time
- no changes to public graph contracts, response schemas, or identity semantics
  without explicit compatibility work
- deterministic ordering on tied scores and tied plans
- explainable fallbacks (prefer explicit non-answer over guessed answers)

## 2) Current baseline behavior to build on

Owned entry points and current behavior:

- CLI: `command_natural_query` → `query_graph_text_page`
  (`crates/compass-cli/src/lib.rs`)
- Search/graph operators: `CodeQueryEngine::search`, `callers`, `callees`,
  `impact`, `explore`, `node_trail` (`crates/compass-query/src/code_query.rs`)
- Candidate ranking primitives: `score_nodes`, `pick_seeds`, `find_node`,
  `pick_scored_endpoint` (`crates/compass-query/src/score.rs`)
- Traversal and context handling: `query_graph_text_page`, `query_terms`,
  `infer_context_filters`, `normalize_context_filters` (`crates/compass-query/src/traversal.rs`,
  `text.rs`)
- Qualification and metrics: `QueryObservation`, `RelevanceMetrics`, `score`, tests,
  fixtures (`crates/compass-query/src/relevance.rs`,
  `crates/compass-query/tests/relevance_qualification.rs`,
  `crates/compass-query/tests/fixtures/relevance/judged.json`)

Current gaps relative to Google-like behavior:

- intent routing is implicit and mainly traversal-oriented
- candidate generation is strict and fallback-light
- ranking is mostly lexical/field-level with weak structural intent features
- no dedicated path ranking model
- path/result interpretation is mostly exact and not robust to near-miss phrasing

## 3) Target architecture (three deterministic layers)

### Layer A: Intent parse + plan extraction

Input: question text + optional explicit `--context` filters.

Output:

- `QueryIntent` (Search/Callers/Callees/Impact/Path/Explain/Unknown)
- extracted slots (symbols, relation cues, direction, depth, limit)
- confidence score + fallback strategy

This layer does not return answers; it returns a structured plan.

### Layer B: Candidate generation (recall-first)

Input: plan + graph + query terms.

Output:

- bounded candidate ID pool (`Vec<String>`)
- candidate provenance (`fts`, `normalized`, `alias`, `fuzzy`, `heuristic`)

The only allowed ordering in this layer is by stable, deterministic source order.

### Layer C: Intent-aware ranking + explanation scores

Input: candidate pool + intent features + graph context.

Output: ordered candidates and ranked edges/paths.

Ranked result is deterministic by score + stable tie policy.

## 4) Success metrics and rollout targets

Track both quality and serviceability. Add explicit baseline assertions for:

- node quality: `Success@1`, `MRR@10`, `Recall@5`, `Recall@20`,
  `Precision@10`, `nDCG@10`
- intent quality: `intent_macro_f1`, per-intent precision/recall/F1
- edge/path quality: directioned edge precision/recall, path acceptance rate,
  mean accepted path rank
- safety/robustness: no-answer precision, false-positive rate
- performance: p50/p95 command latency, response bytes, candidate count, truncation
  rate, and deterministic JSON/store parity

Rollout target is a measurable gain in recall and intent correctness without
regressing no-answer precision or exceeding a <10% median latency regression on the
approved baseline corpus.

## 5) Data and test scaffolding required before code changes

Before implementing phases 1+, lock the evaluation surface:

1. Expand `crates/compass-query/tests/fixtures/relevance/judged.json` to include:

   - more intent-only queries (`who calls`, `what calls`, `what does X call`,
     `path from A to B`, `find callers of X`)
   - typo/variation and fuzzy-style prompts (`calcl`, `retrun`, missing diacritics)
   - path direction and relation-specific variants
   - explicit negatives for common over-generalized terms

2. Extend `QueryClass` if needed:

   - keep existing classes but add stable classes for intent/graph-query stress cases
   - do not repurpose `Negative` for fuzzy misses; keep explicit ambiguity labels

3. Add work counters to `QueryObservation::work` during execution:

   - candidates read
   - postings decoded
   - nodes expanded
   - edges expanded
   - correction candidates generated
   - response bytes (already present)

4. Add deterministic fixture hashes in `PERFORMANCE.md` + query-gate report format.

## 6) Phase 0 — Instrumentation and reproducible baseline (1–2 days)

### A. Add high-clarity observation hooks

- Add internal query-timing buckets (candidate stage, ranking stage, traversal
  stage) and expose in `WorkCounts`.
- Add explicit `query_plan` artifact for relevance harness only (not response
  contract):

  - parsed intent
  - extracted slots
  - generation path used
  - truncated reason codes

- Add explicit counters for fallback depth and truncation reasons (`candidates_cap`,
  `fuzzy_cap`, `vocab_cap`, `alias_cap`).

### B. Baseline verification

- Add regression tests for:
  - stable metric serialization ordering
  - deterministic ranking for equal scores
  - fallback paths being explicit in diagnostics

- Re-run: `python3 scripts/qualify_query_relevance.py` and baseline gates.

### Exit criteria

- Existing fixtures remain stable on unchanged code.
- Per-intent metrics are computable even if some phases are still `Unknown`.
- A full baseline export is committed for future deltas.

## 7) Phase 1 — Intent parser with confidence routing (2–3 days)

### 7.1 module and parser

Add `crates/compass-query/src/intent.rs` (or extend `text.rs` if team preference)
with:

```rust
#[derive(Clone, Copy, Debug)]
pub enum QueryIntent { Search, Callers, Callees, Impact, Path, Explain, Unknown }

pub struct QueryIntentPlan {
    pub intent: QueryIntent,
    pub confidence: u8, // 0..100
    pub symbols: Vec<String>,
    pub relation: Option<String>,
    pub direction: Option<String>,
    pub depth: Option<u32>,
    pub limit: Option<u32>,
    pub context_filters: Vec<String>,
    pub parse_reason: Vec<String>,
}

pub fn parse_intent(question: &str, explicit_contexts: &[String]) -> QueryIntentPlan;
```

### 7.2 heuristic signals to include

- lexical cue weights:
  `who|callers|called by`, `callees`, `path|route|flow|connects`, `who uses`,
  `what calls`, `what returns`, `where is`, `impact`, `references`
- symbol-position cues:
  backtick/quoted tokens, `A -> B`, `from A to B`, qualified names, `Class::method`
- relation hint cues:
  `calls`, `routes_to`, `imports`, `implements`, etc.
- parser confidence gates:
  `unknown` if no symbols and confidence < threshold or if multiple conflicting
  verbs.

### 7.3 CLI/engine routing

- In `command_natural_query`, attempt intent parse first.
- Dispatch:
  - `Callers`/`Callees` → `CodeQueryEngine::callers` / `::callees`
  - `Path` with two symbols → `node_trail` if symbols resolve; fallback to
    traversal if low confidence
  - `Search` → current traversal/search hybrid with updated candidate + ranking
    behavior
  - low-confidence parse → existing traversal semantics unchanged.

### Exit criteria

- On synthetic intent-labeled set, at least 70% intent correctness in phase-1
  harness run.
- No behavior regression for exact-match legacy traversal on unchanged queries.

## 8) Phase 2 — Candidate generation upgrades for recall (3–5 days)

### 8.1 Query term normalization and candidate sets

Build a deterministic candidate aggregator in `CodeQueryEngine::search`:

1. Normalize terms with `search_tokens` and `strip_diacritics`.
2. Run primary candidate source:
   - materialized SQLite FTS with canonical query
   - store snapshot `nodes_for_terms`
3. Merge secondary sources before ranking:
   - normalized-name exact/prefix table
   - alias/split-identifier candidates
   - hash-stable symbol ID candidates when query token includes separators or
     delimiter patterns

### 8.2 Bounded fuzzy fallback path

Trigger fuzzy fallback only if:

- primary candidate count below `candidate_floor`
- or top candidates all fail exact slot checks

Fallback mechanics:

- produce suggestions from limited vocabulary terms (min edit distance thresholds)
- bounded by `max_fuzzy_candidates`, `max_edit_distance`, `max_generation_per_token`
- keep only candidates from existing graph IDs

Do not execute edit-distance expansion for one-character tokens or very
high-cardinality terms.

### 8.3 Backend parity

Keep materialized and store parity by normalizing all candidate IDs before ranking:

- dedupe with deterministic order
- canonical upper bound on candidate pool size by effective `max_candidates`
- preserve deterministic truncation reason and emit it via work counters

### Exit criteria

- typo/noise recall improves in fixture without materially reducing no-answer
  precision.
- candidate stage remains under bounded cardinality and does not add non-determinism.

## 9) Phase 3 — Intent-aware ranking model (5–7 days)

### 9.1 feature-based scoring contract

Replace ad-hoc tuple scores with an internal scoring vector in `score.rs`:

```rust
#[derive(Clone)]
pub struct CandidateFeatures {
    pub lexical_exactness: f64,
    pub lexical_prefix: f64,
    pub lexical_substring: f64,
    pub symbol_id_match: f64,
    pub evidence_confidence: f64,
    pub provenance_quality: f64,
    pub context_alignment: f64,
    pub intent_relation_match: f64,
    pub path_readiness: f64,
    pub degree_signal: f64,
    pub ambiguity_penalty: f64,
    pub test_or_generated_penalty: f64,
}

pub fn rank_candidates(
    graph: &Graph,
    candidates: &[NodeIndex],
    plan: &QueryIntentPlan,
) -> Vec<(f64, NodeIndex, CandidateFeatures)>;
```

### 9.2 Deterministic tie-break policy

Continue using stable secondary keys in order:

1. source-backed evidence presence
2. semantic rank class
3. generated/test node penalty
4. source-backed degree
5. label length
6. node ID

### 9.3 Traversal seeding using intent

If plan intent is relation-like (`callers`, `callees`, `impact`, `path`):

- choose seeds from top relation-aligned scored nodes
- set traversal budget and allowed context by slot filters
- prefer nodes matching explicit direction/depth

### 9.4 Path candidate ranking

For path-like queries, generate multiple path options under the same depth bound,
score each using:

- intent relation fit
- endpoint match confidence
- path length penalty
- edge-kind coherence

Return best path + optionally include secondary alt path IDs in debug profile.

### Exit criteria

- improved nDCG@10 and precision@10 for intent classes
- stable deterministic ordering still passes existing sort/replay tests
- no new schema change to public response

## 10) Phase 4 — Edge and path semantics (5–8 days)

### 10.1 Directed relation retrieval

For edge-intent queries, use operation-specific retrieval:

- `who calls X` / `callers of X` → incoming call edges only
- `X calls` / `what does X call` → outgoing edges with relation filter
- `where is` / `where does` fallback to `explain`-like seeded traversal but keep
  result form as explain-like graph neighborhood unless explicit intent matches

### 10.2 Relationship and direction scoring

- direction mismatch penalty
- relation kind mismatch penalty
- positive relation confidence for explicit relation terms

### 10.3 Path answer shape

- path direction preserved
- path endpoint exactness and endpoint confidence included in path grade
- if multiple equivalent paths exist, prefer shortest + highest relation agreement

### Exit criteria

- edge precision/recall and direction-specific metrics improve while negative/ambiguous
  cases remain stable.

## 11) Phase 5 — Performance hardening and caching (3–5 days)

### 11.1 Caches

- in-memory cache keyed by `(graph_digest, normalized_query)`:
  - parsed intent
  - top seed IDs
  - ranked candidate IDs with features
- per-graph static normalized vocab cache
- optional trie/term index for fuzzy suggestions

### 11.2 Query budgets and hard ceilings

Introduce hard bounds in config/limits:

- max fuzzy candidates
- max alias expansion per symbol token
- max path alternatives per query intent
- max fallback depth steps
- max relation-ambiguous seeds retained

Record truncation reasons whenever any of these bounds trigger.

### 11.3 Memory and startup controls

- use bounded `BTreeMap`/`HashMap` plus explicit capacity reservations
- avoid building all caches unless query volume justifies them
- support cache disable/clear via env variable or CLI debug mode

### Exit criteria

- no >10% p50 latency regression on approved baseline
- no unbounded growth in candidate workspace for repeated equivalent queries
- repeated identical plan runs reuse cache under a warm process scenario

## 12) Phase 6 — Controlled rollout plan

1. **Canary** on support fixture only.
2. **Shadow mode**: execute old behavior + new behavior and log top-N IDs
   + intent.
3. **Dual gate**: move forward only if:
   - intent macro-F1 improves,
   - recall@k improves,
   - no-answer precision does not degrade.
4. **Default enable** with diagnostic debug mode off; keep old behavior available
   via feature flag/config for rollback and A/B tests.

## 13) Ownership and implementation boundaries

- `compass-query` (primary)
  - intent module, candidate extraction, ranking, traversal integration,
    diagnostics and work counters
- `compass-cli` (routing)
  - query command dispatch and defaults
- `crates/compass-query/tests/...` (quality)
  - fixtures, thresholds, and deterministic regression gates
- `PERFORMANCE.md`
  - documented baseline values and new thresholds

Do not move query semantics to `compass-cli`; keep only dispatch decisions there.

## 14) Concrete file-by-file work list

- `crates/compass-query/src/intent.rs` (new): parser + plan + confidence model
- `crates/compass-query/src/lib.rs`
  - export intent types and plan API
- `crates/compass-query/src/text.rs`
  - strengthen token extraction helpers and context canonicalization shared by intent

- `crates/compass-query/src/code_query.rs`
  - integrate intent plan into `search/callers/callees/node_trail`
  - add bounded union candidate generation and provenance counters
- `crates/compass-query/src/score.rs`
  - migrate ranking to explicit features and deterministic profile versions
- `crates/compass-query/src/traversal.rs`
  - intent-aware seed selection and traversal filters
  - path ranking for relation queries
- `crates/compass-query/src/relevance.rs`
  - add optional work fields and intent plan validation (compat-safe)
- `crates/compass-query/tests/relevance_qualification.rs`
  - additional intent/negative/typo/path coverage
- `crates/compass-query/tests/fixtures/relevance/judged.json`
  - expanded judged query corpus
- `PERFORMANCE.md`
  - new acceptance numbers and rollout criteria

## 15) Verification matrix

Run at minimum before merging each phase:

- `cargo test -p compass-query --locked`
- `cargo test -p compass-query --test relevance_qualification --locked`
- `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-<checkout> \
  python3 scripts/qualify_query_relevance.py`

If changing query engine behavior:

- `cargo test -p compass-cli --test compass_product --locked`
- `cargo test -p compass-query --test opencypher_tck --locked` (if parser/plan surface touched)
- targeted deterministic regression tests for `search`, `callers`, `callees`,
  `node_trail`

Quality gate review before promotion:

- strict no regression on node ID order under `sort_stable` and fixture parity,
  plus intent and edge/path metrics above agreed thresholds.

## 16) Risk log and mitigations

- **Over-recall leading to noisy results**
  - mitigate with explicit penalties, narrow caps, and metric-based gate on no-answer
    precision.
- **Intent misclassification for short/ambiguous prompts**
  - mitigate with confidence thresholds and heuristic fallback.
- **Backend behavior divergence (JSON/store)**
  - mitigate by enforcing canonical candidate normalization before ranking.
- **Ranking overfit**
  - mitigate by versioned score profile and phased rollout.
- **Higher latency from fuzzy path**
  - mitigate with budget caps and bounded fallback triggers only.

## 17) Suggested timeline (non-blocking sequencing)

- Week 1: Phase 0 + 1 implementation, fixtures, baseline baseline
- Week 2: Phase 2 candidate generation and recall hardening
- Week 3: Phase 3 ranking + phase 4 path/edge fixes
- Week 4: Phase 5 performance, rollout metrics, and production flags

## 18) Expected outcomes

If executed in order with gates enforced:

- better intent extraction for natural queries and less reliance on broad BFS fallback
- materially better recall on typo/noisy and relation phrasing questions
- improved edge/path ranking precision without changing public contracts
- bounded and explainable ranking behavior with deterministic fallbacks
