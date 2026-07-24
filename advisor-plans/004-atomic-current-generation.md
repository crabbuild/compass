# Plan 004: Publish current outputs as one observable generation

> **Executor instructions:** This is a high-risk storage/output change. Add
> crash and concurrent-reader characterization before implementation. Do not
> remove legacy direct files until a compatibility migration is approved. Do
> not push or open a pull request.
>
> **Drift check:** Run
> `git diff --stat 3837b411..HEAD -- crates/compass-core/src/pipeline.rs crates/compass-files/src/build_guard.rs crates/compass-model/src/document.rs crates/compass-mcp/src/lib.rs crates/compass-output/src/history_bundle.rs docs/reference/outputs.md`
> and stop if current-generation publication already exists.

## Status

- **Priority:** P1
- **Effort:** L
- **Risk:** HIGH
- **Depends on:** plan 001
- **Category:** correctness, architecture
- **Planned at:** `3837b411`, 2026-07-23

## Why this matters

Compass atomically replaces individual files, but readers can observe a new
graph with old labels, report, manifest, semantic marker, or Program IR.
`BuildGuard` leaves an incomplete marker after a crash, yet production readers
do not consult it. History bundles already demonstrate the stronger
Implementation: build a staging directory, validate it, then rename it.

## Current state

- `crates/compass-core/src/pipeline.rs:836-960` writes `graph.json`, root,
  analysis/labels, report, HTML, semantic marker, manifest, and `program.json`
  sequentially before `guard.commit()`.
- `crates/compass-files/src/build_guard.rs:13-32` exposes
  `ensure_complete`, but native graph/MCP readers do not call it.
- `crates/compass-model/src/document.rs:70-98` loads `graph.json` directly.
- `crates/compass-mcp/src/lib.rs:136-167` hot-reloads and caches the direct file.
- `crates/compass-output/src/history_bundle.rs:38-61` builds and validates a
  staging directory, then renames it to the destination.

Use the history-bundle pattern as the exemplar Module. Preserve output
compatibility through an explicit Adapter.

## Commands

| Purpose | Command | Expected result |
|---|---|---|
| Core | `cargo test -p compass-core --all-targets --locked` | all pass |
| Files/model | `cargo test -p compass-files -p compass-model --all-targets --locked` | all pass |
| Output | `cargo test -p compass-output --all-targets --locked` | all pass |
| MCP | `cargo test -p compass-mcp --all-targets --locked` | all pass |
| Product | `cargo test -p compass-cli --test compass_product --locked` | all pass |
| Format/lint | workspace format and scoped all-target clippy | exit 0 |

## Scope

**In scope:**

- A versioned current-generation layout and pointer contract.
- Pipeline staging, validation, publication, and recovery.
- Native reader resolution.
- Compatibility materialization for direct legacy paths.
- Crash/concurrency tests and output documentation.

**Out of scope:**

- Changing graph schema or semantic content.
- Changing immutable history identity.
- Removing legacy files in the first migration.
- Rewriting every exporter.
- Network/distributed storage.

## Git workflow

- Branch: `advisor/004-atomic-current-generation`
- Use small commits: contract/tests, publisher, readers, compatibility Adapter,
  documentation.
- Do not push or open a PR.

## Steps

### Step 1: Specify generation identity and layout

Define a private versioned layout, for example:

```text
compass-out/
  generations/<content-id>/
    graph.json
    manifest.json
    program.json
    ...
  current
  graph.json                compatibility materialization
```

The generation ID must bind all authoritative artifacts and renderer/profile
versions. Specify same-filesystem rename requirements and Windows behavior.

**Verify:** contract tests reject missing/mismatched artifacts, path traversal,
duplicate generation IDs, and unsupported schema versions.

### Step 2: Add pre-implementation crash tests

Introduce a test-only failpoint at each existing publication boundary. Assert
that a reader sees either the old complete generation or the new complete
generation, never a mixture.

Add concurrent MCP reload tests where publication pauses before pointer swap.

**Verify:** at least one test fails on current sequential publication by
observing mixed metadata or ignored incomplete state.

### Step 3: Build and validate a staging generation

Extract publication from `build_graph_inner` behind a narrow Interface. Write
all artifacts to an adjacent staging directory, fsync where the repository's
durability contract requires it, validate digests/schema/references, then rename
to the immutable generation path.

Do not make callers reassemble artifact order.

**Verify:** injected failure before rename leaves the last-good generation
readable and cleans or safely quarantines staging.

### Step 4: Atomically switch readers

Switch a small pointer/descriptor only after validation. Update native graph,
query, MCP, export, and watch readers to resolve the current generation once per
request and retain it for that request.

Readers must not fall through to partially written compatibility files.

**Verify:** concurrent-reader tests pass under repeated publication loops.

### Step 5: Preserve legacy direct paths through an Adapter

Materialize or atomically link/copy legacy direct files after generation
publication. Document which files are authoritative and which are compatibility
views. A failure in compatibility materialization must not corrupt the current
pointer.

**Verify:** existing CLI/product/parity tests using `compass-out/graph.json`
continue to pass.

### Step 6: Recovery and cleanup

On startup/update:

- ignore incomplete staging;
- validate current pointer;
- retain last-good generation;
- bound obsolete-generation cleanup;
- never delete a generation held by an active reader.

**Verify:** tests cover crash windows, corrupt pointer, missing generation,
Windows rename semantics where CI supports them, and active-reader retention.

## Test plan

- Failpoint at every artifact write and before/after pointer swap.
- Concurrent MCP and CLI readers.
- Cross-device/same-filesystem validation.
- Corrupt/missing pointer recovery.
- Compatibility materialization failure.
- Watch rebuild and history materialization regression.
- No mixed `graph.json`/manifest/program/report generation.

## Done criteria

- [ ] One validated generation becomes visible with one atomic switch.
- [ ] Native readers retain one generation per request.
- [ ] Last-good output survives every injected crash window.
- [ ] Legacy direct paths remain compatible.
- [ ] All scoped tests, clippy, and format checks pass.

## STOP conditions

- Output roots cannot guarantee same-filesystem rename.
- Windows requires a materially different pointer contract not covered by CI.
- A legacy consumer requires observing files while a build is in progress.
- Generation identity conflicts with immutable history identity.
- The change requires silently deleting user-owned output files.

## Maintenance notes

The pointer, generation schema, and compatibility projection are public
operational contracts even if their directory names are private. Reviewers
should scrutinize durability assumptions, reader lifetime, Windows behavior,
and cleanup races.
