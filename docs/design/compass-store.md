# Compass store and graph-engine design

**Status:** Planned. This document defines a target architecture and is not
evidence that a store-backed graph engine has shipped.

> **Who this page is for:** maintainers of graph publication and queries,
> storage-adapter authors, cloud-service implementers, and reviewers of the
> `compass-store` contract.
>
> **You will learn:** the portable namespace/partition/key contract, the
> store-independent graph layout, how `graph.json` remains supported, the
> atomic publication and recovery rules, and how SQLite, redb, PostgreSQL, and
> DynamoDB map onto the same semantics.
>
> **Prerequisites:** [Design principles](principles.md),
> [Storage and history](storage-and-history.md), and
> [Query engine](../implementation/query-engine.md).
>
> **Reading time:** 25–35 minutes.

## Decision summary

Compass will introduce a backend-neutral crate named `compass-store`. Its
smallest address is always:

```text
(namespace, partition, key) -> value
```

The ordering is intentional. A caller first receives a handle scoped to one
namespace. Operations on that handle then require one partition and one key,
or a bounded key range within one partition. No portable operation can scan an
entire tenant, silently cross partitions, or issue raw backend queries.

The portable correctness envelope is deliberately smaller than the union of
database features:

- strongly consistent point reads;
- ordered, bounded scans inside exactly one partition;
- durable single-key writes;
- put-if-absent and compare-and-swap on one address;
- immutable, idempotent content writes; and
- explicit limits, deadlines, cursors, and typed failures.

Graph snapshots are built above this key-value contract as immutable,
content-addressed ordered indexes. Publication writes all immutable content
first and commits only one small selector with compare-and-swap. Compass does
not require a transaction spanning graph partitions.

`graph.json` is not deprecated and will not be replaced. It remains a complete
and compatible graph engine for local files, interchange, inspection, and
recovery. Store-backed engines and the JSON engine implement the same graph
read contract and must return equivalent public results. `graph.json` is not
forced to pretend to be a mutable key-value database.

The new internal database format does not need to read disposable query caches
or unpublished storage prototypes. Those can be rebuilt. This freedom does
not permit a silent change to `compass.graph/1`, CLI output, CompassQL,
relationship meaning, stable IDs, or deterministic ordering.

## Goals

The design must:

1. improve incremental graph-build and cold-query performance without loading
   or rewriting one complete JSON document for every operation;
2. preserve the existing typed graph and query contracts independently of the
   selected engine;
3. support embedded local databases and remote managed databases through one
   deliberately small contract;
4. provide namespace isolation suitable for a future multi-tenant Compass
   service;
5. keep all scans, values, retries, memory, network responses, and maintenance
   work bounded;
6. preserve deterministic identities, ordering, directions, multiplicity,
   source anchors, provenance, and unknown graph attributes where the public
   graph contract allows them;
7. publish coherent snapshots without relying on cross-partition or
   distributed transactions;
8. make interrupted builds, partial remote writes, conflicts, corruption, and
   throttling distinguishable and recoverable; and
9. retain Compass's native, offline local path with no credentials, remote
   service, vector database, or runtime download.

## Non-goals

This design does not:

- make SQL, DynamoDB expressions, or a backend query language public Compass
  contracts;
- promise that every backend has identical latency, cost, backup, or
  operational behavior;
- make a namespace string an authorization mechanism by itself;
- require a transaction across namespaces or partitions;
- merge working-tree storage with immutable Git-history identity;
- remove `graph.json`, make it a temporary migration path, or require a
  database to read an explicitly supplied JSON graph;
- put backend-specific connection configuration into `compass-model` or the
  graph schema; or
- claim a performance improvement before the repository's performance gates
  measure it.

## Vocabulary

| Term | Meaning |
| --- | --- |
| Store | An implementation of the `compass-store` key-value contract. |
| Namespace | The first isolation and lifecycle boundary, normally one tenant, repository, data plane, and schema major. |
| Partition | A required locality and sharding key inside one namespace. All ordered scans stay within it. |
| Key | An opaque, ordered byte string unique inside one namespace and partition. |
| Entry | A value plus integrity metadata and an opaque compare-and-swap version. |
| Graph engine | A reader of one selected immutable graph snapshot. JSON and store-backed implementations are peers. |
| Snapshot | A complete immutable graph realization with typed index roots and validation evidence. |
| Active selector | The one small mutable record that makes a prepared store snapshot current. |
| Content object | An immutable, hash-addressed tree node, record block, or manifest. |
| Accelerator | A disposable backend-specific index that can improve speed but cannot define graph meaning. |

## Architectural boundaries

```text
CLI / MCP / reports / CompassQL / graph algorithms
                       |
                       v
          compass-query graph read contract
                       |
          +------------+-------------+
          |                          |
          v                          v
  JSON graph engine          store graph engine
  compass.graph/1            manifests + ordered indexes
          |                          |
     graph.json                 compass-store
                                     |
                  +------------------+------------------+
                  |          |             |            |
                SQLite      redb       PostgreSQL    DynamoDB
```

Ownership follows the existing workspace boundaries:

- `compass-model` owns graph records, graph validation, and public graph
  meaning;
- `compass-store` owns backend-neutral addresses, operations, limits,
  consistency, errors, and the adapter conformance suite;
