# Agent-readable query output design

Status: implemented

Date: 2026-09-16

Owner boundaries: `compass-query`, `compass-output`, `compass-cli`, and
`compass-mcp`

Primary contract: `compass.query.agent-view/1`

Implementation plan: [`../025-agent-readable-query-output.md`](../025-agent-readable-query-output.md)

## Decision summary

Compass will add a bounded, deterministic **Agent Query View** over the existing
typed query responses. The view presents the result state, one evidence-backed
headline, decisive caveats, primary entities, readable relationships and
paths, and exact next actions before low-level graph details.

The Agent Query View is a projection, not a new query engine:

```text
query request
    |
    v
compass-query
    |
    +--> compass.query/1 or compass.query.discovery/1   authoritative result
    |
    v
compass-output agent-query projector
    |
    +--> compass.query.agent-view/1                    bounded agent contract
    +--> answer-first text                             CLI and MCP content
    `--> agentView in MCP transport                    alongside raw result
```

The raw query result remains authoritative and unchanged. The projection may
summarize or omit low-priority records under explicit bounds, but it may not
invent a target, relationship, path, confidence, completeness state, or source
location.

## Problem

Compass already returns source-grounded graph evidence, but the current output
requires an agent to reconstruct the answer shape:

- typed MCP text contains only operation and node/edge/path counts;
- discovery text spends its first lines on seed, traversal, and scope metadata;
- nodes, relationships, paths, and diagnostics are separate collections;
- diagnostics that invalidate a conclusion can appear after the graph records;
- endpoint labels must be joined from node IDs by the consumer;
- the result gives no bounded, exact next action for ambiguity, truncation, or
  deeper inspection.

This is presentation friction rather than a retrieval defect. Changing ranking
or graph semantics would increase risk without addressing the main cost: an
agent spends context and tool calls translating a correct low-level result into
an actionable inspection result.

## Goals

1. Put the interpretation state before graph details.
2. Make decisive evidence understandable without joining arrays by ID.
3. Keep match resolution, relationship evidence, execution completeness, and
   corpus coverage separate.
4. Make ambiguity, missing exact matches, direction mismatch, stale source, and
   truncation impossible to overlook.
5. Provide exact, bounded follow-up actions without shell interpolation.
6. Preserve raw query schemas, ordering, evidence, and digests.
7. Produce byte-deterministic output for equivalent inputs.
8. Keep the projection local, credential-free, provider-free, and bounded.

## Non-goals

- No model-generated summary or answer.
- No new ranking, search, traversal, resolution, or graph facts.
- No automatic selection of an ambiguous or fuzzy candidate.
- No claim that absence proves nonexistence when graph coverage is unknown.
- No source excerpt copied into the compact view; exact source locations point
  the agent to `task_context`, `explore_code`, or the raw evidence.
- No replacement of `compass.query/1`, `compass.query.discovery/1`, CompassQL,
  or `compass.task-context/2`.
- No new discovery-cursor major solely because prose changes.
- No agent-workflow state machine or server-side tool execution.

## Design principles

### Answer first, audit second

The first screenful must say whether the result is usable and why. Full
provenance remains available, but hashes, extractor details, and raw evidence
arrays do not precede the answer.

### Separate meanings that agents commonly conflate

The view never exposes one generic `confidence` field. It reports:

- **result state**: what kind of response this is;
- **match state**: how the requested operand matched graph identity;
- **evidence state**: the weakest evidence supporting displayed graph facts;
- **source execution**: whether query execution hit a bound;
- **projection state**: whether the compact Agent View omitted retained facts;
- **coverage state**: whether incomplete graph coverage is known, otherwise
  `unknown`.

### Evidence references are lossless

Every displayed entity, relationship, and path retains its full stable ID or a
reference to the raw record. Human text may shorten IDs only when it also says
where the complete value is available. Agent JSON always carries complete IDs.

### Unavailability is a result

An ambiguous, no-match, no-path, direction-mismatch, or stale-source condition
is rendered explicitly. It is never converted into an empty success or an
inferred answer.

## Contract

The new strict schema is `compass.query.agent-view/1`.

```text
AgentQueryView
  schema
  request
    operation
    question?
    operands[] { role, value }
  status
    resultState
    matchState
    evidenceState
    sourceExecution
    projection
    coverage
  answer
    headline
    basis[] { kind, id }
  primaryResults[]
  relationships[]
  paths[]
  caveats[]
  nextActions[]
  omissions
  identity
    rawSchema
    graphIdentity
    buildGenerationIdentity
    sourceResultDigest
    viewDigest
  sourceTruncated
  projectionTruncated
