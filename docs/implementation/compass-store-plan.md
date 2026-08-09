# Executable Compass store implementation plan

**Status:** Phases 0–5, the embedded retention/GC subset of Phase 8, and the
local Phase 9 operational/qualification slice are implemented. The CLI
publishes a SQLite query index by default alongside permanent `graph.json`
(`--store json` opts out); redb is a library-only adapter. PostgreSQL,
DynamoDB, hosted operation, distributed leases/GC, and service quotas remain
explicitly deferred until their phases are complete.

## Outcome

At completion, Compass has:

- a backend-neutral `compass-store` crate whose first boundary is a scoped
  namespace and whose complete address is `(namespace, partition, key)`;
- a permanent `JsonGraphEngine` for `graph.json`;
- a `StoreGraphEngine` using immutable content-addressed graph snapshots;
- SQLite and redb embedded adapters;
- PostgreSQL and DynamoDB remote adapters;
- atomic local and store-native publication;
- bounded graph reads that do not require full JSON deserialization;
- adapter conformance, cross-engine differential tests, fault injection,
  garbage collection, backup, and recovery coverage; and
- measured performance evidence before any release claim.

The current local release boundary is intentionally smaller than the eventual
cloud program: logical formats are versioned and validated, while physical
adapter files and query caches can be rebuilt. See the [operations guide](../guides/compass-store-operations.md)
for locations, backups, restore, quotas, recovery, and the first `0.3.x`
upgrade window.

## Shipped initial slice in this branch

The first implementation is intentionally smaller than the full program above
and is independently mergeable:

- `compass-store` exposes the namespace-first `(namespace, partition, key)`
  contract, bounded ordered scans, conditional writes, immutable writes, and a
  versioned SQLite realization in one package. The contract does not expose
  SQL or require a particular backend, so redb, PostgreSQL, DynamoDB, or a
  service adapter can implement the same trait later.
- Every committed local snapshot contains `graph.json`. Builds using the
  default SQLite storage (or explicit `--store sqlite`) additionally contain a
  typed `store.ref`; one shared
  `store/store.sqlite3` lives outside the snapshot
  directories. The store contains immutable digest-addressed projected trees,
  manifests, and a CAS-protected selector, but no legacy complete-graph
  payload. WAL is checkpointed before the BuildGuard switch; streamed
  canonical `graph.json` bytes bind the manifest; and the reference is checked
  before a store query runs.
- Typed code-query opening uses an adjacent validated `store.ref` by default
  and supports explicit `--engine default|json|store`. `json` always selects
  the permanent JSON engine; `store` requires a published database and
  reference and executes directly through projected immutable indexes. A
  default query falls back to JSON only when no store reference is present.

Acceptance criteria for this slice are deliberately concrete:

1. `cargo test -p compass-store --locked` passes namespace isolation,
   lexicographic scans/cursors, CAS, immutable retry, snapshot validation, and
   reopen tests.
2. `cargo test -p compass-query --test store_engine --locked` proves default
   sidecar selection, explicit JSON/store selection, immutable snapshot authority,
   selection, reference failures, pinned readers, cache identity, and
   JSON-equivalent typed query results.
3. The core publication regression test proves the shared database and
   snapshot reference are present, reopenable, digest-valid, and
   byte-identical to canonical `graph.json` after a local build.
4. Deleting or corrupting the sidecar never changes the JSON artifact; default
   query opening falls back only when the sidecar is absent, while an active
   Phase 2 snapshot without a matching `store.ref` and explicit store
   selection fail with typed errors.

The local slice does not claim PostgreSQL/DynamoDB adapters, remote retry
semantics, distributed leases, or hosted quotas. Its local performance claims
are limited to the documented Django qualification. The optional redb adapter
is delivered in the Phase 5 slice below; it is not linked into the released
CLI by default.

## Phase 2 memory-layout slice shipped in this branch

The first executable Phase 2 slice is intentionally independent of a durable
adapter. `compass-graph` now exposes a deterministic immutable snapshot builder
and reader over the namespace-first `compass-store::Store` contract. It writes
content-addressed compact tree objects, a typed manifest, and a CAS-protected
active selector using the memory reference store. The layout includes metadata,
nodes, edges, directional adjacency, files/source anchors, names, terms,
communities, and diagnostics roots.

The reader validates schema majors, object digests, tree ordering, root/index
identity, graph counts, and the complete typed graph before returning data. It
provides bounded point reads, ordered node/edge scans, directional adjacency,
and canonical JSON export. Repeated builds are idempotent and report immutable
object reuse; prepared content never becomes active until the selector CAS.
The permanent `graph.json` engine remains unchanged. Phase 3 persists these
same objects in the SQLite sidecar and verifies their canonical export before
the snapshot is sealed.

The slice's acceptance evidence is in
`crates/compass-graph/tests/store_snapshot.rs`: deterministic identity across
record insertion order and operational snapshot IDs, immutable object reuse,
selector-before-commit behavior, bounded reads, directional multiplicity, key
vectors, and fail-closed tamper detection. Persistent SQLite publication and
CLI snapshot checks are covered by the Phase 3 tests below; full
`JsonGraphEngine` versus streaming store-engine differential qualification and
measured performance claims remain follow-on work.

## Program rules

Every phase follows these rules independently:

1. **Keep phases mergeable.** A phase must leave the default branch usable and
   tested. Feature flags or internal configuration may hide unfinished paths,
   but an incomplete path must fail explicitly rather than silently change
   engines.
2. **Preserve JSON.** `graph.json` remains a supported engine and canonical
   export. No phase may call it “legacy,” “fallback pending removal,” or
   “deprecated.”
3. **Hard-cut only internal formats.** Unreleased store schemas, namespace
   encodings, and disposable query caches may be invalidated and rebuilt.
   Public graph, CLI, CompassQL, MCP, and historical contracts need their
   normal compatibility process.
