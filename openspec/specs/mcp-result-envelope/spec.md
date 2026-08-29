# mcp-result-envelope Specification

## Purpose
TBD - created by archiving change mcp-result-envelope. Update Purpose after archive.

## Requirements

### Requirement: core navigation tools return a common versioned envelope

Compass SHALL return `compass.code_context.v1` structured content for
`search_symbols`, `get_callers`, `get_callees`, and `get_impact`, containing
repository and generation identity, evidence-scoped freshness, the prior typed
query payload, evidence and confidence summaries, truncation state, and
warnings.

#### Scenario: invoke a core navigation tool

- **WHEN** a client successfully invokes any of the four core navigation tools
- **THEN** the MCP result is `resultType: "complete"` and its structured content
  matches the tool's advertised output schema with `schema` equal to
  `compass.code_context.v1`

### Requirement: existing typed navigation semantics are preserved

Compass SHALL embed the complete prior `compass.query/1` response under `data`
without discarding or reinterpreting evidence, direction, multiplicity,
ambiguity, bounds, diagnostics, truncation, or deterministic ordering.

#### Scenario: compare a result before and after the envelope

- **WHEN** the same graph and bounded request are evaluated through a core
  navigation tool
- **THEN** removing the envelope leaves the exact pre-envelope typed response

### Requirement: protocol and product discriminators remain separate

Compass SHALL use MCP `resultType` only as the protocol-level completion
discriminator and SHALL carry the Compass schema name in structured content's
separate `schema` field.

#### Scenario: serialize an MCP 2026 tool response

- **WHEN** rmcp serializes a successful core navigation result for an MCP
  2026-07-28 peer
- **THEN** top-level `resultType` is `complete` and structured content does not
  redefine or overload that field

### Requirement: discovery ordering and tool names remain stable

Compass SHALL preserve the existing deterministic tool ordering and names while
advertising closed output schemas for the four migrated tools.

#### Scenario: list tools repeatedly

- **WHEN** a client requests the tool list more than once
- **THEN** tools appear in the same order with unchanged names and the four
  migrated tools expose equivalent output schemas

### Requirement: remaining text results are explicitly deprecated

Compass SHALL mark every remaining text-only tool result, plus the explicit
text-traversal mode of `query_graph`, deprecated in discovery while retaining
its current name and behavior until a typed replacement is separately shipped.

#### Scenario: discover a legacy text tool

- **WHEN** a client lists tools
- **THEN** each legacy text result has a deprecation description and Compass
  deprecation metadata, while typed tools are not mislabeled

### Requirement: the final envelope obeys the response-byte bound

Compass SHALL measure the encoded structured-content envelope against the
request's `maxResponseBytes` value and fail explicitly rather than publishing a
successful oversized result.

#### Scenario: envelope overhead crosses the bound

- **WHEN** the nested query result is within the byte bound but adding the MCP
  envelope would exceed it
- **THEN** Compass returns the query-response limit failure and no structured
  result larger than the requested bound
