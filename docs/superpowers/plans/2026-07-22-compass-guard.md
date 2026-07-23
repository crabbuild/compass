# Compass Guard Implementation Plan

> **For the implementing agent:** Use `superpowers:executing-plans` and complete the tasks in order. Use test-driven development for every behavioral change. Do not overwrite unrelated changes already present in the Compass worktree.

**Goal:** Turn the implemented CompassQL and versioned-graph foundations into a production CI architecture-governance product that fails only on newly introduced violations, produces reviewable evidence, and works across major CI systems.

**Architecture:** `compass-policy` owns policy and pack schemas, evaluation, fingerprints, exemptions, baselines, differential results, and budgets. `compass-core` owns exact snapshot selection. `compass-output` renders one typed result into terminal, JSON, JSONL, JUnit, and SARIF. `compass-cli` remains the vendor-neutral entry point. The GitHub Action and other CI adapters install a pinned Compass release and consume the same CLI/report contract; they contain no policy logic.

**Technology:** Rust 2024, CompassQL, TOML, JSON Schema-style versioned envelopes, SARIF 2.1.0, JUnit XML, GitHub composite actions, GitLab CI components, Bitbucket Pipelines, Azure Pipelines.

## Scope reconciliation

The approved design and existing implementation plans remain authoritative for CompassQL syntax, zero-row policy semantics, `cmpv1` fingerprints, immutable snapshots, resource limits, exit codes, and the initial schema-1 policy lifecycle:

- `compass/docs/superpowers/specs/2026-07-22-compassql-design.md`
- `compass/docs/superpowers/plans/2026-07-22-compassql-policy.md`
- `compass/docs/superpowers/plans/2026-07-22-compassql-integrations.md`

The current repository already has CompassQL execution, `GraphSelection::{File, Commit}`, historical loading through `--at`, native cycle/god-node analysis, and signed release archives with SHA-256 sidecars. It does not yet contain a `compass-policy` crate, `compass check`, differential checks, or a reusable Action.

Compass Guard deliberately adds four contracts beyond the approved schema-1 design:

1. Policy schema 2 adds rationale, compatibility metadata, and accountable exemptions with approver and ticket.
2. Pack schema 1 adds versioning, dependency-free validation, and fixture-based tests.
3. JUnit and repeated `--report FORMAT=PATH` sinks let one evaluation feed several CI consumers.
4. Budget policies are a first-class policy kind; they do not materialize synthetic nodes or pretend every graph metric is portable Cypher.

Schema 1 stays readable and behaviorally unchanged. Official Compass Guard packs use policy schema 2. Unknown fields remain errors within each schema version.

## Product contract

The primary command is:

```bash
compass check \
  --against origin/main \
  --new-only \
  --policy-root .compass/policies \
  --fail-on error \
  --report sarif=compass-guard.sarif \
  --report json=compass-guard.json \
  --report junit=compass-guard.xml
```

The command evaluates base and head with the same policy-pack digest and effective limits. It partitions active violations by `(policy_id, cmpv1 fingerprint)` into `new`, `resolved`, and `unchanged`. Exit 1 means the selected failure threshold was crossed; exits 2–4 remain configuration, graph/execution, and internal failures as defined by the approved design. `--new-only` never turns a failed base/head evaluation or incompatible comparison into a clean result.

## Milestone 1: Complete the approved schema-1 policy engine

### Task 1: Add `compass-policy` and zero-row evaluation

**Files:**

- Modify: `compass/Cargo.toml`
- Modify: `compass/Cargo.lock`
- Create: `compass/crates/compass-policy/Cargo.toml`
- Create: `compass/crates/compass-policy/src/{lib,config,discovery,evaluate,error}.rs`
- Create: `compass/crates/compass-policy/tests/{support,evaluation,discovery}.rs`

**Steps:**

1. Copy the public `Severity`, `PolicyRequest`, `PolicySuiteResult`, and `Violation` contracts from the approved policy plan. Add the crate to the workspace.
2. Write failing tests for lexical discovery, duplicate IDs, path escape, symlinks, unknown fields, limits that exceed global limits, zero rows, one row per violation, typed evidence, cancellation, timeout, and query failure.
3. Implement strict policy schema 1 and execute every discovered policy against one immutable graph snapshot.
4. Preserve a returned `Path` named `witness` as primary evidence and preserve all remaining returned values as typed evidence.
5. Run:

   ```bash
   cd compass
   cargo test -p compass-policy
   cargo clippy -p compass-policy --all-targets -- -D warnings
   ```

