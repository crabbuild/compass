# Grounded Agent Graph Overlay phased execution plan

**Status:** Implemented

**Date:** 2026-08-23

**Design:** [Grounded Agent Graph Overlay technical design](grounded-agent-graph-overlay-technical-design.md)

## Outcome

Implement a complete local-first path in which an authorized agent can submit
evidence-backed node and edge assertions, Compass deterministically verifies
them, publishes only `GROUNDED` assertions in an immutable Overlay Revision,
and serves a pinned Effective Graph through read-only query, CLI, MCP,
task-context, viewer, export, and exact historical composition.

The work is intentionally vertical. Each phase leaves a testable contract and
does not claim the next phase's behavior.

## Implementation record

Phases 0–10 are implemented in the `compass-agent-graph` domain crate and the
Core, Query, CLI, MCP, task-context, output, viewer, and history adapters named
below. Checked-in V1 fixtures are executable Rust/TypeScript contracts, and
`scripts/qualify_agent_graph_overlay.sh --fixtures-only` is the dedicated
end-to-end qualification entry point.

V1 rebase intentionally performs exact-ID and exact-digest reattachment only.
It never searches labels or selects a candidate heuristically: an absent or
changed target is explicitly unresolved and must be replaced with newly
grounded evidence or retracted. The typed ambiguous disposition remains in the
versioned contract for future deterministic match policies.

## Global constraints

- Preserve the Base Graph and all historical realizations unchanged.
- Keep CompassQL read-only.
- Never accept caller-supplied `GROUNDED` status or certificates.
- Never use fuzzy or first-candidate identity for writes or rebase.
- Treat delete as Retraction and base disagreement as Challenge.
- Use existing closed graph kinds and endpoint rules in V1.
- Require a verified source span for every agent fact projected into topology.
- Keep model/provider invocation outside `compass-agent-graph`.
- Bound all inputs, stored records, scans, diagnostics, composition, and
  transport results.
- Publish immutable objects before conditionally activating a revision.
- Preserve user changes in the current dirty worktree.
- Before every compiling Cargo command, verify `/Volumes/Workspace` is mounted
  and set:

  ```bash
  CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main
  ```

- Use Rust 1.97.1, Edition 2024, `--locked`, no unsafe code, and the workspace
  lint policy.

## Target module map

```text
compass-model
  strict Base Graph types and logical projections

compass-agent-graph
  Agent Assertion + Grounding + Overlay Revision + Challenge + Retraction
  canonical identity + storage orchestration + Effective Graph + rebase

compass-store
  bounded immutable object storage and selector CAS adapter

compass-graph
  effective snapshot acceleration and topology-derived analysis

compass-query
  read-only EffectiveGraphEngine and effective query envelopes

compass-core
  Base Generation selection, write grants, and adapter orchestration

compass-cli / compass-mcp
  public write/read adapters

compass-output / compass-viewer
  visually distinct agent facts, Grounding, Challenges, and omissions

compass-history
  immutable historical Base Generation reader only
```

## Phase dependency graph

```text
0 Contracts
   |
1 Domain + Grounding
   |
2 Revision repository
   |
3 CRUD lifecycle
   |
4 Effective Graph + query
   |
   +----------+-----------+
   |          |           |
5 CLI      6 MCP       7 Rebase
   |          |           |
   +----------+-----------+
              |
8 Context + output + viewer
              |
9 Exact historical composition
              |
10 Hardening + qualification + release
```

Phases 5, 6, and the planning half of Phase 7 can proceed independently after
Phase 4. All other transitions require the previous phase's exit criteria.

## Phase 0 — Freeze terminology and machine contracts

### Purpose

Prevent storage, CLI, and MCP implementations from inventing incompatible
meanings for Agent Assertion, Grounding, `GROUNDED`, identity, or Retraction.

### Files

- Existing: `CONTEXT.md`
- Existing: `docs/implementation/grounded-agent-graph-overlay-technical-design.md`
- Existing: `docs/implementation/grounded-agent-graph-overlay-phased-execution-plan.md`
- Modify: `docs/README.md`
- Create: `fixtures/contracts/agent-graph/`
- Create: `fixtures/contracts/agent-graph/batch-v1.json`
- Create: `fixtures/contracts/agent-graph/receipt-v1.json`
- Create: `fixtures/contracts/agent-graph/overlay-v1.json`
- Create: `fixtures/contracts/agent-graph/effective-v1.json`
- Create: `fixtures/contracts/agent-graph/rebase-plan-v1.json`
- Create: `fixtures/contracts/agent-graph/errors-v1.json`

