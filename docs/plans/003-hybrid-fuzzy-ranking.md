# Plan 003: Add explainable hybrid lexical and fuzzy ranking

> **Executor instructions**: Implement this plan only after Plans 001 and 002
> are DONE. Run each verification gate before continuing. Exact identity and
> backend equivalence are hard invariants; stop rather than tuning around a
> failing relevance fixture. Update `docs/plans/README.md` when complete.
>
> **Drift check (run first)**:
> `git diff --stat 43bceb6e..HEAD -- crates/compass-model/src/search.rs crates/compass-graph/src/snapshot.rs crates/compass-query/src/{retrieval.rs,ranking.rs,score.rs,code_query.rs,index.rs,lib.rs} crates/compass-query/tests PERFORMANCE.md`

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: Plans 001 and 002
- **Category**: correctness / performance / direction
- **Planned at**: commit `43bceb6e`, 2026-08-06

## Why this matters

Current typed ranking has four coarse tiers and reports only name/qualified-name
matches even when aliases, roles, paths, language, or framework produced the
candidate. Natural ranking separately rescans every node and uses exact,
prefix, substring, and source bonuses. This phase replaces both with a shared,
explainable ranker that supports field relevance, vocabulary mismatch, acronym
resolution, and bounded typo recovery without allowing fuzzy evidence to beat
exact identity.

## Current state

- `crates/compass-query/src/code_query.rs:580-620` ranks qualified exact, name
  exact, prefix, and fallback tiers; the public score is
  `tier * 1_000_000 + matched_fields.len()`.
- `crates/compass-query/src/index.rs:306-309` already indexes name, qualified
  name, aliases, kind, roles, language, framework, and normalized path, but the
  common ranker does not score most fields.
- `crates/compass-query/src/score.rs:27-174` scans all graph nodes per natural
  question, recomputes IDF, and calculates different scores from typed search.
- `crates/compass-query/src/score.rs:9-12` hard-codes exact 1000, prefix 100,
  substring 1, and source 0.5 bonuses.
- `crates/compass-model/src/query_contract.rs:162-168` exposes only `nodeId`,
  opaque `score`, and `matchedFields` in v1.
- `crates/compass-graph/Cargo.toml:20` already uses `strsim`; reuse its bounded
  Damerau-Levenshtein approach or move the workspace dependency to the lowest
  non-cyclic owner rather than adding another fuzzy library.

Current typed ranker excerpt:

```rust
// crates/compass-query/src/code_query.rs:585-605
let tier = if normalized_qualified == normalized_query { 4 }
    else if normalized_name == normalized_query { 3 }
    else if normalized_qualified.starts_with(&normalized_query)
         || normalized_name.starts_with(&normalized_query) { 2 }
    else { 1 };
ranked.sort_by(|left, right| right.0.cmp(&left.0)
    .then_with(|| left.1.cmp(&right.1)));
```

## Design

### Ranking invariants

Rank in two stages:

1. **Hard evidence tier**, compared lexicographically:
   `ExactId > ExactQualified > ExactName > ExactAlias > ExactPath > NonExact`.
2. **Non-exact score**, computed in common Rust from portable features. Stable
   node ID is the final tie-breaker.

No fuzzy, semantic, graph, popularity, feedback, or test/production feature may
cross a hard exact tier. Ambiguous candidates within the same exact tier remain
explicit alternatives.

### Fielded lexical score

Implement BM25F-like scoring from portable posting/document statistics. Use a
checked-in `RankProfile` with versioned rational/integer weights to reduce
cross-platform floating drift. Initial relative ordering:

```text
qualified_name = 12
name           = 10
alias          = 8
identifier_part= 6
community      = 5
path           = 4
role           = 3
kind           = 3
framework      = 2
language       = 1
```

These are initial features, not calibrated probabilities. Plan 001 judgments
decide later changes. Score term coverage, rarity, phrase adjacency, and field
length; require at least one positive topical feature before returning a broad
candidate. Do not call SQLite `bm25()` as public semantics.

### Fuzzy candidate generation

Use a bounded two-stage design:

1. retrieve fuzzy candidates from portable character trigrams or SymSpell-like
   deletion keys stored per whole identifier and identifier part;