6. Commit: `feat(policy): evaluate CompassQL architecture policies`.

### Task 2: Add fingerprints, exemptions, and baselines

**Files:**

- Create: `compass/crates/compass-policy/src/{fingerprint,exemption,baseline}.rs`
- Create: `compass/crates/compass-policy/tests/{fingerprint,exemption,baseline}.rs`
- Modify: `compass/crates/compass-policy/src/{lib,config,evaluate}.rs`

**Steps:**

1. Write failing stability tests proving that source-line moves do not change `cmpv1`, while stable node/relationship/path identities do.
2. Implement the approved length-prefixed canonical encoding and SHA-256 fingerprint.
3. Write failing exemption tests for exact matches, expiry, duplicates, unknown/stale fingerprints, and invalid dates. Implement schema-1 fields `reason`, `owner`, and `expires` exactly as approved.
4. Write failing baseline tests for canonical round trips, corruption, path safety, stale entries, and explicit atomic writes.
5. Implement `--write-baseline PATH --confirm`; never let a baseline suppress configuration, query, timeout, cancellation, or graph failures.
6. Run `cargo test -p compass-policy && cargo clippy -p compass-policy --all-targets -- -D warnings`.
7. Commit: `feat(policy): fingerprint and govern policy violations`.

### Task 3: Add source-rich terminal, JSON, JSONL, and SARIF output

**Files:**

- Create: `compass/crates/compass-output/src/{policy,sarif}.rs`
- Create: `compass/crates/compass-output/tests/{policy,sarif}.rs`
- Modify: `compass/crates/compass-output/src/lib.rs`
- Modify: `compass/crates/compass-output/Cargo.toml`

**Evidence contract:**

- Every violation includes policy location, owners, severity, message, fingerprint, exemption/baseline state, and typed evidence.
- A witness hop includes stable ID, label, `source_file`, `source_location`, relationship direction/type, `confidence`, `confidence_score` when present, and `_origin`/`extractor` when present.
- SARIF uses the first concrete source location as the primary location, witness hops as related locations, `cmpv1` as a partial fingerprint, policy owners/tags as properties, and execution failures as invocation notifications.
- Rendering is deterministic and contains no ANSI escapes outside terminal output.

**Steps:**

1. Add failing golden tests for an extracted edge, inferred edge, missing location, multiedge, escaped control characters, exempt violation, and execution failure.
2. Add an `EvidenceProvenance` view that reads existing graph attributes without changing graph identity or the fingerprint contract.
3. Implement renderers and validate SARIF against the checked-in SARIF 2.1.0 fixture/schema used by tests.
4. Run `cargo test -p compass-output && cargo clippy -p compass-output --all-targets -- -D warnings`.
5. Commit: `feat(output): render Compass Guard evidence and SARIF`.

### Task 4: Expose `compass check`

**Files:**

- Create: `compass/crates/compass-cli/src/check_commands.rs`
- Create: `compass/crates/compass-cli/tests/check.rs`
- Modify: `compass/crates/compass-cli/src/lib.rs`
- Modify: `compass/crates/compass-cli/Cargo.toml`
- Create: `compass/docs/COMPASSQL_POLICIES.md`
- Create: `compass/examples/compass-policies/domain-isolation/{policy.toml,query.cypher}`

**Steps:**

1. Write CLI tests for Compass-only exposure, default discovery, explicit roots, every format, fail thresholds, atomic output, Ctrl-C, and exit codes 0–4.
2. Add `check` dispatch beside `query`; do not expose it through the Graphify compatibility frontend.
3. Evaluate once per selected snapshot, render only after evaluation completes, and write files atomically.
4. Run `cargo test -p compass-cli --test check && cargo clippy -p compass-cli --all-targets -- -D warnings`.
5. Commit: `feat(cli): add Compass architecture policy checks`.

## Milestone 2: Make base-versus-head checks trustworthy

### Task 5: Move graph selection behind a shared snapshot service

**Files:**

- Create: `compass/crates/compass-core/src/query_service.rs`
- Create: `compass/crates/compass-core/tests/query_service.rs`
- Modify: `compass/crates/compass-core/src/lib.rs`
- Modify: `compass/crates/compass-cli/src/{lib,query_commands,check_commands}.rs`

