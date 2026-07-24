# Full-Ref Versioned History Build Design

**Date:** 2026-07-23  
**Status:** Approved for implementation planning

## Summary

Compass currently materializes one immutable graph realization per
`compass history build` invocation. Users who want complete versioned graph
history for a branch or tag must combine Compass with a shell loop over
`git rev-list`.

This feature makes that workflow a first-class Compass operation:

```bash
compass history build main --all
```

The command resolves the ref once, enumerates every reachable commit, and
materializes a comparable graph history oldest-first. It resumes safely by
skipping matching validated realizations, continues after individual commit
failures, prints a final summary, and exits nonzero when any commit failed.

## Goals

- Build every commit reachable from a branch, tag, or commit ref with one
  discoverable Compass command.
- Include merged-branch commits by default.
- Offer first-parent-only traversal for users who want the mainline.
- Use one normalized build profile across the entire operation so adjacent
  realizations remain comparable.
- Process oldest-first to maximize compatible ancestor reuse.
- Resume safely after interruption or failure.
- Continue past per-commit failures and report every failure at the end.
- Preserve Compass history's existing offline Git, validation, publication,
  queue, lease, and storage guarantees.
- Provide stable human-readable and JSON results.

## Non-goals

- Parallel materialization or a `--jobs` option.
- Remote fetching or automatic shallow-history deepening.
- Date, count, or path filters such as `--since` or `--max-count`.
- Bulk `history rebuild`.
- Background/detached bulk execution.
- A new SQLite schema or realization format.
- Changing the semantics of existing single-commit history builds.

## CLI contract

### Canonical forms

```text
compass history build <REF> --all [BUILD_PROFILE_OPTIONS] [OPTIONS]
compass history build <REF> --all --first-parent [BUILD_PROFILE_OPTIONS] [OPTIONS]
```

Examples:

```bash
compass history build main --all
compass history build v2.0.0 --all --first-parent
compass history build release/2026.07 --all --code-only
compass history build main --all --profile-from v2.0.0
compass history build main --all --format=json
```

`--all` is valid only for `history build`. `--first-parent` requires `--all`.
Both flags are singleton boolean options and reject inline values. Existing
option ordering and `--` end-of-options behavior remain unchanged.

All existing build-profile options apply to the complete batch. A user may
select the profile with direct profile options or `--profile-from`, subject to
the existing conflicts between those forms. When neither is present, Compass
uses the stored repository profile when configured and otherwise resolves its
normal default profile once.

### Scope

The default scope is every commit reachable from the resolved ref, including
commits introduced through merges:

```text
git rev-list --reverse --topo-order <resolved-tip-sha>
```

`--first-parent` selects only the ref's first-parent lineage:

```text
git rev-list --reverse --topo-order --first-parent <resolved-tip-sha>
```

These commands describe ordering semantics; Compass invokes Git through its
existing repository abstraction rather than a shell.

The ref is resolved to a full commit ID before enumeration. Later movement of
the branch or tag does not change the selected tip or commit set. Compass does
not fetch missing history. A shallow repository therefore builds only commits
available and reachable in that local repository.

## Execution model

### Preflight

Before materializing any commit, Compass:

1. discovers the repository;
2. parses and validates all options;
3. resolves the supplied ref to one immutable tip commit;
4. resolves and validates one normalized build profile;
5. enumerates the complete ordered commit set.

An invalid ref, invalid profile, unsupported option combination, or enumeration
failure exits `1` or `2` according to the existing Compass error taxonomy
before any build is enqueued.

### Sequential materialization

Compass processes commits sequentially in the enumerated oldest-first order.
For each commit:

1. Open the existing history store and inspect its preferred realization.
2. If the preferred realization validates and its stored normalized build
   profile digest equals the batch profile digest, record the commit as
   `skipped`.
3. If no preferred realization exists, use the existing
   `resolve_or_materialize` path and record a successful publication as
   `built`.
4. If a valid preferred realization exists with a different build profile,
   run a new materialization with the batch profile and record a successful
   publication as `rebuilt`.