- backend adapter crates own connections and physical schemas;
- `compass-graph` owns deterministic graph-to-snapshot index construction;
- `compass-core` owns build and publication orchestration;
- `compass-query` owns the graph-engine read contract and query planning; and
- `compass-output` owns deterministic `graph.json` export and other renderers.

The core crate name is `compass-store`. Heavy or platform-specific clients do
not become features of that crate. Planned adapters are separate packages such
as `compass-store-sqlite`, `compass-store-redb`,
`compass-store-postgres`, and `compass-store-dynamodb`. This keeps a local
binary from acquiring a cloud SDK merely because the shared contract exists.

## Two contracts, not one leaky abstraction

There are two intentional abstraction levels.

### The key-value store contract

The low-level contract stores opaque bytes at a
`(namespace, partition, key)` address. It knows nothing about nodes, edges,
CompassQL, files, or graph schema versions.

### The graph-engine contract

The high-level contract opens one validated graph snapshot and provides
bounded graph operations: metadata lookup, node and edge lookup, ordered
iteration, outgoing and incoming adjacency, text candidates, and community
membership. It knows nothing about SQL tables or DynamoDB expressions.

The store-backed graph engine maps those graph operations to immutable ordered
indexes above `compass-store`. The JSON graph engine maps them to a validated
`GraphDocument` and disposable local indexes. This separation is what lets
`graph.json` stay a real engine rather than an emulated database.

## Engine and materialization profiles

Supporting the JSON engine does not require every store-backed build to rewrite
a complete JSON file. The application layer can expose three explicit
profiles:

| Profile | Published graph engines | Intended use |
| --- | --- | --- |
| `json` | `graph.json` | Existing local/interchange behavior, direct inspection, and database-free operation |
| `store` | Store snapshot; deterministic JSON export available on demand | Maximum incremental-build and cold-query benefit |
| `dual` | Store snapshot and co-published `graph.json` | Migration, differential qualification, local recovery, and users requiring both artifacts |

`JsonGraphEngine` is compiled, tested, and supported in all product profiles
that accept JSON input, even when one particular build did not materialize a
JSON file. A store snapshot can always stream a canonical `graph.json` export;
an existing `graph.json` can be imported into a new prepared store snapshot
after full validation.

The existing `init` and `update` output contract remains `json` or `dual` until
a separately reviewed CLI/configuration change introduces `store` as an
explicit choice. The implementation plan begins with `dual`; it does not
silently stop creating an artifact that callers currently expect.

This distinction is essential to honest performance claims. A dual build must
still encode and write all nodes and edges to JSON, so it has an unavoidable
linear materialization floor. Structural sharing can reduce graph-index work
and later query startup, but only the explicit store profile can avoid the
full-file rewrite. Benchmarks report `json`, `store`, and `dual` separately.

## Portable key-value contract

### Address model

The full physical identity of a value is:

```text
StoreAddress {
    namespace: NamespaceId,
    partition: PartitionKey,
    key: Key,
}
```

All three components are non-empty opaque byte strings. The contract compares
keys as unsigned bytes in ascending lexicographic order. It does not apply a
locale, Unicode normalization, database collation, or natural-number sort.

Callers do not repeatedly pass a namespace to every operation. The root store
creates a `NamespaceStore` after authorization and configuration have selected
one namespace:

```text
Store::scope(namespace) -> NamespaceStore
NamespaceStore::get(partition, key)
NamespaceStore::scan(partition, range, limits, cursor)
NamespaceStore::put(partition, key, value, condition)
NamespaceStore::delete(partition, key, condition)
```

The scoped handle prevents accidental cross-namespace reads. In a service, the
handle must be constructed from authenticated server-side identity, never
from an untrusted request field alone.

### Namespace identity

A namespace is the isolation, quota, backup, encryption-policy, and garbage-
collection boundary. The canonical descriptor includes:

```text
tenant identity
repository or project identity
data plane: working | history | derived
logical schema major
optional deployment realm
```

The physical `NamespaceId` is derived from the canonical descriptor with a
versioned, length-prefixed encoding and a cryptographic digest. Display names,
repository paths, credentials, and secrets are not stored in the ID. A catalog
entry inside the namespace may retain validated, non-secret descriptive
metadata.

Working graphs, historical realizations, and disposable derived data use
different namespaces even if one physical backend hosts all three. Sharing a
backend does not merge their identity or retention rules.

### Partition semantics

A partition is mandatory on every data operation. It has three jobs:

1. bound scans and failure impact;
2. express storage locality that all supported backends can implement; and
3. give the graph layer an explicit sharding control for cloud workloads.

The contract has no `scan_namespace` method. Administrative namespace
enumeration, migration, and garbage collection operate from explicit
manifests or a privileged maintenance API with separate limits and auditing.

### Key encoding

Application code must not concatenate user-controlled strings with separators.
`compass-store` provides one canonical key builder with:

- a one-byte encoding major;
- a one-byte record kind;
- length-prefixed byte segments;
- fixed-width big-endian unsigned integers where numeric order matters;
- explicit path normalization performed by the owning domain before encoding;
- a maximum segment count and nesting depth; and
- test vectors shared by every adapter.

Prefix and range construction uses the encoded representation. Backends must
store and compare it as binary data, not text. A new incompatible encoding
uses a new major and a different namespace.

### Proposed portable limits