```

All serialized structs use camelCase and `deny_unknown_fields`. Unknown schema
majors fail explicitly.

### Request

`operation` is a closed enum:

```text
discovery | search | callers | callees | impact | explore | node_trail
```

Operands are ordered and role-typed:

```text
query | symbol | source | target | root
```

The request echoes bounded user data as data. It is not executable prose.

### Status

`resultState` is one of:

| State | Meaning |
| --- | --- |
| `answered` | The operation produced an exact, structurally interpretable result, including a valid zero-count result. |
| `candidates` | Broad discovery produced ranked anchors and relationships, not a single exact answer. |
| `needs_resolution` | Multiple viable exact/fuzzy targets remain; no candidate was selected. |
| `no_match` | No exact match exists. Fallback candidates may still be displayed. |
| `no_path` | Exact endpoints resolved but no valid directed path was returned. |

`matchState` is one of:

```text
exact | fuzzy | ambiguous | none | not_applicable | unknown
```

`evidenceState` is one of:

```text
exact | inferred | mixed | ambiguous | none
```

`sourceExecution` and `projection` are independently `complete` or `partial`.
`coverage` is `incomplete` only when the query result reports that fact;
otherwise it is `unknown`. Version 1 deliberately has no `complete` coverage
state because the current query response cannot prove complete corpus coverage.

### Outcome precedence

The projector applies this deterministic precedence:

1. `ambiguous_match` or an ambiguous discovery seed -> `needs_resolution`;
2. `no_match` -> `no_match`, even when fallback candidates exist;
3. `direction_mismatch` -> `no_path` with a blocking caveat;
4. exact node-trail endpoints with zero paths -> `no_path`;
5. broad discovery -> `candidates`;
6. otherwise -> `answered`.

Truncation does not replace the result state. It sets `sourceExecution` or
`projection` to `partial` and adds a caveat. This lets an agent distinguish
“answer unavailable” from “answer available but incomplete.”

### Deterministic answer templates

The projector uses closed templates by operation. It does not synthesize prose
from source code.

Examples:

```text
Found 2 exact candidates for "Target".
No exact match for "Targat"; 3 fallback candidates are shown.
Found 4 incoming call or route relationships for Fixture.Target.
Found 3 direct callees for Fixture.Caller.
Found 12 potentially affected nodes within depth 4.
Found a 3-hop directed path from Router to TokenVerifier.
No directed path reaches the exact target within the requested bounds.
Found 3 candidate anchors and 8 relationships for the question.
```

Every non-count noun in a headline comes from the request, an exact displayed
entity label, or a closed operation label. A headline's `basis` references the
raw nodes, edges, paths, or diagnostics that justify it.

If the projector cannot identify an exact display subject without
re-resolving, it quotes the requested operand and omits a target ID. It never
runs a second resolver in the output layer.

### Primary results

An `AgentEntity` contains:

```text
id, label, kind, roles, language?, framework?, source?
```

Primary selection is operation-specific and stable:

- discovery: seed order, then stable node ID;
- search: `SearchHit` order joined to nodes;
- callers: exact target followed by unique incoming sources;
- callees: exact source followed by unique outgoing targets;
- impact: traversal root followed by path endpoints in path order;
- explore: retained path endpoints followed by remaining requested-result nodes;
- node trail: source and target from the best path, then alternative endpoints.

If an exact target cannot be proven from the response, the projector omits that
role rather than selecting the first node. The plan must add fixtures for
zero-edge callers/callees, isolated explore seeds, and no-path trails before
finalizing these rules.

### Relationships

Relationships inline readable endpoints so an agent does not need a join:

```json
{
  "id": "edge:router-auth",
  "source": {"id": "node:router", "label": "Router"},
  "relation": "calls",
  "target": {"id": "node:auth", "label": "AuthMiddleware"},
  "site": {"file": "src/routes.rs", "startLine": 18},
  "evidence": {
    "confidence": "exact",
    "resolution": "exact",
    "layers": ["structural_graph"]
  }
}
```

Direction always follows the published edge. Parallel relationships remain
separate. Missing endpoint records produce a caveat and use the stable ID as
the display label; they are not dropped silently.

### Paths

Paths inline ordered steps:

```text
AgentPath
  id
  steps[]
    from { id, label }
    edgeId
    relation
    direction: forward | reverse
    to { id, label }
    site?
  weakestResolution
  weakestConfidence
