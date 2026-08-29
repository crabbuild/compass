## Purpose

Defines closed, bounded graph-native reads over an active Surreal projection and the deterministic evidence required to prove semantic equivalence with Compass's current query engine.

## ADDED Requirements

### Requirement: Active-generation native reads
The Surreal adapter SHALL expose typed operations for exact symbol context, impact, directed path, and connected subgraph reads. Each operation SHALL resolve the repository's active generation exactly once and SHALL read only records from that immutable generation. A missing active generation, incomplete manifest, or changed pointer during the operation MUST return a typed error rather than an empty result or mixed-generation response.

#### Scenario: One immutable generation per operation
- **WHEN** a repository pointer changes while a native traversal is in progress
- **THEN** the response contains records from only the generation selected at operation start or fails explicitly without returning a partial response

#### Scenario: Missing active generation
- **WHEN** a repository has no complete active generation
- **THEN** the operation returns a typed unavailable-generation error distinct from an empty graph result

### Requirement: Closed parameter-bound query plan
Every database statement SHALL be selected from a closed internal plan and every repository, generation, symbol, cursor, and limit value SHALL be parameter-bound. The adapter MUST NOT accept caller-authored SurrealQL or caller-selected table names.

#### Scenario: Query-shaped identity
- **WHEN** any request identity contains SurrealQL syntax or relation-table text
- **THEN** the value remains data and cannot alter the statement, table family, generation, or bound

#### Scenario: Unsupported operation
- **WHEN** a caller requests an operation outside the closed typed surface
- **THEN** the adapter rejects it without executing a database statement

### Requirement: Independent bounded traversal and pagination
Every operation SHALL enforce positive finite depth, node, relation, path, candidate, response-byte, and page-item limits as applicable. Database reads MUST request at most the remaining bound plus one sentinel record, traversal MUST stop at the first exhausted bound, and truncation MUST be explicit. Paginated results SHALL use an opaque generation-bound cursor and SHALL reproduce the same canonical order when pages are concatenated.

#### Scenario: Bound exhaustion
- **WHEN** one independent limit is exhausted before traversal completes
- **THEN** the response stays within every configured limit, reports truncation, and never converts the limit condition into an empty success

#### Scenario: Stable pagination
- **WHEN** the same immutable result is read with different positive page sizes
- **THEN** concatenated pages contain exactly the unpaged canonical identities once, in the same order, with no omission or duplication

#### Scenario: Cursor generation mismatch
- **WHEN** a cursor from another repository, generation, operation, or ordering contract is supplied
- **THEN** the request fails with a typed cursor error before returning records

### Requirement: Semantic dual-engine equivalence
For the supported structural operations, the current engine and Surreal engine SHALL return semantically identical stable identities, edge direction, parallel multiplicity, self-loops, kinds, provenance, source anchors, confidence, paths, ordering, truncation, and pagination for equivalent inputs and limits. Ambiguity, negative cases, and limit failures MUST be compared as outcomes and a mismatch MUST fail qualification.

#### Scenario: Semantic corpus equivalence
- **WHEN** every supported operation runs against the versioned semantic corpus on both engines
- **THEN** canonical comparison reports zero mismatches across every required C-013 dimension

#### Scenario: Deterministic scale samples
- **WHEN** the versioned deterministic samples from the medium and large profiles are evaluated
- **THEN** both engines return the same bounded identities, paths, ordering, truncation, and page boundaries

### Requirement: Pre-ratified qualification gates
Qualification SHALL evaluate the applicable C-013 thresholds without modifying the inputs or thresholds after Surreal results are visible. Results MUST record the exact source identity, graph identity, engine profile, host, commands, limits, samples, p95 method, peak RSS, output digests, and pass or fail decision. Any failed semantic, footprint, query-regression, native-value, recovery, or agent-value gate MUST invoke the recorded falsifier protocol and MUST NOT be reported as success.

#### Scenario: Gate passes
- **WHEN** a measured result satisfies its pre-ratified threshold on the same runner as the current-engine baseline
- **THEN** the retained evidence records the raw samples, ratio, threshold, and passing decision

#### Scenario: Gate fails
- **WHEN** any required result misses its threshold or cannot be measured comparably
- **THEN** C-015 stops or records a revised product decision without weakening the corpus, threshold, or semantic contract post hoc

### Requirement: Optional product boundary
The Surreal native query implementation SHALL remain behind explicit engine features. Default Compass libraries and binaries MUST retain zero Surreal dependencies, and the integration crate MUST expose no MCP, CLI, or model-authored query execution surface.

#### Scenario: Default closure remains unchanged
- **WHEN** the workspace, Compass CLI, MCP server, and current query engine are built without a Surreal engine feature
- **THEN** their dependency closures and linked artifacts contain no SurrealDB package

#### Scenario: Integration crate boundary
- **WHEN** the native query crate is inspected or compiled
- **THEN** it depends only on domain contracts and engine support, with no Compass CLI or MCP presentation dependency