Phase 0 must freeze exact values with executable conformance tests. The target
v1 envelope is intentionally conservative enough for embedded and managed
stores:

| Item | Proposed maximum |
| --- | ---: |
| Namespace ID | 128 bytes |
| Partition key | 256 bytes |
| Key | 1,024 bytes |
| Value | 256 KiB |
| Version token | 64 bytes |
| Cursor | 4 KiB |
| Requested page | 1,000 entries and 1 MiB, whichever comes first |
| Key segments | 32 |

An adapter may support larger values, but portable graph code cannot rely on
that. Oversize content is deterministically chunked above the store layer.
Limit exhaustion returns a typed limit error; it never becomes an empty page
or a missing value.

### Operation surface

The normative semantic surface is small:

```text
capabilities() -> StoreCapabilities
get(GetRequest) -> Entry | Missing
scan(ScanRequest) -> ScanPage
put(PutRequest { condition }) -> WriteResult
delete(DeleteRequest { condition }) -> DeleteResult
```

All requests carry an explicit operation budget or deadline supplied by the
application boundary. `scan` also carries item and byte limits. Adapters may
offer bulk helpers, but helpers split work into these operations and cannot
invent stronger atomicity.

The canonical Rust API is asynchronous so a remote adapter never blocks an
executor thread by contract. The trait uses object-safe returned futures
rather than requiring backend types in callers. Embedded adapters may return
immediately ready futures. `compass-store` does not create a runtime; the CLI,
service, or test harness owns execution.

### Entries and versions

A successful read returns:

```text
Entry {
    key,
    value,
    version: opaque backend-issued token,
    digest: sha256(value),
}
```

The version exists only for conditional mutation. Callers may compare tokens
for equality but cannot parse, order, persist across migrations, or derive
meaning from them. The digest provides end-to-end corruption detection and
idempotence; it is not a substitute for authorization.

### Write conditions

Every mutable write declares one condition:

| Condition | Required behavior |
| --- | --- |
| `Any` | Replace the value at exactly one address. Restricted to operational data that is explicitly last-writer-wins. |
| `Missing` | Succeed only when the address is absent. |
| `Version(token)` | Succeed only when the current opaque version matches. |

Content-addressed objects use a stricter helper, `put_immutable`:

- an absent address receives the value;
- an existing address with the same digest is an idempotent success; and
- an existing address with a different digest is a corruption/conflict error.

The graph layer never uses unconditional replacement for manifests, snapshot
content, or active selectors.

### Read consistency

Strong reads are mandatory for active selectors, leases, publication state,
and compare-and-swap. A backend that cannot provide a strong point read for
these records cannot implement the v1 contract.

An adapter may advertise eventual reads for immutable content as an optional
optimization. The store graph engine may use them only after a strongly read
manifest has selected the content, and it must verify content digests. A
temporarily absent immutable object is `Unavailable` and may be retried within
budget; it is not `NotFound` evidence that the manifest is valid.

### Ordered scans and cursors

A scan names exactly one namespace, one partition, and one half-open key
range. Results are strictly ascending by unsigned key bytes and contain no
duplicates within one cursor sequence.

Each returned page includes:

```text
entries
next_cursor or end
items_read
bytes_read
consistency used
```

A cursor is opaque, versioned, integrity-protected, and bound to the
namespace, partition, requested range, consistency, and adapter instance. It
must fail as `InvalidCursor` or `StaleCursor`; it must not silently restart.
Portable callers do not persist cursors as durable Compass artifacts.

Snapshot graph indexes are immutable, so page-to-page mutation cannot change
their contents. Scans of mutable operational partitions use backend snapshot
facilities where available or document their weaker page boundary; no graph
correctness may depend on a mutable multi-page scan.

### Atomicity boundary

The only required atomic unit is one address. A backend may advertise a
larger atomic batch, but portable publication cannot require it.

This rule is central to portability. SQLite and PostgreSQL can commit large
transactions, redb serializes write transactions, and DynamoDB offers bounded
transactional operations with different constraints. Compass instead gets
cross-record coherence from immutability plus one active-selector CAS.

### Errors and retry policy

`compass-store` errors are typed and preserve actionable backend context
without exposing credentials or full values:

| Error | Meaning and retry rule |
| --- | --- |
| `NotFound` | The exact address is absent after a valid strong read. Not retryable by default. |
| `Conflict` | A condition or immutable-value check failed. Return to orchestration; never blind-retry CAS. |
| `LimitExceeded` | A key, value, page, response, or budget limit was reached. Never treat as empty. |
| `InvalidCursor` / `StaleCursor` | Cursor validation or lifetime failed. Restart only under explicit caller policy. |
| `Corrupt` | Digest, envelope, ordering, or backend invariant failed. Quarantine and recover. |
| `Throttled` | Backend rejected capacity. Retry only idempotent operations with bounded jitter and deadline. |
| `Unavailable` | Transient backend or replication failure. Bounded retry is permitted for reads and immutable writes. |
| `DeadlineExceeded` | The request budget expired. Do not continue work in the background. |
| `Unauthorized` | Namespace or backend access was denied. Never retry as transient. |
| `Unsupported` | The requested consistency or capability is unavailable. Fail before partial work. |
| `Internal` | Adapter invariant failed. Include a safe diagnostic ID and source chain. |