```

`direction` describes whether the path step follows or opposes the published
edge. It prevents an impact path or compatibility traversal from looking like
a forward runtime call when it was traversed in reverse.

### Caveats

Caveats are sorted by severity, stable code, node ID, path, then statement.

```text
blocker | warning | info
```

Initial diagnostic mapping:

| Diagnostic | Severity | Required effect text |
| --- | --- | --- |
| `ambiguous_match` | blocker | Do not select a candidate automatically. |
| `no_match` | blocker | Fallback candidates are suggestions, not an exact answer. |
| `direction_mismatch` | blocker | A reverse-only connection is not a valid directed path. |
| `stale_source_digest` | blocker | Do not quote or edit the stale source excerpt. |
| `incomplete_coverage` | warning | Absence is not proof that the relationship does not exist. |
| `bounded_truncation` | warning | More retained facts may exist beyond the response bound. |
| `unresolved_handler` | warning | The framework target is unresolved. |
| `program_conflict` | warning | Structural and Program IR evidence disagree. |
| `program_orphan` | info | Program evidence could not join to a graph entity. |
| `program_unavailable` | info | Optional Program IR evidence was unavailable. |

The original bounded diagnostic statement is retained as data after safe text
escaping. Blockers and warnings appear before result details in text output.

### Next actions

At most five actions are returned. Actions use argv arrays and JSON argument
objects; Compass never emits a shell command assembled from untrusted values.

```text
AgentNextAction
  kind
  reason
  cli? { argv[] }
  mcp? { tool, arguments }
```

Initial action kinds:

- `retry_with_exact_id` for each retained ambiguity candidate, bounded to three;
- `inspect_target` using `compass context explain` / `task_context` when exactly
  one primary target is proven;
- `continue_result` when a discovery text cursor exists;
- `show_evidence` when compact text hides provenance;
- `narrow_query` when projection or source execution is partial and no cursor
  is available.

Actions are suggestions, not server-side execution. Their tool and command
names are covered by drift tests against the actual CLI/MCP registries.

### Identity and digests

The view records:

- the selected immutable graph identity;
- build generation identity;
- the raw result schema;
- a source-result digest;
- a digest of the Agent View excluding `viewDigest` itself.

Discovery reuses `discovery_response_digest`. `compass-query` adds a canonical
`code_query_response_digest` that clones and stably sorts a response before
serialization. It includes the semantic result and limits, and excludes no
facts because `CodeQueryResponse` contains no timing fields.

The Agent View digest does not become a graph or history identity. It proves
only that the compact projection was not mutated.

## Bounds and omission policy

Version 1 uses a fixed presentation profile:

| Dimension | Default/hard limit |
| --- | ---: |
| Primary results | 12 |
| Relationships | 24 |
| Paths | 5 |
| Caveats | 16 |
| Next actions | 5 |
| Serialized Agent View | 256 KiB |
| Rendered text | 64 KiB |
| One rendered scalar | 512 Unicode scalar values |

The raw query response retains its existing independent limits. Projection
limits never mutate the raw result or source-result digest.

Priority under the Agent View byte bound is:

1. schema, request, status, identities, and blocker caveats;
2. answer and its basis;
3. primary results;
4. warning caveats;
5. best path;
6. relationships;
7. alternative paths;
8. informational caveats;
9. next actions other than ambiguity resolution.

The projector removes complete low-priority records until the view fits. It
never cuts a JSON scalar or emits invalid JSON. Every removal increments an
exact omission counter and sets `projectionTruncated=true`. If the mandatory
prefix alone exceeds 256 KiB, projection fails explicitly.

## Text projection

The default text order is:

```text
RESULT
ANSWER
CAVEATS
PRIMARY RESULTS
PATHS
RELATIONSHIPS
NEXT ACTIONS
DETAILS
```

Example:

```text
RESULT: answered
Match: exact
Evidence: exact
Execution: complete within requested bounds
Coverage: unknown

