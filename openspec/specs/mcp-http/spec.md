# mcp-http Specification

## Purpose
TBD - created by archiving change mcp-stateless-http. Update Purpose after archive.

## Requirements

### Requirement: HTTP requests are stateless MCP 2026-07-28 exchanges

Compass SHALL advertise MCP 2026-07-28 from its HTTP service, route every HTTP
POST without a server session, require the protocol and standard method headers
plus applicable per-request metadata, and SHALL NOT issue or require
`Mcp-Session-Id`.

#### Scenario: discover and list tools without a session handshake

- **WHEN** an authenticated HTTP client sends `server/discover` and then
  `tools/list` with `Mcp-Protocol-Version: 2026-07-28`, matching `Mcp-Method`
  headers, and required request metadata
- **THEN** both requests succeed independently, advertise only 2026-07-28, and
  neither response contains `Mcp-Session-Id`

#### Scenario: reject incomplete per-request protocol metadata

- **WHEN** a non-initialize HTTP request omits its protocol-version header,
  method header, or required request metadata
- **THEN** Compass rejects it before tool dispatch with a typed HTTP/JSON-RPC
  protocol error

### Requirement: no legacy MCP-2025 HTTP mode ships

Compass SHALL NOT expose a runtime flag, hidden fallback, or session path for
MCP-2025 HTTP clients. Stdio compatibility remains independently qualified.

#### Scenario: send an old session handshake

- **WHEN** a client sends an initialize request or an `Mcp-Session-Id` expecting
  the former HTTP session flow
- **THEN** Compass rejects initialize, ignores an obsolete session header on an
  otherwise valid current request, never creates or restores a legacy session,
  and documents the required stateless migration

### Requirement: session timeout follows a bounded deprecation path

Compass SHALL accept both spellings of `--session-timeout` through 0.4.x,
validate the existing value grammar, ignore a valid value, print one warning,
and preserve the command's success or failure exit code. The option SHALL be
removed in 0.5.0.

#### Scenario: use the deprecated option with another runtime failure

- **WHEN** a valid `--session-timeout` accompanies an HTTP invocation that
  otherwise fails
- **THEN** stderr includes the deprecation/removal warning and the invocation
  retains the same runtime-failure exit code

### Requirement: existing HTTP security and selection boundaries remain intact

Compass SHALL preserve bearer and API-key authentication, allowed-host checks,
bounded request and response bodies, and per-request multi-project selection.

#### Scenario: exercise security and project inputs

- **WHEN** requests use invalid credentials, an untrusted host, an oversized
  body, or a valid project selector
- **THEN** the existing rejection or selection behavior remains unchanged