Retry classification belongs to the common contract, while delay and attempt
counts belong to application policy. Every retryable write must be idempotent.

### Capabilities

An adapter reports validated capabilities at open time:

```text
limits
strong point reads
ordered partition scans
conditional single-key writes
durability mode
optional atomic batch size
optional eventual immutable reads
optional disposable accelerator kinds
backend and format identifiers
```

Required v1 capabilities are not negotiable. Optional capabilities can improve
performance but must have a portable path with identical observable results.
Configuration is rejected before publication if an adapter cannot meet the
requested durability or consistency.

### Configuration and adapter selection

`compass-store` defines only backend-neutral open requirements: requested
durability, consistency, operation limits, store identity, and namespace
scope. It does not define one universal connection string whose query
parameters grow to contain every backend option.

Each adapter owns a typed configuration and a `StoreFactory` implementation:

```text
SQLite:     contained database path, busy timeout, durability
redb:       contained database path, durability, writer-queue bound
PostgreSQL: endpoint reference, database/schema, pool bounds, TLS, secret ref
DynamoDB:   region, endpoint policy, table, capacity/retry policy, secret ref
```

Configuration files contain credential references, not secret values. CLI or
service wiring chooses a compiled adapter explicitly, opens it, validates its
reported capabilities, and only then scopes it to an authorized namespace.
Store factories and configurations remain outside graph models and snapshot
manifests. A snapshot is therefore movable between conforming adapters through
the graph-level copy/export path without embedding the source backend's DSN,
file path, account, or region.

## Value envelopes and schema evolution

Every non-trivial stored value has a small binary envelope:

```text
magic
envelope major and minor
record kind
codec identifier
uncompressed length
payload digest
payload bytes
```

Readers bound compressed and uncompressed sizes before allocation. Codecs are
explicit and deterministic; no backend decides compression independently.
Unknown major versions fail. Unknown minor fields are retained or skipped only
when the typed record contract permits it.

There are three different versions and they must not be conflated:

- the store envelope version describes bytes;
- the graph-snapshot layout version describes indexes and manifests; and
- `compass.graph/1` describes public graph meaning and JSON representation.

Changing an internal envelope does not imply a graph-schema change. Changing
graph meaning requires the existing public compatibility process even if no
database table changes.

## Store-backed graph snapshot

### Logical structure

A store snapshot is an immutable manifest pointing to content-addressed
ordered-map roots:

```text
ActiveSelector / local store.ref
              |
              v
      SnapshotManifest
       /    |    |    \
 nodes   edges  outgoing  incoming  ...
   |       |       |         |
 immutable ordered index nodes, addressed by digest
              |
       canonical graph records
```

The initial implementation should reuse the workspace's proven Prolly-tree
ideas where their contract fits. The key-value layer itself remains a normal
store; tree construction, graph key encodings, and structural sharing belong
to the graph repository above it.

### Physical partitions

The graph layer reserves versioned partition families:

| Partition family | Contents | Mutation rule |
| --- | --- | --- |
| `catalog` | Active selector, namespace descriptor, format marker | Selector uses CAS; descriptor is immutable |
| `manifest/<shard>` | Snapshot manifests keyed by snapshot ID | Immutable |
| `object/<shard>` | Tree nodes and record blocks keyed by content digest | Immutable |
| `lease/<shard>` | Bounded reader/build leases | Conditional mutable records |
| `gc/<shard>` | Checkpoints and bounded maintenance state | Conditional mutable records |
| `derived/<shard>` | Disposable accelerators keyed by snapshot and format | Replaceable; never authoritative |

`<shard>` is derived from leading digest bits with a versioned function. It is
not chosen from source order, repository path, or a sequential counter. The
shard count is recorded in the layout version and cannot vary silently across
writers.

### Snapshot manifest

The typed manifest contains at least:

```text
schema: compass.store.graph-snapshot/1
snapshot_id
graph_schema: compass.graph/1
canonical_encoding_version
build_fingerprint
source_identity and working-tree state
created_by implementation identity
counts and bounded-size summaries
roots for every required logical index
root digests and aggregate snapshot digest
optional graph_json_sha256
validation/completion evidence
```

`snapshot_id` is derived from meaning-affecting canonical content, not wall
clock time, database sequence values, host paths, or operational timings. The
manifest may contain operational metadata outside the identity projection,
but that metadata cannot affect graph equivalence.

The manifest is small enough for one portable value. If the root registry can
outgrow the value limit, it becomes its own immutable tree and the manifest
stores only the registry root.

### Required logical indexes

The v1 snapshot needs these authoritative ordered maps:

| Index | Canonical logical key | Value |
| --- | --- | --- |
| Graph metadata | fixed key | typed metadata record |
| Nodes by ID | node ID | complete typed node record or content reference |
| Edges by ID | edge ID | complete typed edge record or content reference |
| Outgoing edges | source ID, edge kind, target ID, edge ID | edge ID/reference |
| Incoming edges | target ID, edge kind, source ID, edge ID | edge ID/reference |
| Files/source anchors | normalized file identity, range, record kind, record ID | node/edge reference |
| Names | normalized lookup name, node kind, qualified name, node ID | node ID and stable projection |
| Text postings | normalized term, node ID | deterministic term statistics |
| Communities | community identity, member order, node ID | member projection |
| Diagnostics | severity, stable diagnostic identity | diagnostic projection |

