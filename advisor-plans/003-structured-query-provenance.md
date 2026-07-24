# Plan 003: Return structured provenance from path and discovery queries

> **Executor instructions:** Follow each verification gate. Keep compatibility
> text rendering as an Adapter over a new structured result; do not duplicate
> query semantics. Do not push or open a pull request.
>
> **Drift check:** Run
> `git diff --stat 3837b411..HEAD -- crates/compass-query/src/traversal.rs crates/compass-graph/src/analyze.rs crates/compass-output/src/report.rs crates/compass-mcp/src/lib.rs crates/compass-cli/src`
> and stop if another structured query-result contract has landed.

## Status

- **Priority:** P1
- **Effort:** M
- **Risk:** MED
- **Depends on:** plan 001
- **Category:** architecture, direction
- **Planned at:** `3837b411`, 2026-07-23

## Why this matters

Compass preserves confidence and provenance in graph records but drops much of
it on the assistant-facing path, natural-query, surprise-report, and MCP
surfaces. Users can see `INFERRED` without knowing the numeric score, evidence
site, origin, or extractor. CLI and MCP also implement path presentation in
different Modules. One structured evidence Interface should preserve facts;
text and transport Adapters should decide presentation and limits.

## Current state

- `crates/compass-graph/src/analyze.rs:87-98` defines
  `SurpriseConnection` without `confidence_score`, source location, origin, or
  provider identity.
- `crates/compass-output/src/report.rs:218-236` renders only the confidence
  category for each surprise.
- `crates/compass-query/src/traversal.rs:77-155` resolves and renders paths.
- `crates/compass-mcp/src/lib.rs:757-852` repeats endpoint resolution,
  traversal, edge direction, confidence, and text rendering.
- Graphify main renders numeric inferred confidence in
  `graphify/report.py:27-29,63-71`.
- `docs/concepts/provenance.md:132-156` requires provenance to remain
  interpretable and warns against treating source-specific scores as calibrated
  truth.

Use `compass-model` records as the authoritative source. Do not parse rendered
text to recover evidence.

## Commands

| Purpose | Command | Expected result |
|---|---|---|
| Query tests | `cargo test -p compass-query --all-targets --locked` | all pass |
| Graph tests | `cargo test -p compass-graph --all-targets --locked` | all pass |
| MCP tests | `cargo test -p compass-mcp --all-targets --locked` | all pass |
| CLI contract | `cargo test -p compass-cli --test compass_product --locked` | all pass |
| Lint | `cargo clippy -p compass-query -p compass-graph -p compass-output -p compass-mcp -p compass-cli --all-targets --locked -- -D warnings` | exit 0 |

## Scope

**In scope:**

- `crates/compass-query/src/traversal.rs` and query exports.
- `crates/compass-graph/src/analyze.rs`.
- `crates/compass-output/src/report.rs`.
- `crates/compass-mcp/src/lib.rs`.
- Thin CLI Adapter changes under `crates/compass-cli/src/`.
- Focused tests in those crates.

**Out of scope:**

- New CompassQL syntax.
- Hyperedge query semantics.
- Changing extraction confidence calculations.
- Calibrating scores across providers.
- Changing authoritative edge multiplicity.

## Git workflow

- Branch: `advisor/003-structured-query-provenance`
- Prefer one commit for the typed result and one for Adapters/tests.
- Example: `Centralize structured path evidence`.
- Do not push or open a PR.

## Steps

### Step 1: Characterize current CLI and MCP behavior

Add fixtures for:

- exact path;
- ambiguous endpoint candidates;
- missing endpoint;
- reversed stored edge;
- inferred edge with `confidence_score`, `source_file`,
  `source_location`, `_origin`, and provider/extractor attributes;
- absent optional attributes.

Capture current compatibility text separately from the future structured
payload.

**Verify:** characterization tests pass before refactoring.

### Step 2: Introduce a typed evidence result

In `compass-query`, define a versioned result that includes:

- resolved endpoints and alternative candidates;
- ordered nodes;
- ordered edge evidence with stored source/target;
- relation, confidence category, optional numeric score;
- source file/location, origin, and provider/extractor when present;
- warnings and typed failure kind;
- truncation/budget metadata.

Keep transport-neutral data types. This is the deep Module Interface; it must
not contain terminal strings or MCP response objects.

**Verify:** unit tests serialize a stable versioned JSON shape and preserve
missing optional values without invention.

### Step 3: Make CLI and MCP thin Adapters

Have CLI text rendering consume the structured result. Have MCP return:

- the compatibility text where existing tools require it;
- a versioned structured payload through an additive tool/result surface.

Remove independent MCP traversal/presentation logic once parity tests prove the
Adapter preserves intentional transport differences such as maximum depth.

**Verify:** query, MCP, and product-contract tests pass; repository search shows
one implementation of endpoint/path evidence selection.

### Step 4: Carry evidence into surprising connections

Extend `SurpriseConnection` with optional evidence fields. Populate them from
the selected edge and render numeric confidence plus source evidence when
available.

Label scores as source-specific evidence. Never call them probabilities unless
the provider contract explicitly does.

**Verify:** add a report fixture requiring `INFERRED 0.82` plus source location,
and a fixture proving absent scores do not render fabricated defaults.

### Step 5: Document the contract

Update the query/MCP/provenance reference pages with:

- schema versioning;
- optionality;
- stored direction;
- score interpretation;
- compatibility text versus structured evidence.

**Verify:** all referenced commands and paths exist; plan 005's docs checker
should later enforce this.

## Test plan

- Exact and ambiguous endpoints.
- Incoming/outgoing/reversed edges.
- Every optional provenance field present/absent.
- Stable JSON ordering and schema version.
- CLI compatibility text snapshots.
- MCP size and depth limits.
- No secret/config values copied from unrelated attributes.

## Done criteria

- [ ] One structured path-evidence Interface owns query semantics.
- [ ] CLI and MCP are presentation/transport Adapters.
- [ ] Numeric confidence and evidence sites reach report and structured query
  results.
- [ ] Existing compatibility text remains covered.
- [ ] All target tests, clippy, and format checks pass.

## STOP conditions

- Existing MCP protocol cannot add a versioned payload without a breaking
  transport change.
- Provider identity has no stable stored attribute contract.
- Exact compatibility requires MCP and CLI to keep different traversal
  membership, not just different limits/presentation.
- The work expands into hyperedge or edge-multiplicity redesign.

## Maintenance notes

New query consumers should depend on the structured result, never scrape text.
Reviewers must verify numeric scores are presented as evidence metadata rather
than universal probability.
