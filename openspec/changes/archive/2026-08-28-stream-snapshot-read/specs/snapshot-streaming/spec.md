## Purpose

Provide bounded manifest-only and chunk-streaming access to current SQLite graph snapshots while preserving integrity checks and the existing full-read compatibility contract.

## ADDED Requirements

### Requirement: Manifest-only snapshot reads
The store SHALL expose a manifest-only read that validates the active snapshot manifest without reading or allocating storage for payload chunks.

#### Scenario: Valid active manifest
- **WHEN** a caller requests metadata for a valid active snapshot
- **THEN** the store returns the validated manifest without reading its payload objects

#### Scenario: Missing or invalid active manifest
- **WHEN** the active catalog entry is missing, malformed, or violates the manifest contract
- **THEN** the store returns the same typed corruption or format failure used by snapshot reads

### Requirement: Bounded chunk-streaming reads
The store SHALL expose a streaming read that delivers one bounded stored chunk at a time and SHALL validate chunk presence, total payload length, the maximum graph-byte limit, and the payload digest before reporting success. The API contract SHALL state that delivered chunks remain provisional until successful return so callers do not commit unverified bytes irreversibly.

#### Scenario: Valid multi-chunk snapshot
- **WHEN** a caller streams a valid snapshot containing more than one chunk
- **THEN** every callback receives at most one stored chunk and the returned manifest matches the published snapshot

#### Scenario: Missing or corrupt payload data
- **WHEN** a chunk is missing, the accumulated length differs from the manifest, the configured bound is exceeded, or the digest differs
- **THEN** the stream terminates with a typed non-empty error and does not report successful validation

#### Scenario: Caller commits streamed content
- **WHEN** a caller performs irreversible work using delivered chunks
- **THEN** the API documentation directs it to stage the work until the stream returns success or independently verify it before commit

#### Scenario: Consumer rejects a chunk
- **WHEN** the consumer callback returns a typed error for a delivered chunk
- **THEN** the store stops reading further chunks and returns that error

### Requirement: Full-read compatibility
The existing full-payload snapshot read SHALL retain its public signature and observable valid/corrupt behavior, while being implemented as an explicit collector over the bounded streaming path.

#### Scenario: Existing caller requests full bytes
- **WHEN** a caller invokes the full snapshot read for a valid snapshot
- **THEN** the store returns the same manifest and byte sequence as the published payload

#### Scenario: Existing caller reads corrupt data
- **WHEN** a caller invokes the full snapshot read for a corrupt snapshot
- **THEN** the store returns the same typed failure as the streaming validation path

### Requirement: Production integrity checks avoid payload-sized allocation
Snapshot validation and snapshot-reference generation SHALL preserve their existing success and failure semantics for valid and corrupt snapshots without allocating memory proportional to the manifest payload size.

#### Scenario: Validate a valid snapshot
- **WHEN** production validation is requested for a valid snapshot
- **THEN** it verifies all chunks and returns the manifest without collecting the payload

#### Scenario: Validate a corrupt snapshot
- **WHEN** production validation is requested for a snapshot with corrupt or missing payload data
- **THEN** it returns the same failure that the previous full-read validation returned

#### Scenario: Generate a reference for corrupt payload data
- **WHEN** snapshot-reference generation encounters a corrupt or missing payload chunk
- **THEN** it rejects the snapshot rather than publishing a reference to unverified content

### Requirement: Snapshot durability behavior remains atomic
The additive read APIs SHALL preserve round-trip, reopen, interruption/corruption rejection, and atomic publication behavior of the existing SQLite snapshot format.

#### Scenario: Reopen after successful publication
- **WHEN** a published store is checkpointed and reopened read-only
- **THEN** manifest-only, streaming, validation, reference, and full-read operations agree on snapshot identity and integrity

#### Scenario: Interrupted candidate publication
- **WHEN** immutable payload objects exist but the active selector was not atomically advanced
- **THEN** every active-snapshot read continues to observe only the prior complete snapshot
