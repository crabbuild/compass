# Plan 006: Publish query v2 and qualify the end-to-end rollout

> **Executor instructions**: Begin only after Plans 001–005 are DONE and their
> qualification reports are green. This plan changes public machine and human
> query behavior; follow compatibility, migration, documentation, security, and
> baseline verification exactly. Do not remove v1 adapters. Update
> `plans/README.md` when complete.
>
> **Drift check (run first)**:
> `git diff --stat 43bceb6e..HEAD -- crates/compass-model/src/query_contract.rs crates/compass-query crates/compass-cli crates/compass-mcp fixtures/contracts docs README.md COMPATIBILITY.md MIGRATION.md CHANGELOG.md PERFORMANCE.md SECURITY.md scripts`

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: Plans 001–005
- **Category**: migration / docs / performance / direction
- **Planned at**: commit `43bceb6e`, 2026-08-06

## Why this matters

The new analyzer, ranker, intent planner, edge search, and graph propagation
need one stable public response that explains what Compass understood and why
each result was selected. Existing `compass.query/1` denies unknown fields and
cannot safely absorb intent alternatives, edge hits, rank components, work
budgets, or planner provenance. This phase publishes a strict v2 contract,
keeps v1 typed commands operational through adapters, migrates natural CLI/MCP
queries, and establishes release evidence.

## Current state

- `crates/compass-model/src/query_contract.rs:8` identifies
  `compass.query/1`.
- `crates/compass-model/src/query_contract.rs:110-160` returns operation,
  results, nodes, edges, files, paths, diagnostics, limits, and truncation.
- `crates/compass-model/src/query_contract.rs:162-168` gives a search hit only
  node ID, scalar score, and matched fields.
- `crates/compass-query/tests/query_contract.rs:65-78` verifies a checked-in v1
  manifest/fingerprint/example and strict serde behavior.
- `crates/compass-mcp/src/lib.rs:383-423` publishes both typed tools and a
  separate text `query_graph`; `tool_query_graph` at lines 575-609 bypasses the
  typed response.
- User-authored working-tree changes add deterministic `--budget`/`--page`
  pagination to natural `query` and `explain`. Preserve that work and make v2
  pagination describe one immutable planned result set.
- `COMPATIBILITY.md` requires native regression coverage, updated command/format
  docs, migration notes, and release notes for incompatible user-visible
  changes.

## Design

### Public v2 request and response

Add new strict types rather than changing v1 in place:

```text
schema: compass.query/2

QueryRequestV2
  question
  explicit_intent?       optional override
  scope[]
  edge_context[]
  include_heuristic
  include_rank_explanation
  limits
  page/cursor?            bound to request + graph + rank fingerprints

QueryResponseV2
  schema
  graph_identity
  analyzer_version
  rank_profile_version
  planner_version
  graph_profile_version
  plan
  hits[]                  tagged node | edge | path | community
  nodes[]
  edges[]
  paths[]
  files[]
  alternatives[]
  diagnostics[]
  limits
  work
  pagination
  truncated
```

Each hit contains a stable identity, result kind, rank ordinal, hard evidence
tier, deterministic score representation, bounded `RankExplanation`, and
supporting graph path/evidence IDs. Do not present numeric scores as calibrated
probabilities.

`work` includes candidates/postings decoded, fuzzy comparisons, nodes/edges
expanded, PageRank iterations, frontier peak, decoded/source/response bytes,
and bounds hit. Elapsed time may be included only in non-canonical diagnostics;
it cannot affect result ordering or fingerprints.

### Pagination/cursor integrity

Human `--page` remains supported. Machine v2 should prefer an opaque bounded
cursor containing or referencing:

- graph identity/snapshot ID;
- normalized request digest;
- analyzer/ranker/planner/graph-profile versions;
- page size/budget and next stable ordinal;
- checksum/version.

Reject a cursor used with a different graph, request, profile, or budget. Do not
embed source content or an unbounded result set in the cursor. For local CLI
page numbers, recompute deterministically against the pinned graph and verify
the same fingerprints; recommend `--at REV` for multi-page historical work.