### Steps

1. Convert the design's Rust sketches into canonical JSON examples.
2. Include create, replace, Retraction, Challenge flag, Challenge mask,
   within-batch node reference, and mixed-operation batches.
3. Include rejected examples for caller-supplied `GROUNDED`, fuzzy targets,
   unknown majors, unknown kinds, duplicate targets, absent source spans, stale
   digests, and dangling edges.
4. Freeze stable error codes and exact fields for revision conflicts,
   Grounding failures, limits, ambiguity, and transport truncation.
5. Record the Base Generation and Effective Graph identity formulas in the
   fixture README.
6. Decide all default and hard limits in one checked manifest.
7. Add the design and plan to the internal documentation map without describing
   the feature as shipped.

### Tests

At this phase, fixtures are reviewed data rather than executable claims. Add a
small schema-lint script only if it can validate version fields, duplicate
fixture IDs, and deterministic JSON formatting without duplicating the future
Rust validator.

### Exit criteria

- Every public contract has a version tag and positive/negative example.
- `GROUNDED` appears only in expected output fixtures.
- No fixture mutates or embeds a complete Base Graph artifact.
- The design has no unresolved decision that would change identity, storage, or
  authorization semantics.

### Suggested commits

1. `docs(agent-graph): define grounded overlay language and design`
2. `test(agent-graph): freeze v1 contract fixtures`

## Phase 1 — Build the domain model and deterministic Grounding

### Purpose

Create the module's typed interface and prove that only Compass can issue a
valid `GROUNDED` certificate.

### Files

- Modify: root `Cargo.toml`
- Modify: `Cargo.lock` only if dependency resolution changes
- Create: `crates/compass-agent-graph/Cargo.toml`
- Create: `crates/compass-agent-graph/src/lib.rs`
- Create: `crates/compass-agent-graph/src/contract.rs`
- Create: `crates/compass-agent-graph/src/assertion.rs`
- Create: `crates/compass-agent-graph/src/grounding.rs`
- Create: `crates/compass-agent-graph/src/canonical.rs`
- Create: `crates/compass-agent-graph/src/limits.rs`
- Create: `crates/compass-agent-graph/tests/contract.rs`
- Create: `crates/compass-agent-graph/tests/grounding.rs`
- Create: `crates/compass-agent-graph/tests/canonical.rs`

### Interface slice

Implement strict parsing and serialization for:

- `BaseGenerationId`;
- `OverlayId`, `OverlayRevisionId`, `AssertionId`, and `ChallengeId`;
- `ChangeBatch` and `ChangeOperation`;
- `AssertionDraft`, `AgentNodeDraft`, and `AgentEdgeDraft`;
- exact base/agent/within-batch references;
- `GroundingSubmission` and closed V1 evidence types;
- `GroundingCertificate` with module-private construction;
- `CommitReceipt` and `AgentGraphError` envelopes;
- all default and ceiling limits.

### Steps

1. Scaffold the crate with `#![forbid(unsafe_code)]` and workspace dependencies.
2. Implement opaque ID types with strict length, encoding, and domain-prefix
   validation.
3. Implement bounded deserialization before semantic validation.
4. Implement canonical JSON/record encoding and digest domains.
5. Implement static evidence-verifier dispatch for source span, Base Fact, Base
   Path, Prior Assertion, and snapshot artifact evidence.
6. Implement a default Grounding policy requiring at least one verified source
   span for topology facts.
7. Make certificate constructors private and expose only immutable views.
8. Map verified citations into conservative existing `Provenance` for strict
   graph projection without creating fake anchors.
9. Produce ordered, field-addressed Grounding diagnostics.

### Required tests

- JSON fixture round trips match Phase 0.
- Unknown major/minor behavior follows the contract.
- Oversized IDs, strings, arrays, evidence, and JSON nesting fail before
  publication.
- A caller cannot deserialize or construct a trusted certificate through a
  draft type.
- File and excerpt digests are recomputed, not trusted.
- A source span outside the repository, file, or inventoried range fails.
- A Base Fact citation validates generation, ID, kind, and record digest.
- Base Path verification preserves direction and exact edge sequence.
- Prior Assertion citations require the exact revision and assertion digest.
- Evidence order permutations produce identical certificates.
- Claim changes alter the claim digest and invalidate an old certificate.
- Errors and candidate lists have deterministic order.

### Verification

```bash
test -d /Volumes/Workspace
test -w /Volumes/Workspace/crabbuild-target
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-agent-graph --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo clippy -p compass-agent-graph --all-targets --all-features --locked -- -D warnings
```