The edge indexes include `edge ID` even when endpoints and kind match. That
preserves multigraph multiplicity. They preserve semantic direction and never
swap endpoints to improve locality. Full records preserve relationship sites,
occurrence rules, evidence, details, weight, context, deferred state, and
diagnostics.

Text normalization and scoring are versioned Compass semantics implemented in
Rust. A backend FTS index may produce candidates only if differential tests
prove the same result ordering and tie-breaking. Backend relevance scores are
not public graph meaning.

### Deterministic construction

Snapshot construction follows a canonical stream:

1. validate the typed graph document and all bounds;
2. sort records by their public canonical identities;
3. derive every logical index entry with versioned encoders;
4. reject duplicate keys unless the index explicitly aggregates them;
5. build content-defined immutable ordered trees;
6. write tree objects with `put_immutable`;
7. construct and validate the manifest; and
8. derive the snapshot ID from its canonical identity projection.

Parallel extraction and tree building are allowed, but the emitted objects and
roots must not depend on task completion order, thread count, host, locale, or
backend.

### Structural sharing and incremental update

An update compares the new canonical index streams with the prior roots. Tree
nodes whose content is unchanged retain the same digest and are not rewritten.
Changed records rewrite only affected paths and content-defined neighbor
chunks. Deleted records disappear from new roots but remain reachable from an
older retained snapshot until garbage collection.

Incremental build state remains evidence, not authority. A reused record still
passes current validation and participates in deterministic index
construction. A cache miss can cost time but cannot change graph meaning.

## Publication protocols

### Common prepare sequence

All publication modes begin with:

1. observe the current selected snapshot and its version, if one exists;
2. build and validate the candidate graph;
3. write immutable content objects idempotently;
4. write the immutable snapshot manifest;
5. re-read required roots and verify digests, counts, and completion evidence;
6. prepare all non-store artifacts; and
7. perform exactly one mode-specific commit action.

Failure before the commit leaves only unreachable immutable objects. They are
safe to retry and later collect.

### Local artifact-set publication

For `compass init` and `compass update`, the existing filesystem build guard
remains the coordinator for a coherent local output generation. A dual-profile
generation contains:

```text
staged generation/
  graph.json
  reports and viewer artifacts
  store.ref -> immutable snapshot ID + manifest digest + store identity
```

The store snapshot is fully prepared but does not independently become
current. The filesystem generation switch atomically publishes `graph.json`,
its reports, and `store.ref` together. A reader opening that generation may
choose either the JSON engine or the referenced store engine and must observe
the same graph.

A JSON-profile generation follows the existing artifact publication without a
store reference. A store-profile generation stages `store.ref` plus reports
derived from that immutable snapshot and deliberately omits eager JSON
materialization; its export command can create JSON as a new coherent artifact
without changing the selected snapshot.

If the filesystem commit fails, the prepared snapshot is an orphan. If the
database later becomes unavailable, the published `graph.json` remains a
usable complete engine. No best-effort sequence of “update database, then
replace JSON” is allowed to expose two different current graphs.

### Store-native or cloud publication

A deployment without a filesystem artifact set commits by conditionally
replacing one `ActiveSelector` in `catalog`:

```text
CAS(observed selector version -> candidate snapshot ID and manifest digest)
```

A conflict means another writer won. The losing writer does not overwrite the
winner, does not reinterpret the candidate as active, and reports a typed
publication conflict. It may rebuild from the newly active roots under an
explicit orchestration decision.

Readers strongly read the selector, validate the manifest digest, then open
only immutable roots. A reader that already opened an older snapshot can
finish against it while a new selector is published.

### Dual-output equivalence

When a build emits both engines, publication validation must prove:

- equal graph schema and build fingerprint;
- equal node and edge counts;
- equal canonical graph digest;
- equal sampled and then fully streamed record identities in qualification;
  and
- byte-identical `graph.json` output to the canonical JSON renderer for the
  same validated graph.

The store engine is not allowed to “fix” or omit data that the JSON engine
contains. Differences fail publication.

## Graph-engine read contract

The graph read abstraction is immutable and snapshot-scoped. Its conceptual
surface is:

```text
metadata()
get_node(id)
get_edge(id)
scan_nodes(range, limits, cursor)
scan_edges(range, limits, cursor)
outgoing(node, edge_filter, limits, cursor)
incoming(node, edge_filter, limits, cursor)
lookup_name(query, filters, limits, cursor)
search_terms(query, filters, limits, cursor)
community_members(community, limits, cursor)
export_graph_json(writer, limits)
```

Every sequence has stable ordering and explicit bounds. “All nodes” remains a
bounded streaming operation, not a request to allocate the full graph. Query
planners may request projections so an engine need not decode unused large
fields.

The graph read contract is asynchronous and supports bounded page/stream
delivery because a store engine may cross a network. It does not create an
executor or expose backend futures in public query results. The JSON engine can
complete local operations immediately, while CLI and service application
layers own runtime, cancellation, and overall command deadlines.

`JsonGraphEngine` validates `compass.graph/1` using `compass-model`. It may use
the existing content-addressed binary and FTS caches, but those remain
disposable and JSON remains authoritative for that engine.