**Steps:**

1. Move the existing CLI-private `GraphSelection` into `compass-core` and add the approved `SnapshotProvider`, `GraphSnapshot`, `SnapshotIdentity`, and `CompassQueryService` interfaces.
2. Write failing tests proving exact commit resolution, immutable in-flight snapshots, full validation before publication, and no reuse across schema/planner compatibility keys.
3. Adapt `query` and `check` to the service. Do not introduce a second revision selector or rebuild history behavior in `compass-policy`.
4. Run `cargo test -p compass-core -p compass-cli && cargo clippy -p compass-core -p compass-cli --all-targets -- -D warnings`.
5. Commit: `refactor(core): centralize snapshot-aware queries and checks`.

### Task 6: Implement differential policy results

**Files:**

- Create: `compass/crates/compass-policy/src/differential.rs`
- Create: `compass/crates/compass-policy/tests/differential.rs`
- Create: `compass/crates/compass-output/src/differential.rs`
- Create: `compass/crates/compass-cli/tests/check_differential.rs`
- Modify: `compass/crates/compass-policy/src/lib.rs`
- Modify: `compass/crates/compass-output/src/lib.rs`
- Modify: `compass/crates/compass-cli/src/check_commands.rs`

**Steps:**

1. Write failing tests for new/resolved/unchanged partitions, policy-set mismatch, policy digest mismatch, typed parameter mismatch, limit mismatch, fingerprint-version mismatch, corrupt base, corrupt head, and identical snapshots.
2. Compare only successful suites that share policy IDs, source digests, CompassQL version, typed parameter digests, effective limits, and fingerprint version.
3. Apply exemptions and baselines independently to each snapshot before comparison. Compare only active `(policy_id, fingerprint)` pairs.
4. Add `--against REV`; require it for `--new-only`. Preserve current evidence for new/unchanged and base evidence for resolved.
5. Render all partitions in text/JSON/JSONL/SARIF. With `--new-only`, SARIF includes only new active results while run properties retain counts for resolved and unchanged.
6. Run:

   ```bash
   cd compass
   cargo test -p compass-policy --test differential
   cargo test -p compass-cli --test check_differential
   cargo test -p compass-output
   cargo clippy -p compass-policy -p compass-cli -p compass-output --all-targets -- -D warnings
   ```

7. Commit: `feat(policy): detect newly introduced architecture violations`.

## Milestone 3: Add the governance and report contracts

### Task 7: Add policy schema 2 without weakening schema 1

**Files:**

- Create: `compass/crates/compass-policy/src/schema_v2.rs`
- Create: `compass/crates/compass-policy/tests/schema_v2.rs`
- Modify: `compass/crates/compass-policy/src/{config,exemption,lib}.rs`
- Modify: `compass/docs/COMPASSQL_POLICIES.md`

**Required schema-2 example:**

```toml
schema = 2
id = "architecture.domain-isolation"
query = "query.cypher"
severity = "error"
message = "Domain code must not depend on database implementation"
rationale = "Keeps business rules independently testable and storage-agnostic."
owners = ["platform-architecture"]
tags = ["architecture", "domain"]

[compatibility]
compass = ">=0.2.0, <0.3.0"
compassql = 1
graph_schema = 1

[[exemptions]]
fingerprint = "cmpv1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
reason = "Legacy adapter is being removed."
approver = "staff-architect@example.com"
ticket = "ARCH-241"
expires = "2026-10-01"
```

**Steps:**

1. Write failing tests for all required fields, invalid semver ranges, incompatible Compass/CompassQL/graph versions, invalid ticket strings, expired approvals, and schema cross-contamination.
2. Dispatch deserialization by the numeric schema before applying `deny_unknown_fields` to the selected version.
3. Keep schema-1 `owner` behavior intact. Require non-empty `approver`, `ticket`, `reason`, and `expires` in schema 2; do not silently translate one schema into the other on disk.
4. Require a schema-2 variable-length primary witness to be produced by `shortestPath` or `allShortestPaths`; fixed-length path witnesses remain valid. Reject a policy whose primary witness can be non-minimal, and test both accepted forms.
5. Include rationale and compatibility in JSON, JUnit properties, SARIF rule help/properties, and policy digests used by differential checks.
6. Run `cargo test -p compass-policy -p compass-output && cargo clippy -p compass-policy -p compass-output --all-targets -- -D warnings`.
7. Commit: `feat(policy): add accountable governance metadata`.

