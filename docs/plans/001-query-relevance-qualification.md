# Plan 001: Establish a judged query-relevance qualification suite

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the STOP conditions occurs, stop and report; do
> not improvise. When done, update this plan's status in `docs/plans/README.md`.
>
> **Drift check (run first)**:
> `git diff --stat 64dcbf60..HEAD -- crates/compass-query crates/compass-cli benchmarks/performance scripts docs PERFORMANCE.md`
> Compare the current benchmark and pagination code with the excerpts below.
> If overlapping user changes are unresolved, stop before editing.

## Status

- **Execution status**: `DONE` for 001.

  Implementation completed in executor commit `d5e077b8`. Real query execution is
  now used for a compact executable corpus over the checked-in support graph with:
  - digest-pinned graph validation,
  - real `CodeQueryEngine` execution for search/callers/path/no-answer,
  - JSON/store parity and repeated-run determinism,
  - measured latency and response-byte counters,
  - direct deterministic metric reporting in the qualification test.

  A dedicated qualification command (`scripts/qualify_query_relevance.py`) runs
  the native test gate, and docs now describe threshold behavior, metric
  coverage, and execution/refresh policy.

  The earlier noted unrelated graph-package Markdown identity failure remains
  in Plan 007.
- **Priority**: P1
- **Effort**: L
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests / performance / direction
- **Planned at**: commit `64dcbf60`, 2026-08-07

## Why this matters

Compass currently measures how much text a focused query returns, but not
whether the correct node, edge, direction, or path was retrieved. Ranking work
without reviewed relevance judgments can improve a few demos while degrading
real repositories. This phase creates the objective release gate used by all
later phases and makes accuracy, recall, boundedness, and latency separately
visible.

## Current state

- `crates/compass-query/src/benchmark.rs:7-13` defines five generic questions.
- `crates/compass-query/src/benchmark.rs:15-31` records token count and reduction
  only.
- `crates/compass-query/src/benchmark.rs:178-197` treats any label substring
  seed as a successful query.
- `crates/compass-query/tests/code_search.rs:10-59` covers exact, prefix, alias,
  Unicode, and stable repetition, but has no graded ranked-list judgments.
- `crates/compass-query/tests/code_query_scale.rs:13-17` provides a useful
  100,000-node in-process ceiling but does not report work counts by query
  stage.
- `benchmarks/performance/repositories.toml` and
  `benchmarks/performance/compass/workloads.py` contain real-repository query
  examples, but their required/forbidden substring checks are not graded
  relevance labels.

Current benchmark shape:

```rust
// crates/compass-query/src/benchmark.rs:15-31
pub struct BenchmarkQuestion {
    pub question: String,
    pub query_tokens: usize,
    pub reduction: f64,
}

pub struct BenchmarkResult {
    // ... corpus and token-reduction fields ...
    pub per_question: Vec<BenchmarkQuestion>,
}
```

Match repository conventions:

- strict serialized contracts use `serde`, `deny_unknown_fields`, explicit
  schema strings, and deterministic ordering; use
  `crates/compass-model/src/query_contract.rs` as the exemplar;
- checked-in contract fixtures use a manifest, fingerprint, and example; use
  `crates/compass-query/tests/query_contract.rs:65-78` and
  `fixtures/contracts/compass-query-v1.*` as the exemplar;
- large-query timing tests open the index outside the measured query interval;
  use `crates/compass-query/tests/code_query_scale.rs:61-64` as the pattern.

## Design

### Qualification data contract

Create a versioned fixture contract `compass.query-judgments/1` with one corpus
manifest per immutable graph realization:

```text
schema                  compass.query-judgments/1
corpus_id               stable human-readable corpus name
graph_schema            compass.graph/1
graph_digest            canonical graph digest
repository_revision     immutable revision, never a mutable branch
analyzer_version        version used to interpret query text
queries[]:
  id                    stable within corpus
  text                  user question
  class                 exact | lexical | fuzzy | intent | edge | path | architecture | negative
  locale                optional BCP-47 tag
  expected_intent       optional typed operation
  expected_slots        optional entity/scope/direction slots
  node_judgments[]      node ID + grade 0..3
  edge_judgments[]      edge ID or endpoint/kind/direction identity + grade
  path_judgments[]      accepted ordered edge-kind/endpoint patterns + grade
  acceptable_ambiguity  node IDs that must remain alternatives
  must_not_return[]     known false positives
  notes                 reviewer rationale, not parsed for scoring
```

