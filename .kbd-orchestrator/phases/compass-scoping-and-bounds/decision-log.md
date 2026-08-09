# Decision log — compass-scoping-and-bounds

### 2026-08-09T12:03:31Z — Scoping rejected as a feature (Assess)
Decision: do NOT build scoping. Provenance: source inspection.
SKIP_DIRS (~40 entries), gitignore-by-default, .compassignore, and --include/--exclude
all already ship and worked correctly. Corrected an earlier in-session claim that the
default ignore policy was missing node_modules/target/compass-out — that claim was wrong.

### 2026-08-09T12:03:31Z — Sharding rejected (Analyze)
Options: record-class partitioning (existing) vs distributed sharding.
Decision: partitioning; sharding rejected. Provenance: research + AGENTS.md invariants.
Not contested (>15% gap): sharding is a multi-machine technique and Compass is
local-first single-process; it would import eventual consistency, which the determinism
invariant forbids.

### 2026-08-09T12:03:31Z — Partitioning already exists
Decision: do not build it. Provenance: grep.
compass-history::PartitionedGraph implements record-class partitioning with
partition()/into_partition()/reconstruct(). Unused outside compass-history. The real
question is why the current-graph path serializes a monolithic document instead.

### 2026-08-09T12:03:31Z — Read-path allocation identified as the true constraint
Decision: stream load_active_snapshot first; gate the COMPASS_MAX_GRAPH_BYTES override
behind it. Provenance: source inspection.
compass-store/src/lib.rs:838-853 pre-allocates payload_bytes and concatenates every
chunk into one contiguous Vec, contradicting the crate's own doc comment at lines 44-51.
The write path streams (DigestWriter); the read path does not. Granting the override
before fixing this would let a user request a multi-gigabyte contiguous allocation.

### 2026-08-09T12:21:35Z — PartitionedGraph to a shared crate (user decision)
Decision: extract to `compass-partition`. Provenance: user directive.
Analysis had recommended deferring behind the streaming fix; user directed extraction.
Scope constrained by dependency reality: the container + key/canonical helpers move;
into_partition and history-coupled logic (CompletionEvidence, sidecars, AnalysisBundle,
ProgramBundle, prolly) stay. Blocking rule: the new crate must not pull prolly-map,
prolly-store-sqlite, compass-ir, or compass-analysis onto the current-graph path.

### 2026-08-09T12:21:35Z — Consumer audit changes C-001 scope (Spec)
Decision: spec C-001 as an API split, not a streaming refactor. Provenance: source audit.
read_snapshot has 8 call sites; both production callers discard the payload with `_`,
and compass-query already uses GraphSnapshotReader::open_selector. The 2 GiB allocation
is incurred for a value nothing in production reads.