4. **Prove semantics before speed.** Cross-engine graph equivalence and
   adapter conformance land before query routing or performance claims.
5. **Bound all work.** Every API and test covers item, byte, depth, frontier,
   response, deadline, retry, and concurrency limits relevant to it.
6. **Keep local operation native.** Default builds and tests require no cloud
   account, service credentials, Python, vector database, or live network.
7. **Test the lowest owner.** Store semantics live in `compass-store`, graph
   layout in `compass-graph`, publication in `compass-core` and
   `compass-files`, query behavior in `compass-query`, and public command
   effects in `compass-cli`.
8. **Use one target directory per checkout.** Every compiling Cargo command
   sets `CARGO_TARGET_DIR` under `<qualification-corpus-root>/crabbuild-target` as
   required by `AGENTS.md`.

## Phase dependency map

```text
Phase 0  portable store contract + memory reference
   |
   +--> Phase 1  graph-engine read boundary + permanent JSON engine
   |       |
   |       v
   +--> Phase 2  immutable graph snapshot layout + memory store engine
                   |
                   v
               Phase 3  SQLite adapter + optional local publication
                   |
                   v
               Phase 4  explicit store-backed query routing
                 /   \
                v     v
          Phase 5     Phase 6
             redb     PostgreSQL
                         |
                         v
                      Phase 7
                      DynamoDB
                 \       |       /
                  \      v      /
                   Phase 8
             multi-tenant operations
                         |
                         v
                      Phase 9
             qualification and release
```

Phases 5 and 6 can proceed independently after Phase 4. Phase 7 can begin
after the conformance harness and graph layout are stable, but it should not be
declared production-ready before the remote operational patterns in Phase 6
have been exercised.

## Shared definition of done

A phase is complete only when:

- its acceptance criteria are represented by automated tests or a checked
  qualification artifact;
- unknown major versions and unsupported capabilities fail explicitly;
- failure and interruption paths clean up or leave only unreachable immutable
  content;
- deterministic result ordering is asserted, not inferred;
- public errors are typed, bounded, actionable, and free of secrets;
- relevant docs identify behavior as available only after the implementation
  lands;
- `git diff` and `git status --short` contain no unrelated or generated noise;
  and
- the implementer reports which repository baseline and surface-specific gates
  ran, and why any applicable gate did not run.

The normal final Rust baseline is:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --locked -- -D warnings
cargo test --workspace --lib --bins --locked
```

Use the actual unique checkout name. Verify `<qualification-corpus-root>` is mounted and
writable before running Cargo; do not fall back to a local `target/`.

## Phase 0: Freeze the contract and build the reference store

### Context and objective

There is no safe backend work until address encoding, ordering, limits,
consistency, conditional writes, cursor behavior, and error semantics are
executable. This phase creates the lowest-level contract without graph logic or
database dependencies. A deterministic in-memory implementation is the
reference model used by every later adapter.

### Owned surfaces

- new `crates/compass-store/` package;
- root workspace membership and workspace dependency wiring;
- `compass-store` unit and public-contract tests; and
- no changes to CLI behavior or generated artifacts.

### Work

1. Create `compass-store` with `#![forbid(unsafe_code)]` and the workspace lint
   policy.
2. Define bounded newtypes for `NamespaceId`, `PartitionKey`, `Key`, `Value`,
   `VersionToken`, and `Cursor`.
3. Freeze the versioned, length-prefixed binary key encoding and checked-in
   golden vectors, including binary zeroes, maximum lengths, and prefix
   boundaries.
4. Freeze the portable v1 maxima proposed by the design. Record why each fits
   all four planned adapters.
5. Define object-safe asynchronous `Store`, namespace-scoped
   `NamespaceStore`, and backend-neutral `StoreFactory` requirements. The
   crate must not create or require one specific async runtime or a universal
   connection-string format.
6. Implement `get`, ordered one-partition `scan`, conditional `put`,
   conditional `delete`, and `put_immutable` semantics.
7. Define `StoreCapabilities`, operation budgets/deadlines, read consistency,
   page accounting, and the complete typed error taxonomy.
8. Implement a deterministic memory store with fault injection, clock
   injection, and controllable conflicts. It must model semantics, not attempt
   to model SQLite or DynamoDB latency.
9. Build a reusable adapter conformance module. Adapter crates must be able to
   invoke it without copying tests.
10. Add property tests for ordering and encode/decode round trips with bounded
    generators. Seed failures must be reproducible.
11. Add crate-level documentation stating that namespaces are isolation
    identifiers but not authorization claims.

### Required tests

- namespace A cannot read, scan, overwrite, or delete namespace B;
- partitions with identical keys remain isolated;
- keys are returned in unsigned lexicographic byte order;
- page item and byte ceilings stop before overflow and produce a valid cursor;
- resuming a cursor returns every matching entry exactly once;
- a cursor fails when reused with another range, partition, namespace,
  consistency, or store identity;
- concurrent `Missing` writes have exactly one winner;
- concurrent `Version` writes have exactly one winner per observed version;
- `put_immutable` accepts identical retries and rejects different bytes;
- limits fail before allocation or mutation;
- deadline and cancellation stop work without a detached mutation;
- errors redact values, keys, source text, and credentials; and
- unsupported capabilities fail before any write.

### Acceptance criteria

- `cargo test -p compass-store --locked` passes and includes all required
  conformance behaviors.
- `cargo clippy -p compass-store --all-targets --all-features --locked -- -D warnings`
  passes.
- Golden key vectors are identical on Linux, macOS, and Windows CI.
- A public API review confirms that no method scans a whole namespace, exposes
  raw SQL/backend expressions, promises cross-partition atomicity, or accepts
  unbounded values.
- The memory store passes the complete conformance suite under at least two
  deterministic fault schedules.
- No existing Compass command or file output changes in this phase.

### Exit and rollback

The output is a new unused contract crate, so rollback is removal of the crate
and workspace entry. Do not preserve an inadequate encoding for compatibility;
change it before Phase 2 and update golden vectors with an explicit review.