Grades mean: `0` irrelevant, `1` contextually useful, `2` relevant, `3` exact
answer. Judgments reference stable graph identities, not labels alone. Do not
copy private repository source into fixtures.

### Metrics

Implement deterministic calculations for:

- Success@1 and MRR@10 for exact/entity lookup;
- Recall@5, Recall@20, precision@10, and nDCG@10 for ranked results;
- intent macro-F1 and per-intent precision/recall;
- entity-slot exact match and accepted-ambiguity recall;
- edge kind/direction precision and recall;
- path acceptance rate and mean accepted-path rank;
- no-answer precision and false-positive rate;
- JSON/store ordered parity and repeated-run equality;
- p50/p95 wall time for non-CI qualification plus deterministic counts for
  candidates read, postings decoded, nodes/edges expanded, and response bytes.

Undefined metrics must serialize as `null` with a diagnostic explaining the
missing denominator; never emit NaN or silently substitute zero.

### Corpus slices

The initial reviewed fixture set must contain at least 80 questions and cover:

- stable ID, exact name, exact qualified name, alias, path, role, language, and
  framework searches;
- snake_case, camelCase, PascalCase, punctuation, accents, non-Latin text,
  acronyms, two-character identifiers (`db`, `io`, `id`, `ui`), and typos;
- common overloaded labels (`run`, `save`, `handler`, `new`), source-backed
  declarations versus placeholders, and test/generated alternatives;
- callers, callees, impact, path, explanation, architecture, data-flow, route,
  ownership, and cross-community questions;
- hub traps, ambiguous identities, missing facts, partial graph coverage, and
  legitimate no-answer cases.

Use small public fixtures in-tree for CI. Real-repository qualification may use
read-only checkouts under `/Volumes/Workspace/Github`, but checked-in judgments
must contain only stable IDs and short reviewer-authored questions/notes.

## Commands you will need

Before every Cargo command, verify `/Volumes/Workspace` is mounted and use the
external target directory.

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Query unit/integration | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked` | all tests pass |
| Query lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-query --all-targets --all-features --locked -- -D warnings` | exit 0, no warnings |
| Code-graph gate | `./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 and all fixture checks pass |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0 |
| Docs/patch | `git diff --check` | no output |

## Scope

**In scope**:

- `crates/compass-query/src/relevance.rs` (create)
- `crates/compass-query/src/benchmark.rs`
- `crates/compass-query/src/lib.rs`
- `crates/compass-query/tests/relevance_qualification.rs` (create)
- `crates/compass-query/tests/fixtures/relevance/` (create)
- `benchmarks/performance/compass/workloads.py`
- `benchmarks/performance/repositories.toml`
- `scripts/qualify_query_relevance.py` or a native Rust binary/test harness
  chosen after checking existing script conventions
- `PERFORMANCE.md`
- `docs/implementation/query-engine.md`

**Out of scope**:

- production ranking, tokenization, traversal, CLI, or MCP behavior;
- graph schema or query response schema changes;
- private-repository source, credentials, model calls, embeddings, or network
  calls in tests;
- weakening existing token-reduction or 100,000-node performance checks.

## Git workflow

- Branch: `advisor/001-query-relevance-qualification`
- Use focused imperative commit subjects matching repository history, for
  example `Add judged query relevance qualification`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Define and validate the judgments contract

Add strict serde types in `relevance.rs`, including schema validation, grade
bounds, duplicate-ID rejection, maximum questions/judgments/text bytes, stable
sorting, graph-digest matching, and actionable typed errors. Reject unknown
major schemas explicitly. Export only the minimum types required by tests and
qualification tooling.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query relevance_contract --locked`
→ valid fixture round-trips; unknown schema, invalid grade, duplicate IDs,
oversized text, and graph mismatch fail with typed errors.

