# Sub-Two-Second Versioned History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Materialize a clean, current, code-only Compass graph into immutable history in under two seconds for repositories below 10,000 lines of code, while preserving the exact-checkout path for every ineligible snapshot.

**Architecture:** Add a builder-owned promotion boundary that can return a previously completed `compass-out` snapshot only after verifying its commit stamp, corpus manifest, requested code-only profile, and Program IR provider inventory. Core history materialization will publish that verified snapshot directly from the repository root; stale, dirty, historical, semantic, rebuild, and corrupt-recovery requests continue through the detached-worktree builder. The VS Code extension continues to invoke only Compass CLI commands.

**Tech Stack:** Rust, Compass history/Prolly store, Git integration tests, Criterion-free wall-clock qualification on the release CLI.

## Global Constraints

- The VS Code extension must invoke Compass only; no Graphify runtime dependency or command is permitted.
- Clean current code-only repositories below 10,000 lines of code must complete `compass history build HEAD --code-only` in less than 2.0 seconds using a release binary.
- Exact historical commits, dirty/stale outputs, semantic profiles, rebuilds, and corrupt recovery must fail closed to the existing detached-worktree path.
- A promoted realization must be byte-for-byte identical to the realization produced by the exact builder for the same commit and profile.
- Existing unrelated worktree changes must be preserved.

---

### Task 1: Verified current-snapshot promotion contract

**Files:**
- Modify: `crates/compass-core/src/history.rs`
- Modify: `crates/compass-core/tests/history_materialize.rs`

**Interfaces:**
- Produces: `CompleteGraphBuilder::promote_current(&self, repository_root: &Path, commit: &CommitId) -> Result<Option<CompletedGraphArtifacts>, MaterializeError>`
- Consumes: existing `MaterializeRequest`, `CompletedGraphArtifacts`, publication observer, and exact fallback.

- [ ] **Step 1: Write the failing test**

Add a builder that returns a commit-stamped completed graph from `promote_current`, records whether `build` was called, and assert that materializing clean `HEAD` publishes the snapshot without calling `build`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p compass-core --test history_materialize current_snapshot`

Expected: FAIL because `CompleteGraphBuilder` has no promotion boundary and the exact builder is invoked.

- [ ] **Step 3: Write minimal implementation**

Add the default method:

```rust
fn promote_current(
    &self,
    _repository_root: &Path,
    _commit: &CommitId,
) -> Result<Option<CompletedGraphArtifacts>, MaterializeError> {
    Ok(None)
}
```

Before creating a detached worktree, request a promoted snapshot only for non-rebuild, non-corrupt-recovery `HEAD`; resolve its fingerprint from its stored Program provider descriptors, validate the commit stamp, and publish through the existing atomic store API.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p compass-core --test history_materialize current_snapshot`

Expected: PASS.

### Task 2: Native Compass snapshot eligibility

**Files:**
- Modify: `crates/compass-cli/src/history_build.rs`
- Modify: `crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Implements: `NativeCompleteGraphBuilder::promote_current`
- Consumes: `GraphArtifacts::load`, `Manifest`, `ManifestKind::Ast`, `detect`, and requested history profile options.

- [ ] **Step 1: Write failing integration tests**

Create a small committed repository and current `compass-out`. Assert:

```rust
// Eligible: current commit, code-only, exact manifest.
let promoted = run_history_build(&fixture, "HEAD", &["--code-only"])?;
assert_eq!(load_published_graph(&fixture, &promoted)?, fixture.current_graph);

// Ineligible: change a tracked source after the snapshot.
fixture.change_and_commit("service.rs", "pub fn changed() {}\n")?;
let rebuilt = run_history_build(&fixture, "HEAD", &["--code-only"])?;
assert_ne!(load_published_graph(&fixture, &rebuilt)?, fixture.stale_graph);
```

Also cover a semantic profile and an explicit rebuild to prove they use the exact path.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p compass-cli --test history_cli current_code_only_snapshot`