## Phase 1: Introduce the graph-engine boundary and preserve JSON

### Context and objective

Current query paths open `graph.json` and build in-memory lookup and adjacency
structures. Before a store engine exists, consumers need one immutable graph
read boundary. This phase moves current behavior behind that boundary while
keeping JSON results, files, and command behavior unchanged.

### Prerequisite inputs

- Phase 0's bounded key/value vocabulary is available, but the JSON engine
  does not depend on a store backend.
- Existing `compass-model::GraphDocument` validation and query tests are the
  behavioral oracle.

### Owned surfaces

- `compass-query` graph snapshot/read traits and planner inputs;
- a JSON implementation wrapping existing load, validation, binary cache, and
  FTS behavior;
- narrow wiring in `compass-cli` and `compass-mcp`; and
- query-engine implementation documentation.

### Work

1. Inventory every graph consumer: typed search, code search, traversal,
   impact, affected, CompassQL, reports, MCP resources/tools, viewer export,
   clustering, and tests.
2. Define an immutable `GraphEngine`/`GraphSnapshot` contract with bounded
   metadata, point lookup, ordered scans, incoming/outgoing adjacency, name and
   text candidates, community membership, projections, and JSON export. Its
   object-safe asynchronous/page surface must work for local and remote
   engines without owning an executor.
3. Define engine-independent cursors and result accounting where feasible. If
   a cursor is engine-specific, make it opaque, request-bound, and explicitly
   non-portable.
4. Implement `JsonGraphEngine` using the current strict graph validation and
   disposable content cache. Preserve its path size checks and corruption
   behavior.
5. Refactor each query family to use the graph snapshot rather than reaching
   into JSON-specific indexes. Do not change query algorithms in the same
   step unless required for bounded streaming.
6. Preserve deterministic tie-breaking and every current public diagnostic.
7. Add a test-only recording engine that asserts query budgets, projection
   use, range direction, and absence of unbounded calls.
8. Keep direct `--graph <path.json>` and equivalent library entry points on
   `JsonGraphEngine`.

### Required tests

- every existing query fixture produces byte-for-byte equal JSON output and
  equivalent human output before and after the refactor;
- graph open still rejects oversize, malformed, invalid-major, and invalid
  cross-reference inputs;
- directed traversal and reverse traversal request the correct index direction;
- multiedges remain distinct and preserve edge IDs and source anchors;
- a limit error is not converted to an empty result;
- query ordering is stable across repeated runs; and
- opening an explicit JSON file never requires a database.

### Acceptance criteria

- Existing `compass-query`, `compass-cypher`, `compass-cli`, and `compass-mcp`
  contract tests pass without approved snapshot changes.
- `cargo test -p compass-cypher --test tck --locked`,
  `cargo test -p compass-query --test opencypher_tck --locked`, and
  `python3 scripts/check_compassql_support.py` pass.
- The recording engine proves every public query supplies finite item/byte and
  traversal limits.
- `graph.json` remains the complete portable authority and all public CLI
  output is unchanged in this phase.
- Documentation names `JsonGraphEngine` as permanent supported behavior.

### Exit and rollback

Because the JSON implementation preserves the old semantics, rollback can
restore direct JSON types without data migration. Do not proceed if the new
trait requires a full graph allocation by definition; that would prevent the
later store engine from improving cold-query memory.

## Phase 2: Implement the immutable snapshot layout in memory

### Context and objective

This phase proves graph encoding, ordered indexes, structural sharing, and
publication semantics against the memory store before database behavior can
hide logical mistakes. It introduces `StoreGraphEngine` but does not publish a
user database.

### Prerequisite inputs

- Phase 0 supplies the store contract and conformance model.
- Phase 1 supplies the graph read contract and JSON oracle.
- The design document supplies the manifest and required logical indexes.

### Owned surfaces

- `compass-graph` snapshot builder and canonical graph-index encoders;
- `compass-query` store graph engine;
- `compass-model` only for new typed internal snapshot records whose ownership
  review confirms they are shared graph contracts; and
- cross-engine fixtures and differential tests.

### Work

1. Freeze `compass.store.graph-snapshot/1`, `SnapshotManifest`, root registry,
   value envelope, content digest, and snapshot identity projection.
2. Implement deterministic graph key encoders with golden vectors for nodes,
   edges, incoming/outgoing adjacency, files, names, terms, communities, and
   diagnostics.
3. Integrate or implement the immutable ordered-map tree. Freeze content-
   defined chunking parameters and enforce maximum fanout, depth, object size,
   decoded size, and traversal reads.
4. Stream canonical `GraphDocument` records into every required index. Reject
   duplicate logical keys and invalid references before publishing a manifest.
5. Use `put_immutable` for all content and manifest objects.
6. Implement manifest validation, root verification, and a prepared-snapshot
   result that is not active by construction.
7. Implement the selector-CAS protocol in the memory store and concurrent
   reader semantics.
8. Implement `StoreGraphEngine` point reads, ordered iteration, adjacency,
   search candidates, projections, and streaming JSON export.
9. Add incremental rebuild from prior roots and measure object reuse in test
   fixtures for no-change, one-node, one-edge, rename, and delete updates.
10. Add full differential tests between `JsonGraphEngine` and
    `StoreGraphEngine` over generated bounded graphs as well as repository
    fixtures.

### Required tests

- identical validated graphs produce identical snapshot IDs and roots across
  repeated builds and insertion orders;
- operational timestamps, thread count, and host paths do not change snapshot
  identity;
- every logical graph field survives store-to-JSON round trip;
- outgoing and incoming indexes preserve direction;
- duplicate endpoint/kind edges preserve multiplicity through edge IDs;
- missing/corrupt tree objects fail as corruption or unavailable, never empty;
- a reader opened before selector CAS completes on the old snapshot;
- a reader opened after CAS observes only the new complete snapshot;
- two concurrent publishers produce one selector winner;
- no-change update writes no new content objects after idempotence checks;
- small updates reuse unchanged tree objects; and
- an interrupted prepare never becomes active.

