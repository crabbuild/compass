# Plan 014: Ship typed pull-request risk review and a reusable GitHub Action

> **Executor instructions**: Follow the phases in order. Each phase is a
> separately deliverable pull request and must meet its acceptance criteria
> before the next phase starts. Preserve advisory risk and deterministic gates
> as separate concepts. Do not execute code from an untrusted pull request with
> write-capable GitHub credentials.
>
> **Drift check (run first)**:
> `git diff --stat 6680842c..HEAD -- crates/compass-prs crates/compass-semantic-diff crates/compass-core crates/compass-output crates/compass-cli crates/compass-mcp action.yml docs .github`
> If current types or the approved PR Intelligence design no longer match the
> excerpts below, stop and reconcile the design before implementing.

## Status

- **Priority**: P1 — highest product leverage
- **Effort**: L (four independently shippable phases)
- **Risk**: HIGH
- **Depends on**: immutable history and semantic diff already shipped; coordinate with the planned Compass Guard contract before adding a blocking gate
- **Category**: direction / CI / review
- **Planned at**: commit `6680842c`, 2026-08-10

## Why this matters

Compass already computes semantic changes, affected consumers, verification
gaps, and graph completeness, but its GitHub-facing `prs` command only counts
entities in changed files. The missing product layer is one canonical,
evidence-qualified review report that local CLI, MCP, GitHub summaries, and a
sticky comment can all consume. This plan makes that report reproducible and
keeps low-completeness evidence from producing a falsely reassuring score.

## Current state and constraints

- `crates/compass-prs/src/lib.rs:35-50` defines `PrInfo` with changed files,
  touched communities, and `nodes_affected`; it has no revision identity, risk
  factor, witness path, test gap, or completeness state.
- `crates/compass-prs/src/lib.rs:457-503` implements `compute_pr_impact` as a
  source-file match and count. It does not traverse callers or consumers.
- `crates/compass-cli/src/prs_commands.rs:92-118` attaches that count only for
  detail, triage, and conflict views.
- `docs/superpowers/specs/2026-07-22-pr-intelligence-design.md:59-78`
  approves `compass.pr_intelligence.report/1`, canonical finding fingerprints,
  advisory risk, deterministic gates, and completeness metadata.
- `crates/compass-semantic-diff/src/model.rs` is the closest existing model for
  affected consumers, reviewer actions, verification evidence, and confidence.
- The approved design says advisory risk does not block merging. Preserve that
  decision: the initial Action may fail only on a separately reported,
  deterministic gate. Do not implement a generic `fail-on-risk` input that
  turns a heuristic band into a merge decision.
- GitHub delivery is an adapter. It must not calculate risk or change finding
  semantics.

## Target design

Add a domain crate, `compass-pr-intelligence`, whose primary output is strict
schema `compass.pr_intelligence.report/1`. Its canonical identity binds:

- repository identity and PR number;
- merge base, PR head, target head, and synthetic merge-result object IDs;
- graph schema, extractor/configuration fingerprint, and analysis versions;
- evidence-manifest digest and graph completeness;
- an ordered set of findings with stable fingerprints;
- explainable `RiskFactor` values and one advisory `RiskBand`;
- independent deterministic `GateResult` values (`pass`, `fail`,
  `indeterminate`, or `error`).

The decision module uses a versioned integer rubric, not an opaque model.
Initial factors are changed public contracts, affected callers/consumers,
cross-community or bridge impact, cycles, weak-confidence witnesses,
verification gaps, and incomplete evidence. Incomplete evidence may increase
uncertainty or make the risk band unavailable; it may never lower risk.

## Commands executors will need

All Cargo commands must use the external checkout-specific target directory:

| Purpose | Command | Expected result |
| --- | --- | --- |
| Target preflight | `test -d /Volumes/Workspace && mkdir -p /Volumes/Workspace/crabbuild-target/compass-main && test -w /Volumes/Workspace/crabbuild-target/compass-main` | exit 0 |
| Domain tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-pr-intelligence --locked` | all tests pass |
| Adapter tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-prs -p compass-cli -p compass-mcp --locked` | all tests pass |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-pr-intelligence -p compass-prs -p compass-output -p compass-cli --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0 |

## Scope

**In scope**:

- root `Cargo.toml`/`Cargo.lock` and a new `crates/compass-pr-intelligence/`;
- reusable snapshot/diff services in `compass-core` and semantic-diff adapters;
- typed renderers in `compass-output`;
- thin `compass review` CLI and MCP adapters;
- root `action.yml`, GitHub delivery helper, fixtures, docs, compatibility and
  release notes.

**Out of scope**:

- auto-requesting reviewers, modifying source, or running arbitrary PR code;
- organization-wide federation, historical ML risk, or opaque scores;
- storing GitHub tokens or source excerpts in report history;
- making advisory risk a deterministic merge gate;
- using `pull_request_target` to check out or execute the contributor's head.

## Phase 1: Canonical local report foundation

**Context**: No forge interaction belongs in the first phase. The engine must
accept an already captured immutable change request, validated graph snapshots,
and semantic diff evidence.

**Deliverables**:

1. Create `compass-pr-intelligence` with strict input/report structs, typed
   errors, canonical JSON, and `cmpprv1:<sha256>` finding fingerprints.
2. Define `EvidenceManifest`, `RevisionSet`, `Completeness`, `RiskFactor`,
   `RiskBand`, and `GateResult`. Unknown major schemas and unknown fields fail.
3. Add a pure `analyze(request, base, result, semantic_diff)` operation. Inputs
   are borrowed/validated; the engine performs no Git, GitHub, filesystem, or
   provider access.
4. Implement the versioned rubric with deterministic ordering and checked,
   bounded arithmetic. Keep timestamps/durations outside canonical findings.
