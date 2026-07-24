# Full-Ref Versioned History Build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. The user explicitly waived TDD; implement each task first, then add and run its tests.

**Goal:** Add `compass history build <REF> --all` so one command materializes every locally reachable commit, resumes safely, continues after failures, and returns a complete summary.

**Architecture:** Extend `compass-history::Repository` with offline parent-before-child commit enumeration. Parse bulk flags with existing build-profile options, then delegate sequential batch orchestration to a focused `history_batch.rs` module that reuses the current queue/materialization path. Preserve the existing store schema and single-commit behavior.

**Tech Stack:** Rust 2024, Git subprocess API, Serde JSON, SQLite/Prolly history store, Cargo integration tests, shell completion files.

## Global Constraints

- Default scope includes every locally reachable commit, including merged branches.
- `--first-parent` requires `--all`.
- Resolve the ref and build profile once before starting.
- Process sequentially in reverse topological order.
- Continue after per-commit failures and exit `1` if any commit failed.
- Skip only validated preferred realizations with the selected profile.
- Do not fetch, add parallelism, or change the history/store schema.
- Progress goes to stderr; JSON stdout is one stable object.
- The user's explicit instruction disables TDD for this implementation.

---

### Task 1: Offline reachable-commit enumeration

**Files:**
- Modify: `crates/compass-history/src/git.rs`
- Test: `crates/compass-history/tests/git.rs`

**Interfaces:**
- Consumes: `Repository`, resolved `CommitId`, existing `git_output`.
- Produces:

```rust
pub fn reachable_commits(
    &self,
    tip: &CommitId,
    first_parent: bool,
) -> Result<Vec<CommitId>, HistoryError>
```

- [ ] **Step 1: Implement repository traversal**

Add a method beside `first_parent_ancestors`:

```rust
pub fn reachable_commits(
    &self,
    tip: &CommitId,
    first_parent: bool,
) -> Result<Vec<CommitId>, HistoryError> {
    let mut arguments = vec!["rev-list", "--reverse", "--topo-order"];
    if first_parent {
        arguments.push("--first-parent");
    }
    arguments.push("--end-of-options");
    arguments.push(tip.as_str());
    let output = git_output(&self.root, &arguments)?;
    std::str::from_utf8(&output)
        .map_err(|error| HistoryError::Git(format!("Git returned non-UTF-8 history: {error}")))?
        .lines()
        .map(|value| {
            value.parse().map_err(|_| {
                HistoryError::Git(format!("Git returned invalid reachable commit ID {value}"))
            })
        })
        .collect()
}
```

- [ ] **Step 2: Add merge-DAG, linear-order, and SHA-256 tests**

Create real temporary repositories. Assert:

```rust
let tip = repository.resolve("main")?;
let all = repository.reachable_commits(&tip, false)?;
assert_eq!(all.last(), Some(&tip));
assert!(all.iter().position(|id| id == &side_commit).is_some());
assert!(
    all.iter().position(|id| id == &parent).unwrap()
        < all.iter().position(|id| id == &child).unwrap()
);

let first_parent = repository.reachable_commits(&tip, true)?;
assert!(!first_parent.contains(&side_commit));
assert_eq!(first_parent.last(), Some(&tip));
```

Reuse the existing conditional SHA-256 repository setup and verify every returned
ID has the configured object length.

- [ ] **Step 3: Run traversal tests**

Run:

```bash
cargo test -p compass-history --test git
```

Expected: all Git history tests pass.

---

### Task 2: Parse and document bulk build flags