2. verify candidates with bounded Damerau-Levenshtein in common Rust.

Distance policy:

```text
length 1-2: exact only
length 3-4: maximum distance 1
length 5-8: maximum distance 2
length 9+:  maximum distance min(3, floor(length / 4))
```

Require minimum trigram overlap before edit-distance work. Cap analyzed fuzzy
terms, posting keys per term, candidates per term, aggregate fuzzy candidates,
and total edit comparisons. When a cap is reached, return a typed truncation
diagnostic rather than pretending no fuzzy matches exist.

### Acronyms and repository vocabulary

The canonical analyzer should already emit deterministic acronym forms for
identifiers. Add a bounded repository-derived vocabulary from:

- aliases and qualified names;
- community labels;
- path segments and framework roles;
- a small checked-in, versioned code-query synonym map such as
  `invoke -> call`, `persist -> save/write`, `affected -> impact`, and
  `entry point -> route/handler`.

Repository-derived terms must be evidence-linked and snapshot-scoped. Do not
index arbitrary source prose or comments in this phase.

### Rank fusion and explanations

Within the non-exact tier, either sum normalized portable features or use
deterministic reciprocal-rank fusion (RRF) across lexical, acronym, and fuzzy
channels. Prefer RRF if channel score scales cannot be made comparable. Pin the
RRF constant and tie semantics in `RANK_PROFILE_VERSION`.

Add an internal explanation now and expose it publicly in Plan 006:

```text
RankExplanation
  hard_tier
  matched_fields[]
  matched_terms[]
  channel_ranks[]
  field_scores[]
  phrase_bonus
  fuzzy[] {query_term, candidate_term, edit_distance, overlap}
  acronym_matches[]
  penalties[]
  total_non_exact_score
  rank_profile_version
```

Every list is bounded and canonically ordered. Explanation component sums must
reproduce the total score exactly under the chosen numeric representation.

### Noise and diversity

Apply only conservative within-tier penalties in this phase:

- source-less unresolved placeholders;
- test/generated roles when not explicitly requested;
- obvious builtin noise already recognized by graph analysis.

Do not use raw degree, PageRank, or community diversification yet; those belong
to Plan 005. Scope/community may contribute only when matched by query text.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Query tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked` | all pass |
| Graph tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-graph --locked` | all pass |
| Query lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-model -p compass-graph -p compass-query --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Relevance | `./scripts/qualify_query_relevance.py --fixtures-only` or the native equivalent from Plan 001 | all thresholds pass |
| Code graph | `./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |

## Scope

**In scope**:

- `crates/compass-model/src/search.rs`
- `crates/compass-graph/src/snapshot.rs`
- `crates/compass-query/Cargo.toml` only if the existing workspace `strsim`
  dependency must be added
- `crates/compass-query/src/retrieval.rs`
- `crates/compass-query/src/ranking.rs` (create)
- `crates/compass-query/src/score.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/index.rs`
- `crates/compass-query/src/lib.rs`
- `crates/compass-query/tests/code_search.rs`
- `crates/compass-query/tests/store_engine.rs`
- `crates/compass-query/tests/code_query_scale.rs`
- `crates/compass-query/tests/relevance_qualification.rs`
- `PERFORMANCE.md`
- `docs/implementation/query-engine.md`

**Out of scope**:

- public query v2 serialization, CLI/MCP changes, intent planning, edge search,
  PageRank, graph traversal changes, embeddings, model providers, or learning
  from user behavior;
- modifying graph extraction facts or historical artifacts;
- tuning weights against unreviewed production logs or private code.

## Git workflow

- Branch: `advisor/003-hybrid-fuzzy-ranking`
- Suggested commits: field statistics/explanations; BM25F ranker; fuzzy index
  and verifier; natural/typed convergence; qualification docs.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Add rank-profile and explanation types

Create explicit, versioned types and deterministic arithmetic. Keep v1
serialization unchanged; store explanations internally or behind test-only
accessors until Plan 006. Add unit tests proving hard-tier precedence,
component-total equality, stable ties, and bounded explanation output.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query ranking::tests --locked`
→ all rank invariants pass.

