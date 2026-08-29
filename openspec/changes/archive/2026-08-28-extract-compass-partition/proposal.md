# Proposal: extract compass-partition

## Why

Canonical partition records and their stable encodings are useful outside immutable
history, but their definitions currently live behind history-specific dependencies.

## What Changes

- Add a storage-neutral `compass-partition` workspace crate.
- Move `PartitionedGraph`, canonical JSON encoding, and stable node, edge, and
  hyperedge key construction into it.
- Preserve the existing `compass-history` public surface through boundary adapters.

## Compatibility

Canonical bytes and typed-key bytes remain unchanged. `compass-history` continues
to expose the same helpers with its existing error boundary.
