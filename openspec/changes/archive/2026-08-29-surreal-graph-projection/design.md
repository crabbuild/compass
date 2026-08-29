## Context

The canonical typed graph is `compass_model::code_graph::GraphDocument`; its build metadata already carries the immutable `generation_id`, source-tree digest, configuration digest, and schema fingerprint. `compass-graphdb` is the closest graph-database integration boundary, while C-012 proved SurrealDB 3.2.4 Mem/SurrealKV/RocksDB storage semantics independently and C-011 approved the named artifact profiles. The default JSON/SQLite realization remains authoritative.

## Goals / Non-Goals

**Goals:**

- Keep projection planning deterministic, inspectable, and testable without compiling SurrealDB.
- Preserve exact canonical node and edge payloads while exposing stable indexed fields for later native reads.
- Make candidate publication one database transaction whose final mutation is the active-generation pointer.
- Keep the engine SDK and BSL-covered code behind explicit, non-default features.

**Non-Goals:**

- No query-engine routing, MCP/CLI surface, remote server mode, live subscription, or Store implementation in C-014.
- No garbage collection of inactive generations; published generations are immutable and cleanup needs its own retention contract.
- No claim that SurrealDB becomes the canonical or default backend.

## Decisions

### Separate the deterministic plan from the optional runtime

The always-compiled crate core builds a `ProjectionPlan` from a validated typed graph. It owns typed projected node/relation records, record keys, ordering, closed relation-family mapping, expected counts, and validation. An `engine` module compiled only by `mem`, `surrealkv`, or `rocksdb` executes that plan against `Surreal<local::Db>`.

This keeps ordinary workspace builds free of SurrealDB while allowing most semantic properties to be tested cheaply. Putting plan construction directly inside SDK calls was rejected because it makes determinism, feature-off footprint, and failure injection harder to prove.

### Preserve full typed records plus indexed contract fields

Each row stores the stable Compass ID, repository/generation/schema fields, kind, source endpoints where applicable, effective confidence, and a canonical JSON payload of the original typed record. The exact payload is the round-trip authority; indexed fields support validation and C-015 reads. Database record keys are SHA-256 hashes of the record class and length-delimited identity components, avoiding identifier grammar ambiguity and preserving deterministic collision-resistant addressing.

Dropping unknown or less frequently queried typed fields was rejected because projection is an equivalence surface. Treating database record IDs as the Compass IDs was rejected because arbitrary valid Compass identities need not be valid unquoted Surreal record keys.

### Use a closed set of relation families

Every `EdgeKind` maps exhaustively to one of five internal families: structural, dependency, execution, data-flow, or evidence. Each family is a schemafull Surreal relation table constrained from `code_node` to `code_node`, and every relation row also requires the original Compass `kind`. The executor selects one of five static parameterized statements; no table or statement string is accepted from a caller.

One table per edge kind was rejected as unnecessarily large schema churn. One generic table without a closed family was rejected because it weakens type constraints and makes arbitrary relation naming possible.

### Activate inside the staging transaction

The runtime ensures schema definitions first, then begins one transaction, rejects mutation of an already recorded immutable generation, writes every node and relation, validates counts and distinct stable IDs inside that transaction, records the complete generation manifest, and finally upserts the repository pointer before commit. Any SDK or validation failure cancels the transaction. A test-only interruption hook cancels after a bounded number of mutations to prove the previous pointer remains visible.

Writing candidate batches outside the activation transaction was rejected for C-014 because it permits orphan candidates and requires a more complex recovery protocol. The separately retained C-012 probes cover dirty-shutdown behavior for persistent engines; C-014's runtime uses the stronger single-transaction model.

### Pin and isolate engine features

The root workspace dependency pins `surrealdb = "=3.2.4"` with default features disabled. The new crate has an empty default feature set and maps `mem`, `surrealkv`, and `rocksdb` separately to the SDK's `kv-mem`, `kv-surrealkv`, and `kv-rocksdb` features. Engine constructors are available only for the selected feature. License metadata and the exact tagged license fixture are referenced in crate documentation.

## Risks / Trade-offs

- **SurrealDB remains expensive to compile even for Mem tests** → keep SDK checks in a dedicated feature-enabled gate and prove default dependency isolation with `cargo tree`.
- **A future Compass edge kind could make the mapping incomplete** → exhaustively match the closed Rust enum and assert `EdgeKind::ALL` coverage.
- **One large transaction can consume substantial resources** → C-014 accepts only already bounded canonical generations; C-015 owns bounded reads and later measurement gates. If C-013 resource budgets fail, the falsifier protocol stops the feature.
- **Canonical JSON serialization can fail on non-finite numbers** → typed graph validation already rejects non-finite evidence, and plan construction returns a typed serialization error without touching the database.
- **Schema changes can strand older rows** → projection schema is explicit and unknown majors fail; inactive generations are never rewritten in place.

## Migration Plan

This is additive and disabled by default. Land the crate, exact dependency pin, license notice, and feature-off gate first. Surreal-enabled callers explicitly select one embedded engine and build a fresh projection from a canonical graph. Rollback disables the feature and continues using the unchanged JSON/SQLite path; no canonical data migration or rewrite is required.
