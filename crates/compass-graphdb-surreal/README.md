# compass-graphdb-surreal

This optional integration crate projects one validated, immutable
`compass.graph/1` generation into schemafull SurrealDB node and relation
records. It does not replace Compass's canonical JSON/SQLite realization and
contains no CLI or MCP presentation logic.

The crate has no default engine feature. Enable exactly the embedded profile
you need:

- `mem` for deterministic tests and ephemeral sessions;
- `surrealkv` for an embedded SurrealKV database; or
- `rocksdb` for an embedded RocksDB database.

These features resolve exactly `surrealdb` 3.2.4. That dependency and its core
are licensed under Business Source License 1.1 before conversion, with a
Database Service restriction, Change Date 2030-01-01, and Apache-2.0 Change
License. Surreal-enabled binaries, libraries, containers, and archives must
preserve the applicable SurrealDB license and notices and must not be described
as exclusively OSI-open-source. See
[`docs/future/surrealdb-license-decision.md`](../../docs/future/surrealdb-license-decision.md)
and the exact tagged license fixture under
[`scripts/fixtures/surreal-persistent-probes/`](../../scripts/fixtures/surreal-persistent-probes/).

No API in this crate accepts arbitrary SurrealQL. Values are parameter-bound
through a closed internal statement set.

Projection publication claims an incomplete generation, stages idempotent node
and relation batches in bounded 512-record commits, validates the exact staged
identity sets, completes the immutable manifest, and switches the active
repository pointer last. Interrupted candidates remain invisible and can be
resumed without retaining one graph-sized transaction or rewriting the prior
active generation.

With an engine feature enabled, `SurrealProjection` exposes generation-pinned,
bounded native reads for callers, callees, impact, directed node trails,
connected structural subgraphs, and relation pagination. These operations
return the shared Compass structural query contract. Relation-page cursors are
opaque, checksummed, and bound to one repository, generation, and operation.
