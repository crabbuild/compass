## Purpose

Defines a lossless, optional, generation-atomic SurrealDB projection of canonical Compass graph generations without changing the default graph authority or product surface.

## ADDED Requirements

### Requirement: Optional dependency isolation
The projection capability SHALL live in a focused integration crate, SHALL require an explicit engine feature, and SHALL not link SurrealDB into default Compass library or binary builds. Surreal-enabled profiles MUST use exactly the reviewed SurrealDB 3.2.4 release and retain the recorded license obligations.

#### Scenario: Default workspace build
- **WHEN** Compass is built without a `compass-graphdb-surreal` engine feature
- **THEN** no SurrealDB package is present in that build's dependency graph or linked artifact

#### Scenario: Enabled embedded profile
- **WHEN** a caller explicitly enables Mem, SurrealKV, or RocksDB support
- **THEN** the adapter uses pinned SurrealDB 3.2.4 and exposes only that selected embedded-engine capability

### Requirement: Canonical immutable generation input
The projection SHALL accept only a validated `compass.graph/1` document and a non-empty repository identity. Every projected record SHALL carry the repository identity, the document generation identity, the projection schema version, and its stable Compass identity. A projection MUST be deterministic for equivalent inputs and MUST reject unsupported graph or projection schema versions explicitly.

#### Scenario: Valid generation is planned
- **WHEN** a validated canonical graph and non-empty repository identity are projected twice
- **THEN** both plans contain byte-equivalent, identically ordered node and relation records with the same identities

#### Scenario: Invalid input is rejected
- **WHEN** the repository identity is empty or the graph fails `compass.graph/1` validation
- **THEN** projection fails with a typed error and publishes no active generation

### Requirement: Lossless typed relation mapping
The projection SHALL use schemafull node and relation tables and a closed mapping from every Compass edge kind to a typed relation family with a required original `kind`. Stable edge identity, source-to-target direction, parallel multiplicity, self-loops, source anchors, provenance, and confidence MUST survive a projection round trip without semantic loss.

#### Scenario: Relationship semantics round trip
- **WHEN** one generation contains parallel same-direction edges, a reverse edge, and a self-loop with distinct evidence and confidence
- **THEN** reading the projected generation returns the same edge IDs, endpoints, kinds, ordering, evidence, anchors, and confidence values

#### Scenario: Closed mapping remains exhaustive
- **WHEN** the Compass edge vocabulary is compared with the projection mapping
- **THEN** every supported edge kind maps to exactly one typed relation family and no arbitrary relation-table name can be supplied by a caller

### Requirement: Generation-atomic activation
The adapter SHALL stage all records for one candidate generation, validate candidate counts and identities, and update a repository-scoped active-generation pointer only in the same successful transaction. Readers SHALL resolve the active generation once per operation. Failed, cancelled, or interrupted staging MUST leave the previously active generation visible and MUST never expose the partial candidate.

#### Scenario: Successful activation
- **WHEN** a complete candidate passes validation and its transaction commits
- **THEN** the active pointer selects that exact generation and all of its projected records are visible together

#### Scenario: Interrupted candidate
- **WHEN** staging stops before activation or the transaction is cancelled after only part of a candidate is written
- **THEN** readers continue to observe the previous complete generation and no partial candidate is active

### Requirement: Typed and bounded adapter surface
The adapter SHALL expose typed projection and activation operations only. It MUST NOT expose arbitrary caller-authored SurrealQL, MCP rendering, CLI presentation, or an unbounded result path. Database statements SHALL come from a closed internal query plan and all untrusted values SHALL be parameter-bound.

#### Scenario: Untrusted identifiers
- **WHEN** repository, generation, node, or edge identities contain SurrealQL metacharacters
- **THEN** they remain data values and cannot alter the selected statement or relation family

#### Scenario: Presentation boundary
- **WHEN** the integration crate is compiled in isolation
- **THEN** it has no dependency on `compass-cli` or `compass-mcp` and exports no presentation-specific response type