### Exit criteria

- The crate can verify drafts against an in-memory exact Base Generation.
- Only successful verification yields `GROUNDED`.
- No filesystem, store, CLI, MCP, query, or model dependency is required.
- Canonical contract fixtures pass byte-for-byte.

### Suggested commits

1. `feat(agent-graph): add strict overlay contract types`
2. `feat(agent-graph): add deterministic grounding verification`
3. `test(agent-graph): prove canonical grounded certificates`

## Phase 2 — Publish immutable Overlay Revisions safely

### Purpose

Add durable append-only storage, idempotency, CAS activation, reopen validation,
and bounded GC without exposing partial active state.

### Files

- Modify: `crates/compass-agent-graph/Cargo.toml`
- Create: `crates/compass-agent-graph/src/overlay.rs`
- Create: `crates/compass-agent-graph/src/repository.rs`
- Create: `crates/compass-agent-graph/src/policy.rs`
- Create: `crates/compass-agent-graph/tests/publication.rs`
- Create: `crates/compass-agent-graph/tests/repository_conformance.rs`
- Modify: `crates/compass-store/src/lib.rs` only if a missing primitive is
  proven by the repository conformance test
- Modify: nearest `compass-store` tests if its interface changes

### Storage protocol

1. Materialize ordered state roots for assertions, Challenges, Retractions,
   certificates, references, and revision metadata.
2. Publish all content-addressed immutable objects and the idempotency receipt.
3. Validate the complete revision manifest.
4. Update the active head with `WriteCondition::Missing` for creation or
   `WriteCondition::Version(observed)` for replacement.
5. Reopen the head and all roots before acknowledging success.

Unreachable immutable objects after a CAS loss are safe and GC-eligible. Never
acknowledge before the active selector points to readable content.

### Steps

1. Implement repository path selection for Git common-dir and non-Git state.
2. Enforce owner-only permissions where supported and reject symlink targets.
3. Define the dedicated `compass.agent-graph.v1` namespace and ordered keys.
4. Implement materialized revision roots; do not require unbounded event replay.
5. Implement idempotency mapping from `(principal, overlay, key)` to batch digest
   and committed receipt.
6. Implement prepare/activate publication using existing immutable writes and
   version-token CAS.
7. Implement reopen/root/count/digest/reference validation.
8. Implement active-head and explicit-pin reachability.
9. Implement dry-run GC followed by explicit bounded sweep.
10. Add a store interface only if the conformance suite proves the current
    prepare/activate pattern cannot satisfy a named invariant.

### Required tests

- Create, reopen, replace, and list revisions.
- Same idempotency key and batch returns the original receipt.
- Same idempotency key with different content fails.
- Two concurrent writers yield one success and one `revision_conflict`.
- Crash/fault before head activation leaves the previous head readable.
- Crash/fault after immutable publication leaves only unreachable objects.
- A corrupt manifest, root, certificate, count, or selector fails closed.
- Input ordering does not change revision identity.
- Unknown canonical/store versions fail explicitly.
- Scans and GC respect item/byte limits and stable order.
- Active and pinned revisions survive GC; unreachable objects are planned.
- Non-Git and linked-worktree path behavior is deterministic and confined.

