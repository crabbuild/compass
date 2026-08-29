## Context

See `proposal.md`. Both engines must run the identical persistent workload using
the pinned 3.2.4 Rust SDK. SurrealKV is a beta embedded/local-first engine;
RocksDB is the conservative on-disk comparison. Mem is excluded from acceptance.

## Goals / Non-Goals

**Goals:**

- Produce replayable, deterministic input and expected-output vectors.
- Verify identity, direction, multiplicity, provenance, confidence, generation
  isolation, activation, dirty-shutdown behavior, ordering, and pagination.
- Measure enough dependency and resource cost to expose cheap disqualifiers.
- Leave the Compass dependency graph and default binary unchanged.

**Non-Goals:**

- Create a production abstraction, schema migration, CLI, or MCP surface.
- Compare results against budgets that C-013 has not yet ratified.
- Treat successful probes as license approval or release authorization.

## Decisions

### One engine-neutral vector

The retained vector uses stable string IDs, two directed parallel relations
between the same endpoints, exact provenance fields, confidence encoded as a
finite decimal, two immutable generations, and enough generated relations to
exercise multi-page ordering. Both engines must produce the same canonical JSON.

### Application-level generation activation

Candidate records are written under a generation ID. Readers first resolve one
active-generation pointer and then scope every record query to it. Activation is
a final transaction that updates only the pointer. A killed writer may leave an
incomplete candidate generation, but it must not change the visible active
generation. This matches Compass's immutable-publication model.

### Dirty shutdown is an OS-process probe

The child process opens the persistent engine, writes a candidate in bounded
batches, emits a readiness marker after durable candidate work begins, and waits
before activation. The parent terminates the child at that marker, reopens the
same database, and verifies the previous generation exactly. This avoids treating
a normal Rust error return as crash-recovery evidence.

### Ordering and pagination are explicit

Queries order by stable record ID and use a fixed page size. Concatenated pages
must equal one canonical sorted result with no duplicate or missing record. No
storage-engine iteration order is trusted.

### Measurements are descriptive

For each feature-isolated release build, record clean build wall time, executable
bytes, resolved-package count, cold-start wall time, workload wall time, and peak
RSS using the host's native timing tool. C-012 reports values but does not ratify
budgets or make performance claims.

### Disposal is part of the gate

The temporary project and its dedicated Cargo target directories are removed
after the report is generated. Repository verification must prove that root
manifests and `Cargo.lock` contain no SurrealDB packages.

## Risks / Trade-offs

- **Persistent engine build exceeds local resources** → Record the exact failure
  as a failed probe rather than substituting Mem.
- **Crash timing is nondeterministic** → Use an explicit child readiness marker
  before termination and verify the old active generation, not the amount of
  orphaned candidate data.
- **Float rendering obscures equality** → Compare a canonical finite decimal
  representation in retained output.
- **Host measurements are not portable benchmarks** → Record host/tool/version
  metadata and treat values as a baseline, not a universal claim.
