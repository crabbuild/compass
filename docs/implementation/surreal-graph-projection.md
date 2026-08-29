# Optional Surreal graph projection

`compass-graphdb-surreal` is an additive library integration. It projects one
validated, immutable `compass.graph/1` document into SurrealDB while the normal
JSON/SQLite realization remains canonical and fully supported. No CLI or MCP
route selects this adapter yet.

## Dependency and engine boundary

The crate's default feature set includes projection planning but no SurrealDB
dependency. Three explicit features select embedded engines:

| Feature | Purpose | C-014 evidence |
| --- | --- | --- |
| `mem` | Deterministic ephemeral integration qualification | Full schema, round-trip, idempotence, interrupted staging, reactivation, native-query, and scale-differential coverage |
| `surrealkv` | Persistent embedded SurrealKV | Constructor/SDK compile gate; persistent semantic and recovery evidence is retained by C-012 |
| `rocksdb` | Persistent embedded RocksDB | Constructor/SDK compile gate; persistent semantic and recovery evidence is retained by C-012 |

All profiles resolve exactly SurrealDB 3.2.4 with default SDK features disabled.
The repository gate `scripts/check_surreal_feature_isolation.sh` proves that
`compass-cli`, `compass-mcp`, `compass-core`, and the projection crate without
an engine feature have no SurrealDB dependency path. Its `--binary` mode also
builds the default `compass` binary only after proving its complete dependency
closure contains no SurrealDB package.

## Projection contract

Projection planning validates the source graph before producing records. Every
record carries repository, generation, and projection-schema identities. Nodes
retain stable Compass identity, kind, name, language, source, confidence, and
the exact typed JSON payload. Relations additionally retain stable edge
identity, source and target, original edge kind, source evidence, confidence,
and one of five closed relation families.

The families are structural, dependency, execution, data flow, and evidence.
The exhaustive Rust match over `EdgeKind` makes a new Compass edge kind a
compile-time integration decision. Callers cannot supply a relation table or a
SurrealQL statement.

Every engine client owns finite `ProjectionLimits`. The default ceilings are
1,000,000 nodes, 2,500,000 relations, and the canonical `GraphDocument` reader's
1 GiB serialized-byte bound (distinct from the store publication cap configured
by `COMPASS_MAX_GRAPH_BYTES`). These are independent ceilings: the byte limit may
bind before either count limit for payload-rich graphs. C-015 measures the
ratified qualification corpora rather than assuming all three ceilings are
jointly attainable. Activation validates the limits before claiming or writing
the generation. Reads validate manifest counts and bytes before record queries,
apply query-side row limits, and compare the returned counts and serialized
bytes with the immutable manifest before returning a plan. Callers may choose
smaller positive limits for constrained deployments.

Database record keys are deterministic SHA-256 digests over length-delimited
schema, record-class, repository, generation, and Compass identity values.
Compass IDs containing SurrealQL punctuation therefore remain bound data and
never become query syntax.

## Atomic activation

The runtime defines schemafull node, manifest, pointer, and relation tables.
Publication keeps the visibility switch atomic without retaining an
unbounded transaction:

1. atomically claim the generation with an incomplete manifest carrying the
   expected immutable fingerprint, identities, counts, and byte size;
2. idempotently stage generation-keyed nodes and typed relations in bounded
   512-record commits;
3. read back and compare the exact ordered node and edge identity sets;
4. mark the immutable generation manifest complete; and
5. update the repository-scoped active-generation pointer last.

Readers can observe only the pointer-selected complete generation. Any write,
validation, process, or injected-interruption failure leaves the prior pointer
unchanged; an incomplete candidate remains invisible and a later identical
activation safely resumes its idempotent batches. A generation claim with a
different fingerprint fails before candidate records are changed. This avoids
SurrealDB Mem retaining a savepoint clone of an ever-growing map while
preserving coherent generation publication.

An identical complete generation is idempotent: the adapter validates its
fingerprint and exact record identities, then may reactivate it without
rewriting records. A generation ID already claimed or completed with different
content fails explicitly.

## Native structural reads

The feature-gated engine exposes only closed native operations: callers,
callees, impact, directed node trail, connected structural subgraph, and
relation pagination. Each operation pins the complete active-generation
pointer and matching manifest before reading, binds every value parameter,
uses only static internal SurrealQL statements, and revalidates the pointer
before publishing the response. A concurrent activation therefore yields
either one coherent generation or a typed `ActiveGenerationChanged` error.

Structural operations return the shared `compass.query/1` node, edge, path,
diagnostic, limit, and truncation records. Exact-name lookup uses the same
normalization contract as the canonical query engine. Direction, parallel
edges, reverse edges, self-loops, typed details, provenance, confidence, and
heuristic filtering are decoded from the canonical payload rather than
reconstructed from database metadata.

Relation pagination is deterministic by Compass edge identity across all five
closed relation families. Its cursor is size-bounded, checksummed, and bound to
the repository digest, immutable generation, schema, and operation. A cursor
cannot select a table, inject a predicate, or be replayed against another
repository or generation.

## License and distribution

SurrealDB 3.2.4 and its core use Business Source License 1.1 before conversion.
The accepted artifact-profile decision, exact license digest, Database Service
restriction, Change Date, Change License, notice requirements, and downstream
redistribution conditions are recorded in
[`surrealdb-license-decision.md`](../future/surrealdb-license-decision.md).
Enabling a Cargo feature contains default-build impact; it does not remove
license obligations from a Surreal-enabled binary, library, container, or
archive.

## Phase-end verification

After the entire development phase is implemented, run the Surreal integration
targets in the single coordinated verification wave:

```bash
cargo test -p compass-graphdb-surreal --test '*' --features mem --locked
cargo test -p compass-graphdb-surreal --test '*' --features surrealkv --locked
cargo test -p compass-graphdb-surreal --test '*' --features rocksdb --locked
sh scripts/check_surreal_feature_isolation.sh
```