### Verification

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-agent-graph --test publication --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-agent-graph --test repository_conformance --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-store --locked
```

### Exit criteria

- A committed Overlay Revision survives reopen and validates fully.
- Lost updates are impossible under the tested adapters.
- Interrupted writes never expose a partial active revision.
- No Base Graph snapshot selector or history realization is modified.

### Suggested commits

1. `feat(agent-graph): store immutable overlay revisions`
2. `feat(agent-graph): activate revisions with compare-and-swap`
3. `feat(agent-graph): add overlay reachability and gc`

## Phase 3 — Complete CRUD, Challenge, Retraction, and ownership

### Purpose

Implement the full write lifecycle through one atomic batch.

### Files

- Modify: `crates/compass-agent-graph/src/assertion.rs`
- Modify: `crates/compass-agent-graph/src/overlay.rs`
- Modify: `crates/compass-agent-graph/src/policy.rs`
- Create: `crates/compass-agent-graph/tests/lifecycle.rs`
- Create: `crates/compass-agent-graph/tests/ownership.rs`

### Steps

1. Derive stable assertion IDs from repository, overlay, owner, fact class, and
   logical key.
2. Implement create and replace with expected assertion digest.
3. Implement Retraction tombstones and preserved prior versions.
4. Implement Challenge flag and separately authorized mask effects.
5. Resolve `CreatedInThisBatch` references through a bounded dependency DAG.
6. Reject cycles, duplicate targets, contradictory operations, and type-class
   changes.
7. Validate the complete simultaneous post-state rather than operation order.
8. Reject node Retraction while active incident agent edges remain unless the
   same batch retracts/replaces them.
9. Preserve edge direction, occurrence identity, and parallel multiplicity.
10. Mint a `WriteGrant` from trusted adapter context; request JSON cannot set
    owner or permissions.

### Required tests

- Create node and edge in one batch through an exact within-batch reference.
- Replace retains Assertion ID and changes version/digest.
- Replace using a stale assertion digest fails.
- Retraction removes active contribution and retains audit/history.
- Reusing a retracted logical key follows the chosen contract and never erases
  the tombstone.
- One principal cannot replace or retract another principal's assertion.
- Flag and mask require valid Grounding; mask requires stronger permission.
- Base node/edge update or delete operations cannot be expressed.
- Node Retraction with active edges fails with bounded dependent IDs.
- Multiple independent operation orderings yield one canonical post-state.

### Exit criteria

- Every logical CRUD use case is expressed through `apply`.
- Base Graph facts remain immutable and retrievable.
- All active agent facts are GROUNDED and ownership-valid.

### Suggested commits

1. `feat(agent-graph): implement grounded assertion lifecycle`
2. `feat(agent-graph): add challenges masks and retractions`
3. `test(agent-graph): cover ownership and batch invariants`

## Phase 4 — Compose and query the Effective Graph

### Purpose

Make grounded topology useful through a deterministic read view while keeping
existing Base Graph queries unchanged.

### Files

- Create: `crates/compass-agent-graph/src/compose.rs`
- Create: `crates/compass-agent-graph/tests/composition.rs`
- Modify: `crates/compass-query/src/graph_engine.rs`
- Modify: `crates/compass-query/src/lib.rs`
- Create: `crates/compass-query/tests/effective_graph.rs`
- Modify: `crates/compass-graph/src/snapshot.rs` only for proven effective-delta
  acceleration needs
- Add/modify relevant query contract fixtures

### Composition order

1. Validate and pin the exact Base Generation and Overlay Revision.
2. Select augment or curated composition.
3. Stream Base Graph records in canonical order.
4. Report Challenges; apply masks only in curated mode.
5. Remove incident edges of masked base nodes and count every cascade.
6. Add active grounded agent nodes.
7. Add active grounded agent edges.
8. Attach ordered agent-fact metadata outside the strict graph projection.
9. Validate endpoint integrity, kinds, multiplicity, and all limits.
10. Derive Effective Graph identity.

### Steps

1. Implement `ReadRequest::{Overlay, EffectiveGraph, History, Diff,
   PrepareRebase}` and bounded results.
2. Implement the augment profile first.
3. Add curated masks after augment parity tests pass.
4. Implement `compass.agent-graph.effective/1` and composition omissions.
5. Add `EffectiveGraphEngine` as a read-only query adapter.
6. Key query caches by Effective Graph identity.
7. Initially use validated in-memory composition.
8. Add a separate effective snapshot namespace and graph-delta acceleration only
   after semantic differential tests prove equivalence.
9. Ensure existing `JsonGraphEngine` and store selection remain unchanged when
   no overlay is selected.

### Required tests

- Agent nodes and edges are visible in exact search/traversal/CompassQL through
  the effective adapter.
- Existing Base Graph queries are byte/semantic unchanged without selection.
- Augment retains challenged facts; curated applies masks.
- Masking a node removes incident edges and reports exact omissions.
- Parallel agent edges and reciprocal edges remain distinct.
- Missing endpoints and invalid kind pairs fail before publication.
- Effective identity changes for base, revision, profile, or composition-version
  changes.
- Same semantic inputs produce identical Effective Graph bytes.
- Direct and accelerated effective engines return equivalent results and
  truncation.
- Query limit errors remain distinct from no-match.

### Verification

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-agent-graph --test composition --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-query --test effective_graph --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-cypher --test tck --locked
python3 scripts/check_compassql_support.py
```

### Exit criteria

- One exact `(Base Generation, Overlay Revision, profile)` is queryable.
- Existing queries remain Base Graph-only and compatible by default.
- CompassQL remains read-only and mutation TCK cases still reject.

### Suggested commits

1. `feat(agent-graph): compose validated effective graphs`
2. `feat(query): add read-only effective graph engine`
3. `test(agent-graph): prove base and effective query isolation`

