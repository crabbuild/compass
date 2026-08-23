# Grounded Agent Graph Overlay technical design

**Status:** Implemented

**Date:** 2026-08-23

**Scope:** Local agent-authored graph enhancement, deterministic Grounding,
versioned CRUD, Effective Graph reads, CLI/MCP adapters, and optional exact-base
history composition

## Summary

Compass includes a deep `compass-agent-graph` module that owns a versioned Agent
Graph Overlay over an immutable Base Graph. AI agents may create and replace
agent-owned nodes and edges, retract their prior assertions, and challenge Base
Graph facts. They never edit `graph.json`, rewrite a historical realization, or
physically delete source-derived facts.

An Agent Assertion enters an active Overlay Revision only after Compass verifies
its bounded citations against the exact Base Generation. Compass, not the agent,
then assigns the assertion the state `GROUNDED`. Grounding proves that the cited
evidence exists, retains the expected content and identity, and satisfies a
named deterministic policy. It does not claim that an agent's interpretation is
universally true.

The Base Graph and an exact Overlay Revision compose into an Effective Graph.
Existing CompassQL remains read-only. New CLI and MCP adapters call the same
overlay interface and return typed receipts, conflicts, Grounding failures,
omissions, and Effective Graph identities.

## Domain language

The normative vocabulary is in [`CONTEXT.md`](../../CONTEXT.md). In particular:

- `INFERRED` remains structural confidence produced by Compass resolution.
- `GROUNDED` is the verified publication state of an Agent Assertion.
- authorship, Grounding, and structural confidence are separate claims.
- deletion of an Agent Assertion is a Retraction.
- disagreement with a Base Graph fact is a Challenge, never a source-fact edit.

## Problem

Compass has two adjacent implementations, but neither owns interactive graph
enhancement:

- `compass-semantic` validates provider output and `compass-core::SemanticLayer`
  merges it during a coherent graph build. Its ownership is source-file refresh,
  not session CRUD.
- `compass-reflect` derives `learning.json` from saved results. That sidecar
  decorates existing nodes and deliberately does not own topology.

Directly editing `graph.json` would invalidate the store reference, graph seal,
query caches, communities, orientation, output snapshots, and historical
identity. Adding write clauses to CompassQL would also contradict its bounded,
read-only contract.

The missing module must concentrate these rules behind one interface:

- exact Base Generation binding;
- deterministic Grounding;
- agent ownership;
- create, replace, Retraction, and Challenge transitions;
- stable assertion and revision identity;
- atomic compare-and-swap publication;
- rebase and explicit ambiguity;
- Effective Graph composition;
- authorization, idempotency, bounds, audit, and privacy.

By the deletion test, this module is deep: removing it would scatter those rules
across CLI handlers, MCP tools, query loaders, storage code, and output adapters.

## Goals

1. Let an authorized AI agent enhance a graph during chat initialization or a
   later agent session.
2. Publish only Agent Assertions that earn `GROUNDED` through deterministic,
   local verification.
3. Support logical CRUD for agent-owned nodes and edges without mutating Base
   Graph facts.
4. Preserve graph direction, multiplicity, stable identity, evidence, and exact
   Base Generation provenance.
5. Make concurrent updates fail with typed revision conflicts rather than lose
   changes.
6. Keep publication deterministic, bounded, atomic, and recoverable.
7. Expose one machine contract through thin CLI and MCP adapters.
8. Let query, task-context, viewer, report, and history readers select an exact
   Effective Graph.
9. Preserve Compass's local-first structural path without models, credentials,
   or network access.
10. Retain an auditable, append-only history of Agent Assertions and
    Retractions.

## Non-goals

- The module does not ask a model to generate assertions.
- `GROUNDED` does not mean formally proven, human-approved, or certain.
- V1 does not accept arbitrary node kinds, relationship strings, or executable
  verifier plugins.
