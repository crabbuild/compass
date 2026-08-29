# Proposal: migrate the MCP SDK to rmcp 3.1.4

## Why

Compass currently builds its MCP server on rmcp 2.2.0. The next MCP changes
depend on rmcp 3.x protocol models and discovery support, so the dependency must
move first without combining the migration with transport or result redesign.

## What Changes

- Pin the workspace dependency to exactly rmcp 3.1.4.
- Port the existing handler and transports to the 3.x API, with stdio parity
  established first.
- Add a 2.2-derived discovery golden that freezes the existing server identity,
  tool names, tool input schemas, and resource inventory.
- Measure and review the transitive dependency, license, and unsafe-policy delta.

## Compatibility

The public MCP tools, schemas, resources, CLI flags, and stdio behavior remain
unchanged. This change does not introduce the 2026 stateless HTTP contract or a
new result envelope; those are separate follow-up changes.