## Phase 5 — Add the CLI adapter and local write policy

### Purpose

Expose complete local workflows with strict machine output before enabling
remote agent writes.

### Files

- Create: `crates/compass-cli/src/agent_graph_commands.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Create: `crates/compass-cli/tests/agent_graph_cli.rs`
- Modify: `docs/reference/commands.md`
- Modify: `docs/reference/configuration.md`
- Modify: `docs/reference/outputs.md`

### Steps

1. Add `status`, `apply`, `show`, `history`, `diff`, `rebase-plan`,
   `rebase-commit`, `query`, and `export` subcommands.
2. Make `apply --request FILE --format json` the canonical mutation path.
3. Resolve project, Base Generation, overlay state path, and local principal in
   `compass-core`; keep parsing/presentation in CLI.
4. Require explicit local write enablement and reject ambiguous scope.
5. Use stdout for results, stderr for progress/diagnostics, exit `2` for usage,
   and exit `1` for runtime/conflict/Grounding failures.
6. Write JSON outputs atomically when `--output` is used.
7. Never print source excerpts, credentials, or full draft payloads in errors.

### Required tests

- Root and subcommand help are complete.
- Unknown/repeated options and incompatible selectors fail.
- JSON output emits exactly one versioned value.
- Write-disabled, Grounding failure, conflict, idempotent retry, and success have
  stable exits and streams.
- Project/output paths are root-confined across macOS/Linux/Windows semantics.
- Interrupted output does not change the active overlay revision.
- Base `graph.json`, `store.ref`, and current snapshot remain byte-identical
  after overlay writes.

### Verification

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-cli --test agent_graph_cli --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-cli --test compass_product --locked
sh scripts/check_product_boundary.sh
```

### Exit criteria

- A local user can complete the full CRUD/query workflow without MCP.
- Machine contracts and failure paths are stable and documented.
- No existing default command consumes the overlay.

### Suggested commits

1. `feat(cli): add agent graph inspection and apply commands`
2. `feat(cli): query and export effective graph revisions`
3. `docs(agent-graph): document local command contracts`

## Phase 6 — Add deny-by-default MCP write and read adapters

### Purpose

Expose agent enhancement safely to chat/session clients.

### Files

- Modify: `crates/compass-mcp/src/lib.rs`
- Modify: `crates/compass-mcp/src/transport.rs`
- Create or modify focused MCP authorization module
- Create: `crates/compass-mcp/tests/agent_graph_tools.rs`
- Create: `crates/compass-mcp/tests/agent_graph_http_auth.rs`
- Modify: `docs/guides/integrating-compass.md`
- Modify: `docs/design/security-and-privacy.md`
- Modify: `SECURITY.md`

### Tools

- `inspect_agent_graph`: read-only overlay/effective/history/diff/rebase-plan.
- `apply_agent_graph`: exact versioned batch application.

Do not add one independent tool per CRUD verb. Tool schemas translate to the
same closed request enums used by Rust and CLI.

### Steps

1. Add explicit server configuration for read selection and write enablement.
2. Omit `apply_agent_graph` from `tools/list` when writes are disabled.
3. Add a scoped authorizer that mints `WriteGrant` from trusted transport
   context, never request fields.
4. Require authenticated write credentials for HTTP even on loopback.
5. Separate read authentication from write capabilities and mask permission.
6. Bind grants to canonical project allowlist, overlay, Base Generation,
   expected revision, operation set, expiry, and limits.
7. Require idempotency keys and reject replay with different content.
8. Invalidate/reload MCP `GraphContext` using Effective Graph identity; do not
   key only by `graph.json` mtime and size.
9. Keep domain truncation separate from MCP transport truncation.
10. Emit bounded audit records without credentials or source excerpts.

### Required tests

- Tools list differs correctly under write-disabled/enabled configuration.
- A request cannot self-assign principal, project, mask permission, or limits.
- HTTP write enablement without authenticated capability fails at startup.
- Wrong project/overlay/base/revision scopes return stable authorization or
  conflict errors.
- stdio remains local and requires explicit enablement.
- Concurrent sessions produce CAS conflicts rather than lost updates.
- Shared multi-project cache does not cross scopes or return stale revisions.
- Request/response limits, cancellation, malformed JSON, and transport
  truncation are covered.
- Existing read tools remain unchanged when no overlay selector is used.