### Acceptance criteria

- The memory-backed store engine passes every Phase 1 graph-engine contract
  test.
- Canonical JSON exported by both engines is byte-identical for all fixtures
  and bounded generated graphs.
- A checked qualification report records structural-sharing ratios for the
  five update cases; ratios are evidence, not yet release promises.
- Tree reads fail before exceeding configured depth, object, byte, or result
  limits.
- The snapshot layout has an explicit unknown-major failure test and no
  backend-specific field.
- `./scripts/qualify_code_graph_v1.sh --fixtures-only` passes.

### Exit and rollback

This is still an unpublished internal format. If the layout is insufficient,
change its major or discard it; do not add a migration reader merely to retain
test data. Phase 3 must not begin until cross-engine equivalence is complete.

## Phase 3: Add SQLite and optional local publication (implemented local slice)

### Context and objective

SQLite is the first persistent adapter and exercises reopen, filesystem,
locking, crash, and durability behavior. Local builds selected with
the default SQLite storage (or `--store sqlite`) writes the store snapshot while
the JSON artifact remains a permanent independent engine. This isolates
write/publication risk from the
broader remote-adapter work.

### Prerequisite inputs

- Phase 0's adapter conformance harness;
- Phase 2's immutable snapshot builder; and
- the existing `BuildGuard` snapshot publication primitive.

### Owned surfaces

- the SQLite adapter in `compass-store` for the initial local slice (a future
  split into `compass-store-sqlite` remains an internal packaging choice);
- local store path and file lifecycle in `compass-files`;
- snapshot preparation and `store.ref` staging in `compass-core`; and
- CLI contract tests for `init` and `update` side effects.

### Work

1. Implement the SQLite physical schema, binary ordering, metadata/format
   table, conditional writes, ordered scans, and capability report.
2. Configure WAL, full synchronous durability, bounded busy timeout, bounded
   statement inputs/results, and coherent close/checkpoint behavior.
3. Reuse atomic path, containment, permissions, symlink, and recovery helpers
   from `compass-files`.
4. Run the complete adapter conformance suite against a newly created and a
   reopened database.
5. Add reopen/durability, WAL checkpoint, orphan-discovery, stale-format, and
   snapshot-publication tests. Fault-injected crash qualification remains a
   release gate for the larger Phase 3 evidence set.
6. Define a typed, versioned `store.ref` containing store identity, namespace,
   snapshot ID, manifest digest, and graph digest but no machine-specific
   absolute path.
7. During `init`, `update`, `extract`, or `watch` with default storage (or
   `--store sqlite`), prepare the SQLite snapshot and stage `store.ref` with
   `graph.json` and other output artifacts. The filesystem snapshot switch is
   the only local commit.
8. Keep `graph.json` as the permanent compatible engine. The default query
   uses a validated adjacent store reference, while explicit JSON always opens
   the JSON reader; SQLite publication compares canonical graph identity
   before commit.
9. Make SQLite publication the default dual profile. `--store json` opts out
   of the sidecar; selecting SQLite never removes JSON.
10. Retain two complete local snapshots and run bounded root-based object GC
    after coherent publication. Distributed lease-aware GC remains Phase 8.

### Required tests

- creation, reopen, write, read, scan, CAS, and delete pass conformance;
- acknowledged writes survive process reopen under the configured durability;
- copying or recovering SQLite accounts for WAL state;
- an interrupted database write cannot change the active filesystem
  snapshot;
- an interrupted filesystem switch leaves the prior snapshot readable;
- published `graph.json` and `store.ref` select equal graph digests;
- a store database missing or corrupt after publication does not make the
  co-published JSON unreadable;
- concurrent local updates produce a guarded conflict rather than mixed
  artifacts;
- local paths cannot escape through symlinks or untrusted namespace text; and
- stale/unreleased SQLite formats fail with a rebuild instruction.

### Acceptance criteria

- the initial adapter's `cargo test -p compass-store --locked` passes
  namespace/CAS/scan conformance, WAL reopen durability, orphan discovery,
  and stale-format checks; injected busy/interrupt qualification remains a
  separate release gate.
- `compass init --store sqlite` and `compass update --store sqlite` CLI
  integration tests assert successful exit, `graph.json`, `store.ref`,
  complete manifest, and no visible partial snapshot.
- Cross-engine comparison covers the published core fixture and CLI init/update
  snapshots and rejects publication on any canonical mismatch.
- The permanent `graph.json` engine remains independently readable when the
  store is missing or corrupt. Default queries use the validated sidecar when
  present and fail closed when a published reference is corrupt; explicit
  `--engine json` remains independent.
- The old disposable query SQLite cache may be hard-cut or rebuilt, but the
  new durable snapshot store uses a different format identity and location.

### Exit and rollback

Disable SQLite publication and remove unpublished databases. Existing
`graph.json` snapshots remain valid. Do not attempt to repair a partially
published store by rewriting its immutable objects; rebuild a new snapshot.

## Phase 4: Route local queries to the store engine (implemented local slice)

### Context and objective

After optional publication proves equivalence and durability, local readers
can explicitly select the co-published store snapshot. The local slice reads
the immutable Phase 2 snapshot, validates the active selector and `store.ref`,
and compares every typed code-query operation with the JSON engine. Typed
queries use projected point/name/term/adjacency reads without materializing or
cloning a complete `GraphDocument`. JSON remains selectable and is used by
default or whenever the caller explicitly selects it.

### Prerequisite inputs

- Phase 3 has qualification evidence from real local build snapshots.
- Every current query family is already behind the Phase 1 graph read contract.

### Owned surfaces

- engine selection in `compass-core` and thin CLI/MCP wiring;
- store-aware plans and projection batching in `compass-query`;
- query cache lifecycle; and
- performance and differential qualification.

