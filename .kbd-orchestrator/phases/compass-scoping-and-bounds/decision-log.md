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

### 2026-08-28T12:45:00Z — Execution handed off to Codex / gpt-5.6-sol (user decision)
Decision: planning stages (assess, plan) completed in Claude Code; execution delegated
to gpt-5.6-sol via Codex CLI 0.146.0. Provenance: user directive.
Handoff document: `handoffs/EXECUTION-HANDOFF.md` — self-contained, assumes no chat
history. Backend remains OpenSpec (skills in `.agents/skills/`, Codex uses skills not
slash commands). Compass MCP wired via `.codex/config.toml`.

### 2026-08-28T12:45:00Z — C-001 verify-first check PASSED (closes adversarial r2 CRITICAL)
Decision: the uncommitted `compass-store/src/lib.rs` diff is a valid starting draft.
Provenance: direct verification at handoff.
Evidence: `git diff --stat` → +152/−17; `read_snapshot_manifest` at lib.rs:837,
`read_snapshot_chunks` at lib.rs:856, `read_snapshot` retained at lib.rs:901;
`cargo check -p compass-store --locked` clean in 27.95s.
This closes the round-2 CRITICAL finding on `assessment.md`, which was unverifiable
only because artifact-mode review packets carry no git state. Remaining C-001 work is
unchanged: reimplement validate_snapshot/snapshot_reference on the manifest reader,
add round-trip/reopen/corruption/atomicity tests, update CHANGELOG.

### 2026-08-28T13:05:00Z — Preserve canonical/OpenSpec identifiers without ledger inflation
Decision: use `C-001` for typed KBD change/task transitions and
`stream-snapshot-read` for the OpenSpec backend, pairing the transitions with the
same task hooks and `kbd-apply mark-done` operation. Provenance: execution-time
wrapper audit. The installed `kbd-apply` passes its backend change argument through
as the canonical change ID and has no ID-mapping option; invoking
`begin-task stream-snapshot-read` would create a spurious twenty-first canonical
change. The mapping is kept explicit until the wrapper grows first-class aliases.

### 2026-08-29 — Latest MCP revision governs C-010 (user decision)
Decision: retain the latest published MCP revision as Compass's normative HTTP
contract and do not restore an obsolete lifecycle solely for a lagging named
client. Provenance: the user's active Codex task message, “C-010: Use the latest
version of MCP.” Context7's official `modelcontextprotocol/modelcontextprotocol`
documentation identifies 2026-07-28 as the latest published revision and confirms
that revision removed sessions, resumable SSE, and GET/DELETE on the MCP endpoint.
The exact OpenCode 1.18.25 HTTP outcome remains recorded as incompatible; both
independent current-protocol conformance legs remain mandatory merge gates.

### 2026-08-29 — C-011 SurrealDB 3.2.4 artifact profiles accepted (user decision)
Decision: `ACCEPT`, which approves every artifact profile named in
`docs/future/surrealdb-license-decision.md` under its stated release conditions.
Provenance: the user's active Codex task message, “C-011: ACCEPT.” Decision
authority: project user; authority role: user. This applies only to the pinned
SurrealDB 3.2.4 license digest. Covered artifacts and redistributions must retain
the exact BSL license, notices, version, Change Date, Change License, and Database
Service restriction; every version upgrade requires a fresh license/profile
review. C-011 passed mandatory pre-archive review and is published in the dated
OpenSpec archive; C-012 and C-013 are already certified complete. The
C-014/C-015 entry gate is therefore satisfied and those changes are unblocked.
This decision does not satisfy
C-020's separate independent-user-problem condition.

### 2026-08-29 — C-020 Surreal Store adapter cancelled by condition precedent
Decision: drop C-020 and close the phase at nineteen implemented changes.
Provenance: the pre-ratified Wave 8 condition in `plan.md` and
`EXECUTION-HANDOFF.md`. By the end of Wave 7, no independently stated user
problem had been recorded that required replacing the unchanged synchronous
`compass-store` contract with a Surreal-backed adapter. The accepted C-011
license decision and the existence of the C-014 projection explicitly do not
satisfy that product-need condition. No `compass-store-surreal` crate, executor
boundary, or speculative dependency is added.

### 2026-08-29 — C-015 native Surreal adoption rejected by pre-ratified falsifier
Decision: retain the bounded, feature-isolated projection/query implementation
as experimental evidence, but reject routing Compass workloads to it and do not
claim C-015 budget qualification success. Provenance: the exact pre-ratified
`qualification-medium` run recorded in
`benchmarks/qualification/surreal-dual-engine-decision-v1.json`.
The implementation produced zero semantic mismatches, but warm in-process
Surreal p95 was 8,132x–38,904x the current engine across the measured structural
workloads and canonical response-byte reduction was 0%. This fails the
pre-ratified <=1.10x query-regression gate and both native-value conjuncts.
Thresholds were not changed after results were visible. An initial single-
transaction activation also reached 87.6 GB peak physical footprint; bounded
idempotent 512-record staging corrected that defect, and the exact rerun passed
with 3,646,308,352 bytes maximum RSS, 100,000 nodes, and 250,000 edges. The
falsifier stops further adoption-oriented artifact/recovery measurement because
those measurements cannot reverse the decisive query/native-value failures.