**Files:**
- Modify: `crates/compass-cli/src/history_build.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`
- Test: `crates/compass-cli/src/history_build.rs`
- Test: `crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Consumes: existing `parse_build_command`.
- Produces two fields on `ParsedBuildCommand`:

```rust
pub(crate) all: bool,
pub(crate) first_parent: bool,
```

- [ ] **Step 1: Extend build parsing**

Initialize `all` and `first_parent` to false. Recognize both with the existing
singleton boolean-option branch:

```rust
"--all" | "--first-parent" => {
    if inline.is_some() {
        return Err(format!("{name} does not accept a value"));
    }
    if !seen.insert(name.to_owned()) {
        return Err(format!("duplicate {name}"));
    }
    match name {
        "--all" => all = true,
        "--first-parent" => first_parent = true,
        _ => unreachable!(),
    }
}
```

After parsing:

```rust
if all && command != "build" {
    return Err("--all is only valid for history build".to_owned());
}
if first_parent && !all {
    return Err("--first-parent requires --all".to_owned());
}
```

Exclude both flags from `direct_profile_option`, then return them in
`ParsedBuildCommand`.

- [ ] **Step 2: Update help surfaces**

Change canonical usage to:

```text
compass history build <REF> [--all [--first-parent]] [BUILD_PROFILE_OPTIONS] [OPTIONS]
```

Document:

```text
--all                    Build every commit reachable from REF
--first-parent           With --all, build only the first-parent lineage
```

Add examples for `main --all` and `main --all --first-parent --code-only`.

- [ ] **Step 3: Add parser and help tests**

Assert:

```rust
let parsed = parse_build_command(
    "build",
    &["main".into(), "--all".into(), "--first-parent".into()],
)?;
assert!(parsed.all);
assert!(parsed.first_parent);
assert!(!parsed.use_repository_profile || parsed.profile_from.is_none());
```

Also assert duplicate flags, valued flags, `rebuild --all`, and
`--first-parent` without `--all` return usage errors.

- [ ] **Step 4: Run parser/help tests**

Run:

```bash
cargo test -p compass-cli history_build
cargo test -p compass-cli --test history_cli history_help_and_empty_status_are_actionable_and_non_mutating
```

Expected: all selected tests pass.

---

### Task 3: Sequential batch orchestration and stable results

**Files:**
- Create: `crates/compass-cli/src/history_batch.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/history_commands.rs`
- Test: `crates/compass-cli/tests/history_cli.rs`

**Interfaces:**
- Consumes:

```rust
Repository::reachable_commits(&CommitId, bool)
HistoryBuildOptions::profile()
resolve_or_materialize(
    &Repository,
    CommitId,
    &HistoryBuildOptions,
    bool,
    bool,
) -> Result<(HistoryStore, PublishedVersion), String>
```

- Produces:

```rust
pub(crate) fn execute(
    repository: &Repository,
    reference: &str,
    tip: CommitId,
    commits: Vec<CommitId>,
    options: &HistoryBuildOptions,
    first_parent: bool,
    format: &str,
) -> Result<BatchExecution, String>

pub(crate) struct BatchExecution {
    pub(crate) stdout: String,
    pub(crate) failed: bool,
}
```

- [ ] **Step 1: Add serializable batch result types**

In `history_batch.rs` define:

```rust
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CommitBuildStatus {
    Built,
    Rebuilt,
    Skipped,
    Failed,
}

#[derive(Serialize)]
struct CommitBuildResult {
    commit: String,
    status: CommitBuildStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    realization: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<String>,
}
```

Add an envelope with `schema_version`, `ref`, `tip`, `scope`,
`profile_digest`, exact counts, and ordered results. Format the profile digest
as lowercase hexadecimal.

- [ ] **Step 2: Implement validation-aware classification**

For each commit, inspect the preferred realization before building:

```rust
let existing = HistoryStore::open_existing(repository)?;
let preferred = existing
    .as_ref()
    .map(|store| store.preferred(&commit))
    .transpose()?
    .flatten();