- Base nodes and edges cannot be updated, replaced, or physically deleted.
- CompassQL does not gain `CREATE`, `MERGE`, `SET`, `REMOVE`, or `DELETE`.
- Existing query commands do not silently start consuming overlays.
- A graph build does not silently reattach assertions to new same-named facts.
- Prompts, chain-of-thought, credentials, and complete chat transcripts are not
  stored.
- V1 does not synchronize overlays between machines or provide a hosted
  multi-tenant write endpoint.

## Chosen architectural decisions

### Two truth planes

```text
Base Graph                              Agent Graph Overlay
----------                              -------------------
source-derived                          agent-authored
compass.graph/1                         compass.agent-graph.overlay/1
immutable generation                    immutable revisions + mutable head
build-owned                              compass-agent-graph-owned
exact/inferred/ambiguous confidence      GROUNDED/retracted lifecycle
```

An Effective Graph is a read view, not a third source of truth:

```text
Base Generation + Overlay Revision + Composition Profile
                              |
                              v
                    Effective Graph identity
```

### Separate Grounding from confidence

`EvidenceConfidence::{Exact, Inferred, Ambiguous}` continues to describe the
strength of a structural graph claim. `GROUNDED` belongs to the Agent Assertion
lifecycle and is issued only by `compass-agent-graph`.

The agent submits a draft and citations. It cannot submit a trusted status or
construct a `GroundingCertificate` through the public Rust interface.

### Append-only logical CRUD

- Create publishes a new stable Agent Assertion.
- Update publishes a replacement version under the same Assertion ID.
- Delete publishes a Retraction tombstone.
- Read selects an exact or active Overlay Revision.
- Base Graph disagreement publishes a Challenge.
- A Challenge may flag a base fact in the augment profile.
- A separately authorized mask may hide a challenged fact only in the curated
  composition profile.

No earlier Overlay Revision or assertion version is rewritten.

### Explicit selection

Existing commands remain Base Graph-only. Agent-aware callers select an
Overlay Revision or explicitly request the active overlay. An agent chat
initializer may choose the augment profile, but that choice is visible in the
result identity and metadata.

### One deep interface

The selected external seam has two operations:

```rust
pub trait AgentGraphOverlay: Send + Sync {
    fn read(&self, request: ReadRequest) -> Result<ReadResult, AgentGraphError>;

    fn apply(
        &self,
        grant: &WriteGrant,
        batch: ChangeBatch,
    ) -> Result<CommitReceipt, AgentGraphError>;
}
```

`read` covers overlay inspection, Effective Graph composition, history, diff,
and rebase planning through a closed request enum. `apply` covers create,
replace, Retraction, Challenge, Challenge Retraction, and rebase commit through
one atomic batch. This keeps adapters shallow and concentrates behavior for
locality.

An optional apply-then-query convenience belongs in CLI/MCP orchestration. It
does not enter this interface or make overlay publication depend on query
execution.

## Ownership and dependency direction

Add a new workspace crate:

```text
crates/compass-agent-graph/
├── Cargo.toml
├── src/
│   ├── lib.rs          interface and re-exports
│   ├── contract.rs     versioned requests, responses, IDs, and errors
│   ├── assertion.rs    assertion lifecycle and ownership
│   ├── grounding.rs    verifier registry and certificates
│   ├── canonical.rs    canonical encoding and semantic digests
│   ├── overlay.rs      materialized Overlay Revision state
│   ├── compose.rs      Effective Graph construction
│   ├── rebase.rs       exact reattachment planning and commit validation
│   ├── repository.rs   immutable objects, head selector, idempotency, GC
│   ├── policy.rs       write grants and composition authorization
│   └── limits.rs       shared work accounting
└── tests/
    ├── contract.rs
    ├── grounding.rs
    ├── lifecycle.rs
    ├── publication.rs
    ├── composition.rs
    ├── rebase.rs
    └── conformance.rs
```