### Task 8: Add one-evaluation multi-report output and JUnit

**Files:**

- Create: `compass/crates/compass-output/src/junit.rs`
- Create: `compass/crates/compass-output/tests/junit.rs`
- Modify: `compass/crates/compass-output/src/{lib,policy,differential}.rs`
- Modify: `compass/crates/compass-cli/src/check_commands.rs`
- Modify: `compass/crates/compass-cli/tests/check.rs`

**Contract:**

- `--report FORMAT=PATH` is repeatable for `json`, `jsonl`, `junit`, and `sarif`.
- The suite is evaluated exactly once per snapshot; all renderers receive the same immutable typed result.
- JUnit uses one test case per policy, failures for threshold-crossing active violations, skipped cases for fully exempt policies, `<system-out>` for concise witnesses, and properties for snapshots, pack digests, owners, rationale, fingerprints, and counts.
- Operational/configuration errors remain nonzero CLI failures and also produce JUnit `<error>` elements when a JUnit sink was requested.

**Steps:**

1. Write failing XML escaping, deterministic ordering, new-only, exempt, and operational-error tests.
2. Implement JUnit without embedding arbitrary source content or terminal control sequences.
3. Parse and validate every report sink before evaluation; reject duplicate paths and prevent symlink/path escape.
4. Atomically write all reports. If a write fails, report the failing sink and return the approved output-failure exit without presenting a clean run.
5. Run `cargo test -p compass-output --test junit && cargo test -p compass-cli --test check`.
6. Commit: `feat(output): emit JUnit and multiple Guard reports`.

## Milestone 4: Ship the GitHub-first CI product

### Task 9: Publish a checksum-verifying composite Action

**Files:**

- Create: `compass/action.yml`
- Create: `compass/.github/actions/compass-guard/install.sh`
- Create: `compass/.github/actions/compass-guard/install.ps1`
- Create: `compass/.github/actions/compass-guard/run.sh`
- Create: `compass/.github/actions/compass-guard/run.ps1`
- Create: `compass/.github/actions/compass-guard/finalize.sh`
- Create: `compass/.github/actions/compass-guard/finalize.ps1`
- Create: `compass/.github/workflows/compass-guard-action-ci.yml`
- Create: `compass/tests/action/{passing,existing-only,new-violation,expired-exemption,invalid-policy}/`
- Modify: `compass/.github/workflows/compass-release.yml`
- Modify: `compass/README.md`

**Inputs:** `version`, `base`, `working-directory`, `build`, `graph`, `policy-root`, `fail-on`, `new-only`, `upload-sarif`, and optional `baseline`. `build` defaults to true and runs the local AST graph build; `build=false` requires `graph`.

**Outputs:** `sarif-path`, `json-path`, `junit-path`, `new-count`, `resolved-count`, `unchanged-count`, and `check-exit-code`.

**Steps:**

1. Write fixture workflows that assert pass, newly introduced violation, unchanged legacy violation, expired exemption, invalid policy, missing base ref, and fork-PR behavior.
2. On Linux/macOS/Windows, map `runner.os` and `runner.arch` to the exact release names already produced by `compass-release.yml`.
3. Download the requested immutable release and its `.sha256` sidecar, verify the archive before extraction, and refuse `latest` or an unqualified version. Reuse the runner tool cache for the exact version/target tuple.
4. Resolve the comparison side to an exact commit SHA from `base` or the pull-request event. If it is absent locally, fetch only that SHA into a private Compass ref. Never compare against an unresolved mutable branch name in the Action, and emit a precise shallow-clone diagnostic when the host refuses the fetch.
5. With `build=true`, run `compass update` in `working-directory` before checking. With `build=false`, validate the supplied graph path. The Action must not invoke semantic/model extraction implicitly.
6. Run `compass check` once with all three report sinks. Capture its exit long enough to upload artifacts and write `$GITHUB_STEP_SUMMARY`, then return the original exit in the finalizer.
7. When `upload-sarif=true`, upload the SARIF file through the current major `github/codeql-action/upload-sarif` action pinned to a full commit SHA. Document that the calling job requires `security-events: write`; automatically disable upload for untrusted fork PRs while preserving downloadable artifacts and action failure.
8. Upload JSON/JUnit/SARIF test artifacts from the Action CI matrix and verify the SARIF appears in an integration repository's code-scanning check before release.
9. Add release qualification that invokes the packaged Action against `new-violation` using the just-built archive, not a previously published binary.
10. Commit: `feat(ci): publish Compass Guard GitHub Action`.