Expected: FAIL because the native builder does not offer a promotable snapshot.

- [ ] **Step 3: Implement eligibility checks**

Return `None` unless all conditions hold:

```rust
if !self.code_only {
    return Ok(None);
}
let artifacts = GraphArtifacts::load(output_dir)?;
if artifacts.document.extras.get("built_at_commit").and_then(Value::as_str)
    != Some(commit.as_str())
{
    return Ok(None);
}
let detection = detect(repository_root, &self.detect_options(repository_root))?;
let manifest = Manifest::load(&output_dir.join("manifest.json"), Some(repository_root));
if !manifest.is_unchanged(&detection.files, ManifestKind::Ast) {
    return Ok(None);
}
```

Load authoritative artifacts and zero-semantic completion evidence only after the checks succeed. Resolve the fast-path fingerprint from `artifacts.program.program.providers`; publication validates the Program bundle and artifact registry. Any read, parse, corpus, or provider mismatch falls back rather than weakening correctness.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p compass-cli --test history_cli current_code_only_snapshot`

Expected: PASS.

### Task 3: End-to-end release performance gate

**Files:**
- Modify: `crates/compass-cli/tests/history_cli.rs`
- Modify: `crates/compass-history/tests/performance.rs` if a reusable fixture helper is needed

**Interfaces:**
- Consumes: release `compass`, a clean current `compass-out`, and `history build HEAD --code-only --format json`.
- Produces: measured elapsed duration and identical realization evidence.

- [ ] **Step 1: Add the qualification fixture**

Use a deterministic repository below 10,000 source lines, build current `compass-out`, materialize once in a fresh history store, and assert:

```rust
assert!(elapsed < Duration::from_secs(2), "elapsed={elapsed:?}");
assert_eq!(promoted_realization, exact_realization);
```

Keep the strict wall-clock assertion ignored in ordinary debug test runs and execute it explicitly against the release binary.

- [ ] **Step 2: Verify the pre-fix release gate fails**

Run the release CLI against `<qualification-corpus-root>/fjall` in a fresh shared clone.

Expected baseline: approximately 3.94 seconds, exceeding the 2.0-second limit.

- [ ] **Step 3: Run the post-fix gate**

Run the same command and fixture with `target/release/compass`.

Expected: elapsed `< 2.0s`, exit code 0, and the same realization ID as the exact path.

### Task 4: Regression, extension boundary, and graph refresh

**Files:**
- Verify: `editors/vscode/src/views/historyPanel.ts`
- Verify: `editors/vscode/src/history/buildArguments.ts`
- Refresh: `graphify-out/`

**Interfaces:**
- Consumes: existing Compass JSONL progress protocol.
- Produces: no extension runtime dependency beyond Compass.

- [ ] **Step 1: Run Rust regression suites**

Run:

```bash
cargo fmt --all -- --check
cargo test -p compass-history --lib --tests
cargo test -p compass-core --test history_materialize
cargo test -p compass-cli --test history_cli
```

- [ ] **Step 2: Run VS Code and browser regression suites**

Run:

```bash
npm test -w compass-vscode
npm run typecheck -w compass-vscode
npm test -w @compass/viewer-tests -- history.spec.ts
```

- [ ] **Step 3: Confirm the extension boundary**

Inspect the extension build arguments and process launch path; verify versioned-graph operations invoke `compass history build`, `compass history timeline`, `compass history change-counts`, or `compass diff`, and no extension source imports or executes Graphify.

- [ ] **Step 4: Refresh the required development index**

Run `graphify update .` from the Compass repository solely because `AGENTS.md` requires it after code edits. This is a contributor-time operation and must not appear in extension code or packaged runtime assets.

- [ ] **Step 5: Final diff and performance verification**

Run `git diff --check`, repeat the fjall release benchmark in a fresh clone, and report the exact before/after stage timings.
