# Plan 025: Make query output answer-first for coding agents

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report; do not improvise. When done, update this plan's row in
> `advisor-plans/README.md` unless a reviewer told you they maintain the index.
>
> **Normative design**: Read
> [`advisor-plans/designs/025-agent-readable-query-output-design.md`](designs/025-agent-readable-query-output-design.md)
> completely before editing. This plan restates the implementation-critical
> decisions, but the design owns the full schema and outcome rules.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat b14f6907..HEAD -- \
>   crates/compass-model/src/query_contract.rs \
>   crates/compass-query/src/lib.rs \
>   crates/compass-query/src/code_query.rs \
>   crates/compass-query/src/discovery_text.rs \
>   crates/compass-output/src/lib.rs \
>   crates/compass-output/src/review.rs \
>   crates/compass-cli/src/code_query_commands.rs \
>   crates/compass-cli/src/lib.rs \
>   crates/compass-cli/src/help.rs \
>   crates/compass-mcp/src/lib.rs \
>   crates/compass-mcp/src/code_query.rs \
>   crates/compass-cli/assets/compass-skill \
>   crates/compass-cli/assets/compass-integrations \
>   docs COMPATIBILITY.md MIGRATION.md CHANGELOG.md
> ```
>
> If an Agent View, answer-first query projection, changed MCP result envelope,
> or changed discovery pagination ledger has landed, reconcile it with the
> design and stop rather than creating a parallel contract.

## Status

- **Priority**: P1 — highest-leverage agent-facing output improvement
- **Effort**: L (six implementation phases)
- **Risk**: MED — public text and additive machine output change, raw query semantics stay fixed
- **Depends on**: none
- **Coordinates with**: Plan 018 MCP workflow prompts; if both are selected, land this first
- **Category**: direction / DX / output
- **Planned at**: commit `b14f6907`, 2026-09-16

## Why this matters

Compass already retrieves source-grounded nodes, relationships, paths, and
diagnostics. An agent still pays a large interpretation cost: MCP text gives
only counts, raw JSON requires joining arrays by ID, and caveats that invalidate
an answer can arrive after graph details. A deterministic Agent Query View
turns the same evidence into an answer-first, bounded result without changing
ranking, resolution, provenance, or graph semantics.

The intended outcome is not prettier prose. It is a typed result in which an
agent can determine, without guesswork, whether there is an exact answer,
whether evidence is inferred or ambiguous, whether execution was truncated,
what source-backed entities and relationships matter, and what exact bounded
action to take next.

## Current state

### Authoritative query contracts are flat evidence collections

`crates/compass-model/src/query_contract.rs:358-376`:

```rust
pub struct DiscoveryQueryResponse {
    pub schema: String,
    pub question: String,
    // direction, scope, and traversal fields
    pub seeds: Vec<DiscoverySeed>,
    pub nodes: Vec<QueryNode>,
    pub edges: Vec<DiscoveryEdge>,
    pub diagnostics: Vec<QueryDiagnostic>,
    pub limits: DiscoveryLimits,
    pub stats: DiscoveryStats,
    pub omissions: DiscoveryOmissions,
    pub truncated: bool,
}
```

`crates/compass-model/src/query_contract.rs:535-548`:

```rust
pub struct CodeQueryResponse {
    pub schema: String,
    pub operation: CodeQueryOperation,
    pub results: Vec<SearchHit>,
    pub nodes: Vec<QueryNode>,
    pub edges: Vec<QueryEdge>,
    pub files: Vec<QueryFile>,
    pub paths: Vec<QueryPath>,
    pub diagnostics: Vec<QueryDiagnostic>,
    pub limits: CodeQueryLimits,
    pub truncated: bool,
}
```

These remain the authoritative raw contracts. Do not add display-only fields
to either schema.

### MCP text discards the useful evidence

`crates/compass-mcp/src/lib.rs:1042-1072` and `1085-1120` render only counts:

```rust
let text = format!(
    "{:?}: {} nodes, {} edges, {} paths{}",
    response.operation,
    response.nodes.len(),
    response.edges.len(),
    response.paths.len(),
    // ...
);
```

The raw evidence is available in `structuredContent`, but an agent or client
that prioritizes MCP text cannot identify the target, relationships, caveats,
or next action.

### CLI rendering is duplicated above the output crate

`crates/compass-cli/src/code_query_commands.rs:226-311` constructs typed-query
text locally. It joins node labels for paths, but it prints all nodes before
non-no-match diagnostics and has no versioned compact machine projection.

### Discovery text puts mechanism before answer

`crates/compass-query/src/discovery_text.rs:156-218` prints match signal, seed
terms, counts, direction, ambiguity count, coverage, truncation, traversal,
relationship contexts, and scope before the first entity. Diagnostics are
ordinary paginated entries at lines 551-568. The current cursor is
`compass.query.discovery-text-page/2` and binds to the stable entry ledger by
`section`, `item`, and `offset`.

### Existing output precedent

`crates/compass-output/src/review.rs` is the pattern to follow:

- typed machine data retains full IDs;
- readable text shortens presentation-only identifiers;
- missing evidence and omissions are explicit;
- render size is bounded;
- text and JSON derive from the same verified domain result.

`crates/compass-output/tests/pr_review.rs:100-133` verifies machine/human
projection parity without requiring human text to carry every full identifier.

### Product constraints

The implementation must retain these documented rules:

- `docs/design/principles.md`: structure before similarity, evidence stays
  attached, deterministic output, bounded work, and explicit machine contracts;
- `docs/implementation/extending-compass.md`: output transformations belong in
  `compass-output`; direction, parallel edges, provenance, escaping, bounds,
  and semantic-equivalence tests are mandatory;
- `AGENTS.md`: CLI/MCP stay thin; unknown majors fail explicitly; a limit is
  not an empty result; ambiguous identities are never first-match selected.

## Target contract

Add strict schema `compass.query.agent-view/1` in `compass-output` with these
top-level fields:

```text
schema, request, status, answer,
primaryResults, relationships, paths,
caveats, nextActions, omissions, identity,
sourceTruncated, projectionTruncated
```

Critical status fields remain separate:

```text
resultState      answered | candidates | needs_resolution | no_match | no_path
matchState       exact | fuzzy | ambiguous | none | not_applicable | unknown
evidenceState    exact | inferred | mixed | ambiguous | none
sourceExecution  complete | partial
projection       complete | partial
coverage         incomplete | unknown
```

The fixed profile caps primary results at 12, relationships at 24, paths at 5,
caveats at 16, next actions at 5, serialized view size at 256 KiB, rendered
text at 64 KiB, and one scalar at 512 Unicode scalar values. Every omission is
counted; required status/identity/blocker records are never dropped.

Raw `compass.query/1`, `compass.query.discovery/1`, and raw CLI JSON stay
unchanged.

## Commands you will need

Use one target directory for this exact worktree and no other checkout:

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Target preflight | `test -d /Volumes/Workspace && mkdir -p /Volumes/Workspace/crabbuild-target/compass-7af7-agent-view && test -w /Volumes/Workspace/crabbuild-target/compass-7af7-agent-view` | exit 0 |
| Query tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view cargo test -p compass-query --locked` | all pass |
| Output tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view cargo test -p compass-output --locked` | all pass |
| CLI contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view cargo test -p compass-cli --test code_query_cli --locked` | all pass |
| MCP contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view cargo test -p compass-mcp --test code_query_tools --locked` | all pass |
| Install assets | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view cargo test -p compass-cli --test install_cli --locked` | all pass |
| Focused Clippy | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view cargo clippy -p compass-query -p compass-output -p compass-cli -p compass-mcp --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Query qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view python3 scripts/qualify_query_relevance.py` | thresholds and backend parity pass |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Workspace baseline | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view cargo clippy --workspace --lib --bins --locked -- -D warnings && CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view cargo test --workspace --lib --bins --locked` | exit 0 |

