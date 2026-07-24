# Plan 005: Gate pull requests and releases on production qualification

> **Executor instructions:** Keep external/provider-dependent tests explicitly
> classified. Do not make required CI depend on credentials or live services.
> Do not push or open a pull request.
>
> **Drift check:** Run
> `git diff --stat 3837b411..HEAD -- .github/workflows/compass-ci.yml .github/workflows/compass-hardening.yml .github/workflows/compass-release.yml Makefile PERFORMANCE.md scripts`
> and stop if a tagged-SHA qualification gate already exists.

## Status

- **Priority:** P1
- **Effort:** M
- **Risk:** LOW
- **Depends on:** plan 001
- **Category:** tests, performance, DX, docs
- **Planned at:** `3837b411`, 2026-07-23

## Why this matters

Graphify main runs its complete 293-test suite on each pull request. Compass has
far more and deeper integration tests, but normal CI uses `--lib --bins` and
weekly hardening owns all-target coverage. Release builds do not require
qualification evidence for the tagged commit, despite `PERFORMANCE.md` saying
they do. The performance matrix also excludes clustering and does not exercise
hot in-process MCP/query paths.

## Current state

- `.github/workflows/compass-ci.yml:71-91` runs workspace `--lib --bins`, one
  CLI integration target, and selected TCKs.
- The workspace contains 87 integration-test files; 29 are under
  `crates/compass-cli/tests/`.
- `.github/workflows/compass-hardening.yml:47-62` owns all-target execution.
- `.github/workflows/compass-hardening.yml:252-310` owns performance evidence.
- `.github/workflows/compass-release.yml:28-95` depends on metadata and builds,
  not hardening evidence for the tag SHA.
- `scripts/qualify_phase1.sh:78-83` and `PERFORMANCE.md:43-46` deliberately use
  `--no-cluster`.
- `PERFORMANCE.md:8-18` documents nonexistent `rust/scripts/...` paths.

Use action SHA pinning and retained artifacts exactly as current workflows do.
Consume the compatibility manifest from plan 001.

## Commands

| Purpose | Command | Expected result |
|---|---|---|
| Test inventory | `cargo metadata --no-deps --format-version 1` plus classifier | every integration target has one class |
| Native PR suite | `cargo nextest run --workspace --all-targets --locked` with committed filters | all self-contained tests pass |
| Qualification | `scripts/qualify_phase1_matrix.sh` | raw evidence and pass/fail summary |
| Docs check | `python3 scripts/check_docs.py --check` | exit 0 |
| Workflow syntax | existing shell/Python syntax checks | exit 0 |

## Scope

**In scope:**

- Test classification and native all-target PR job.
- Exact-SHA reusable qualification workflow.
- Release dependency on approved qualification evidence.
- Clustered and hot in-process benchmark cases.
- Documentation command/link checker.
- `Makefile`, `PERFORMANCE.md`, and relevant scripts/workflows.

**Out of scope:**

- Live model/provider calls in required CI.
- Changing benchmark thresholds without recorded baseline evidence.
- Publishing Linux/Windows artifacts.
- Replacing correctness parity with performance metrics.

## Git workflow

- Branch: `advisor/005-production-qualification-gate`
- Commit separately: test classification, qualification workflow, release gate,
  docs validation.
- Do not push or open a PR.

## Steps

### Step 1: Classify integration targets

Create a committed inventory with exactly three classes:

- `native-self-contained`;
- `oracle-dependent`;
- `external-or-scheduled`.

Fail CI when a newly discovered integration target is unclassified. Keep
network, credentials, large media, mutation, and fuzz suites out of the native
self-contained class unless hermetic fixtures prove otherwise.

**Verify:** the inventory command reports zero unclassified and zero duplicate
targets.

### Step 2: Run self-contained all-target tests on pull requests

Add a PR job using nextest filters generated from or checked against the
inventory. Add `make test-native-all` with the same contract.

Keep oracle-dependent differential tests in a separate job pinned through plan
001.

**Verify:** both commands run from a clean checkout without Python, model
credentials, databases, or network services after dependency fetch.

### Step 3: Create a reusable exact-SHA qualification workflow

Refactor hardening qualification into a callable workflow that records:

- Compass commit;
- Graphify oracle commit and environment digest;
- Rust/Python/tool versions;
- target and runner identity;
- corpus digest;
- profile/features;
- raw samples and thresholds.

Include existing compatibility-isolating `--no-cluster` cases plus:

- clustered end-to-end update;
- hot same-process natural query;
- hot MCP reload/query;
- cancellation and budget behavior;
- history materialization;
- memory residency after multiple project loads.

**Verify:** every artifact contains the exact tested Compass SHA and manifest
identity.

### Step 4: Require qualification for release

Before release publication, resolve a successful qualification run for the
exact tag SHA and verify its artifact signature/digests. Do not accept a branch
head, cache key alone, or prior commit.

Keep per-target packaging smoke tests.

**Verify:** a dry-run release with no matching evidence fails before build
publication; a run with valid exact-SHA evidence proceeds.

### Step 5: Add executable documentation validation

Create `scripts/check_docs.py --check` to validate:

- relative links;
- referenced local files;
- documented script paths;
- selected non-mutating `--help` examples;
- stale `rust/scripts/` prefixes.

Fix `PERFORMANCE.md` to use `scripts/qualify_phase1.sh` and
`scripts/qualify_phase1_matrix.sh`.

**Verify:** checker passes, and changing one path to a missing file makes it
fail with file and line.

### Step 6: Retain reviewable evidence

Upload raw samples, summaries, compatibility manifest, and logs even on
qualification failure. Use bounded retention and avoid secrets/environment
values.

**Verify:** a deliberately failed threshold still uploads a complete,
secret-free evidence bundle.

## Test plan

- Inventory positive/negative tests.
- Clean native all-target run.
- Oracle-dependent job with immutable SHA.
- Release dry run with missing, wrong-SHA, corrupt, and valid evidence.
- Benchmark smoke mode for PRs and full mode for qualification.
- Documentation checker broken-link/path fixtures.

## Done criteria

- [ ] Every integration target is classified.
- [ ] Self-contained all-target tests run on every PR.
- [ ] Qualification binds exact Compass and Graphify commits.
- [ ] Release publication requires exact-tag qualification evidence.
- [ ] Production clustered/hot paths have baselines.
- [ ] Canonical docs commands are mechanically checked.
- [ ] No required job depends on live credentials/services.

## STOP conditions

- The repository cannot distinguish hermetic and external integration tests.
- Qualification evidence cannot be bound to an exact tag SHA.
- Required benchmark runners are too noisy to sustain existing thresholds;
  collect data and request threshold approval instead.
- A release platform cannot consume the reusable workflow's evidence safely.

## Maintenance notes

Adding a test target requires classifying it. Adding a release path requires
declaring its qualification evidence. Benchmark thresholds should move only
with retained before/after raw data and an explicit rationale.
