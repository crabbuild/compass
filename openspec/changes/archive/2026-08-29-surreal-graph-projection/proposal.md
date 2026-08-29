## Why

Compass needs an optional native graph-database projection that can accelerate graph-shaped reads without replacing the canonical immutable `compass.graph/1` generation or weakening its identity, evidence, determinism, and bounded-work contracts. The C-011 license decision, C-012 persistent-engine probes, and C-013 precommitted qualification budgets now satisfy the hard gates for implementing that projection against pinned SurrealDB 3.2.4.

## What Changes

- Add an optional `compass-graphdb-surreal` integration crate with no default SurrealDB feature.
- Define a schemafull, generation-keyed node and relation projection with a closed mapping from every Compass edge kind to a typed relation family and a required original `kind` field.
- Stage and validate one immutable candidate generation before atomically swapping a repository-scoped active-generation pointer.
- Preserve stable node and edge IDs, direction, parallel edges, self-loops, provenance, confidence, source anchors, schema version, repository identity, and generation identity.
- Provide only typed projection operations; do not expose arbitrary caller- or model-authored SurrealQL and do not add MCP or CLI presentation logic.
- Support deterministic Mem tests and separately gated SurrealKV and RocksDB embedded profiles using exactly pinned SurrealDB 3.2.4.

## Capabilities

### New Capabilities

- `surreal-graph-projection`: Optional, generation-atomic projection of a validated Compass graph into schemafull SurrealDB node and relation records.

### Modified Capabilities

None.

## Impact

The workspace gains one focused integration crate and an exact optional SurrealDB dependency with non-default Mem, SurrealKV, and RocksDB features. Default Compass binaries, the canonical JSON/SQLite path, CLI and MCP contracts, and the `compass-model` public graph schema remain unchanged. Surreal-enabled artifacts remain subject to the recorded BSL 1.1 notice and redistribution conditions for SurrealDB 3.2.4.