### Compatibility policy

- Keep all `compass.query/1` request/response types and contract fixtures.
- Existing explicit CLI/MCP operations (`search`, callers, callees, impact,
  explore, node trail) continue to emit v1 by default during the migration
  window, implemented through v2-compatible internal services and an adapter
  that drops v2-only explanation fields.
- Natural `compass query` human output moves to v2 planning/ranking while
  retaining `--budget`, `--page`, `--dfs`, and existing exit behavior. Treat
  `--dfs` as an explicit trace-oriented profile, not a request to bypass bounds.
- Add `--scope VALUE` for subsystem anchoring and introduce/advertise
  `--edge-context VALUE`. Retain `--context` as a deprecated alias for its
  existing edge-context semantics for at least the documented support window;
  do not silently reinterpret it.
- Add a natural-query JSON mode that emits `compass.query/2`; choose the exact
  spelling after checking CLI parser conventions, document it, and test mutual
  exclusions with CompassQL `--format` behavior.
- MCP `query_graph` returns v2 in `structuredContent` and retains a bounded
  human text summary for compatible clients. Existing typed tool schemas remain
  available.
- Unknown major versions fail explicitly. No implicit v2-to-v1 parse fallback.

### Ambiguity and explanations

Human output begins with:

```text
Intent: incoming_relations (rule: callers_of, confidence: high)
Resolved: authorize -> sha256:...
Alternatives: ... (only when material)
Ranking: exact name + incoming call profile + trusted evidence
Bounds/Pagination: ...
```

Do not overwhelm normal output with every component; `--explain-ranking` or
machine JSON exposes the bounded detail. Low-confidence intent or close entity
scores produce alternatives and a retry suggestion. Operations requiring a
unique identity do not execute until resolved.

### Optional semantic/model integration boundary

Do not ship a provider in this plan unless separately approved. Document the
future extension contract:

- opt-in and explicit provider/network boundary;
- repository/query content labeled untrusted and minimized;
- provider returns only `UntrustedPlanProposal` or semantic candidate IDs/text,
  never graph facts or commands;
- strict schema/enum/limit validation and native entity resolution;
- provider/model/prompt version enters provenance and fingerprints;
- provider failure is explicit; native deterministic planning remains usable;
- no raw model-generated CompassQL/tool execution;
- local mocks only in tests, no real credentials.

### Rollout stages

1. **Shadow qualification**: v2 executes in tests/benchmarks beside legacy;
   collect only local aggregate metrics, no source/query telemetry by default.
2. **Explicit preview**: opt-in CLI/MCP v2 output; publish migration docs.
3. **Natural-query default**: human natural queries use v2; typed v1 shortcuts
   remain unchanged.
4. **Stabilize**: freeze v2 manifest/fingerprint and enforce performance gates.
5. **Future review**: deprecating any v1 surface requires a separate approved
   migration and release timeline.

Rollback is switching the natural-query adapter back to legacy v1 behavior;
never rewrite graph/store realizations. Keep old disposable indexes rebuildable.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Contract/model | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-model --locked` | all pass |
| Query | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked` | all pass |
| CLI | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-cli --locked` | all pass |
| MCP | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-mcp --locked` | all pass |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy --workspace --lib --bins --locked -- -D warnings` | exit 0 |
| Baseline tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test --workspace --lib --bins --locked` | all pass |
| Product | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-cli --test compass_product --locked` | all pass |
| Relevance | Plan 001 qualification command | final thresholds pass |
| Graph fixtures | `./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Boundary | `sh scripts/check_product_boundary.sh` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Patch | `git diff --check` | no output |

## Scope

**In scope**:

- `crates/compass-model/src/query_contract.rs`
- `crates/compass-model/src/lib.rs`
- `fixtures/contracts/compass-query-v2.manifest.json` (create)
- `fixtures/contracts/compass-query-v2.fingerprint` (create)
- `fixtures/contracts/compass-query-v2.example.json` (create)
- `crates/compass-query/src/` v2 orchestration/adapters and pagination cursor
- `crates/compass-query/tests/query_contract.rs`
- `crates/compass-query/tests/relevance_qualification.rs`
- `crates/compass-query/tests/code_query_scale.rs`
- `crates/compass-cli/src/lib.rs`, `help.rs`, and focused query argument modules
- `crates/compass-cli/tests/code_query_cli.rs`, `help_cli.rs`, and product tests
- `crates/compass-cli/assets/compass-skill/references/query.md`
- `crates/compass-mcp/src/lib.rs` and MCP contract/invocation tests
- `docs/reference/commands.md`
- `docs/reference/configuration.md`
- `docs/reference/outputs.md`
- `docs/implementation/query-engine.md`
- `docs/guides/exploring-a-codebase.md`
- `README.md`, `COMPATIBILITY.md`, `MIGRATION.md`, `CHANGELOG.md`,
  `PERFORMANCE.md`, and `SECURITY.md`
- qualification scripts/workflow hooks directly required by Plan 001 metrics

**Out of scope**:

- removing v1 types/tools/fixtures, changing `compass.graph/1`, changing graph
  extraction facts, mandatory network/model/vector dependencies, query
  mutations, telemetry that captures source/question content, or rewriting
  immutable history;
- unrelated viewer/VS Code changes;
- resolving or overwriting pre-existing user pagination edits without explicit
  reconciliation.

## Git workflow

- Branch: `advisor/006-query-v2-rollout`
- Suggested commits: v2 model/fixtures; query adapters/cursor; CLI; MCP;
  compatibility/security/docs; final qualification.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Freeze the v2 contract before interface work

Add strict v2 serde types, validation, sorting, limits, and canonical example,
manifest, and fingerprint. Test unknown fields/major versions, non-finite
scores, duplicate IDs, invalid ordinals, explanation component mismatch,
over-limit arrays/text, cursor mismatch, and stable serialization. Keep v1
tests byte-for-byte green.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --test query_contract --locked`
→ v1 and v2 fixtures validate and fingerprints match.

### Step 2: Add v2 orchestration and v1 adapters

Assemble analyzer, plan, retrieval, ranking, graph expansion, diagnostics,
work accounting, and pagination into one service. Implement explicit adapters
for current v1 operations; document every field dropped or mapped. Add
differential tests proving v1 exact/call/impact/explore semantics remain within
their compatibility contract.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-query --locked`
→ v1 regression and v2 end-to-end tests pass.

### Step 3: Integrate natural CLI behavior without losing pagination