Dependency direction:

```text
compass-model      compass-store      compass-files
      \                 |                 /
       +--------- compass-agent-graph ---+
                          |
                     compass-core
                    /      |       \
          compass-query  compass-cli  compass-mcp
```

- `compass-model` retains the strict Base Graph vocabulary and validation.
- `compass-agent-graph` owns every overlay domain rule.
- `compass-store` supplies bounded immutable writes, scans, and conditional
  selector updates.
- `compass-files` supplies path containment and atomic sidecar writes.
- `compass-core` selects Base Generations and joins read/write adapters.
- `compass-query` adds a read-only Effective Graph adapter.
- `compass-cli` and `compass-mcp` translate public requests and outcomes.
- `compass-history` remains immutable and supplies an exact-base reader adapter;
  it does not depend on `compass-agent-graph`.
- `compass-semantic` and `compass-reflect` remain independent producers with
  their current ownership.

## Core contract

### Identity types

```rust
pub struct BaseGenerationId {
    pub generation_id: String,
    pub graph_digest: Digest,
}

pub struct OverlayId(String);
pub struct OverlayRevisionId(Digest);
pub struct AssertionKey(String);
pub struct AssertionId(String);
pub struct ChallengeId(String);
pub struct PrincipalId(String);
pub struct IdempotencyKey(String);
pub struct AssertionDigest(Digest);
pub struct GroundingCertificateDigest(Digest);
```

IDs are opaque, length-bounded, ASCII wire values with strict parsing. Digests
are lowercase SHA-256. Base Generation identity includes both the build
generation and canonical graph digest so a reused display path cannot imply
equivalence.

### Change batches

```rust
pub struct ChangeBatch {
    pub schema: String, // compass.agent-graph.batch/1
    pub overlay: OverlayId,
    pub base_generation: BaseGenerationId,
    pub expected_revision: Option<OverlayRevisionId>,
    pub idempotency_key: IdempotencyKey,
    pub operations: Vec<ChangeOperation>,
}

pub enum ChangeOperation {
    PutAssertion(AssertionDraft),
    RetractAssertion {
        assertion: AssertionId,
        expected_assertion_digest: AssertionDigest,
        reason_code: String,
        explanation: String,
    },
    PutChallenge(ChallengeDraft),
    RetractChallenge {
        challenge: ChallengeId,
        expected_challenge_digest: Digest,
        reason_code: String,
        explanation: String,
    },
    CommitRebase(RebaseCommit),
}
```

`expected_revision` is `None` only when creating an overlay. Every later write
requires the exact current revision. There is no last-write-wins mode.

`PutAssertion` creates when its selector is `New` and replaces when its selector
is `Existing`. A replacement must include the current assertion digest. Node
assertions cannot become edge assertions or vice versa.

### Assertion drafts

```rust
pub struct AssertionDraft {
    pub selector: AssertionSelector,
    pub fact: AgentFactDraft,
    pub grounding: GroundingSubmission,
    pub summary: String,
}

pub enum AssertionSelector {
    New {
        key: AssertionKey,
    },
    Existing {
        id: AssertionId,
        expected_assertion_digest: AssertionDigest,
    },
}

pub enum AgentFactDraft {
    Node(AgentNodeDraft),
    Edge(AgentEdgeDraft),
}

pub enum NodeRef {
    Base(BaseNodeRef),
    Agent(AssertionId),
    CreatedInThisBatch(AssertionKey),
}
```

Mutation targets never accept labels, fuzzy text, or a first candidate. Base
references contain the Base Generation, exact ID, kind, and canonical record
digest. Agent references contain the exact Assertion ID. A within-batch
reference is valid only when the same batch creates that node assertion.

Agent node and edge drafts use the existing closed `NodeKind`, `NodeRole`,
`NodeDetails`, `EdgeKind`, `EdgeDetails`, direction, and multiplicity contract.
V1 rejects arbitrary relationship strings and unknown endpoint-kind pairs.