### Work

1. Define deterministic engine selection:
   - explicit JSON path selects `JsonGraphEngine`;
   - a validated active snapshot with a valid `store.ref` selects
     `StoreGraphEngine` by default;
   - an explicit `--engine json|store` diagnostic option may override the
     default where the command surface review approves it.
2. Validate `store.ref`, store identity, namespace, snapshot ID, manifest
   digest, and graph digest before executing a query.
3. Make store query plans use projections, bounded point-read batches, ordered
   adjacency ranges, and finite traversal frontiers.
4. Port deterministic name/text ranking to portable indexes. Keep SQLite FTS
   only as a disposable, differential-tested accelerator.
5. Make corruption and unavailability actionable. An automatic switch to JSON
   is permitted only before results are emitted, only when the selected local
   snapshot proves digest equivalence, and only with an explicit diagnostic.
6. Remove assumptions that opening a current graph loads every node and edge.
7. Add engine identity to safe diagnostics and telemetry, not to public result
   meaning.
8. Benchmark complete cold command latency, including engine open and any
   accelerator build.
9. Define a typed storage/materialization profile at the application boundary:
   `json`, `store`, or `dual`. Preserve the existing output profile by default;
   expose `store` only through a separately reviewed CLI/configuration change.
   Store-only snapshots must support bounded deterministic JSON export.

The shipped local slice implements items 1–8 for typed code queries and the
dual local profile. It pins opened readers, uses backend-neutral candidate
ordering and tokenization, and records Django build/query/RSS qualification.
Store-only publication and broader CompassQL/MCP projection plumbing remain
separately gated.

### Required tests

- every public query produces the same ordered machine output under JSON and
  SQLite store engines;
- human output differs only by an approved diagnostic when engine recovery is
  exercised;
- partial results from one engine are never combined with retry results from
  another;
- an explicit JSON input never opens an adjacent store, while the default
  engine uses a validated adjacent store reference when present;
- stale `store.ref`, wrong namespace, wrong digest, missing object, corrupt
  envelope, unsupported major, and store timeout each have distinct tests;
- query item, byte, frontier, depth, and deadline limits are enforced on store
  plans;
- current graph updates do not invalidate readers already pinned to the prior
  immutable snapshot; and
- accelerator deletion changes latency only, not ordered results;
- `json` publishes the existing artifact set without requiring a database;
- `dual` publishes equal JSON and store snapshots in one snapshot; and
- `store` avoids full JSON materialization but can export byte-identical
  canonical JSON on demand.

### Acceptance criteria for the shipped local slice

- `compass-query` differential tests cover typed search, callers, callees,
  impact, explore, node trails, explicit JSON isolation, stale references,
  immutable snapshot authority, and reader pinning.
- The default and explicit store query paths select the immutable snapshot only when its
  active selector and `store.ref` validate; malformed or missing references
  fail closed, while explicit JSON remains independent.
- The JSON engine's disposable `compass-code-index/2` and the store term index
  share token and candidate-order semantics; store queries do not construct the
  disposable index.
- Existing CompassQL TCK, OpenCypher TCK, CLI product tests, product-boundary
  check, and fixture-only code-graph qualification remain green.

Full cross-engine differential coverage for CompassQL/MCP and any additional
storage profiles remain follow-on gates. Typed projected-query and
`PERFORMANCE.md` cold-open/RSS measurements are complete for the local slice.
- Documentation clearly states that `graph.json` is a compatible permanent
  engine, not a deprecated fallback.

### Exit and rollback

Engine selection can return to JSON without converting data because each local
snapshot still contains it. Keep store databases as disposable/unselected or
rebuild them. A rollback must not remove the graph read abstraction.

## Phase 5: Add the redb embedded adapter (implemented local slice)

### Context and objective

redb provides a second local implementation with different concurrency and
physical-storage behavior. Its purpose is both a usable embedded option and a
test that the common contract did not accidentally become SQLite-shaped. The
implemented slice keeps redb in `compass-store-redb`, uses the same graph
snapshot builder and a backend-neutral query opening hook, and deliberately
does not add redb to the default CLI binary.

### Prerequisite inputs

- Phase 0 conformance and Phase 2 graph differential suites are stable.
- Phase 4 graph-engine boundary can consume a backend-neutral `Store` without
  changing graph meaning.

### Owned surfaces

- new `compass-store-redb` crate;
- backend-neutral local selection hook and adapter documentation; and
- adapter-specific durability, contention, and reopen tests.

### Work

1. Add redb as a pinned workspace dependency after dependency-policy review.
2. Implement the composite binary address encoding, read transactions,
   conditional write transactions, ordered scans, versions, and capabilities.
3. Bound the single-writer queue and expose backpressure rather than allowing
   unbounded waiting tasks.
4. Map commit durability and process reopen to the common acknowledgement
   contract.
5. Run graph snapshot construction and every query family through redb.
6. Add backend selection without changing namespace, manifest, graph-index, or
   JSON contracts.
7. Document file backup and recovery separately from SQLite.

The shipped local slice implements all seven items for library consumers and
tests. Explicit CLI/backend configuration remains gated on packaging and
performance policy; `graph.json` and SQLite remain the supported local command
engines.

### Required tests

- `cargo test -p compass-store-redb --locked` runs the shared conformance,
  reopen/backup, binary-order, writer-gate, and graph-snapshot differential
  tests.
- `cargo test -p compass-query --test store_engine --locked` exercises
  `open_with_store` against redb for search, callers, callees, impact,
  explore, and node trails.
- conformance under create, reopen, and contention; injected commit-failure
  schedules remain a follow-on fault-injection gate;
- byte ordering across composite namespace/partition/key boundaries;
- bounded single-writer backpressure and cancellation;
- reader snapshot stability during selector publication; and
- full graph/query differential coverage against JSON and SQLite.