5. Add golden fixtures for exact, inferred, ambiguous, partial, identical,
   conflicted, and corrupt/incompatible inputs.

**Acceptance criteria**:

- repeated analysis of identical inputs is byte-identical;
- line-only moves do not change a finding fingerprint, while entity or witness
  identity changes do;
- partial evidence cannot produce a lower band than the same known factors
  under complete evidence;
- a conflicted synthetic merge makes merge-dependent findings/gates
  `indeterminate`, not clean;
- the domain crate has no `gh`, network, provider, or rendering dependency;
- domain tests and Clippy commands above pass.

## Phase 2: Exact revision capture and reusable review operation

**Context**: Current `compass prs` fetches mutable PR state and compares file
names. A trustworthy report must freeze revision IDs before opening graphs.

**Deliverables**:

1. In `compass-prs`, introduce a forge-neutral `ChangeRequestSource` and a
   GitHub adapter that resolves full object IDs, pagination, changed hunks, and
   merge outcome with bounded output/time.
2. In `compass-core`, add one review orchestration service that retains history
   leases while loading exact base/result snapshots and validates profile
   comparability before calling the domain engine.
3. Reuse `compass-semantic-diff` rather than recreating identity alignment,
   affected-consumer, or verification logic.
4. Add local-Git input for offline `--base REV --head REV`; it never fetches
   objects implicitly.
5. Add failure tests for force-push drift, missing objects, profile mismatch,
   unavailable graph, size/timeout limits, and merge conflicts.

**Acceptance criteria**:

- the evidence manifest is captured once and no adapter refetches mutable PR
  state after capture;
- reports bind exact full object IDs and graph/profile fingerprints;
- base/head profile mismatch fails explicitly before risk evaluation;
- downstream traversal retains direction, anchors, confidence, and shortest
  witness paths under explicit node/edge/depth/time limits;
- `cargo test -p compass-prs -p compass-core -p compass-pr-intelligence --locked`
  passes with the required target directory.

## Phase 3: CLI, JSON, Markdown, SARIF, and MCP projections

**Context**: One typed result must feed every presentation. Renderers may omit
details under a stated budget but may not recalculate findings.

**Deliverables**:

1. Add `compass review --base REV --head REV [--format text|json|markdown|sarif]`
   plus an explicit `--pr NUMBER --repo OWNER/REPO` adapter.
2. Add deterministic renderers in `compass-output`; include completeness,
   factors, witness locations, test gaps, omissions, and gate states.
3. Add an MCP tool returning the same structured report under existing MCP
   transport bounds; semantic content truncation must fail explicitly.
4. Add `--output PATH` using atomic writes and stable exit categories.
5. Update `COMPATIBILITY.md`, command/output references, security/privacy docs,
   `CHANGELOG.md`, and `MIGRATION.md` only if users must change behavior.

**Acceptance criteria**:

- JSON validates as `compass.pr_intelligence.report/1` and round-trips;
- text, Markdown, SARIF, and MCP findings share the same fingerprints/counts;
- renderer truncation reports exact omitted counts and never changes the
  canonical report digest;
- CLI tests assert stdout, stderr, exit code, files, unknown schema, limits,
  conflict, and incomplete-evidence behavior;
- product boundary, targeted tests, format, and Clippy pass.

## Phase 4: Safe sticky GitHub Action delivery

**Context**: The Action is a transport adapter. Analysis should run in a
read-only job; comment writing must be isolated so fork code never executes
with write-capable credentials.

**Deliverables**:

1. Add a composite or JavaScript `action.yml` that installs a pinned Compass
   release with checksum verification, runs `compass review`, uploads the JSON
   report, and writes the job summary.
2. Add a separate delivery step that upserts one marker-owned comment keyed by
   repository, PR, report schema, and Action identity. It validates report
   bytes/schema before posting and applies a hard comment-size bound.
3. For fork PRs, always produce the artifact/job summary; comment only when the
   workflow has safe write permission. Never use `pull_request_target` to run
   the head revision.
4. Support `fail-on: none|deterministic`. `deterministic` fails only for a
   `GateResult::Fail`; `indeterminate` and analysis errors get distinct
   documented outcomes. Do not add heuristic `fail-on-risk` in this phase.
5. Add a local mock GitHub API integration suite for create/update, duplicate
   markers, pagination, permission denial, rate limit, oversized comment, fork
   mode, retries, and stale report identity.

**Acceptance criteria**:

- rerunning the Action updates exactly one owned comment;
- no write token is exposed to a job executing untrusted PR code;
- missing comment permission still publishes a truthful job summary/artifact;
- `fail-on: deterministic` is driven only by typed gate results;
- Action fixtures use no real credentials/network and all targeted checks pass.

## Done criteria

- [ ] All four phases meet their acceptance criteria in order.
- [ ] Risk factors, completeness, and gates have versioned machine contracts.
- [ ] CLI, MCP, Action, and renderers consume one domain report.
- [ ] Exact revisions and profile comparability are proven in tests.
- [ ] Fork and permission behavior is documented and locally mocked.
- [ ] Relevant baseline/gate commands pass; any unrun full-workspace check is reported.
- [ ] `advisor-plans/README.md` marks this plan DONE.

## STOP conditions

Stop and report rather than improvising if the approved PR Intelligence design
has been superseded, semantic diff cannot expose a reusable typed operation,
exact merge-result snapshots cannot be constructed without executing untrusted
checkout code, or delivery would require write credentials in the analysis
job. Also stop if a proposed gate depends on an advisory/ambiguous factor.

## Maintenance notes

Version the factor rubric independently from presentation. Review every new
factor for completeness monotonicity: less evidence must never reduce apparent
risk. GitHub API fields and Action inputs are adapters; keep them out of
canonical finding identity so another forge can reuse the same report.