`StoreGraphEngine` validates the selector or `store.ref`, manifest, index
roots, envelopes, and record contracts. It performs point reads and ordered
tree walks through a scoped `NamespaceStore`. It must not downcast to a
specific adapter for correctness.

CompassQL, traversal, impact, reports, MCP, and CLI layers consume the graph
read contract. They do not select SQL tables or DynamoDB indexes.

## Query planning and accelerators

The first portable plan should use:

- direct node and edge ID lookup;
- outgoing and incoming ordered indexes for traversal;
- name and kind indexes for structural discovery;
- deterministic term postings for text candidates; and
- bounded record projection and batched point reads.

Each query budget accounts for objects read, decoded bytes, result rows,
frontier size, traversal depth, and elapsed time. Crossing a limit returns a
limit error or an explicitly marked partial-result contract where the public
command already permits partial results.

SQLite FTS, PostgreSQL text search, redb-local caches, or another service can
be registered as disposable accelerators. Each accelerator is keyed by
snapshot ID and its own format version. It is safe to delete and rebuild. A
miss, corrupt accelerator, or unsupported backend falls back to the portable
plan within the same declared budget.

Accelerator qualification compares full ordered results, scores, truncation,
and diagnostics against the portable implementation. Faster but observably
different results do not qualify.

## Backend mappings

These mappings are adapter designs, not leaked public schema. An adapter may
change its physical representation if it still passes the contract and its
own format migration policy.

| Contract concept | SQLite | redb | PostgreSQL | DynamoDB |
| --- | --- | --- | --- | --- |
| Deployment | Embedded file | Embedded file | Remote or local service | Managed remote service |
| Namespace + partition | Leading binary primary-key columns | Leading composite-key segments | Leading `bytea` primary-key columns | Encoded together as table partition key |
| Key order | Binary primary-key order | Byte-key table order | Binary `bytea` order | Table sort-key order |
| Strong read | Read transaction | Read transaction | Primary/qualifying synchronous path | Consistent base-table read |
| Conditional write | Transaction plus row/version predicate | Write transaction plus version check | Unique constraint/conditional update | Conditional expression |
| Publication commit | One selector-row CAS or filesystem generation switch | One selector-key CAS or filesystem generation switch | One selector-row CAS | One selector-item CAS |
| Optional acceleration | FTS/derived tables | Derived local tables | Derived indexes/text search | Derived items or separately qualified service |

Only the contract column meanings are portable. For example, availability of a
large SQL transaction in one adapter does not raise the common atomicity
boundary.

### SQLite

The reference local adapter uses one database per configured local store and a
binary primary key:

```sql
CREATE TABLE kv (
    namespace  BLOB    NOT NULL,
    partition  BLOB    NOT NULL,
    key        BLOB    NOT NULL,
    value      BLOB    NOT NULL,
    digest     BLOB    NOT NULL,
    version    INTEGER NOT NULL,
    PRIMARY KEY (namespace, partition, key)
) WITHOUT ROWID;
```

A separate metadata table records the adapter format, store identity,
creation state, and migration state. Ordered scans specify all three leading
primary-key components and use binary comparisons. Conditional updates include
the observed `version` in the `WHERE` clause and require exactly one affected
row. Put-if-absent uses the primary-key constraint and verifies the existing
digest on conflict.

WAL mode, full synchronous durability, a bounded busy timeout, bounded
prepared statements, and explicit transactions follow the repository's
existing local durability posture. Whether `WITHOUT ROWID` is retained must
be benchmarked on Compass workloads; SQLite's own documentation recommends
measuring this optimization rather than assuming it always wins.

Backups must include committed WAL state. File creation, permissions, symlink
handling, corruption recovery, and atomic replacement reuse `compass-files`
primitives rather than new weaker helpers.

### redb

The redb adapter stores a composite binary address whose encoding preserves
the contract's namespace, partition, and key ordering. Read transactions
provide stable local snapshots. Writes map conditional operations to one
redb write transaction and commit before success is returned.

redb permits one write transaction at a time, so the adapter uses bounded
write queues or reports backpressure; graph construction must not spawn an
unbounded number of waiting writers. Large immutable batches are chunked,
committed, and safe to retry because no chunk becomes active before the final
selector CAS.

Table names, database files, and durability configuration are adapter-private.
The conformance suite, not matching the SQLite schema, establishes
compatibility.

### PostgreSQL

The reference mapping uses binary columns and the same composite primary key:

```sql
CREATE TABLE compass_kv (
    namespace  bytea  NOT NULL,
    partition  bytea  NOT NULL,
    key        bytea  NOT NULL,
    value      bytea  NOT NULL,
    digest     bytea  NOT NULL,
    version    bigint NOT NULL,
    PRIMARY KEY (namespace, partition, key)
);
```

Point reads and ordered range scans always constrain `namespace` and
`partition`. Put-if-absent uses the unique primary key. Compare-and-swap uses a
conditional update on the observed version, returning the new version only if
one row changed. PostgreSQL documents `INSERT ... ON CONFLICT` as an atomic
insert-or-update primitive, but Compass still implements its narrower explicit
conditions and reports conflicts rather than giving all writes last-writer-
wins behavior.

The adapter uses a bounded connection pool, statement timeouts, response byte
limits, TLS policy, scoped database credentials, and cancellation. A future
service may add database/schema isolation or row-level security, but those are
defense in depth; application authorization still produces a namespace-scoped
handle.

