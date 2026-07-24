# Plan 001: Make upstream compatibility lineage machine-checkable

> **Executor instructions:** Follow every step and verification gate. Do not
> push or open a pull request. Update this plan's status in
> `advisor-plans/README.md` when done. Stop on any STOP condition.
>
> **Drift check:** Run
> `git diff --stat 3837b411..HEAD -- COMPATIBILITY.md compatibility.toml scripts/check_compatibility_manifest.py .github/workflows/compass-ci.yml .github/workflows/compass-hardening.yml`
> before editing. If an in-scope file has changed, compare it with the current
> state below and stop when the contract no longer matches.

## Status

- **Priority:** P1
- **Effort:** M
- **Risk:** MED
- **Depends on:** none
- **Category:** dependencies, tests, docs
- **Planned at:** `3837b411`, 2026-07-23

## Why this matters

Compass declares Graphify v0.9.20 at an immutable commit as its oracle, while
CI checks out mutable `v8`. Graphify `origin/main` is a separate v1 lineage with
36 unique commits, not a newer commit on the oracle's ancestry. A single
machine-readable compatibility Module should own lineage identity,
normalizations, feature disposition, and required evidence so prose, CI, and
benchmarks cannot silently disagree.

## Current state

- `COMPATIBILITY.md:7-16` declares:

  ```text
  Python baseline: Graphify v0.9.20
  Baseline commit: edec9eabeceeae6aa2375eddb3835efa1a32c0a3
  ```

- `.github/workflows/compass-ci.yml:27-41` checks out `ref: v8`.
- `.github/workflows/compass-hardening.yml:260-272` also checks out `ref: v8`.
- Graphify `origin/main` is
  `91f4d120b630ee35c79bf3c75ccd186870a808f9`, package version `0.1.14`, and
  diverges from the v8 line after `81a43f028ff1d3fd9a0893318272348a38dad660`.
- The parity suite is intentionally coupled to v8-only fixtures and commands:
  running it against main produced 24 passes and 74 failures.

Use the repository's existing convention of pinned action SHAs and Python
checkers under `scripts/`.

## Commands

| Purpose | Command | Expected result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | exit 0 |
| Manifest check | `python3 scripts/check_compatibility_manifest.py --check` | exit 0 and prints exact oracle SHA |
| Workflow refs | `rg -n "ref: (v8|main)$" .github/workflows` | no mutable Graphify oracle refs |
| Parity | `cargo test -p compass-parity --locked` with manifest-declared oracle env | all tests pass |

## Scope

**In scope:**

- Create `compatibility.toml`.
- Create `scripts/check_compatibility_manifest.py`.
- Update `COMPATIBILITY.md`.
- Update `.github/workflows/compass-ci.yml`.
- Update `.github/workflows/compass-hardening.yml`.
- Add checker tests under `scripts/tests/` if that directory is established;
  otherwise test through a temporary manifest in the checker itself.

**Out of scope:**

- Changing native runtime behavior.
- Advancing the frozen v8 oracle.
- Claiming byte parity with Graphify `origin/main`.
- Installing Graphify at Compass runtime.

## Git workflow

- Branch: `advisor/001-compatibility-lineage`
- Commit style: imperative summary, for example
  `Make Graphify compatibility lineage machine-checkable`.
- Do not push or open a PR.

## Steps

### Step 1: Define one compatibility manifest

Create `compatibility.toml` with a versioned schema and these required fields:

- repository and immutable oracle commit;
- upstream lineage/ref label and merge-base metadata;
- Python version and enabled extras;
- allowed ordering normalizations;
- certified command families;
- required test and benchmark evidence targets;
- a separate capability-audit entry for Graphify main at `91f4d120...` with
  dispositions `compatible`, `superseded`, `intentional-divergence`, or
  `not-supported`.

Do not put executable shell fragments in the manifest. Keep it declarative.

**Verify:** parse it with Python's `tomllib`; expected result is exit 0 and the
exact 40-character frozen SHA.

### Step 2: Add a strict checker

Create `scripts/check_compatibility_manifest.py`. It must:

1. validate schema and full immutable SHAs;
2. reject mutable oracle refs in the two workflows;
3. verify the workflow checkout SHA equals the manifest oracle;
4. verify `COMPATIBILITY.md` contains the same oracle identity;
5. reject duplicate capability keys or unknown dispositions;
6. print a concise summary and return nonzero on drift.

Add negative tests for a shortened SHA, mutable `v8`, mismatched docs, duplicate
capabilities, and unknown disposition.

**Verify:** `python3 scripts/check_compatibility_manifest.py --check` exits 0;
each negative fixture exits nonzero with the expected field name.

### Step 3: Pin CI and hardening to the immutable oracle

Replace `ref: v8` only for compatibility-oracle checkouts with the manifest's
immutable SHA. Do not alter unrelated Git refs.

Add the manifest checker before environment installation so identity drift
fails quickly.

**Verify:** `rg -n "repository: Graphify-Labs/graphify|ref:" .github/workflows`
shows each compatibility checkout followed by the exact immutable SHA.

### Step 4: Reconcile the human ledger

Update `COMPATIBILITY.md` to explain:

- frozen v8 behavioral oracle;
- Graphify main legacy-line capability audit;
- why main parity is not byte parity;
- how to advance either record;
- which file is authoritative.

Do not claim the main capability audit is a release oracle.

**Verify:** checker exits 0 and Markdown links resolve locally.

## Test plan

- Unit-test all manifest validation branches.
- Run the checker on the real repository.
- Run the existing parity crate against the manifest-declared checkout.
- In a temporary copy, change the workflow ref to `v8` and confirm the checker
  fails before tests start.

## Done criteria

- [ ] `compatibility.toml` owns exact immutable lineage identity.
- [ ] CI and hardening use the declared full SHA.
- [ ] `COMPATIBILITY.md` distinguishes v8 oracle and main capability audit.
- [ ] Checker positive and negative tests pass.
- [ ] `cargo fmt --all -- --check` exits 0.
- [ ] No runtime source file changed.

## STOP conditions

- The maintainer wants Graphify main to replace, rather than supplement, the
  v8 oracle.
- The pinned v8 commit cannot reproduce the declared test environment.
- A workflow depends on a branch-only file not present at the frozen SHA.
- Implementing the checker requires changing runtime Rust behavior.

## Maintenance notes

Any future Graphify change must update the manifest, fixtures, evidence, and
human ledger together. Reviewers should reject mutable compatibility refs even
when a branch name appears stable.