### Verification

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-mcp --test agent_graph_tools --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-mcp --test agent_graph_http_auth --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-mcp --locked
```

### Exit criteria

- A chat agent can inspect, apply, receive a receipt, and query the exact
  Effective Graph.
- Remote writes cannot operate with only the existing optional read API key.
- Write capability is absent unless explicitly enabled.

### Suggested commits

1. `feat(mcp): expose grounded agent graph reads`
2. `feat(mcp): add scoped agent graph write capability`
3. `test(mcp): harden agent graph transport authorization`

## Phase 7 — Implement exact rebase and stale-evidence recovery

### Purpose

Keep Agent Assertions useful after a Base Graph rebuild without silently
inventing new attachment meaning.

### Files

- Create: `crates/compass-agent-graph/src/rebase.rs`
- Create: `crates/compass-agent-graph/tests/rebase.rs`
- Modify: core/CLI/MCP adapters for plan and commit requests
- Add rebase fixtures for rename, deletion, changed digest, ambiguity, and
  unchanged identity

### Steps

1. Require exact source Overlay Revision and target Base Generation.
2. Retain a reference only when exact ID and canonical record digest remain
   unchanged.
3. Identify changed Grounding dependencies and require verification again.
4. Return zero-candidate references as unresolved.
5. Return multiple candidates as ambiguous with bounded candidates and exact
   omitted count.
6. Permit exact registered reattachment rules only when they produce one target
   and a retained proof.
7. Require explicit Grounded mappings or Retractions for remaining references.
8. Bind the plan digest to source revision, target generation, rules, mappings,
   candidates, and limits.
9. Reject commit when the source head or target generation changed.
10. Publish rebase as an ordinary immutable Overlay Revision.

### Required tests

- Unchanged exact facts reattach deterministically.
- Same label with changed identity does not reattach.
- Rename with an exact existing identity proof reattaches.
- Zero candidates remain unresolved.
- Multiple candidates remain ambiguous regardless of input order.
- Changed source bytes require new Grounding.
- Stale plan/head/target changes fail before publication.
- Explicit Retraction resolves an otherwise blocked rebase.
- Old revision remains readable and unchanged.
- Rebase output is deterministic and bounded.

### Exit criteria

- Base mismatch never silently composes.
- Every activated rebase has complete exact mappings or explicit Retractions.
- No first-candidate behavior exists in any adapter.

### Suggested commits

1. `feat(agent-graph): plan exact overlay rebases`
2. `feat(agent-graph): commit regrounded overlay rebases`
3. `test(agent-graph): preserve ambiguity and stale evidence`

## Phase 8 — Integrate task context, output, orientation, and viewer

### Purpose

Make Agent Assertions understandable and visually distinct across agent-facing
and human-facing read surfaces.

### Files

- Modify: `crates/compass-core/src/task_context.rs`
- Modify: `crates/compass-core/src/lib.rs` to replace duplicated effective
  context loading
- Modify: `crates/compass-mcp/src/lib.rs` GraphContext use
- Modify: `crates/compass-output` effective rendering modules
- Modify: `packages/compass-viewer/src/contracts/`
- Modify: `packages/compass-viewer/src/graph/`
- Regenerate: `crates/compass-output/assets/viewer/graph.js`
- Regenerate: `crates/compass-output/assets/viewer/viewer.css` if changed
- Regenerate: `crates/compass-output/assets/viewer/manifest.json`
- Add focused Rust and TypeScript tests

### Steps

1. Deepen one Effective Graph context module used by core and MCP; remove
   duplicated base/overlay/identity loading rules.
2. Add a bounded `agentKnowledge` task-context section with exact assertion,
   revision, certificate, citation, Challenge, and omission metadata.
3. Keep Base Graph provenance and agent Grounding visually and structurally
   separate.
4. Render agent nodes/edges with a distinct accessible appearance and
   `GROUNDED` badge.
5. Expose the exact pinned augment/curated profile, Challenge inspection, and
   bounded Retraction history. Profile changes require a new Effective Graph
   read because the profile is part of identity; the viewer must not implement
   a client-side toggle that relabels one identity as another.
6. Recompute topology-derived analysis for the exact Effective Graph identity,
   or mark base-only analysis explicitly unavailable for changed topology.
7. Never show stale base communities as effective communities.
8. Escape all agent-authored text and retain output size limits.
9. Rebuild generated viewer assets only from source.

### Required tests

- Task context exact-target resolution includes only relevant bounded Agent
  Assertions.
- Grounding and structural confidence remain separate fields.
- Agent text cannot inject HTML/script or corrupt JSON embedding.
- Viewer distinguishes base, agent, challenged, masked, and retracted states.
- Direction and parallel edges remain correct visually and in contracts.
- Base-only analysis is never presented as effective after topology changes.
- Generated assets match source and manifest.

### Verification

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-core --test task_context --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-output --locked
npm run typecheck:js
npm run test:js
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
```