5. If inspection, materialization, validation, or publication fails, record
   the commit as `failed` and continue to the next commit.

The selected profile is immutable for the duration of the batch. Environment
changes or repository configuration edits during execution do not alter later
commits.

Single-commit builds continue to use their current path. Batch orchestration
must delegate each actual build to the same queue, lease, exact detached
worktree, materializer, validator, and publisher used by single-commit builds.

### Resume and interruption

Every successful realization is published atomically before Compass proceeds
to the next commit. An interrupted process leaves all prior publications
valid. Re-running the same command selects the same profile and skips those
matching validated realizations.

If another Compass process is already materializing the same commit and
profile, batch mode uses the existing join/lease behavior. It does not create
a parallel materialization path.

On an interrupt, Compass stops scheduling new commits, preserves all completed
work, and exits with the platform-appropriate interrupted status. It does not
emit a misleading successful summary.

## Results and exit status

### Text mode

Progress is written to stderr so stdout remains suitable for the final result:

```text
Building 1,981 commits reachable from main (71f9cc9d)
[1/1981] 0a41bc12 built
[2/1981] 91c062f0 skipped
[3/1981] af7300de failed: unsupported Git filter
```

The final stdout summary is:

```text
ref: main
tip: 71f9cc9dc693080310181a2d011fb737420f7907
scope: reachable
profile: 52e0ef243c20b2a78e41bb36059dac1d61963b0b792d6c79a7ff0c12fd9b91b8
total: 1981
built: 320
rebuilt: 4
skipped: 1656
failed: 1
```

When failures exist, deterministic commit/diagnostic lines follow the counts.
Diagnostics use the existing bounded and redacted history diagnostic policy.

### JSON mode

`--format=json` writes progress to stderr and exactly one JSON object to
stdout:

```json
{
  "schema_version": 1,
  "ref": "main",
  "tip": "71f9cc9dc693080310181a2d011fb737420f7907",
  "scope": "reachable",
  "profile_digest": "52e0ef243c20b2a78e41bb36059dac1d61963b0b792d6c79a7ff0c12fd9b91b8",
  "counts": {
    "total": 1981,
    "built": 320,
    "rebuilt": 4,
    "skipped": 1656,
    "failed": 1
  },
  "results": [
    {
      "commit": "0a41bc12...",
      "status": "built",
      "realization": "..."
    },
    {
      "commit": "91c062f0...",
      "status": "skipped",
      "realization": "..."
    },
    {
      "commit": "af7300de...",
      "status": "failed",
      "diagnostic": "unsupported Git filter"
    }
  ]
}
```

Results retain the same oldest-first commit order used for execution.
Successful `built`, `rebuilt`, and `skipped` results contain the validated
realization ID. Failed results contain a bounded diagnostic and no realization
field.

The command exits:

- `0` when every commit is built, rebuilt, or skipped;
- `1` when one or more individual commits fail or a runtime/preflight error
  occurs;
- `2` for command-line usage errors.

A final stdout write failure returns `1`. Realizations already published
before that failure remain valid.

## Architecture

### `compass-history`

`Repository` gains an offline commit-enumeration method that accepts a resolved
tip and a first-parent boolean. It invokes Git without a shell, rejects
malformed output, accepts the repository's configured SHA-1 or SHA-256 object
format, and returns `Vec<CommitId>` in parent-before-child order.

This method owns Git traversal semantics so the CLI does not duplicate
repository discovery, object-format, environment, or error-handling rules.

### `compass-cli/history_build.rs`

`ParsedBuildCommand` gains:

```rust
all: bool,
first_parent: bool,
```

The parser enforces command and option conflicts. Existing profile resolution
continues to produce one `HistoryBuildOptions`; batch execution reuses that
resolved value rather than parsing or inspecting provider environment for each
commit.

### `compass-cli/history_batch.rs`

A new focused module owns bulk orchestration and rendering. Its internal model
is equivalent to:

```rust
struct BatchBuildResult {
    reference: String,
    tip: CommitId,
    scope: BatchScope,
    profile_digest: String,
    results: Vec<CommitBuildResult>,
}

enum CommitBuildStatus {
    Built,
    Rebuilt,
    Skipped,
    Failed,
}
```