Table partitioning and replicas are deployment choices. An authoritative
active-selector read cannot be routed to a replica whose consistency is too
weak for the contract.

### DynamoDB

The DynamoDB adapter maps one Compass address to one table item:

```text
PK = versioned binary encoding(namespace, partition)
SK = key
V  = value envelope
D  = digest
R  = compare-and-swap version
```

DynamoDB groups items with the same partition key and orders them by sort key,
which directly matches one-partition scans. The adapter uses base-table Query
with strongly consistent reads for authoritative data, handles the service's
pagination internally, and binds Compass cursors to the complete request.
Global secondary indexes are not authoritative selector paths because their
reads are eventually consistent.

Put-if-absent and compare-and-swap use conditional expressions. A conditional
failure becomes `Conflict`, not a generic outage. Values stay within the
portable envelope and below the service item limit after all attributes are
included. Graph content is digest-sharded across partition keys so a large
repository does not make one hot partition.

The adapter bounds retries, exponential backoff with jitter, consumed
capacity, response bytes, and total deadline. Throttling is surfaced distinctly
when the budget is exhausted. Tests use a controlled local implementation or
protocol mock, never real credentials or a production table.

### Additional adapters

Any database can qualify if it implements the common contract. An adapter
must not weaken ordering, turn a consistency failure into absence, drop
unknown bytes, or hide a limit behind truncation. A backend without ordered
partition scans may build an adapter-private ordered index only if it can
provide the exact semantics durably and within bounds.

Adapter-specific features belong behind capability flags. A graph engine that
requires one optional feature is not portable and must supply a portable plan
before adoption.

## Adapter conformance suite

`compass-store` owns a reusable test suite executed against every adapter. At
minimum it covers:

- namespace and partition isolation;
- unsigned byte ordering, prefix edges, empty and maximum keys;
- point-read and missing-value behavior;
- exact page limits, continuation, invalid cursors, and no duplicates;
- put-if-absent races and compare-and-swap races;
- idempotent immutable writes and mismatched-value detection;
- reopen durability after successful acknowledgement;
- interruption before, during, and after commit;
- value, key, cursor, response, and deadline limits;
- corruption and malformed envelope detection;
- retry classification, throttling, cancellation, and safe diagnostics;
- capability rejection before writes;
- concurrent readers during publication; and
- deterministic behavior across repeated runs.

The suite supplies logical tests plus a backend fault-injection interface. A
backend cannot qualify from happy-path CRUD tests alone.

## Garbage collection, leases, and retention

Immutable publication makes orphaned content expected. Garbage collection is
mark-and-sweep, rooted at:

- active selectors;
- filesystem `store.ref` records for retained local generations;
- retained historical realization roots;
- explicit pins; and
- unexpired reader or build leases.

The mark phase traverses bounded manifests and tree roots. It records a
versioned checkpoint so remote runs can resume. The sweep phase visits explicit
digest shards with page and delete budgets. Objects younger than a safety
window are not collected. A conditional delete verifies that the object and
retention state did not change since marking.

GC never infers liveness from “the newest timestamp,” never requires an
unbounded namespace scan in a user query, and never rewrites a published
snapshot. Operational indexes may accelerate enumeration, but the root set
remains explicit.

## Recovery and backup

Recovery is ordered by authority:

1. validate the selected snapshot reference;
2. validate its manifest and roots;
3. use a coherent backend backup or an alternate retained snapshot if the
   store is corrupt;
4. for local artifact generations, open the co-published `graph.json` engine;
5. rebuild disposable indexes or the store snapshot from validated JSON or
   source; and
6. publish a new generation rather than editing an immutable snapshot.

A missing object reachable from a committed manifest is corruption or
incomplete replication, not an empty graph. A dangling prepared manifest is
an orphan, not current data.

Backup procedures are backend-specific but must capture the selector and all
content it can reach coherently. Restore validates every root before making a
selector active. Cloud backup and disaster-recovery objectives are deployment
policy and cannot be inferred from the key-value API.

## Multi-tenancy and security

The future cloud design uses defense in depth:

- authentication and authorization select an internal namespace descriptor;
- callers receive a namespace-scoped store handle, not raw database access;
- quotas apply per tenant, namespace, build, operation, and response;
- database credentials are scoped to the service, rotated, and never encoded
  into keys or diagnostics;
- remote endpoints, TLS, timeouts, and credential sources are explicit
  configuration;
- local Compass remains offline by default and never contacts a backend unless
  the user selected it;
- untrusted stored bytes pass size, envelope, digest, graph-record, path, and
  source-anchor validation before rendering;
- namespace IDs are safe opaque identifiers, not accepted authorization
  claims;
- backend logs and metrics redact values, source text, credentials, and
  sensitive repository paths; and
- administrative enumeration, migration, backup, and GC use separate
  privileged APIs and audit trails.

Encryption at rest is normally provided by the selected backend and deployment.
Application-level value encryption can be added later, but its key identifier,
rotation, and range-query implications require a separate versioned design.

## Observability

The common layer emits structured, low-cardinality measurements:

- operation kind, adapter kind, result class, and consistency;
- duration and retry count;
- requested and returned items/bytes;
- throttle and deadline counts;
- cache/accelerator outcome;
- objects reused versus written during build;
- snapshot preparation and selector-CAS duration;
- query objects read, decoded bytes, and frontier peak; and
- GC objects marked, retained, swept, and deferred.

