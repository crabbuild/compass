# Proposal: make MCP HTTP stateless for protocol 2026-07-28

## Why

Compass still defaults its Streamable HTTP endpoint to the pre-2026 session
handshake even though rmcp 3.1.4 supports the MCP 2026-07-28 per-request
contract. The default must become stateless without weakening authentication,
host validation, size limits, or project selection.

## What Changes

- Serve HTTP requests without `Mcp-Session-Id` and require the 2026-07-28
  `Mcp-Protocol-Version`, `Mcp-Method`, and request metadata contract.
- Advertise only MCP 2026-07-28 from the HTTP service while leaving stdio's
  already-qualified compatibility contract unchanged.
- Keep `--stateless` accepted as a compatibility spelling for the new default.
- Accept `--session-timeout` with a deprecation warning through the 0.4.x
  release line, ignore its value because HTTP has no sessions, and remove the
  flag in 0.5.0.
- Do not ship a legacy MCP-2025 HTTP mode.

## Compatibility

Bearer-token authentication, host validation, request and response limits,
multi-project selection, stdio behavior, tool names, and schemas are unchanged.
Clients using the old HTTP initialize/session handshake must migrate to
`server/discover` plus per-request 2026-07-28 metadata.