The module receives the resolved repository, tip, ordered commits, and build
options. It does not implement Git checkout, extraction, storage, or
publication itself.

### `compass-cli/history_commands.rs`

`execute_build` dispatches to batch orchestration only when `parsed.all` is
true. Existing single-commit behavior remains the default. Small shared
helpers may become `pub(crate)` so both paths use identical profile lookup,
materialization, and validation logic.

### Help and documentation

Update:

- canonical CLI help and shell completions;
- `docs/reference/commands.md`;
- `docs/guides/versioned-history.md`;
- the embedded Compass skill history reference;
- the real-repository qualification documentation.

The primary guide example becomes:

```bash
compass history build main --all --code-only
```

## Safety and consistency

- Enumeration and materialization remain offline and never fetch.
- Exact historical worktrees keep hooks, credentials, filters, LFS smudging,
  and submodule recursion disabled under existing policy.
- The command never enables eager history implicitly.
- The batch profile contains no secrets and is resolved once.
- Each publication remains atomic and independently valid.
- A failed commit cannot publish an incomplete realization.
- A profile-mismatched existing realization is never counted as resumable
  success.
- Concurrent preference changes use existing compare-and-swap behavior and
  become per-commit failures rather than silent overwrites.
- Bulk mode introduces no new database transaction spanning multiple commits;
  that preserves resumability and avoids long-lived global locks.

## Testing strategy

### Repository traversal tests

- A linear SHA-1 repository returns root-to-tip order.
- A merge DAG includes merged-branch commits by default.
- The same merge DAG excludes side-branch commits with `first_parent`.
- SHA-256 repositories work when supported by installed Git.
- Unknown commits, malformed Git output, and unavailable Git objects fail
  without fetching.
- A resolved tip remains stable if the named ref moves afterward.

### Parser and help tests

- `build main --all` and `build main --all --first-parent` parse.
- `--first-parent` without `--all` is a usage error.
- `rebuild main --all` is a usage error.
- Duplicate or valued boolean flags are rejected.
- Direct profile options and `--profile-from` retain existing conflicts.
- A revision beginning with `-` works after `--`.
- Help, completions, and examples expose the canonical command.

### Batch integration tests

- A linear repository builds every commit oldest-first.
- A second identical run skips every commit.
- Explicit and stored profiles apply uniformly to every realization.
- Disabling eager history retains the profile for bulk builds.
- A mismatched preferred profile causes a new comparable realization.
- A middle commit that requires unavailable semantic credentials fails while
  earlier and later code-only-corpus commits complete.
- Per-commit failure produces a complete final summary and exit code `1`.
- All-success and all-skipped runs exit `0`.
- Text progress stays on stderr.
- JSON stdout is one stable object with ordered results and exact counts.
- Interrupt/restart leaves published commits resumable.
- Existing single-commit build and rebuild tests remain unchanged and green.

### Real-repository qualification

Create a shallow disposable clone of
`/Volumes/Workspace/Github/cocoindex` containing a small number of commits.
Run:

```bash
compass history build HEAD --all --code-only --format=json
```

Verify:

- every locally reachable commit has one matching validated preferred
  realization;
- a second run reports every commit as skipped;
- reported totals match `git rev-list --count HEAD`;
- adjacent diffs remain profile-compatible;
- the original checkout is unchanged.

## Acceptance criteria

The feature is complete when:

1. `compass history build <REF> --all` materializes every locally reachable
   commit, including merge-side history.
2. `--first-parent` restricts the set to the first-parent lineage.
3. All commits use one selected normalized profile.
4. Matching validated realizations are skipped on rerun.
5. A per-commit failure does not stop later commits.
6. Any per-commit failure makes the final exit code `1`.
7. Text and JSON summaries report exact, internally consistent counts.
8. Existing single-commit history behavior and storage formats do not change.
9. Unit, integration, resume, merge-DAG, profile, output, and real-repository
   qualification tests pass.
