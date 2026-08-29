# Proposal: add typed MCP navigation result envelopes

## Why

Compass's typed code-query payload preserves detailed graph evidence, but its
core navigation tools do not yet expose the common MCP-facing identity,
freshness, confidence, warning, and pagination contract clients need.

## What Changes

- Wrap results from `search_symbols`, `get_callers`, `get_callees`, and
  `get_impact` in `compass.code_context.v1`.
- Advertise a closed structured output schema for those four tools.
- Keep MCP `resultType: "complete"` as the protocol discriminator; the Compass
  schema identifier remains a separate `schema` field.
- Preserve the existing `compass.query/1` payload unchanged under `data`.
- Keep all tool names and discovery ordering stable.
- Mark every remaining text-only result deprecated in discovery while keeping
  it callable until a typed replacement is separately reviewed.

## Compatibility

This is an additive structured-result contract at the MCP boundary, but it
changes the top-level `structuredContent` shape for four tools. Clients that
read those results must move their existing query-response parsing beneath
`data`. Text fallback content and tool names remain unchanged.
