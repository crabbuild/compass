# Plan 002: Unify analysis and backend-neutral candidate retrieval

> **Executor instructions**: Follow every step and verification gate in order.
> Preserve unrelated changes. Stop on any condition listed below instead of
> inventing a migration or weakening backend equivalence. Update
> `plans/README.md` when complete.
>
> **Drift check (run first)**:
> `git diff --stat 43bceb6e..HEAD -- crates/compass-model crates/compass-graph/src/snapshot.rs crates/compass-query/src/{text.rs,index.rs,code_query.rs,graph_engine.rs,lib.rs} crates/compass-query/tests docs/implementation/query-engine.md COMPATIBILITY.md MIGRATION.md`

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `plans/001-query-relevance-qualification.md`
- **Category**: correctness / performance / tech-debt / migration
- **Planned at**: commit `43bceb6e`, 2026-08-06

## Why this matters

Compass has three different query analyzers and applies the public candidate
bound before relevance ranking. JSON FTS removes accents while immutable store
postings merely lowercase, and natural discovery has richer identifier
splitting than typed search. A common prefix can therefore exclude the best
candidate because its stable ID sorts later, and equivalent backends can
interpret the same Unicode query differently. This phase creates one portable,
versioned analysis and retrieval contract that later lexical, fuzzy, and intent
ranking can safely reuse.

## Current state

- `crates/compass-query/src/text.rs:166-217` strips diacritics, splits
  identifiers, emits Chinese bigrams, removes stopwords, and discards many
  two-character ASCII terms.
- `crates/compass-query/src/code_query.rs:1299-1335` separately tokenizes typed
  search and joins all terms as FTS prefix clauses with `AND`.
- `crates/compass-graph/src/snapshot.rs:3059-3064` has a third analyzer that
  only splits punctuation and lowercases.
- `crates/compass-query/src/index.rs:306-310` uses SQLite FTS5 `unicode61
  remove_diacritics 2`, so `cafe` may retrieve `café` from JSON while the store
  postings cannot.
- `crates/compass-query/src/code_query.rs:528-578` selects and truncates
  candidates in canonical node-ID order; exact/name/prefix tiers are computed
  only afterward at lines 580-605.
- `crates/compass-query/tests/store_engine.rs:77-113` verifies backend equality
  for an over-bound prefix but does not assert that a late-sorting exact result
  survives.

Relevant current code:

```rust
// crates/compass-query/src/code_query.rs:546-554
// common Rust ranking is applied only after that bound.
"SELECT n.id ... WHERE node_fts MATCH ?1 ORDER BY n.id LIMIT ?2"

// crates/compass-graph/src/snapshot.rs:3059-3064
fn search_terms(value: &str) -> impl Iterator<Item = String> + '_ {
    value.split(/* punctuation */).filter(/* nonempty */).map(str::to_lowercase)
}
```

Ownership rules:

- portable analysis types and semantics belong in `compass-model` because both
  graph snapshot construction and query execution require them;
- immutable index construction belongs in `compass-graph`;
- candidate orchestration, bounds, and common ranking inputs belong in
  `compass-query`;
- SQLite FTS is a disposable accelerator and must not define public ordering.

## Design

### Canonical analyzer

Add `compass_model::search` with a versioned `SearchAnalyzer` whose pure output
is independent of backend, locale, filesystem order, and hash iteration:

```text
SearchAnalysis
  analyzer_version
  normalized_phrase
  tokens[]
    normalized
    original span
    kind: whole_identifier | identifier_part | path_part | acronym | cjk_bigram
    exact_only: bool
  dropped[]
    original span
    reason: stopword | too_short | over_limit | punctuation
```

Required normalization order:

1. Unicode NFKD and combining-mark removal using existing workspace crates;
2. Unicode-aware lowercase/case fold with pinned semantics;
3. split paths and punctuation without assuming `/` is the host separator;
4. split snake_case, kebab-case, camelCase, PascalCase, and digit boundaries;
5. preserve the whole normalized identifier in addition to parts;
6. emit deterministic CJK bigrams plus the whole token within limits;
7. preserve two-character terms for exact/name/path lookup; one-character
   terms are exact-only unless the request explicitly opts into them;
8. apply versioned stopwords only to natural-question noise, never to quoted or
   exact symbol lookup;