### Stable assertion identity

For creation, the client supplies a bounded logical Assertion Key. Compass derives the public
Assertion ID from:

```text
schema + repository identity + overlay ID + owner principal + fact class + key
```

Session ID, model name, timestamps, prompt text, and mutable fact fields do not
enter the Assertion ID. They also cannot change ownership. Replacement creates
a new assertion digest while retaining the Assertion ID.

Effective agent node IDs and edge IDs derive from Assertion IDs with distinct
domains. Parallel edges require distinct assertion keys and retain independent
identities.

## Grounding

### Meaning of `GROUNDED`

`GROUNDED` means all of the following:

1. every citation resolved inside the selected repository and Base Generation;
2. every referenced record, file, range, path, and artifact matched its digest;
3. source ranges were non-empty, in bounds, and bound to the inventoried file;
4. edge direction and endpoint identity were preserved;
5. the named Grounding policy accepted the evidence kinds for that fact;
6. the verifier and policy versions were included in the certificate;
7. the resulting assertion passed graph vocabulary and endpoint validation.

It does not mean semantic entailment, runtime reachability, or human approval.
Consumers must be able to inspect the policy and evidence digest.

### V1 evidence types

```rust
pub enum GroundingEvidence {
    SourceSpan {
        file: String,
        anchor: SourceAnchor,
        file_digest: Digest,
        excerpt_digest: Digest,
    },
    BaseFact {
        fact: BaseFactRef,
        record_digest: Digest,
    },
    BasePath {
        nodes: Vec<BaseNodeRef>,
        edges: Vec<BaseEdgeRef>,
        path_digest: Digest,
    },
    PriorAssertion {
        assertion: AssertionId,
        revision: OverlayRevisionId,
        assertion_digest: AssertionDigest,
    },
    SnapshotArtifact {
        artifact: String,
        artifact_digest: Digest,
        json_pointer: Option<String>,
    },
}
```

At least one verified `SourceSpan` is required for every agent node or edge that
enters the strict graph projection in V1. Other evidence may strengthen the
Grounding policy but cannot create a source-less topology fact. This preserves
the current provenance anchor invariant and avoids synthetic wiring sites.

Evidence values use strict typed schemas with unknown fields rejected. Internal
`EvidenceVerifier` adapters are registered statically. V1 does not load verifier
code from a repository or network.

### Grounding certificate

```rust
pub struct GroundingCertificate {
    status: GroundingStatus, // only compass-agent-graph can construct
    claim_digest: Digest,
    evidence_digest: Digest,
    base_generation: BaseGenerationId,
    policy_id: String,
    policy_version: String,
    verifier_versions: Vec<String>,
    permitted_effects: Vec<GroundedEffect>,
}

pub enum GroundingStatus {
    Grounded, // serialized as GROUNDED
}
```

Masking requires `GroundedEffect::MaskBaseFact`; a generic certificate cannot
authorize it. The public request never contains a certificate or status field.

### Projection into `compass.graph/1`

The Base Graph is never widened or rewritten. `compass.agent-graph.effective/1`
contains:

- a strict `compass.graph/1` projection for existing read machinery;
- an ordered map from projected fact ID to Agent Assertion, authorship, Overlay
  Revision, `GROUNDED` certificate digest, Challenge state, and full evidence;
- exact composition and omission metadata.

The strict projection uses the verified source anchor, extractor
`compass.agent-graph`, a closed `grounded-agent-assertion` rule, and conservative
existing structural confidence. `GROUNDED` is authoritative only in the
effective wrapper and is not added to `EvidenceConfidence`.

## Lifecycle and state machine

```text
draft request
    |
    +-- Grounding fails -----------------> rejected; nothing published
    |
    `-- Grounding succeeds --------------> GROUNDED assertion
                                                |
                           replace + Grounding --+--> new GROUNDED version
                                                |
                           Retraction -----------+--> retracted