### Acceptance criteria for the shipped local slice

- redb passes the unmodified shared adapter conformance suite, including
  deterministic reopen, ordering, CAS, immutable-write, and bounded writer
  backpressure tests.
- Memory, SQLite, and redb produce identical graph snapshot IDs, logical roots,
  JSON exports, and ordered query results for the same graph.
- A concurrent writer test proves the adapter rejects a full process-local
  writer gate rather than accumulating unbounded tasks.
- No redb type appears in `compass-store`, `compass-model`, public graph
  schemas, or query result contracts.
- `compass-query::open_with_store` runs all typed code-query families through a
  redb snapshot and matches JSON output; the local binary does not include redb
  unless packaging/product policy intentionally selects or exposes it.

Full injected commit-failure schedules, cross-process deadline cancellation,
CompassQL/MCP differential coverage, and measured redb-versus-SQLite
performance are explicit follow-on gates.

### Exit and rollback

Remove the optional adapter and its configuration. The store and graph formats
remain usable through SQLite and JSON, so there is no graph migration.

## Phase 6: Add the PostgreSQL remote adapter

### Context and objective

PostgreSQL proves that the contract works across a network, connection pool,
server transactions, cancellation, and multiple service workers. This phase
does not yet ship a multi-tenant Compass cloud product; it produces a hardened
remote adapter and controlled service-boundary tests.

### Prerequisite inputs

- Store and graph contracts are stable through Phase 4.
- Security review has defined endpoint, TLS, and credential configuration.
- CI can provide an isolated disposable PostgreSQL service without real
  credentials or persistent user data.

### Owned surfaces

- new `compass-store-postgres` crate;
- backend-neutral remote connection configuration at an application boundary;
- local mock/disposable-service integration tests; and
- security, operations, and adapter documentation.

### Work

1. Add a bounded asynchronous PostgreSQL client and pool after dependency and
   feature review.
2. Implement the binary composite primary key, format metadata, point reads,
   constrained ordered scans, conditional insert/update/delete, and versions.
3. Set connection, acquisition, statement, response, and total operation
   deadlines. Wire cancellation so expired work does not continue unnoticed.
4. Enforce TLS and endpoint policy appropriate to the configured deployment.
   Redact DSNs, credentials, query values, and repository identifiers.
5. Run schema installation/migration through an explicit administrative path;
   ordinary graph callers cannot create arbitrary tables or schemas.
6. Exercise concurrent publisher CAS from multiple processes and readers
   pinned before/after publication.
7. Test connection loss after request send and before response. Immutable
   retries verify digest; CAS returns an indeterminate-safe diagnostic unless
   the selector can be strongly reread and resolved.
8. Validate namespace isolation in application handles and, where configured,
   database permissions or row-level security as defense in depth.
9. Record request counts, bytes, pool wait, query time, retries, and conflict
   rates with low-cardinality metrics.

### Required tests

- schema creation is restricted to the administrative path;
- CAS races from separate connections have exactly one winner;
- ordered scans survive service page boundaries without gaps or duplicates;
- connection acquisition, statement, response, and total deadlines each
  terminate with the correct error;
- an ambiguous connection failure is resolved by a strong reread or reported
  without an unsafe retry;
- primary and permitted replica routes meet the requested consistency; and
- namespace and cursor replay attacks fail without information disclosure.

### Acceptance criteria

- PostgreSQL passes the shared conformance suite against a disposable local CI
  service, including reopen, concurrent processes, cancellation, and injected
  connection failures.
- Tests prove every normal statement constrains namespace and partition; no
  unbounded table scan is available to graph callers.
- Two authenticated test principals cannot cross namespace boundaries through
  point reads, scans, cursors, diagnostics, or maintenance APIs.
- Store and JSON engines have identical query and export results.
- Secrets do not appear in snapshots, errors, logs, metric labels, or test
  artifacts.
- Default local Compass packaging and operation still require no PostgreSQL
  client configuration or network.

### Exit and rollback

Remove or disable the remote adapter. Immutable snapshots remain exportable to
JSON before teardown. Because the physical schema is adapter-private, no other
backend must read PostgreSQL tables directly.

## Phase 7: Add the DynamoDB adapter

### Context and objective

DynamoDB exercises the strictest partition-first deployment: exact partition
key locality, paginated Query, conditional writes, capacity throttling, item
limits, and eventually consistent secondary indexes. The adapter must use the
base table's strong-read path for authoritative records and must not require
global transactions.

### Prerequisite inputs

- Phase 0 limits fit DynamoDB key and item constraints with measured envelope
  overhead.
- Phase 2 digest sharding avoids sequential or repository-wide hot partitions.
- Phase 6 has established remote cancellation, retry, and secret-handling
  patterns.

### Owned surfaces

- new `compass-store-dynamodb` crate;
- AWS endpoint, region, credential-source, retry, and capacity configuration at
  the application boundary;
- protocol mocks or a controlled local DynamoDB implementation; and
- cloud cost/capacity qualification artifacts.

### Work

1. Add only the required AWS SDK features after dependency-size and build-time
   review. Keep the adapter out of default local packages unless selected.
2. Encode `PK = (namespace, partition)` and `SK = key` with checked limits and
   golden test vectors.
3. Implement strongly consistent GetItem and base-table Query, conditional
   PutItem/UpdateItem/DeleteItem, digest verification, and opaque versions.
4. Translate one Compass page across service 1 MiB pages without exceeding the
   caller's byte/item/deadline budget. Protect and bind continuation cursors.
5. Handle throttling with bounded exponential backoff and jitter. Surface
   exhausted throttling separately from not found and conflict.
6. Prohibit authoritative selector or manifest reads through an eventually
   consistent global secondary index.
7. Measure partition distribution for large and adversarial graph fixtures.
   Revise the unreleased shard function if hot keys appear.
8. Model ambiguous network outcomes: reread immutable content by digest; reread
   selectors strongly and compare the intended snapshot before reporting the
   final outcome.