## Scope

**In scope**:

- `crates/compass-query/src/lib.rs`
- `crates/compass-query/src/discovery_text.rs`
- a focused query digest module if extracting it keeps ownership clear
- `crates/compass-query/tests/query_contract.rs` or a new focused digest test
- `crates/compass-output/src/agent_query.rs` (new)
- `crates/compass-output/src/lib.rs`
- `crates/compass-output/tests/agent_query.rs` (new)
- `crates/compass-cli/src/code_query_commands.rs`
- discovery formatting integration in `crates/compass-cli/src/lib.rs`
- `crates/compass-cli/src/help.rs`
- `crates/compass-cli/tests/code_query_cli.rs`
- `crates/compass-mcp/src/lib.rs`
- `crates/compass-mcp/src/code_query.rs` only if invocation context belongs there
- `crates/compass-mcp/tests/code_query_tools.rs`
- applicable assistant assets and install tests
- `docs/implementation/query-engine.md`
- `docs/reference/commands.md`
- `docs/reference/outputs.md`
- `docs/guides/integrating-compass.md`
- `docs/guides/exploring-a-codebase.md`
- `COMPATIBILITY.md`, `CHANGELOG.md`, and `MIGRATION.md` only if the final
  compatibility decision requires user action
- `advisor-plans/README.md` status update