```

Overlay Revision state contains only active GROUNDED assertions plus retained
Retractions and Challenges. Failed drafts may appear in bounded diagnostics but
are not durable graph facts.

Retracting a node with active agent edges fails unless the same batch retracts
or replaces every dependent edge. There is no hidden cascade for agent-owned
facts.

## Challenges and composition profiles

```rust
pub enum ChallengeEffect {
    Flag,
    Mask,
}

pub enum CompositionProfile {
    Augment,
    Curated,
}
```

- `Augment` adds GROUNDED assertions and reports Challenges but preserves every
  Base Graph fact.
- `Curated` additionally applies authorized masks.
- masking a base edge omits that edge from the curated projection;
- masking a base node omits the node and every incident base or agent edge;
- every direct and cascaded omission is counted and bounded examples are
  retained;
- direct inspection can still retrieve the challenged Base Graph fact and its
  evidence.

Existing Compass readers remain Base Graph-only. Agent-aware reads must name a
profile. No profile silently claims that a masked fact never existed.

## Deterministic validation and publication

`apply` runs in this fixed order:

1. Verify the opaque `WriteGrant` and its repository, overlay, principal,
   operation, Base Generation, and limit scopes.
2. Reject unknown schema majors and enforce input byte, depth, count, and string
   bounds before large allocation.
3. Open and validate the exact Base Generation.
4. Load the active overlay head and compare `expected_revision`.
5. Check the idempotency key and batch digest.
6. Reject duplicate or contradictory targets in the batch.
7. Validate ownership and expected assertion/challenge digests.
8. Resolve exact base and agent references; topologically order within-batch
   dependencies and reject cycles.
9. Run deterministic Grounding and issue certificates.
10. Apply the batch to a temporary materialized post-state.
11. Validate Retractions, Challenges, masks, endpoints, multiplicity, kinds, and
    graph limits.
12. Canonically order operations, assertions, evidence, candidates, errors, and
    omissions.
13. Derive immutable object digests and the Overlay Revision ID.
14. Publish immutable objects and idempotency receipt.
15. Advance the one active head with a conditional version-token write.
16. Reopen and verify the selected revision before returning success.

Steps 14 and 15 reuse the existing prepare/activate pattern. A crash before the
head update can leave unreachable immutable objects but cannot expose partial
state. GC may remove those objects later. A crash after the head update is
recoverable through the already-published idempotency receipt. Success is
acknowledged only after the selector points to a fully readable revision.

Equivalent semantic inputs produce equivalent revision bytes. Clocks, process
IDs, machine paths, timings, tokens, and audit timestamps are excluded from the
revision digest.

## Storage

### Layout

For Git repositories, store overlay revisions below the Git common directory:

```text
<git-common-dir>/compass/
├── agent-graph.sqlite3
├── agent-graph.sqlite3-wal
└── agent-graph.sqlite3-shm
```

For non-Git corpora, use a contained owner-protected state directory adjacent
to the selected Compass output. Path selection is explicit in configuration and
never trusts an agent-supplied absolute path.

The database uses a dedicated `compass.agent-graph.v1` namespace with ordered
partitions for immutable objects, revision manifests, idempotency receipts,
active heads, pins, and operational audit records. Overlay data is never stored
in the Base Graph query-index namespace.

### Revision contents

Each revision records:

- schema and canonical encoding versions;
- overlay ID, parent revision, and sequence;
- Base Generation;
- owner principal and ownership policy;
- roots for assertions, Challenges, Retractions, certificates, and references;
- mutation digest and composition compatibility version;
- exact counts and bounded limits used;
- completion evidence.

Materialized ordered roots avoid unbounded replay. Parent links retain audit and
diff history. Reachability GC preserves active heads and explicit pins.

## Rebase across Base Generations

An Overlay Revision composes only with its exact Base Generation. A mismatched
read returns `rebase_required`; it never performs a fuzzy implicit carry-forward.

`read(ReadRequest::PrepareRebase)` returns a versioned plan containing:

- retained exact references whose ID and canonical record digest are unchanged;
- assertions that require Grounding again because evidence changed;
- unresolved references;
- ambiguous bounded candidate sets with exact omitted counts;
- proposed explicit Retractions;
- source revision, target Base Generation, policy versions, and plan digest.

`apply(ChangeOperation::CommitRebase)` requires:

- the source revision is still active;
- the target Base Generation still matches;
- the exact plan digest;
- explicit mappings or Retractions for every unresolved item;
- new evidence for every assertion that requires Grounding;
- no ambiguous first-candidate selection.

The rebase produces a new immutable Overlay Revision. It never edits the old
revision or historical Base Generation.

## Effective Graph reads

```rust
pub struct EffectiveGraph {
    pub schema: String, // compass.agent-graph.effective/1
    pub base_generation: BaseGenerationId,
    pub overlay_revision: OverlayRevisionId,
    pub composition_profile: CompositionProfile,
    pub effective_identity: Digest,
    pub graph: compass_model::code_graph::GraphDocument,
    pub agent_facts: Vec<EffectiveAgentFact>,
    pub challenges: Vec<EffectiveChallenge>,
    pub retractions: EffectiveRetractions,
    pub omissions: CompositionOmissions,
}
```

Identity is the digest of the Base Generation, Overlay Revision, composition
profile/version, and canonical effective semantics. The implementation composes
in deterministic ID order and validates the final strict projection.

`compass-query` adds `EffectiveGraphEngine`, a read-only adapter. Query caches
key the Effective Graph identity, not the Base Graph digest alone. JSON and
store-backed effective readers must pass differential tests. The initial
implementation may use `DirectGraphEngine`; the optimized implementation uses
`GraphSnapshotBuilder::prepare_graph_delta` in a separate effective snapshot
namespace and never activates over the Base Graph selector.

Communities, orientation, reports, and viewer output either derive from the
same Effective Graph identity or explicitly report that only Base Graph
analysis is available. They never silently combine base-derived analysis with
changed effective topology.

Task context exposes Agent Assertions in a separate bounded section with their
Assertion IDs, Grounding policy/certificate digests, citations, Challenges,
omissions, and Overlay Revision. This prevents structural and agent-authored
evidence from becoming indistinguishable.

## Public adapters

### CLI

```text
compass agent-graph status [--overlay ID] [--format text|json]
compass agent-graph apply --request FILE [--format text|json]
compass agent-graph show ASSERTION_ID [--revision REV] [--format text|json]
compass agent-graph history [--overlay ID] [--format text|json]
compass agent-graph diff OLD NEW [--format text|json]
compass agent-graph rebase-plan --to-current [--format json]
compass agent-graph rebase-commit --request FILE [--format json]
compass agent-graph query --revision REV --profile augment|curated [query options]
compass agent-graph export --revision REV --profile augment|curated --output PATH
```

`apply` is the canonical mutation command. Convenience create/update/retract
commands may translate into the same `ChangeBatch`; they do not own a second
implementation.

### MCP

Expose two primary tools when enabled:

- `inspect_agent_graph` maps to `read`.
- `apply_agent_graph` maps to `apply`.

Existing read tools remain unchanged. An optional exact overlay selector may be
added only through an effective-result envelope that preserves the existing
domain result and carries Base Generation, Overlay Revision, composition
profile, Grounding, and omission metadata.

The write tool is absent from `tools/list` unless writes were explicitly
enabled. Request JSON cannot choose its authenticated principal or construct a
`WriteGrant`.

## Authorization and security

Write capability is deny-by-default.

- stdio requires explicit `--enable-agent-graph-writes` and a project-confined
  local principal;
- HTTP requires explicit write enablement plus authenticated, scoped write
  credentials even on loopback;
- the existing optional read API key is not by itself a write grant;
- a grant binds principal, repository, overlay, Base Generation, expected
  revision, allowed operations, mask permission, expiry, and hard limits;
- `project_path` is resolved through an operator allowlist and canonical root
  containment, not arbitrary request paths;
- credentials never enter assertions, receipts, logs, or digests;
- source excerpts are not stored by default; only anchors and digests are;
- summaries and explanations are bounded and must not contain chain-of-thought;
- untrusted repository content cannot register verifiers or authorization
  policy;
- audit output redacts source text and transport secrets.

Prompt injection remains relevant because an agent can be influenced by source
content. Grounding therefore verifies evidence integrity and permissions even
when the caller is already authenticated.

## Attestation and audit

Semantic identity includes the stable owner principal because ownership affects
legal future mutations. Operational Attestation records may include bounded
session ID, adapter name, model identifier, request ID, timestamp, and outcome.
They exclude prompts, chain-of-thought, credentials, token payloads, and source
excerpts.

Audit records use `compass.agent-graph.audit/1`, are append-only, size-bounded,
and are not inputs to Overlay Revision identity. Equivalent mutations from the
same owner remain semantically identical despite different execution times.

## History semantics

Published Compass history realizations stay immutable and unchanged. An Agent
Graph Overlay may bind to a current Base Generation or an exact historical
realization. The separate overlay store records that binding.

Historical composition requires both selectors:

```text
base realization + exact Overlay Revision -> Effective Graph
```

There is no automatic preferred overlay for a historical realization. Export
records both identities. Diffs may compare overlay revisions only when Base
Generation and composition versions are compatible; otherwise an explicit
rebase is required first.

## Limits

V1 defaults and hard ceilings:

| Resource | Default | Hard ceiling |
| --- | ---: | ---: |
| Encoded change batch | 1 MiB | 16 MiB |
| Operations per batch | 100 | 1,000 |
| Citations per assertion | 16 | 64 |
| Grounding evidence bytes per batch | 1 MiB | 8 MiB |
| Summary/explanation | 2 KiB | 8 KiB |
| Assertion dependency depth | 16 | 32 |
| Reattachment candidates per reference | 10 | 20 |
| Agent nodes per overlay | 10,000 | 100,000 |
| Agent edges per overlay | 100,000 | 500,000 |
| Bounded diagnostic examples | 20 | 100 |
| Operational audit record | 4 KiB | 16 KiB |

Effective Graph size also obeys the selected graph engine's existing limits.
Verification and composition have explicit deadlines and cancellation. A limit
error is never reported as an empty overlay, no match, or successful Grounding.

## Typed errors and diagnostics

Stable error families include:

- `unsupported_schema`;
- `writes_disabled`;
- `unauthenticated`;
- `unauthorized`;
- `invalid_identifier`;
- `limit_exceeded`;
- `unknown_base_generation`;
- `unknown_overlay`;
- `revision_conflict`;
- `idempotency_conflict`;
- `assertion_not_found`;
- `assertion_digest_conflict`;
- `ownership_violation`;
- `duplicate_operation`;
- `invalid_transition`;
- `invalid_citation`;
- `grounding_failed`;
- `grounding_policy_unsupported`;
- `mask_not_permitted`;
- `missing_endpoint`;
- `active_dependents`;
- `assertion_cycle`;
- `rebase_required`;
- `rebase_plan_stale`;
- `rebase_unresolved`;
- `rebase_ambiguous`;
- `corrupt_overlay`;
- `publication_conflict`;
- `storage_failure`.

Conflicts return the observed revision. Ambiguity returns deterministic bounded
candidates and an exact omitted count. Grounding problems name the assertion,
evidence index, stable code, and field path without echoing sensitive content.

## Compatibility and versioning

New machine contracts use independent majors:

- `compass.agent-graph.batch/1`;
- `compass.agent-graph.receipt/1`;
- `compass.agent-graph.overlay/1`;
- `compass.agent-graph.effective/1`;
- `compass.agent-graph.rebase-plan/1`;
- `compass.agent-graph.audit/1`.

Unknown majors fail explicitly. Meaning-affecting changes to canonicalization,
Grounding policies, verifier versions, composition, identity, masks, or limits
enter the relevant revision identity and compatibility checks.

The initial implementation avoids adding `GROUNDED` to `compass.graph/1`,
`EvidenceOrigin`, or `EvidenceConfidence`. A later native graph-schema revision
may model agent authorship directly, but it is not required for this vertical
slice.

Public release work must update `COMPATIBILITY.md`, command/configuration/output
references, provenance and graph-model concepts, security documentation, the
assistant skill, `CHANGELOG.md`, and `MIGRATION.md` only if users must act.

## Verification strategy

### Lowest-layer tests

- canonical bytes and IDs are invariant under input ordering;
- unknown majors and unknown evidence kinds fail closed;
- callers cannot construct `GROUNDED` certificates;
- source spans, file digests, record digests, paths, and artifact pointers are
  reverified;
- missing, stale, and ambiguous references never bind by first match;
- creates, replacements, Retractions, Challenges, and masks enforce ownership;
- node Retraction rejects active dependent edges;
- equivalent retries return the original idempotent receipt;
- different content under one idempotency key fails;
- concurrent CAS writers produce one winner and one typed conflict;
- interruption before selector activation preserves the old head;
- reopen validates every immutable root and certificate;
- GC preserves active and pinned revisions;
- rebase is deterministic and requires new Grounding where evidence changed;
- augment and curated composition preserve direction and multiplicity;
- masks report direct and cascaded omissions;
- Effective Graph identity changes with base, overlay, profile, or composition
  semantics;
- JSON and store query adapters are semantically equivalent.

### Interface tests

- CLI help, JSON schemas, streams, exit codes, path containment, idempotency,
  conflicts, and no-partial-output behavior;
- MCP tool advertisement, strict schemas, transport envelopes, domain versus
  transport truncation, project scoping, and write-disabled behavior;
- HTTP rejects write enablement without authentication and scope;
- CompassQL remains mutation-free;
- task context reports Agent Assertions separately and retains Grounding;
- viewer/export distinguishes agent facts, shows active Challenges, and
  exposes bounded Retraction history under the exact pinned profile;
- history realization bytes are unchanged by overlay CRUD;
- exact historical composition round-trips by both selectors.

### Product gates

- `scripts/check_product_boundary.sh` continues to pass;
- code-only build/query behavior is unchanged when no overlay is selected;
- graph qualification remains source-truth evidence and does not count agent
  assertions as extractor recall;
- all Cargo commands use the required external `CARGO_TARGET_DIR`;
- optional model/provider tests use fixtures only and are not needed for
  Grounding verification.

## Rollout principles

The feature ships behind explicit local enablement until model, storage,
security, query parity, and end-to-end gates pass. Read-only overlay inspection
may ship before write adapters. Masking ships after ordinary augment composition
and requires a separate capability. Historical composition ships after current
Base Generation behavior is stable.

No phase may advertise an Agent Assertion as `GROUNDED` before the verifier,
certificate, publication, reopen, query, and audit tests for that assertion pass.

## Related documents

- [Phased execution plan](grounded-agent-graph-overlay-phased-execution-plan.md)
- [Graph model](../concepts/graph-model.md)
- [Provenance and confidence](../concepts/provenance.md)
- [Storage and history design](../design/storage-and-history.md)
- [Security and privacy](../design/security-and-privacy.md)
- [Extending Compass](extending-compass.md)

**Next step:** execute Phase 0 of the phased plan and freeze the contract
fixtures before implementing storage or write adapters.