### Step 2: Add portable field/document statistics

During immutable index construction, calculate document frequency, per-field
token length, and corpus field averages. Encode them with the analyzer and
rank-profile versions. Add validation for missing, duplicate, corrupt, or
over-limit statistics. Ensure equivalent graph inputs produce byte-identical
statistics.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-graph snapshot --locked`
→ statistics round-trip and deterministic snapshot tests pass.

### Step 3: Implement shared fielded ranking

Replace typed four-tier ranking with the common ranker, then adapt natural
`score_nodes` to use the same candidate/features path instead of scanning every
node. Preserve existing exact seed preferences and ambiguity behavior.
Instrument deterministic work counts: postings read, candidate nodes loaded,
features scored, and candidates discarded by each bound.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test code_search --test coverage_paths --locked`
→ exact precedence, field matches, aliases, path/role queries, and natural
pagination tests pass.

### Step 4: Add bounded fuzzy keys and edit verification

Build portable fuzzy postings for identifier fields only. Implement distance
policy, trigram/deletion overlap, quotas, and truncation diagnostics. Add
positive and negative tests including transpositions, missing/extra character,
short symbols, distant terms, overloaded labels, Unicode normalization, and
adversarially repetitive identifiers.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query fuzzy --locked`
→ intended typos recover; over-distance and short noisy queries do not.

### Step 5: Add acronym and controlled synonym evidence

Index analyzer-generated acronyms and a small versioned code-domain synonym
map. Require explanation entries to identify whether evidence came from the
repository or checked-in vocabulary. Keep all expansions bounded and avoid
recursive synonym expansion.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query acronym synonym --locked`
→ expected expansions retrieve reviewed candidates and unrelated candidates
remain below thresholds.

### Step 6: Qualify relevance and performance

Run the Plan 001 corpus. Require exact Success@1 and backend parity at 100%,
typo/acronym Recall@20 at least 0.90, overall nDCG@10 improvement, and no
unwaived slice regression over two points. Extend the 100,000-node test with
work-count assertions proving exact and fuzzy queries touch bounded candidates,
not every node.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test relevance_qualification --test code_query_scale --locked`
→ thresholds and work bounds pass.

## Test plan

- Unit tests for tier order, BM25F/RRF arithmetic, exact component sums,
  distance limits, acronym formation, synonym provenance, and stable ties.
- Differential tests for JSON/store full ordered hits, explanations,
  diagnostics, and truncation.
- Regression tests for typo, transposition, alias, acronym, field weighting,
  common-name ambiguity, test/generated penalty, and exact-over-fuzzy behavior.
- Scale tests assert maximum postings, candidates, edit comparisons, decoded
  bytes, and elapsed ceiling on 100,000 nodes.
- Relevance corpus gates all metrics defined in Plan 001.

## Done criteria

- [ ] Natural and typed discovery consume one fielded ranker.
- [ ] Exact tiers are lexicographically dominant and tested.
- [ ] All indexed fields can contribute explainable score evidence.
- [ ] Typo and acronym retrieval are bounded, deterministic, and meet corpus
  recall targets.
- [ ] JSON/store ordered results, explanations, truncation, and diagnostics are
  identical.
- [ ] Exact/prefix/fuzzy query work is candidate-bounded rather than O(nodes).
- [ ] All targeted tests, Clippy, format, relevance, and code-graph gates pass.

## STOP conditions

Stop and report if:

- portable score equality requires backend-native BM25 or locale behavior;
- fuzzy index size exceeds the documented snapshot/item budgets on qualification
  fixtures;
- fuzzy evidence can outrank a hard exact tier;
- a weight change is justified only by a hand-picked example rather than the
  reviewed corpus;
- natural-query pagination changes would be overwritten;
- required performance can only be achieved by silently dropping candidates;
- `/Volumes/Workspace` is unavailable.

## Maintenance notes

- Rank profiles and synonym maps are semantic versions. Change their version
  whenever weights, expansion, distance, or tie behavior changes.
- Keep explanations compact enough for machine consumers and debugging; they
  are not permission to leak source text.
- Optional semantic embeddings, feedback priors, and graph propagation must
  remain lower-priority evidence channels added by later plans.
