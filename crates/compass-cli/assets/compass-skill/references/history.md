# Versioned graph history

Load this reference for questions about an exact Git commit, architecture drift,
or differences between revisions.

## Enable and build

```bash
compass history enable
compass history enable --code-only
compass history build HEAD
compass history build main --all --code-only
compass history status
```

History stores immutable realizations outside normal Git history. Enabling eager
history records a repository build profile and installs managed enqueueing
hooks. `--code-only` is the explicit local no-model profile.

History uses only the current `networkx-node-link/v1` contract. Any store,
profile, or realization written under another contract is intentionally
unsupported and must be archived or removed before building fresh history.

Use `history build REF --all` to build every locally reachable commit in one
oldest-first batch. Add `--first-parent` to exclude merged branch histories.
The batch continues after commit failures and exits nonzero after printing its
complete report. Rerunning resumes safely by validating and skipping preferred
realizations that already match the selected profile:

```bash
compass history build main --all --first-parent --code-only --format json
compass history list --format json
```

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
compass diff HEAD~1 HEAD --all
compass diff HEAD~1 HEAD --format json
compass diff HEAD~1 HEAD --explain sd1-...
compass history diff HEAD~1 HEAD --format jsonl
compass history diff HEAD~1 HEAD --root nodes --root edges --output exact.jsonl
compass history list HEAD --format json
compass history show HEAD
compass history export HEAD --format compass-out --output historical-output
```

Use `compass diff` for a ranked semantic review. Use `compass history diff`
when the user needs an exhaustive, deterministic record-level comparison of
immutable graph roots.

Semantic realizations with different extraction fingerprints are not silently
treated as equivalent. Use `history list`, `show`, and `prefer` to inspect or
select an intended realization.

`history gc` and pruning options can delete unreferenced or alternate stored
data. Run their help and honor confirmation flags; do not prune merely to answer
a read-only question.

Use `compass history disable` only when the user wants eager enqueueing stopped.
It does not erase the history store.