9. deduplicate without changing first canonical occurrence order;
10. reject queries over 4,096 bytes or 32 semantic terms with typed errors.

Do not treat `AND`, `OR`, `NOT`, or `NEAR` as syntax in ordinary symbol text.
If advanced search syntax is ever supported, it must be an explicit request
mode and separate parser.

### Candidate contract

Add internal backend-neutral types in `compass-query`:

```text
CandidateChannel = ExactId | ExactQualified | ExactName | ExactAlias |
                   ExactPath | Term | Prefix
CandidateEvidence = channel + matched field + analyzed term IDs
CandidateSet = canonical node ID -> union of bounded evidence
CandidateDiagnostics = channel counts + channel truncation + aggregate truncation
```

The retrieval order is lexicographic by evidence tier, not a single floating
score:

1. resolve exact stable ID without consuming a broad candidate quota;
2. retrieve exact qualified/name/alias/path candidates with reserved quota;
3. retrieve term and prefix candidates per deterministic channel;
4. union by node ID while preserving every matched channel;
5. apply the aggregate bound after exact candidates are protected;
6. use canonical node ID only as the final tie-breaker.

Recommended default quota under the existing hard maximum of 256 candidates:
all exact candidates up to the aggregate cap, then 128 fielded term candidates
and 64 prefix candidates. The implementation may reduce broad quotas to honor
the caller's smaller `maxCandidates`; it must never silently exceed the bound.
Ambiguous exact matches are returned as alternatives, not collapsed.

### Index versioning

- bump the disposable JSON cache from current `compass-code-index/3` to `/4`;
- introduce `compass.store.graph-index/3` rather than rewriting an existing v2
  snapshot realization;
- store analyzer version and field identity with postings;
- add portable exact maps for normalized ID, qualified name, name, alias, and
  path;
- encode term postings by field and normalized token so common Rust scoring can
  distinguish why a node was retrieved;
- rebuild old disposable JSON indexes automatically, but reject/rebuild an
  unsupported store graph-index layout through the documented publication
  path; never mutate historical snapshot objects.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Model tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-model --locked` | all pass |
| Graph snapshot tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-graph snapshot --locked` | all pass |
| Query tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked` | all pass |
| Query lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-model -p compass-graph -p compass-query --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Fixtures | `./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |

## Scope

**In scope**:

- `crates/compass-model/Cargo.toml`
- `crates/compass-model/src/lib.rs`
- `crates/compass-model/src/search.rs` (create)
- `crates/compass-model/tests/search_analysis.rs` (create if integration tests
  are preferable to an inline unit module)
- `crates/compass-graph/src/snapshot.rs`
- `crates/compass-graph/tests/` snapshot qualification fixtures as required
- `crates/compass-query/src/text.rs`
- `crates/compass-query/src/index.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/src/retrieval.rs` (create)
- `crates/compass-query/src/lib.rs`
- `crates/compass-query/tests/code_search.rs`
- `crates/compass-query/tests/store_engine.rs`
- `crates/compass-query/tests/index_recovery.rs`
- `crates/compass-query/tests/relevance_qualification.rs`
- `docs/implementation/query-engine.md`
- `COMPATIBILITY.md`, `MIGRATION.md`, and `CHANGELOG.md` only for the index
  format/rebuild implications

**Out of scope**:

- fuzzy edit distance, BM25F weights, PageRank, intent selection, edge search,
  model providers, CLI/MCP command changes, or graph fact extraction;
- changing `compass.graph/1` nodes or edges;
- publishing SQLite FTS rank values;
- reusing or cleaning another checkout's Cargo target directory.

## Git workflow

- Branch: `advisor/002-canonical-analysis-retrieval`
- Suggested commits: analyzer contract and tests; snapshot/index migration;
  retrieval orchestration; differential qualification and docs.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Characterize current analyzer differences