let matching = preferred.as_ref().is_some_and(|version| {
    version.version.build_profile == selected_profile
        && existing.as_ref().is_some_and(|store| store.validate(&version.id).is_ok())
});
```

Matching validated preferred realizations become `skipped`. A missing
preferred uses `rebuild = false` and becomes `built`. A valid mismatched
preferred uses `rebuild = true` and becomes `rebuilt`. Any error becomes
`failed`; redact and bound its diagnostic with the existing history diagnostic
limit.

- [ ] **Step 3: Implement progress, continuation, and rendering**

Before the loop:

```rust
eprintln!(
    "Building {} commits reachable from {} ({})",
    commits.len(),
    reference,
    short_commit(&tip)
);
```

After every commit, print `[index/total] short-sha status` to stderr. Never
return early for a per-commit failure. Render text or one JSON object only
after the loop.

Use `crate::process_cancellation()` once and check the atomic flag between
commits. Make `process_cancellation` `pub(crate)`; on interruption stop before
scheduling the next commit and return a runtime error that preserves completed
publications.

- [ ] **Step 4: Dispatch bulk builds**

Register `mod history_batch;` in `lib.rs`. In `execute_build`, resolve the
profile exactly as single-build mode does, then branch:

```rust
if parsed.all {
    let commits = repository
        .reachable_commits(&commit, parsed.first_parent)
        .map_err(runtime)?;
    let batch = crate::history_batch::execute(
        repository,
        &parsed.revision,
        commit,
        commits,
        &options,
        parsed.first_parent,
        &parsed.format,
    )
    .map_err(runtime)?;
    return if batch.failed {
        Err(report_failure(batch.stdout, "one or more history builds failed"))
    } else {
        Ok(batch.stdout)
    };
}
```

Expose only the minimal existing helpers required by `history_batch`.

- [ ] **Step 5: Add batch integration tests**

Build a real Git fixture with:

1. a code-only root commit;
2. a Markdown commit that fails without semantic credentials;
3. a commit deleting the Markdown and changing code.

Verify the batch continues, results are ordered, counts are exact, and exit
code is `1`. Add successful merge-DAG, profile-mismatch rebuild, configured
profile after disable, and rerun-all-skipped cases.

- [ ] **Step 6: Run batch tests**

Run:

```bash
cargo test -p compass-cli --test history_cli
```

Expected: all history CLI tests pass.

---

### Task 4: Completion and user documentation

**Files:**
- Modify: `completions/compass.bash`
- Modify: `completions/compass.fish`
- Modify: `completions/compass.ps1`
- Modify: `completions/_compass`
- Modify: `docs/reference/commands.md`
- Modify: `docs/guides/versioned-history.md`
- Modify: `crates/compass-cli/assets/compass-skill/references/history.md`

**Interfaces:**
- Consumes: approved CLI contract.
- Produces: discoverable `--all` and `--first-parent` flags across supported shells and docs.

- [ ] **Step 1: Extend shell completions**

Add `history`, history subcommands, and build flags where missing. Scope
`--all` and `--first-parent` to `history build`; keep existing global flags
unchanged.

- [ ] **Step 2: Update reference and guide**

Document:

```bash
compass history build main --all --code-only
compass history build main --all --first-parent
```

Explain reachable-DAG scope, stable ref resolution, oldest-first sequential
execution, profile consistency, skip/resume semantics, failure continuation,
and exit codes.

- [ ] **Step 3: Update embedded Compass skill**

Replace the shell-loop requirement with the canonical bulk command and include
`history list --format=json` as post-build verification.

- [ ] **Step 4: Check generated/user-facing text**

Run:

```bash
rg -n "history build.*--all|--first-parent" \
  completions docs/reference/commands.md docs/guides/versioned-history.md \
  crates/compass-cli/assets/compass-skill/references/history.md
```

Expected: every supported user-facing surface contains the new command.

---

### Task 5: Full verification and real-repository qualification

**Files:**
- Create: `scripts/qualify_history_build_all.sh`
- Test: all touched Compass crates and documentation surfaces.

**Interfaces:**
- Consumes: completed feature.
- Produces: release evidence.

- [ ] **Step 1: Run formatting, lint, and focused suites**

Run:

```bash
cargo fmt --all --check
cargo clippy -p compass-cli -p compass-history --all-targets -- -D warnings
cargo test -p compass-history
cargo test -p compass-cli --test history_cli
cargo test -p compass-core --test history_materialize
cargo test -p compass-output --test history_bundle
```

Expected: zero failures and zero clippy warnings.

- [ ] **Step 2: Qualify a shallow disposable cocoindex clone**

Create `scripts/qualify_history_build_all.sh` with arguments
`REPOSITORY REF [DEPTH]`. It must create a `file://` shallow clone, preserve
the original checkout, default `DEPTH` to `5`, and run:

```bash
compass history build HEAD --all --code-only --format=json
```

Assert with `jq`:

```bash
jq -e '
  .counts.total > 1 and
  .counts.failed == 0 and
  (.counts.built + .counts.rebuilt + .counts.skipped == .counts.total)
' first.json
```

Run the command again and assert:

```bash
jq -e '
  .counts.failed == 0 and
  .counts.skipped == .counts.total
' second.json
```

Compare `.counts.total` with `git rev-list --count HEAD`, validate every
preferred realization via `history status`, and verify the original cocoindex
checkout status is unchanged.

- [ ] **Step 3: Refresh graphify knowledge graph**

From `/Users/haipingfu/graphify` run:

```bash
graphify update .
```

Expected: `graphify-out/graph.json` and `GRAPH_REPORT.md` update successfully.

- [ ] **Step 4: Review final diff**

Run:

```bash
git diff --check
git status --short
git diff --stat
```

Expected: only intended feature, test, completion, documentation, and generated
graph changes attributable to this work.