### Exit criteria

- Agent-aware context and viewer surfaces preserve exact identities and
  Grounding.
- Every topology-derived artifact is bound to the Effective Graph identity or
  explicitly unavailable.
- Generated assets are coherent and verified.

### Suggested commits

1. `feat(context): include grounded agent knowledge`
2. `feat(output): render effective graph provenance`
3. `feat(viewer): distinguish grounded overlay facts`

## Phase 9 — Compose overlays with exact historical Base Generations

### Purpose

Support immutable overlay review against exact Git/history realizations without
changing Compass history content or preference.

### Files

- Modify: `crates/compass-core/src/history.rs`
- Modify: `crates/compass-agent-graph` Base Generation reader adapters
- Modify: `crates/compass-cli/src/history_commands.rs` only for explicit overlay
  selectors
- Modify: relevant history export/diff orchestration
- Add: `crates/compass-history` integration tests without changing stored
  realization schemas unless proven necessary

### Steps

1. Implement a Base Generation reader adapter over an exact validated history
   realization.
2. Require both Base realization and Overlay Revision selectors.
3. Compose without updating preferred realization or history roots.
4. Export a self-describing effective bundle with both identities and
   composition profile.
5. Permit overlay diffs only under compatible Base Generation/composition rules;
   require explicit rebase otherwise.
6. Add overlay pins for long-lived review artifacts and include them in overlay
   GC reachability.
7. Keep historical materialization offline and isolated.

### Required tests

- Exact historical Base Graph bytes remain unchanged after overlay writes.
- Reopen and compose the same pair to identical Effective Graph identity.
- Wrong Base realization or fingerprint fails explicitly.
- Current-tree overlays do not silently attach to historical bases.
- Export round-trips Base Generation, Overlay Revision, Grounding, direction,
  multiplicity, and Challenges.
- Overlay pins survive GC; unpinned unreachable revisions appear in dry-run.
- Historical checkout policy executes no hooks, fetches, filters, or providers.

### Verification

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-history --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-core --locked history
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-cli --test history_cli --locked
```

### Exit criteria

- Exact historical composition is reproducible by two explicit selectors.
- History realizations and preferred pointers remain untouched.
- Overlay GC and export preserve auditability.

### Suggested commits

1. `feat(agent-graph): read exact historical base generations`
2. `feat(history): compose explicit grounded overlays`
3. `test(history): prove base realization immutability`

## Phase 10 — Harden, qualify, document, and release

### Purpose

Move from opt-in experimental implementation to a supported Compass contract.

### Documentation files

- Modify: `docs/concepts/graph-model.md`
- Modify: `docs/concepts/provenance.md`
- Create: `docs/concepts/agent-graph-overlays.md`
- Create: `docs/guides/enhancing-a-graph-with-an-agent.md`
- Modify: `docs/guides/task-context.md`
- Modify: `docs/guides/integrating-compass.md`
- Modify: `docs/reference/commands.md`
- Modify: `docs/reference/configuration.md`
- Modify: `docs/reference/outputs.md`
- Modify: `docs/design/security-and-privacy.md`
- Modify: `docs/design/storage-and-history.md`
- Modify: `COMPATIBILITY.md`
- Modify: `CHANGELOG.md`
- Modify: `MIGRATION.md` only if users must act
- Modify: assistant skill assets and exact-tree tests

### Hardening work

1. Fuzz/bound deserializers, canonical encoding, evidence verification, rebase,
   composition, and path handling.
2. Add fault injection around every publication and selector transition.
3. Add concurrent process tests for CLI/MCP writers and readers.
4. Add corruption tests for roots, manifests, certificates, idempotency, audit,
   and effective cache selectors.
5. Measure current and maximum overlay sizes, cold/warm composition, write
   latency, query latency, store growth, and GC.
6. Confirm no credentials, prompts, chain-of-thought, or source excerpts enter
   logs, audit, artifacts, or errors.
7. Verify overlay-disabled structural build/query performance and output remain
   within existing baselines.
8. Run platform tests for path encoding, locking, permissions, atomic replace,
   SQLite WAL, and interrupted process behavior.
9. Run security review for HTTP capability scope, project allowlists, replay,
   confused deputy, and prompt-influenced writes.
10. Keep graph-quality qualification source-derived; add a separate overlay
    conformance suite rather than mixing Agent Assertions into extractor recall.

### End-to-end acceptance scenarios

1. **Chat initialization:** start read-only MCP, inspect Base Generation, enable
   scoped agent writes explicitly, submit one node and edge with source spans,
   receive `GROUNDED`, then query the exact augment Effective Graph.
2. **Update:** replace the edge using expected revision and assertion digest;
   observe a new Overlay Revision and unchanged Assertion ID.
3. **Retraction:** retract the edge; verify old revisions retain it and the new
   effective view does not.
4. **Challenge:** flag a base edge; augment reports it while retaining the edge.
5. **Curated mask:** with stronger capability, mask the edge; curated omits it
   and reports the omission while Base Graph inspection still returns it.
6. **Concurrency:** two sessions write from one expected revision; one commits,
   the other receives the observed head and no partial changes.
7. **Rebuild:** create a new Base Generation; effective read requires rebase,
   changed evidence is grounded again, and ambiguity remains explicit.
8. **History:** bind an overlay to an exact historical realization, export, reopen,
   and reproduce the Effective Graph identity without changing history.
9. **Security:** remote write without scoped authenticated capability is not
   advertised and cannot be invoked.
10. **Limits:** oversized evidence fails as `limit_exceeded`, never empty or
    partially committed.

### Final verification baseline

```bash
test -d /Volumes/Workspace
test -w /Volumes/Workspace/crabbuild-target

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo fmt --all -- --check

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo clippy --workspace --lib --bins --locked -- -D warnings

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test --workspace --lib --bins --locked

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-cli --test compass_product --locked

