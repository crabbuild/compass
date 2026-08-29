# Design: actionable snapshot publication limit error

## Ownership

`compass-graph` owns the snapshot limit classification and remediation text.
`compass-cli` continues its thin presentation behavior by prefixing the domain
error with `error:` and returning runtime exit code 1.

## Error contract

An empty manifest byte count remains corruption. A count above
`MAX_GRAPH_BYTES` becomes `SnapshotError::Limit` with this stable actionable
suffix:

`retry or rebuild with a smaller scope using --exclude <pattern> or persistent patterns in .compassignore`

The rebuild wording is deliberate: the same typed manifest validation runs
during publication and while opening an immutable snapshot. A publication can
retry immediately; a reader can regenerate the snapshot from a narrower source
scope. Both contexts therefore receive an accurate recovery action.

## Variant and call-site audit

The `SnapshotError` variant audit found no production consumer that branches on
`Corrupt` versus `Limit`. `compass-core` converts the complete error through its
typed `Snapshot` wrapper, while `compass-query` maps every snapshot error to the
same `CorruptArtifact` query category and preserves the domain text. The only
variant-specific matches outside the owning module are assertions in
`compass-graph` tests. Reclassification therefore changes the public domain
category intentionally without altering fallback, quarantine, or cleanup flow.

The bounded serializer helper is named `digest_canonical_graph_json` and is
called only by canonical graph identity generation (plus its focused unit test),
so graph-scope remediation cannot leak into selector, delta, or tree encoding.

The message deliberately omits `COMPASS_MAX_GRAPH_BYTES`; C-004 owns adding it
only after the publication path honors the override.

## CLI evidence

The integration test first creates a valid SQLite-backed graph through the
released binary, then replaces only the active immutable manifest/selector
with a digest-consistent manifest whose byte count exceeds the cap. A second
binary invocation forces the store query engine, reaches native manifest
validation without allocating a large fixture, and asserts stderr plus exit 1.
The fixture uses only a local temporary SQLite database and preserves the
normal digest/selector trust chain so it exercises the intended limit branch.

## Compatibility and rollback

No migration action is required. Reverting the domain message and its test
restores prior behavior without touching persisted data.
