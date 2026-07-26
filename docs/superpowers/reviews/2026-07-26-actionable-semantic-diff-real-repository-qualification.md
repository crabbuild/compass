# Actionable Semantic Diff Real-Repository Qualification

**Date:** 2026-07-26
**Binary:** local debug build from `codex/improve-vscode-diff-experience`

## Outcome

Versioned history and semantic diff completed successfully for arbitrary,
non-tip revisions in Podman, CocoIndex, and LevelDB. Reports contained semantic
contract, behavior, dependency, affected-consumer, source, and graph-structure
evidence where supported. Graph-only body changes now use exact, line-scoped
branch-condition evidence when available instead of always degrading to an
opaque implementation digest.

The qualification used immutable history realizations; none of the source
checkouts were switched to the historical commits.

## Podman

Repository: `/Volumes/workspace/Github/podman`

Compared:

- old: `d8380c9c80d9c4acf5afd59b65c4c779aaacbbf5`
- new: `7ac3e837075460ecdea5ce59e607cdaa6b6709fc`

Each realization contained 119,293 nodes and 262,724 edges. The source change
removes `if len(args) > 1` from
`libpod/container_inspect.go::getContainerInspectData`.

The final report produced an exact behavioral finding:

```text
libpod_container_getcontainerinspectdata control flow changed
Control flow changed: removed condition `if len(args) > 1`.
```

The finding records the before/after body digests, the removed condition,
`control_flow: partial`, and source evidence. Two JSON runs produced the same
SHA-256 digest:

```text
1a3434e316744ede6c9ef44a55bc586d2c139ff4a0ffbc7e56d33ff6bdb74ee5
```

`--explain` resolved the stable finding ID, and the checkout status before and
after materialization was byte-identical.

## CocoIndex

Repository: `/Volumes/workspace/Github/cocoindex-compass-audit-20260726`

Compared:

- old: `90571539fa291fc6e6b248095bd2c8a2ff68bab4`
- new: `71f9cc9dc693080310181a2d011fb737420f7907`

The old realization contained 17,729 nodes, 40,237 edges, 91,163 Program IR
facts, and 10,931 summaries. The new realization contained 17,808 nodes,
40,412 edges, 91,537 facts, and 10,981 summaries.

The final canonical JSON report contained:

- 333 findings in 12 feature groups;
- 9 source-file changes;
- 149 added, 70 removed, and 9 changed graph nodes;
- 247 added and 72 removed graph edges;
- public API, behavior, dependency, affected-consumer, effect, error, and test
  evidence;
- three exact, partial-control-flow explanations for added branch conditions.

`HEAD~1` and `HEAD` resolved to the expected full commit IDs. `--all` preserved
the same exhaustive 333-finding JSON contract. The removed `--detailed` flag
failed closed with exit status 2. No temporary-worktree path leaked into JSON
or HTML output.

## LevelDB

Repository: `/Volumes/workspace/Github/leveldb`

Compared:

- deadlock body edit: `78a352f47ed6c1e9d750545e9b242289185b87e1`
  to `4a0c572440c7df2f56a6f5fb5aec9e366d522edb`
- Zstd addition: `bfae97ff7d9fd0ceccb49b90a1e4c19fe7b57652`
  to `1d6e8d64ee8489a85ce939b819d106d2b54abb15`

The deadlock edit produced one explicit indeterminate implementation finding,
never location churn or an invented behavioral claim. The Zstd comparison
produced 67 findings, including exact added branch conditions and retained
call, reference, and import dependencies. Its graph delta contained 14 added,
2 removed, and 6 changed nodes plus 45 added and 12 removed edges.

All four revisions were materialized with one comparable, deterministic
code-only extraction profile. The LevelDB checkout remained clean.

## Verification

Passed:

```text
cargo fmt --all -- --check
cargo test -p compass-semantic-diff
cargo test -p compass-cli --test history_cli semantic_diff_end_to_end_languages -- --exact --nocapture
cargo test --workspace
cargo clippy -p compass-semantic-diff --all-targets --all-features --no-deps -- -D warnings
git diff --check
graphify update .
```

Strict workspace Clippy is not green because five unchanged
`clippy::needless_borrow` findings remain in
`crates/compass-graph/src/analyze.rs`. The semantic-diff crate is clean with
warnings denied, and the complete workspace test suite passes.

## Scale observation

The Podman debug-build qualification completed, but history publication peaked
at approximately 4.7–5.2 GB RSS. Initial materialization took roughly six
minutes; profile-based materialization of the adjacent revision took roughly
eleven minutes, including source-realization validation. These are debug-binary
observations rather than a release benchmark, so they do not establish a
production performance regression. Release-mode history publication should be
measured separately before adopting a memory or latency SLO.