9. Add request, byte, consumed-capacity, retry, throttle, and projected-cost
   reporting without high-cardinality identifiers.
10. Test only against controlled local resources or mocks in normal CI. A
    separately authorized qualification environment may run non-default cloud
    tests with synthetic data.

### Required tests

- item encoding reaches every portable length boundary without crossing a
  service key or item limit;
- service pagination maps to Compass pages without gaps, duplicates, false
  completion, or byte-budget overflow;
- conditional failures map to `Conflict`, throttling to `Throttled`, and an
  absent strong read to `NotFound`;
- retries stop at the request deadline and never repeat a non-idempotent write;
- a global secondary index cannot be selected for an authoritative read;
- hot-partition fixtures exercise the documented shard calculation; and
- ambiguous request outcomes resolve through strong selector or digest reads.

### Acceptance criteria

- The adapter passes the same conformance suite as memory, SQLite, redb, and
  PostgreSQL, with service pagination and throttling explicitly exercised.
- No value can exceed the portable limit after envelope and DynamoDB attribute
  overhead.
- Authoritative reads request strong consistency and tests fail if routed
  through an eventual-only index.
- Synthetic large graphs distribute content across the documented shard
  envelope; the report includes the hottest partition and request count.
- A throttled or partially paginated scan never returns a false complete page.
- Credentials and endpoints remain optional and explicit; offline Compass
  tests and structural extraction never contact AWS.

### Exit and rollback

Disable the adapter and export retained snapshots through the graph engine
before deleting deployment tables under a separately authorized operational
procedure. Never put table deletion into ordinary Compass rollback code.

## Phase 8: Add multi-tenant operations, retention, and recovery

### Context and objective

Passing CRUD conformance is insufficient for a cloud service. This phase adds
the backend-neutral control plane: authenticated namespace scoping, quotas,
leases, garbage collection, backup validation, recovery, and safe
observability. It runs over PostgreSQL and DynamoDB and remains usable for
embedded maintenance.

### Prerequisite inputs

- At least one embedded and one remote adapter have passed conformance and
  graph differential qualification.
- Product/security owners have defined tenant, project, repository, and data-
  retention identities.

### Owned surfaces

- a service/application crate chosen by architecture review, not
  `compass-store` itself;
- namespace catalog and authorization integration;
- build/reader leases and active-selector orchestration;
- portable root-based GC and restore validation; and
- security, privacy, operations, and support documentation.

### Work

1. Derive namespace IDs server-side from authenticated tenant, repository,
   data-plane, realm, and schema-major descriptors.
2. Issue namespace-scoped handles after authorization. Remove raw namespace
   selection from user-controlled graph/query requests.
3. Enforce quotas for concurrent builds, stored bytes, snapshots, query
   objects/bytes/frontier/time, response size, and backend capacity.
4. Implement versioned build and reader leases with bounded renewal and
   explicit expiry. A crashed worker cannot hold data forever.
5. Implement mark-and-sweep GC rooted at active selectors, retained local
   references, pins, history roots, and unexpired leases. Use checkpoints,
   safety windows, shard budgets, and conditional deletion.
6. Add namespace deletion as a separately authorized, auditable, resumable
   workflow. It must enumerate explicit manifests/checkpoints rather than run
   an unbounded synchronous request.
7. Implement backend-specific coherent backup procedures and a common restore
   validator that checks selector, manifest, all roots, digests, graph schema,
   and completion evidence before activation.
8. Exercise disaster recovery into a new store identity and namespace. Opaque
   old version tokens and cursors must not be reused.
9. Add low-cardinality metrics, structured audit records, correlation IDs,
   redaction tests, and per-tenant cost attribution without source disclosure.
10. Threat-model confused deputy, namespace collision, cursor replay, quota
    bypass, malicious stored values, hot partitions, credential exposure, and
    cross-tenant accelerators.

### Required tests

- authenticated tenant A cannot derive or use tenant B's scoped handle;
- namespace display names and repository paths cannot alter physical identity
  through encoding ambiguity;
- quota exhaustion produces a typed limit result and no background overrun;
- expired leases stop protecting data, while live readers remain protected;
- GC preserves every reachable object and eventually removes only eligible
  orphans under deterministic fault schedules;
- interrupted GC and namespace deletion resume from bounded checkpoints;
- restore refuses missing, corrupt, wrong-major, wrong-namespace, and
  incomplete snapshots;
- accelerator data never crosses snapshot or namespace boundaries;
- audit and metrics contain no source, values, credentials, raw keys, or
  sensitive paths; and
- a compromised/untrusted stored envelope cannot force an oversized
  allocation or unsafe path.

### Acceptance criteria

- Security review signs off on namespace authorization, credentials, remote
  endpoint policy, cursor integrity, quotas, redaction, and deletion.
- `SECURITY.md` and security/privacy design docs describe all new trust,
  credential, network, storage, and disclosure boundaries.
- GC fault tests prove no reachable-object deletion across every supported
  adapter.
- A documented backup/restore drill succeeds for SQLite, PostgreSQL, and
  DynamoDB qualification environments and produces equal canonical JSON.
- Service load tests demonstrate bounded resources during tenant concurrency,
  throttling, worker crashes, and large responses.
- Local Compass remains usable without enabling or configuring any service
  surface.

### Exit and rollback

Stop accepting new remote publications, retain selectors and leases, export or
back up reachable snapshots, then disable the service control plane. Data
deletion requires its explicit audited workflow and is never implied by code
rollback.

## Phase 9: Qualify, document, and release the storage contract

### Context and objective

This phase converts implemented behavior into a supported product contract.
Until it completes, store formats may still hard-cut and performance remains a
development observation. Release requires compatibility, migration, security,
performance, packaging, and operational evidence.

### Prerequisite inputs

- All adapters intended for the release have passed conformance.
- Local store-backed queries have passed full JSON differential testing.
- Cloud operations are included only if Phase 8 is complete for that surface.