## Milestone 5: Versioned, tested policy packs

### Task 10: Add pack schema, validation, and fixture tests

**Files:**

- Create: `compass/crates/compass-policy/src/{pack,pack_test}.rs`
- Create: `compass/crates/compass-policy/tests/{pack,pack_test}.rs`
- Create: `compass/crates/compass-cli/src/pack_commands.rs`
- Create: `compass/crates/compass-cli/tests/pack.rs`
- Modify: `compass/crates/compass-policy/src/lib.rs`
- Modify: `compass/crates/compass-cli/src/lib.rs`
- Create: `compass/docs/POLICY_PACKS.md`

**Pack manifest:**

```toml
schema = 1
id = "compass.official.layered"
version = "1.0.0"
description = "Dependency-direction rules for layered systems"
owners = ["compass-maintainers"]
license = "Apache-2.0 OR MIT"
policies = ["no-upward-dependencies", "no-storage-from-domain"]

[compatibility]
compass = ">=0.2.0, <0.3.0"
compassql = 1
graph_schema = 1
```

Each policy directory contains `tests/cases/<case>/graph.json` plus `expected.json`. Expected data records pass/fail and stable evidence IDs, not source line numbers.

**Steps:**

1. Write failing tests for manifest parsing, duplicate IDs, undeclared policy directories, missing policies, incompatible versions, path escape, symlinks, deterministic digest, and fixture mismatch.
2. Implement `compass pack validate PATH` and `compass pack test PATH`; both are offline and deterministic.
3. Include manifest bytes, ordered policy digests, and compatibility metadata in a `cmppack1:` SHA-256 digest. Record this digest in every suite/report and require equality in differential checks.
4. Do not add remote fetching in this milestone. Consumers vendor a pack directory or obtain an immutable release archive through their existing dependency workflow.
5. Run `cargo test -p compass-policy --test pack --test pack_test && cargo test -p compass-cli --test pack`.
6. Commit: `feat(policy): validate and test versioned policy packs`.

### Task 11: Ship the first five official packs

**Files:**

- Create: `compass/policy-packs/layered/`
- Create: `compass/policy-packs/hexagonal/`
- Create: `compass/policy-packs/domain-isolation/`
- Create: `compass/policy-packs/data-access/`
- Create: `compass/policy-packs/public-interface-stability/`
- Create: `compass/.github/workflows/policy-packs-ci.yml`

**Minimum policies:**

- Layered: presentation cannot reach persistence directly; dependencies point inward.
- Hexagonal: domain cannot depend on adapters/frameworks; adapters access domain only through ports.
- Domain isolation: cross-domain calls go through declared public interfaces; no shared persistence models.
- Data access: application/domain code cannot issue SQL or import concrete database clients; migrations cannot be called by runtime code.
- Public-interface stability: a differential check reports removal or incompatible movement of nodes tagged as public API.

**Steps:**

1. For every policy, write one allowed fixture, one direct violation, one transitive violation where applicable, and one false-positive regression fixture.
2. Require schema-2 rationale, owners, tags, compatibility, a minimal witness query, and a README showing customization parameters.
3. Test packs on Linux/macOS/Windows and against the minimum and current supported Compass versions.
4. Package each pack independently with manifest, policies, fixtures, digest file, SBOM, and provenance attestation.
5. Commit: `feat(packs): add official Compass Guard architecture packs`.

## Milestone 6: Architecture budgets

### Task 12: Add first-class budget policies

**Files:**

- Create: `compass/crates/compass-policy/src/{budget,budget_evidence,ownership}.rs`
- Create: `compass/crates/compass-policy/tests/{budget,ownership}.rs`
- Modify: `compass/crates/compass-policy/src/{config,evaluate,fingerprint,lib,schema_v2}.rs`
- Modify: `compass/crates/compass-graph/src/lib.rs`
- Modify: `compass/crates/compass-query/src/affected.rs`
- Create: `compass/policy-packs/budgets/`

**Design:** Policy schema 2 requires exactly one of `query` or `[budget]`. A budget evaluator returns deterministic typed evidence, then uses the same violation, fingerprint, exemption, baseline, differential, and output pipeline as query policies.