**Out of scope**:

- graph extraction, ranking, search, traversal, resolution, or graph schemas;
- CompassQL result rendering;
- PR, Agent Graph, legacy compatibility, or task-context tools;
- source excerpts in the compact view;
- a model/provider-generated answer;
- MCP workflow prompts from Plan 018;
- ranked execution flows from Plan 017;
- changing the default query limits;
- deleting or renaming existing tools;
- advancing discovery cursor version 2 while its entry ledger remains intact.

## Git workflow

- Suggested branch: `codex/agent-readable-query-output`
- Use focused conventional commits matching repository history, for example:
  - `feat(output): add agent query view contract`
  - `feat(cli): render answer-first query output`
  - `feat(mcp): expose agent-readable query results`
  - `docs(query): document the agent view contract`
- Do not push, open a PR, merge, or release unless explicitly instructed.

## Phase 1: Add canonical source-result identity

**Context**: Discovery already has `discovery_response_digest`; ordinary
`CodeQueryResponse` values do not have a public semantic digest. Agent View
must bind to its exact raw result without making the renderer own query
identity.

### Changes

1. Add `code_query_response_digest(&CodeQueryResponse)` in `compass-query`.
2. Clone the response, call `sort_stable`, serialize the full response, and
   return lowercase SHA-256 without a prefix, matching discovery's helper.
3. Re-export it from `crates/compass-query/src/lib.rs`.
4. Add tests proving:
   - stable-equivalent ordering yields one digest;
   - a changed node, edge direction, path, diagnostic, truncation flag, or
     limit changes the digest;
   - repeated JSON/store results have the same digest.
5. Do not add the digest to `CodeQueryResponse`; it belongs in transport/view
   envelopes.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view \
  cargo test -p compass-query --locked code_query_response_digest
