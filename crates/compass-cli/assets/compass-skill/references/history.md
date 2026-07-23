# Versioned graph history

Load this reference for questions about an exact Git commit, architecture drift,
or differences between revisions.

## Enable and build

```bash
compass history enable
compass history enable --code-only
compass history build HEAD
compass history status
```

History stores immutable realizations outside normal Git history. Enabling eager
history records a repository build profile and installs managed enqueueing
hooks. `--code-only` is the explicit local no-model profile.

Explicit historical queries can materialize a missing revision even when eager
history is disabled:

```bash
compass query "authentication flow" --at HEAD~20
compass path OldHandler Database --at v1.2.0
compass explain LegacyGateway --at RELEASE_TAG
```

Compass resolves a revision to an exact commit and builds it in a protected,
offline worktree. Report the resolved revision when answering.

## Compare and inspect

```bash
compass diff v1.2.0 HEAD
compass diff HEAD~1 HEAD --detailed
compass diff HEAD~1 HEAD --topology-only
compass history list HEAD --format json
compass history show HEAD
compass history export HEAD --format compass-out --output historical-output
```

Semantic realizations with different extraction fingerprints are not silently
treated as equivalent. Use `history list`, `show`, and `prefer` to inspect or
select an intended realization.

`history gc` and pruning options can delete unreferenced or alternate stored
data. Run their help and honor confirmation flags; do not prune merely to answer
a read-only question.

Use `compass history disable` only when the user wants eager enqueueing stopped.
It does not erase the history store.