sh scripts/check_product_boundary.sh
./scripts/qualify_code_graph_v1.sh --fixtures-only
python3 scripts/check_compassql_support.py
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
```

Add dedicated qualification commands before release:

```text
scripts/qualify_agent_graph_overlay.sh --fixtures-only
scripts/check_agent_graph_contracts.py
```

The overlay qualification must cover Grounding precision, false acceptance,
identity, CRUD, conflicts, rebase ambiguity, effective query parity, masks,
limits, corruption, deterministic repetition, and unauthorized writes.

### Release gate

- Every end-to-end scenario passes on supported platforms.
- Contract fingerprints and fixtures match all Rust/TypeScript consumers.
- No open P0/P1 correctness, integrity, or authorization defect remains.
- Overlay-disabled product behavior is unchanged.
- Documentation clearly separates Base Graph truth, GROUNDED assertions, and
  structural confidence.
- `COMPATIBILITY.md` states additive contracts and exact opt-in behavior.
- `CHANGELOG.md` describes the new feature without calling GROUNDED assertions
  extracted or structurally inferred.

### Suggested commits

1. `test(agent-graph): add end-to-end overlay qualification`
2. `security(agent-graph): harden scoped writes and audit`
3. `docs(agent-graph): publish concepts guide and references`
4. `chore(agent-graph): enable supported contract gates`

## Completion checklist

- [x] Base Graph and history remain immutable.
- [x] `GROUNDED` is issued only by deterministic verification.
- [x] Authorship, Grounding, and structural confidence remain separate.
- [x] Agent CRUD is append-only and ownership-checked.
- [x] Base fact changes use Challenge/mask, never destructive mutation.
- [x] All batches bind Base Generation, expected revision, and idempotency key.
- [x] Every operation is versioned, bounded, and explicit about exact identity
      and truncation where applicable.
- [x] Rebase preserves unresolved and ambiguous outcomes explicitly.
- [x] Query, task context, output, viewer, and history use one Effective Graph
      identity.
- [x] CLI and MCP are thin adapters over one interface.
- [x] Remote writes are explicit, authenticated, scoped, bounded, and audited.
- [x] Prompts, chain-of-thought, credentials, and source excerpts are excluded.
- [x] Targeted, baseline, product, query, viewer, security, and Agent Graph
      qualification gates pass. The independent Code Graph fixture gate could
      not run locally because its offline parser-source bundle was unavailable;
      this does not affect the dedicated overlay qualification.
- [x] Documentation and compatibility records match implemented behavior.

## Related documents

- [Technical design](grounded-agent-graph-overlay-technical-design.md)
- [Workspace tour](workspace-tour.md)
- [Extending Compass](extending-compass.md)
- [Storage and history design](../design/storage-and-history.md)
- [Security and privacy](../design/security-and-privacy.md)

**Next step:** keep the checked-in qualification script in the release matrix
and extend V1 only through new versioned contracts and deterministic policies.