Reconcile the user-authored `--budget`/`--page` work first. Add intent summary,
scope/edge-context arguments, JSON v2 mode, ranking explanation switch, strict
argument validation, and cursor/page mismatch errors. Preserve stdout/stderr,
exit codes, `--at` pinning, and deterministic pages. Update help snapshots and
subprocess tests.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-cli --test code_query_cli --test help_cli --locked`
→ human/JSON, ambiguity, pagination, scope/context compatibility, usage errors,
and historical pinning pass.

### Step 4: Publish MCP structured v2

Make `query_graph` invoke the same planner/service and return v2
`structuredContent` plus bounded text. Validate schema inputs, depth/budget
limits, cursors, graph identity, and project selection. Preserve typed tools
and ensure repository evidence cannot affect tool authorization or instruction
priority.

**Verify**:
`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-mcp --locked`
→ schema, invocation, parity, bounds, and untrusted-evidence tests pass.

### Step 5: Complete compatibility, migration, security, and user docs

Document:

- exact v1/v2 support and adapter behavior;
- changed natural-query ranking and intent behavior;
- `--scope`, `--edge-context`, deprecated `--context`, JSON mode, explanations,
  limits, pagination/cursors, ambiguity, and deterministic fallbacks;
- index/snapshot rebuild procedure from Plans 002–003;
- optional-provider privacy, prompt-injection, network, credential, and failure
  boundaries;
- release-visible changes and any user action.

Do not describe optional semantic/model search as shipped unless implemented
and qualified separately.

**Verify**:
`git diff --check && sh scripts/check_product_boundary.sh`
→ no whitespace/link-command errors and boundary passes.

### Step 6: Run final relevance and performance qualification

Run all query judgments on JSON and store engines, repeated cold/warm where
defined. Enforce the cross-plan targets from `plans/README.md`; publish corpus
version, graph digests, profile versions, hardware context for wall time, work
counts, p50/p95, peak RSS/index size if the existing harness supports them, and
all waivers. Add CI gates that are deterministic and keep hardware-sensitive
numbers as documented qualification evidence.

**Verify**:
Plan 001 qualification command plus `code_query_scale` and code-graph fixtures
→ all accuracy, recall, intent, direction, boundedness, parity, and current
elapsed ceilings pass.

### Step 7: Run the repository native baseline

Run the exact baseline from AGENTS.md using the external Cargo target:

```bash
cargo fmt --all -- --check
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo clippy --workspace --lib --bins --locked -- -D warnings
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test --workspace --lib --bins --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-cli --test compass_product --locked
./scripts/qualify_code_graph_v1.sh --fixtures-only
sh scripts/check_product_boundary.sh
```

**Verify**: every command exits 0. Report any platform or optional gate not run
and why; do not claim completion if a changed-surface required gate is skipped.

## Test plan

- Strict v1/v2 serialization, fingerprint, unknown-major, bounds, sorting, and
  cursor-integrity tests.
- V1 adapter tests for all existing operations and diagnostics.
- CLI subprocess tests for help, human/JSON, intent, scope/context, ambiguity,
  no-match, truncation, pagination, invalid cursor/page, `--at`, and exit codes.
- MCP schema/invocation tests for structured v2, text compatibility, limits,
  project selection, and prompt-injection isolation.
- Full Plan 001 relevance and Plan 003/005 performance qualification on JSON
  and store engines.
- Cold/warm/repeated deterministic output; no real credentials or network.

## Done criteria

- [ ] `compass.query/2` is strict, versioned, fingerprinted, bounded, and
  documented.
- [ ] V1 contract fixtures and explicit typed tools remain supported.
- [ ] Natural CLI and MCP queries use one typed intent/retrieval/ranking service.
- [ ] Node, edge, path, architecture, fuzzy, and intention-only results expose
  evidence, alternatives, truncation, and bounded ranking explanations.
- [ ] Pagination/cursors are bound to graph, request, and profile fingerprints.
- [ ] Exact Success@1 and backend parity are 100%; final relevance, recall,
  intent, edge-direction, and no-regression thresholds pass.
- [ ] Query work stays bounded and the existing 100,000-node ceiling passes.
- [ ] Compatibility, migration, release, performance, security, reference, and
  assistant docs are updated accurately.
- [ ] Repository baseline and all changed-surface gates pass.
- [ ] Only in-scope files and `plans/README.md` status are modified.

## STOP conditions

Stop and report if:

- v2 requires mutating v1 structs or accepting unknown fields under v1;
- current pagination changes are uncommitted/ambiguous and would be overwritten;
- JSON/store ordered responses differ after common ranking;
- optional model/network behavior is required for native query success;
- repository evidence can influence tool authorization or execute generated
  CompassQL/actions;
- cursors cannot detect graph/request/profile mismatch;
- an accuracy target can be met only by weakening bounds or hiding diagnostics;
- any required baseline gate fails twice after a scoped correction;
- `/Volumes/Workspace` is unavailable.

## Maintenance notes

- V2 fingerprints include analyzer, ranker, planner, graph profile, synonym,
  and index semantics. Review every future change against those versions.
- Keep legacy adapters until a separately approved deprecation with explicit
  release timing; do not infer removal from internal convergence.
- Treat query logs and repository text as potentially sensitive. Local-first
  means no source/question telemetry or provider calls by default.
- Reviewers should compare metric slices, not only the overall average, and
  should scrutinize waivers for language/framework bias.