```

Expected: all digest tests pass and no query result fixture changes.

## Phase 2: Implement the strict Agent Query View

**Context**: Presentation-only derivation belongs in `compass-output`, which
already depends on `compass-model` and `compass-query`. Do not make
`compass-query` depend on `compass-output`.

### Changes

1. Add `crates/compass-output/src/agent_query.rs` and export its public types
   and functions from `lib.rs`.
2. Implement the exact contract, enums, fixed limits, outcome precedence,
   diagnostic-to-caveat mapping, stable sorting, omission accounting, source
   result identity, and view digest from the design.
3. Accept an explicit invocation context containing:
   - operation;
   - ordered question/operands;
   - graph identity;
   - build generation identity;
   - optional discovery continuation cursor and evidence-hidden flag.
4. Provide separate pure constructors for `CodeQueryResponse` and
   `DiscoveryQueryResponse`. They may inspect only the request context and
   response; they must not open a graph or re-run resolution.
5. Inline readable endpoint labels and path steps while retaining full IDs,
   direction, weakest resolution, weakest confidence, and source sites.
6. Generate only the closed headline templates and bounded next actions in the
   design. Use argv arrays and JSON objects, never a shell command string.
7. Implement `AgentQueryView::validate`, strict `from_json`, canonical JSON,
   and `viewDigest` verification.
8. Follow `review.rs` for bounded output and compact/full identifier behavior.

### Required unit/integration cases

Create `crates/compass-output/tests/agent_query.rs` covering:

- exact search;
- no exact match with fallback candidates;
- ambiguous candidates with no selected answer;
- exact zero callers;
- exact and mixed-evidence caller/callee relationships;
- impact paths traversed opposite published edge direction;
- exact node trail, no path, and direction mismatch;
- broad discovery;
- source truncation versus projection truncation;
- incomplete coverage and stale-source caveats;
- invalid basis/endpoint references;
- deterministic ordering and digest;
- control characters, bidirectional controls, and forged section/footer text;
- the mandatory prefix exceeding its byte limit as an explicit error.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view \
  cargo test -p compass-output --test agent_query --locked
```

Expected: all Agent View projection, validation, escaping, and bound tests pass.

## Phase 3: Add shared answer-first text rendering

**Context**: CLI and MCP must not maintain separate summaries. The renderer
must derive solely from a validated `AgentQueryView`.

### Changes

1. Add `render_agent_query_text` and a smaller
   `render_agent_query_header_lines` in `compass-output`.
2. Render sections in this order:

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

3. Put blocker and warning caveats before primary results. Never paginate them
   away.
4. Escape every untrusted scalar and enforce the 64 KiB text bound using the
   design's whole-record removal order.
5. For natural discovery pagination, add a new
   `render_discovery_text_page_with_prefix` (or equivalently named) function in
   `compass-query` that accepts already escaped fixed prefix lines from the
   caller, then uses the existing entry ledger and footer unchanged. Keep the
   current `render_discovery_text_page` as a compatibility wrapper using its
   existing prefix.
6. Do not reorder, insert, or remove discovery page entries. Preserve
   `section`, `item`, `offset`, cursor validation, and
   `compass.query.discovery-text-page/2`.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view \
  cargo test -p compass-query --locked discovery_text
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view \
  cargo test -p compass-output --test agent_query --locked text
```

Expected: old v2 cursors still address the same next entry; answer-first text
passes ordering, escaping, and size assertions.

## Phase 4: Wire CLI text and Agent JSON without changing raw JSON

**Context**: `code_query_commands.rs` currently renders text locally, while the
natural-discovery path in `lib.rs` owns cursor handling. Both adapters have
access to the query engine identities required by Agent View.

### Changes

1. Refactor typed command execution to retain a bounded invocation descriptor
   and the engine's `graph_identity()` and `build_generation_identity()` beside
   the raw response.
2. Replace the local `render_text` implementation with the shared output
   projector/renderer.
3. Add `--format agent-json` to `ask`, `search`, `callers`, `callees`, `impact`,
   `explore`, `node`, and natural discovery.
4. Keep `--format json` byte-semantically equivalent to the existing raw query
   response. Do not wrap or add fields.
5. For natural discovery text:
   - construct one Agent View from the full response;
   - render its fixed answer/caveat header;
   - pass that header to the prefix-aware discovery page renderer;
   - retain the current entries, footer, next cursor, semantic result digest,
     and evidence behavior.
6. Reject `agent-json` with `--cursor`, `--text-budget`, `--evidence`, or
   `--result-envelope` using an actionable usage error.
7. Update help text and examples.

### Required CLI tests

Extend `crates/compass-cli/tests/code_query_cli.rs` to assert:

- result state and answer precede nodes/edges;
- ambiguity/no-match caveats precede fallback candidates;
- exact zero-neighbor queries are `answered`, not `no_match`;
- `agent-json` parses as `compass.query.agent-view/1` and validates;
- raw JSON equals the pre-projection shape and contains no `agentView`;
- discovery page 1 and a carried v2 cursor return disjoint unchanged detail
  entries with the same semantic result identity;
- control text cannot forge `RESULT`, `ANSWER`, `CAVEATS`, or `Pagination`;
- invalid format combinations fail on stderr with nonzero status.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view \
  cargo test -p compass-cli --test code_query_cli --locked
```