### Owned surfaces

- `COMPATIBILITY.md`, `MIGRATION.md`, `CHANGELOG.md`, `PERFORMANCE.md`,
  `SECURITY.md`, `SUPPORT.md`, command/configuration/output references, and
  operations guides;
- packaging, install, upgrade, and downgrade tests; and
- release qualification artifacts.

### Work

1. Declare exactly which store envelope, graph-snapshot layout, adapter
   physical formats, and configuration surfaces are public and which remain
   rebuildable implementation details.
2. Decide the first supported upgrade window. Provide migration or explicit
   rebuild tooling for formats now declared durable.
3. Document backend selection, locations/endpoints, credentials, TLS,
   durability, backup, restore, GC, quotas, and recovery.
4. Document that `graph.json` remains supported for direct input, publication,
   interchange, inspection, recovery, and deterministic export.
5. Run clean/no-change/small-change builds and complete cold-query benchmarks
   at documented graph sizes. Include peak memory, bytes, request counts,
   database size, write amplification, and GC.
6. Run all cross-engine differential, adapter conformance, crash, corruption,
   concurrency, namespace isolation, and restore suites on the release commit.
7. Validate packaging does not accidentally include cloud SDKs, credentials,
   or local database files in unrelated distributions.
8. Add release-visible changes to `CHANGELOG.md` and user actions to
   `MIGRATION.md`.

### Local `0.3.x` implementation in this phase

The local release work closes the first support window without claiming cloud
readiness:

- `compass store status|validate|backup|restore` validates the co-published
  SQLite snapshot and writes digest-bound backup bundles;
- `scripts/rebuild_compass_store.sh` provides an explicit, rollback-preserving
  hard-cut rebuild path;
- `compass-store-qualification` and
  `scripts/qualify_compass_store_release.sh` measure SQLite/redb graph sizes,
  peak RSS, bytes, request counts, database size, write amplification, GC
  state, and CLI clean/no-change/small-change/cold-query behavior;
- the harness compares canonical JSON, typed query responses, and CompassQL
  results and runs the adapter, corruption, concurrency, snapshot, packaging,
  and product-boundary gates; and
- the operations, compatibility, migration, security, support, and
  configuration references declare SQLite as the local CLI backend, redb as a
  library-only adapter, and PostgreSQL/DynamoDB as deferred service phases.

The generated qualification directory is external evidence and is never
committed. Run the harness again on the final release commit after all docs and
release metadata have settled.

### Release acceptance criteria

- Canonical JSON exported from every released engine is byte-identical for the
  release qualification corpus.
- All public query and CompassQL outputs are equal across JSON and released
  store engines.
- `PERFORMANCE.md` contains reproducible evidence for each performance claim;
  regressions outside agreed budgets block the default cutover.
- `COMPATIBILITY.md` lists supported store/snapshot majors, backend/platform
  support, hard-cut boundaries, and the permanent JSON engine.
- Backup and restore procedures have been executed, not only reviewed.
- The repository native baseline and every applicable surface gate pass on the
  release commit.
- No generated graph, `.compass/` state, database, credentials, private source,
  or machine-specific path is committed.

### Release gates

In addition to targeted crate tests, run:

```bash
cargo test -p compass-cli --test compass_product --locked
sh scripts/check_product_boundary.sh

cargo test -p compass-cypher --test tck --locked
cargo test -p compass-query --test opencypher_tck --locked
python3 scripts/check_compassql_support.py

./scripts/qualify_code_graph_v1.sh --fixtures-only
```

Run repository qualification on external repositories only under
`<qualification-corpus-root>/Github` and treat them as read-only, following
`AGENTS.md`.

### Exit and rollback

Before public format declaration, rollback may rebuild store state and route
local queries to JSON. After declaration, use the documented migration and
support policy; do not silently invalidate durable cloud or user data. Query
cutover can still return to the co-published JSON engine without changing
public graph meaning.

## Work-package checklist

Every pull request implementing part of a phase should include this information
in its description:

```text
Phase and bounded work package:
Owning crate(s):
Contract/version affected:
Failure modes added or changed:
Limits enforced:
JSON equivalence evidence:
Adapter conformance evidence:
Performance evidence or “no claim”:
Compatibility decision: hard cut | rebuildable | public-compatible
Security/network/credential impact:
Targeted checks run:
Baseline/gates run or reason omitted:
Rollback path:
```

Do not combine a store encoding change, a query semantic change, and a public
CLI change in one work package unless the phase cannot be made coherent any
other way.

## Final system acceptance

The program is complete only when a reviewer can demonstrate this sequence
without inspecting backend internals:

1. build one graph and publish a coherent JSON plus store snapshot;
2. query both engines and obtain equal ordered outputs;
3. interrupt an update at each publication boundary and retain the old graph;
4. complete an update and keep an already-open reader on its old snapshot;
5. reopen every embedded backend and recover every remote backend from a
   coherent backup;
6. race two publishers and observe one active-selector winner;
7. exhaust scan, query, retry, throttle, and quota budgets without false empty
   results or background overruns;
8. attempt cross-namespace access and receive denial without information
   disclosure;
9. remove disposable accelerators and obtain the same results; and
10. export deterministic `compass.graph/1` JSON from every store backend.

## Related pages

- [Compass store and graph-engine design](../design/compass-store.md)
- [Storage and history](../design/storage-and-history.md)
- [Extraction pipeline](extraction-pipeline.md)
- [Query engine](query-engine.md)
- [Workspace tour](workspace-tour.md)
- [Compatibility policy](../../COMPATIBILITY.md)
- [Performance qualification](../../PERFORMANCE.md)

**Next step:** after the local release is accepted, start the separately scoped
PostgreSQL service adapter only with endpoint, credential, TLS, quota, lease,
and tenant-isolation evidence. Do not widen the local CLI backend implicitly.