ANSWER
Found 2 incoming call or route relationships for Fixture.Target.

CAVEATS
- Coverage is unknown; absence is not proof of no additional caller.

PRIMARY RESULTS
- Fixture.Target [function] src/lib.rs:20
  id: n:target

RELATIONSHIPS
- Fixture.Caller --calls--> Fixture.Target
  src/lib.rs:10 · exact

NEXT ACTIONS
- Inspect the exact target:
  MCP task_context {"intent":"explain","target":"n:target"}

DETAILS
2 nodes · 1 relationship · 0 paths
Full provenance is available in the raw JSON/evidence view.
```

For `no_match` and `needs_resolution`, `ANSWER` explains why no exact answer is
available. It must never turn fallback candidates into a positive answer.

All headings and control-sensitive text are renderer-owned. Repository labels,
paths, diagnostic messages, and operands are escaped so they cannot forge a
heading, pagination footer, terminal control sequence, or bidirectional text.

## CLI integration

The typed command family accepts:

```text
--format text        default Agent View text
--format json        unchanged raw compass.query/1 or discovery response
--format agent-json  strict compass.query.agent-view/1
```

`--format agent-json` is incompatible with text-only `--cursor`,
`--text-budget`, and `--evidence`. The existing `--result-envelope` remains a
raw discovery JSON feature and does not wrap Agent View JSON.

Natural discovery retains the current `compass.query.discovery-text-page/2`
cursor. The implementation changes only the fixed answer-first header and
keeps the ordered entry ledger (`section`, `item`, `offset`) exactly intact.
Therefore an existing v2 cursor still identifies the same semantic next entry.

This does not reverse the crate dependency: `compass-output` renders the
validated, escaped Agent View header lines, while `compass-query` retains page
selection, cursor validation, the entry ledger, and the pagination footer. A
new `render_discovery_text_page_with_prefix` entry point accepts those fixed
prefix lines. The existing `render_discovery_text_page` remains a compatibility
wrapper with its current prefix, so library callers are not forced onto the new
presentation.

If implementation requires reordering, inserting, or removing ledger entries,
it must stop for compatibility review. It may introduce a separate Agent View
pagination contract, but it must not silently reinterpret or gratuitously
advance discovery text cursor version 2.

## MCP integration

For `search_symbols`, `get_callers`, `get_callees`, `get_impact`,
`explore_code`, `get_node`, and typed `query_graph`:

- `content` contains the bounded Agent View text instead of a count-only line;
- `structuredContent.result` remains the unchanged raw typed result;
- `structuredContent.agentView` contains `compass.query.agent-view/1`;
- `structuredContent.semanticResultDigest` is present for both code-query and
  discovery results;
- `transportTruncation` keeps its existing meaning and is calculated after the
  view is inserted.

`compass.mcp.tool-result/1` remains the transport schema because existing
fields retain their meaning and `agentView` is an optional, separately
versioned projection. The MCP reference must explicitly document that v1
transport consumers ignore unknown optional sibling projections while still
rejecting unknown `agentView` majors they choose to consume.

If compatibility evidence shows that shipped consumers require a closed MCP
envelope, the implementation must instead introduce an opt-in
`compass.mcp.tool-result/2`; it must not silently break a strict v1 reader.

Legacy compatibility tools, PR tools, Agent Graph tools, and `task_context` are
out of scope for version 1.

## Ownership

### `compass-query`

- remains authoritative for retrieval, resolution, diagnostics, and source
  result digests;
- adds only the canonical digest helper for `CodeQueryResponse`;
- owns the prefix-aware discovery page seam because cursor and entry-ledger
  semantics already live here;
- does not construct Agent View text or select display priorities.

### `compass-output`

- owns `AgentQueryView`, validation, projection limits, deterministic summary
  templates, safe text rendering, and view digest;
- never opens a graph, runs search, or resolves an operand;
- follows the existing PR-review renderer pattern: full machine IDs, compact
  readable text, explicit omissions, and bounded output.

### `compass-cli`

- captures the invocation operands and graph identities;
- calls the shared projector and renderer;
- preserves raw JSON behavior and exit codes.

### `compass-mcp`

- captures the same invocation context;
- uses the shared view for text and structured projection;
- keeps tool validation, raw results, protocol errors, and transport bounds.

## Compatibility

Unchanged:

- `compass.query/1`;
- `compass.query.discovery/1`;
- `compass.query.discovery-result/1`;
- graph and history schemas;
- query ranking, resolution, diagnostics, and limits;
- current raw CLI JSON;
- the discovery cursor's semantic position contract.

Additive:

- `compass.query.agent-view/1`;
- CLI `--format agent-json`;
- optional MCP `agentView`;
- a code-query semantic result digest;
- answer-first human/MCP text.

Human text is documented as presentation, not a durable parser contract.
Nevertheless, release notes must call out the new headings and tell automation
to use raw JSON or Agent View JSON.

## Validation and invariants

`AgentQueryView::validate` rejects:

- an unknown schema;
- empty graph/generation/result/view identities;
- an invalid digest;
- a relationship or path endpoint absent from both the raw result and the
  view's retained entity references;
- a basis reference that does not exist in the raw result;
- `answered` combined with `ambiguous` or `none` match state;
- `no_match` without a no-match caveat;
- `needs_resolution` without at least two retained candidates or an explicit
  ambiguity omission;
- `sourceExecution=complete` when the raw result is truncated;
- `projection=complete` when any projection omission is nonzero;
- a `task_context` next action without one proven exact target;
- output above its byte or item bounds.

Projection tests must also prove that every statement is a closed template
whose interpolated values are request or raw-result data.

## Test matrix

At minimum, cover:

1. exact search;
2. no exact match with fuzzy candidates;
3. ambiguous duplicate names;
4. exact target with zero callers;
5. exact callers/callees with mixed evidence;
6. impact with reverse traversal steps;
7. exact node trail and direction mismatch;
8. broad discovery with several seeds;
9. source execution truncation;
10. projection truncation with exact omission counts;
11. incomplete coverage and stale-source blocker placement;
12. control characters, bidi controls, and forged heading/footer strings;
13. identical JSON/store results producing identical Agent Views;
14. CLI `agent-json` and MCP `agentView` semantic parity;
15. raw CLI JSON and MCP `result` remaining unchanged;
16. current discovery cursor v2 continuing to the same next entry.

## Rollout

1. Land the contract, validator, projector, and output tests without wiring a
   public adapter.
2. Switch typed CLI text and add `agent-json`; retain raw JSON.
3. Replace MCP count-only text and add the optional structured projection.
4. Update assistant guidance, command/MCP/output references, compatibility,
   and changelog.
5. Run query relevance and backend-parity gates to prove presentation work did
   not alter retrieval.

## Alternatives considered

### Teach agents to join the existing arrays

Rejected. It repeats orchestration in every client, consumes context, and makes
critical diagnostics easy to miss.

### Replace raw query JSON with Agent View JSON

Rejected. Raw results are the audit contract and contain evidence the compact
view intentionally omits.

### Generate natural-language answers with a model

Rejected. It violates local-first operation, adds nondeterminism and
credentials, and can turn uncertain evidence into plausible prose.

### Put the projector in the CLI or MCP crate

Rejected. The two surfaces would drift and reusable presentation behavior
would live above its ownership boundary.

### Change discovery pagination to match the new visual order

Rejected for version 1. The answer-first fixed header provides the usability
gain while preserving the current cursor's semantic entry positions.

## Success criteria

- A tool consumer can determine outcome, ambiguity, truncation, and coverage
  without joining raw arrays.
- The first MCP text content contains the result state and headline, not only
  counts.
- Every positive headline has exact raw-result basis references.
- No-match and ambiguous results never publish a positive answer.
- CLI Agent JSON and MCP Agent View are byte-equivalent after transport-only
  fields are removed.
- Raw JSON responses remain semantically and byte-order equivalent.
- Equivalent JSON/store queries produce identical Agent Views.
- All output remains inside named bounds and deterministic across repeats.