### Step 2: Implement deterministic metrics

Add pure functions for each metric listed above. Define tie handling and empty
denominators explicitly. Use stable query IDs and stable node/edge IDs for
aggregation; report macro and per-slice results. Serialize a machine report
with schema `compass.query-qualification/1` and include graph digest, analyzer
version, ranker version, planner version, engine, limits, metrics, work counts,
and diagnostics.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query relevance_metrics --locked`
→ hand-calculated fixtures produce exact expected values and no serialized
NaN/Infinity.

### Step 3: Add reviewed CI fixtures

Create at least 80 judgments using existing small Compass fixtures. Use stable
IDs, edge kinds, directions, ambiguity sets, and negative results. Include a
short `README.md` in the fixture directory explaining review rules and how to
add a question without tuning it against the same result under test.

Split authorship where possible: one contributor proposes expected results and
another reviews them. Never derive expected IDs automatically from current
ranking, because that would encode the implementation as truth.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test relevance_qualification --locked`
→ all fixture contracts validate and a baseline report is generated in memory.

### Step 4: Add backend and determinism qualification

Run every applicable judgment against JSON and store engines. Compare full
ordered IDs, component scores when present, diagnostics, alternatives,
truncation, and work counters. Repeat each request and require byte-equivalent
machine output after removing explicitly non-deterministic timing fields.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test relevance_qualification backend_parity --locked`
→ JSON/store and repeated runs match exactly.

### Step 5: Publish the baseline and qualification command

Extend performance documentation with metric definitions, corpus review rules,
the checked-in baseline, and a command that exits nonzero on threshold failure.
Keep token reduction as a separate efficiency metric; do not relabel it as
accuracy. Add the command to the code-graph qualification workflow only after
its runtime is suitable for CI.

**Verify**:
`./scripts/qualify_code_graph_v1.sh --fixtures-only`
→ existing qualification plus relevance checks exit 0.

## Test plan

- Contract tests: valid round-trip, unknown major, duplicate IDs, invalid
  grades, over-limit inputs, graph mismatch, and deterministic ordering.
- Metric tests: perfect, partial, tied, empty, ambiguous, and no-answer cases.
- Integration tests: JSON/store parity, repeated-run parity, truncation,
  incomplete coverage, and the initial 80-question corpus.
- Performance tests: work counters are bounded; timing remains outside index
  construction and is informational unless existing stable ceilings apply.
- Model after `crates/compass-query/tests/query_contract.rs` for strict contract
  fixtures and `code_query_scale.rs` for timing boundaries.

## Done criteria

- [ ] At least 80 reviewed questions validate under
  `compass.query-judgments/1`.
- [ ] MRR, Recall@k, precision@k, nDCG, intent, edge, path, ambiguity, and
  no-answer metrics have unit coverage.
- [ ] A baseline machine report is deterministic and contains graph/ranker
  fingerprints plus bounded work counts.
- [ ] JSON/store applicable outputs match exactly.
- [ ] Token reduction remains reported separately from relevance.
- [ ] Targeted test, Clippy, fixture qualification, product-boundary, format,
  and `git diff --check` commands pass.
- [ ] No files outside the in-scope list are modified, except
  `docs/plans/README.md` status.

## STOP conditions

Stop and report if:

- the live pagination/query changes are not committed or intentionally included
  in the executor branch;
- fixture graph IDs are unstable across equivalent rebuilds;
- meaningful judgments require checking private repository content into this
  repository;
- JSON/store differences cannot be explained without changing production
  behavior (that belongs in Plan 002);
- a stable CI threshold cannot be expressed without relying only on wall-clock
  timing;
- `/Volumes/Workspace` is unavailable for Cargo verification.

## Maintenance notes

- Every ranking, analyzer, planner, edge-weight, or query-contract change must
  update the qualification report fingerprint and run this suite.
- Reviewers should reject corpus changes that merely bless a new ranking.
  Judgment changes require an evidence-based rationale independent of the
  implementation.
- Add new language/framework slices when their graph evidence becomes directly
  qualified; do not claim universal relevance from one ecosystem.