Before production edits, add a table-driven test matrix for accents, Unicode
case, camel/snake/path segmentation, CJK, punctuation, FTS operator words,
short identifiers, quoted exact terms, and over-limit inputs. Run each case
through natural analysis, JSON retrieval, and store retrieval and record the
known pre-change differences as explicit failing/todo assertions on the branch.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test store_engine search_analyzer --locked`
→ test demonstrates the current mismatch before implementation and becomes
green by Step 4.

### Step 2: Implement the portable analyzer

Add bounded analyzer types and pure normalization to `compass-model`. Move the
portable behavior from `compass-query::text` without moving question-specific
stopword or intent policy into the model. Retain compatibility wrappers in
`text.rs` temporarily so current callers and pagination work continue to
compile.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-model search --locked`
→ analyzer golden cases pass twice with identical serialized output.

### Step 3: Build versioned exact and fielded postings

Change snapshot and disposable index construction to consume the analyzer.
Encode field identity, term statistics, and analyzer version. Keep ordered maps
and explicitly sorted values. Bump formats and add reopen/rebuild, corrupt
version, unknown-major, deterministic snapshot digest, and old-layout rejection
tests.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-graph snapshot --locked`
→ v3 snapshot round-trip/reopen is deterministic and v2 remains immutable.

### Step 4: Introduce exact-first bounded retrieval

Add `retrieval.rs` and route typed search through it. Reserve exact channels,
merge evidence, and apply aggregate limits after evidence-aware selection.
JSON and store implementations may retrieve differently internally but must
feed the same canonical merger. Add an over-bound fixture where the exact
qualified-name node has the lexicographically last ID and assert it ranks first.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test code_search --test store_engine --locked`
→ exact candidate survives and full JSON/store responses match.

### Step 5: Route legacy natural scoring through the shared analyzer

Replace duplicate normalization in natural discovery while preserving its
current ranking and paginated output semantics. This step changes analysis, not
intent or traversal. Update exact floating score fixtures only when the new
portable token evidence intentionally changes them; explain each change in the
commit message.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked`
→ natural pagination, explanation, typed search, and store tests all pass.

### Step 6: Document migration and qualification

Document the analyzer, exact-before-broad contract, index version changes,
automatic JSON cache rebuild, store snapshot rebuild procedure, and backend
parity. Update the relevance baseline from Plan 001 and require exact-query
Success@1 to be 100%.

**Verify**:
`./scripts/qualify_code_graph_v1.sh --fixtures-only && sh scripts/check_product_boundary.sh && git diff --check`
→ all commands exit 0.

## Test plan

- Analyzer unit matrix for every normalization class and bound.
- JSON/store differential tests for exact ID/name/qualified/alias/path, accent,
  short terms, operator words, CJK, camel/snake, empty and over-limit queries.
- Regression proving a late-ID exact match survives an over-bound prefix.
- Snapshot v3 deterministic build/reopen/corruption/unknown-major tests.
- Index recovery test proving a `/3` disposable JSON index rebuilds to `/4`.
- Plan 001 corpus comparison: exact Success@1 100%, ordered backend parity
  100%, and no slice regression above the allowed threshold.

## Done criteria

- [ ] One exported portable analyzer is used by natural analysis, JSON index
  construction, store postings, and query parsing.
- [ ] Analyzer output is versioned, bounded, deterministic, and tested across
  Unicode and identifier forms.
- [ ] Exact candidates cannot be lost to broad node-ID truncation.
- [ ] JSON/store full ordered responses and diagnostics match for all analyzer
  fixtures.
- [ ] Disposable index `/4` rebuild and immutable graph-index v3 migration are
  documented and tested.
- [ ] No public graph facts or v1 query response fields changed.
- [ ] All targeted tests, lint, format, qualification, and boundary checks pass.

## STOP conditions

Stop and report if:

- an implementation would make `compass-graph` depend on `compass-query` or
  create another crate cycle;
- exact candidates cannot be protected within the declared aggregate bound;
- adding fielded postings exceeds existing snapshot item/byte limits on
  qualification fixtures without an approved format design change;
- existing user pagination changes conflict with compatibility wrappers;
- historical snapshot objects would need in-place mutation;
- backend parity appears to require exposing backend-native rank values;
- `/Volumes/Workspace` is unavailable.

## Maintenance notes

- Any analyzer semantic change requires a new analyzer/index version and
  qualification baseline; never change normalization under an existing format
  identifier.
- Review candidate bounds as semantic behavior. A faster accelerator does not
  qualify if ordered candidates, truncation, or diagnostics differ.
- Keep exact, prefix, and later fuzzy channel evidence separate so ranking
  explanations remain auditable.