Expected: all typed and natural query CLI tests pass; raw JSON assertions are
unchanged except for new tests.

## Phase 5: Replace MCP count-only text and add the structured projection

**Context**: MCP is the primary coding-agent interface. It must expose the same
view as CLI without weakening its raw audit result or transport bounds.

### Changes

1. In typed and discovery tool invocation, capture request operands plus the
   query engine graph/build-generation identities.
2. Project one `AgentQueryView` and use `render_agent_query_text` for MCP
   `content`.
3. Extend `transport_envelope_with_digest` through a typed helper that accepts
   an optional Agent View and emits it as sibling `agentView`.
4. Preserve the existing `result`, `transportTruncation`, and discovery
   `semanticResultDigest` meanings. Add the code-query source-result digest to
   `semanticResultDigest` for typed code-query tools.
5. Recalculate `requiredBytes` after adding the view. Never truncate raw result
   or Agent View to satisfy the MCP bound; return the existing explicit
   transport-limit error.
6. Limit the projection to the seven typed query tools named in the design.
   Do not change PR, legacy, task-context, or Agent Graph tools.
7. Add a compatibility test proving that removing `agentView` and the new
   code-query digest from the envelope leaves the old raw result unchanged.
8. Add CLI/MCP parity fixtures: for an identical graph and request,
   CLI `agent-json` equals MCP `agentView` byte-semantically after transport-only
   fields are removed.

### Compatibility gate

Before committing the envelope change, inspect current documentation and any
typed consumer in the repository. If a shipped consumer rejects unknown
siblings in `compass.mcp.tool-result/1`, STOP. Introduce an opt-in v2 envelope
and add a migration plan instead of changing the v1 envelope silently.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view \
  cargo test -p compass-mcp --test code_query_tools --locked
```

Expected: agent-readable text, Agent View/raw-result parity, protocol errors,
and transport-bound tests all pass.

## Phase 6: Document, qualify, and prevent drift

### Changes

1. Document `compass.query.agent-view/1`, its status dimensions, fixed bounds,
   raw-result relationship, and examples in the query/output references.
2. Update command help/reference for `agent-json` and state plainly that human
   text may evolve while raw and Agent View JSON are versioned.
3. Update MCP documentation to define `agentView` as an optional separately
   versioned sibling and to preserve the raw `result` as authoritative.
4. Update assistant assets so agents:
   - read `RESULT`, `ANSWER`, and `CAVEATS` before details;
   - never treat fallback candidates as an answer;
   - follow exact next-action arguments rather than reconstructing a command;
   - use raw/evidence output for an audit.
5. Add an install-tree drift test for the changed assistant guidance.
6. Add a `CHANGELOG.md` entry and `COMPATIBILITY.md` contract note. Update
   `MIGRATION.md` only if an MCP envelope major or user action is required.
7. Run query qualification to prove ranking, no-answer behavior, path
   direction, backend parity, and deterministic raw results did not change.
8. Mark Plan 025 `DONE` only after all targeted and workspace checks pass.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view \
  cargo test -p compass-cli --test install_cli --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-7af7-agent-view \
  python3 scripts/qualify_query_relevance.py
sh scripts/check_product_boundary.sh
```

Expected: assistant assets match, relevance thresholds/backend parity pass, and
the product boundary remains clean.

## Test plan

Use these existing tests as structural patterns:

- `crates/compass-output/tests/pr_review.rs` for readable/full projection
  parity, bounded omissions, and compact identifiers;
- `crates/compass-query/src/discovery_text.rs` tests for cursor binding,
  injection-resistant text, pagination, and digest stability;
- `crates/compass-cli/tests/code_query_cli.rs` for typed/raw CLI contracts,
  exact/no-match behavior, and end-to-end paths;
- `crates/compass-mcp/tests/code_query_tools.rs` for raw structured content,
  store parity, semantic digest, protocol errors, and transport behavior.

The new test suite must prove these cross-surface invariants:

- every entity/relationship/path/basis ID exists in the raw result;
- no positive answer for no-match or ambiguous input;
- edge and path direction is never reversed by presentation;
- source and projection truncation remain distinct;
- coverage defaults to unknown, never complete;
- blocker caveats occur before graph detail in text;
- Agent View is deterministic across repeated and JSON/store executions;
- Agent View remains within all item/byte bounds;
- raw results do not change;
- discovery v2 cursors retain their semantic positions.

## Done criteria

- [ ] `compass.query.agent-view/1` is strict, validated, bounded, and documented.
- [ ] All seven typed MCP query tools return answer-first text from the shared renderer.
- [ ] MCP structured content retains the raw result and exposes the same Agent View as CLI.
- [ ] Typed CLI commands default to Agent View text and accept `--format agent-json`.
- [ ] Natural discovery has an answer-first fixed header without changing its v2 entry ledger.
- [ ] Raw `--format json` output remains semantically and order equivalent.
- [ ] No-match, ambiguity, direction mismatch, stale source, incomplete coverage, and both truncation kinds have regression tests.
- [ ] JSON/store backend parity and repeated-run determinism pass.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] Focused Clippy and all targeted tests in the command table pass.
- [ ] Query relevance qualification passes unchanged thresholds.
- [ ] Workspace lib/bin Clippy and tests pass.
- [ ] `sh scripts/check_product_boundary.sh` passes.
- [ ] `git diff --check` passes and `git status --short` contains only in-scope changes.
- [ ] Documentation, compatibility, changelog, assistant assets, and plan status are updated.

## STOP conditions

Stop and report; do not improvise if:

- producing an accurate headline requires re-running resolution or traversal in
  `compass-output`;
- the view would need to modify `CodeQueryResponse` or
  `DiscoveryQueryResponse` to work;
- a display rule would select the first ambiguous or fuzzy candidate;
- coverage would have to be labeled complete without direct completeness
  evidence;
- the implementation needs a model, embeddings, credentials, or network;
- the raw CLI JSON or MCP `result` changes;
- discovery page entry ordering or identity must change; preserve cursor v2 or
  stop for a separate compatibility design;
- a strict shipped MCP v1 consumer cannot accept the optional `agentView`
  sibling;
- the mandatory status/identity/blocker prefix cannot fit the Agent View byte
  bound;
- any test can pass only by weakening direction, provenance, ambiguity,
  truncation, or unknown-major validation;
- `/Volumes/Workspace` is unavailable or the dedicated target directory is not
  writable.

## Maintenance notes

- Any new `QueryDiagnosticCode` must add a deliberate caveat severity/effect
  mapping or fail an exhaustiveness test.
- Any new typed query operation must add an invocation role mapping, headline
  template, primary-result rule, CLI/MCP parity test, and documentation before
  it can advertise Agent View support.
- Tool or command renames must update next-action drift tests. Do not leave
  executable-looking stale actions in a versioned view.
- Agent View profile limits and ordering affect output digests and must be
  reviewed as contract behavior.
- Plan 018 prompt templates should consume `RESULT`, `ANSWER`, `CAVEATS`, and
  exact next actions after this plan lands; they should not replicate the
  interpretation rules in prompt prose.
- A later real-repository agent workflow benchmark may evaluate task success,
  tool calls, and context bytes. It is deliberately outside this output plan.