Supported kinds in the first release:

- `import_cycles`: maximum count, maximum cycle length, and relationship filter; reuse `find_import_cycles`.
- `god_nodes`: maximum count above a configured degree; reuse `god_nodes` but return stable node evidence.
- `coupling`: maximum directed cross-scope edges for configured path scopes and relations.
- `blast_radius`: maximum affected nodes from head-changed files/symbols, with configured relations and depth; valid only with `--against`.
- `unowned_assets`: maximum source files without an owner after resolving `.compass/owners.toml` and, optionally, repository CODEOWNERS.

**Steps:**

1. Write failing tests for each metric, deterministic ties, multiedges, missing communities, renamed files, changed-only behavior, invalid scope patterns, CODEOWNERS precedence, and empty ownership configuration.
2. Refactor existing analysis functions only enough to return source-rich stable evidence; keep existing public behavior compatible.
3. Parse `.compass/owners.toml` as the vendor-neutral source. If CODEOWNERS support is enabled, implement its documented last-match-wins semantics and repository lookup order with conformance fixtures.
4. For budget overages, emit the smallest deterministic evidence set that proves the threshold was crossed: shortest cycles first, highest-degree nodes first, sorted crossing edges, bounded affected paths, and sorted unowned files.
5. Fingerprint individual budget evidence items, not the aggregate count, so changed-only reports the newly offending cycle/node/edge/path/file.
6. Run `cargo test -p compass-policy -p compass-graph -p compass-query && cargo clippy -p compass-policy -p compass-graph -p compass-query --all-targets -- -D warnings`.
7. Commit: `feat(policy): enforce architecture budgets`.

## Milestone 7: GitLab, Bitbucket, and Azure adapters

### Task 13: Add thin, credential-free CI templates

**Files:**

- Create: `compass/ci/gitlab/compass-guard.yml`
- Create: `compass/ci/bitbucket/compass-guard.yml`
- Create: `compass/ci/azure/compass-guard.yml`
- Create: `compass/ci/scripts/install-compass.sh`
- Create: `compass/ci/scripts/run-compass-guard.sh`
- Create: `compass/tests/ci-contracts/`
- Create: `compass/docs/CI_INTEGRATIONS.md`

**Steps:**

1. Reuse the release checksum verifier and the exact CLI contract from the GitHub Action. Do not duplicate policy evaluation or parse human-readable output.
2. GitLab: publish `compass-guard.xml` as a JUnit report and JSON/SARIF as artifacts; preserve the Compass exit code.
3. Bitbucket: publish JUnit through test reporting and JSON/SARIF as artifacts; preserve the Compass exit code.
4. Azure: use `PublishTestResults@2` for JUnit and publish JSON/SARIF artifacts; preserve the Compass exit code.
5. Add shell contract tests that run each generated command locally with representative CI variables, a shallow clone, a missing base ref, and an existing-only violation.
6. Keep provider APIs and long-lived credentials out of this milestone. Native annotations come from SARIF/JUnit capabilities supplied by each platform.
7. Commit: `feat(ci): add portable Compass Guard pipeline templates`.

## Final qualification gate

Run from `compass/`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo test -p compass-policy
cargo test -p compass-output
cargo test -p compass-cli --test check --test check_differential --test pack
scripts/benchmark_compass_policies.sh
```

Then run the Action matrix and the three CI contract suites. The release is ready only when:

- unchanged debt does not fail `--new-only`;
- every new violation has a stable fingerprint and source-rich minimal witness;
- expired exemptions fail closed and every schema-2 exemption has approver, ticket, reason, and expiry;
- one evaluation produces byte-stable JSON/JUnit/SARIF;
- official packs validate and pass all fixtures on the supported Compass compatibility range;
- budget overages produce individual actionable evidence rather than only an aggregate number;
- release archives, the GitHub Action, and CI installers verify checksums before execution;
- policy, comparison, report, and pack schemas are versioned and documented.

## Explicit non-goals for the first Guard release

- A hosted policy service, dashboard, SSO/RBAC, or central waiver database.
- Automatic mutation of baselines or exemptions from CI.
- Pull-request commenting through provider tokens; SARIF, JUnit, job summaries, and artifacts are the initial review surfaces.
- Remote pack resolution at evaluation time.
- AI-authored policies in the enforcement path. AI may propose a policy, but checked-in deterministic policy files remain the authority.