Namespace IDs, keys, graph record IDs, query text, values, and source paths are
not metric labels. A diagnostic may include a short safe correlation ID and
bounded hashes where policy permits.

## Performance qualification

The design predicts improvements; it does not assert them. Qualification must
measure at least:

- clean `compass init` wall time and peak memory;
- no-change and small-change `compass update` time, bytes read, and bytes
  written;
- cold engine-open time and memory;
- point lookup, name search, one-hop traversal, multi-hop bounded traversal,
  impact, and representative CompassQL latency;
- concurrent readers during an update;
- JSON export time and byte equivalence;
- database size, temporary publication amplification, and GC work;
- local SQLite and redb at small, medium, and large graph sizes; and
- PostgreSQL and DynamoDB request count, transferred bytes, throttling, and
  estimated cost under controlled workloads.

Measurements include engine-open and index-build time. A benchmark that starts
after a full graph has already been deserialized cannot support a cold-query
claim. Results and thresholds belong in `PERFORMANCE.md` before a release makes
performance claims.

## Compatibility and migration policy

### What may hard-cut

Before a store format is released as durable user or cloud data, Compass may:

- replace its physical adapter schema;
- invalidate and rebuild store snapshots;
- discard old disposable query caches;
- change unpublished namespace or key encodings; and
- require regeneration from source or validated `graph.json`.

The implementation plan should prefer a clean hard cut over permanent support
for a prototype. Unknown store and snapshot majors fail explicitly.

### What remains compatible

The following are not waived:

- `graph.json` remains an available, validated graph engine;
- the existing `compass.graph/1` contract changes only through its normal
  compatibility process;
- public CLI arguments, streams, exits, output schemas, CompassQL semantics,
  MCP schemas, and stable graph IDs remain compatible unless separately
  versioned and documented;
- deterministic JSON export remains available from a store snapshot;
- relationship direction, multiplicity, anchors, provenance, and diagnostics
  survive round trips; and
- historical realizations already published remain immutable.

When a durable store format is first declared public, its schema identifier,
supported upgrade window, backup procedure, and migration tooling must be
added to `COMPATIBILITY.md` and `MIGRATION.md`.

## Rejected alternatives

### Replace `graph.json` with SQLite

Rejected. It breaks a useful inspection and interchange engine, creates a
forced migration, and couples public graph access to one database. Store
snapshots instead coexist with deterministic JSON export.

### Define the common API as SQL

Rejected. SQL syntax, collation, isolation, indexes, and query planners do not
map faithfully to redb or DynamoDB. Raw SQL would leak backend behavior into
graph meaning.

### Require cross-partition transactions

Rejected. It selects for a subset of backends and makes cloud scale and
failure semantics unnecessarily expensive. Immutable preparation plus one CAS
provides the needed publication atomicity.

### One namespace for all repositories and data planes

Rejected. It weakens tenant isolation, makes quotas and deletion ambiguous,
and increases blast radius. The namespace is the first explicit boundary.

### Backend-native indexes as graph authority

Rejected. SQLite FTS, PostgreSQL search, and DynamoDB secondary indexes differ
in scoring and consistency. They are useful accelerators only after
differential qualification.

### Serialize the complete graph as one database value

Rejected. It retains the rewrite, memory, and item-size problems of one large
JSON file while losing JSON's portability and inspectability.

## Open decisions to close in Phase 0

Before implementation proceeds beyond the reference memory store, maintainers
must freeze:

1. the exact v1 byte limits and key test vectors;
2. the async trait representation and cancellation/deadline type;
3. the content digest and deterministic codec set;
4. the ordered-tree implementation and chunking parameters;
5. namespace descriptor inputs and local store location policy;
6. the exact graph-read projection and cursor contracts;
7. the portable text normalization and ranking rules; and
8. whether filesystem `store.ref` is embedded in the existing manifest or is a
   separate typed artifact.

Each decision must land with tests and a compatibility assessment. None should
be left to the first backend adapter to decide implicitly.

## External implementation references

- [SQLite documentation](https://sqlite.org/docs.html), including WAL and
  `WITHOUT ROWID` guidance
- [redb API documentation](https://docs.rs/redb/latest/redb/)
- [PostgreSQL `INSERT ... ON CONFLICT`](https://www.postgresql.org/docs/current/sql-insert.html)
  and [transaction isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [DynamoDB partition and sort keys](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/HowItWorks.Partitions.html),
  [Query semantics](https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_Query.html),
  and [service constraints](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/Constraints.html)

These links explain backend behavior. The normative Compass contract is this
document plus its conformance tests, not a backend's superset of features.

## Related pages

- [Executable Compass store plan](../implementation/compass-store-plan.md)
- [Storage and history](storage-and-history.md)
- [System architecture](architecture.md)
- [Query engine](../implementation/query-engine.md)
- [Compatibility policy](../../COMPATIBILITY.md)
- [Performance qualification](../../PERFORMANCE.md)

**Next step:** execute
[Phase 0 of the implementation plan](../implementation/compass-store-plan.md#phase-0-freeze-the-contract-and-build-the-reference-store)
to turn the planned semantics into tested Rust contracts.
