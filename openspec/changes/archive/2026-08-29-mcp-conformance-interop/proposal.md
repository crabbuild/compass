# Proposal: gate latest MCP on conformance and record named-client interoperability

## Why

Compass now exposes a stateless MCP 2026-07-28 HTTP transport and typed tool
results, but repository tests alone do not prove that the wire contract passes
the independent MCP conformance runner or that real coding-agent clients can
discover and invoke the server.

## What Changes

- Add a deterministic transport-conformance suite covering stdio and HTTP.
- Run the official MCP conformance runner against HTTP at an exact upstream
  commit that includes the frozen 2026-07-28 requirements.
- Add CI coverage that fails on a conformance regression rather than carrying
  an unreviewed expected-failure baseline.
- Record actual discovery and tool-invocation outcomes for exact versions of
  Codex CLI, Claude Code, and OpenCode using isolated temporary configuration.
- Treat the latest published MCP revision and its independent conformance suite
  as the normative merge gate. Record clients that implement an older revision
  as incompatible without weakening Compass to their obsolete lifecycle.

## Compatibility

The test and CI additions do not rename tools or alter successful result
payloads. Any production wire correction found by conformance is
compatibility-sensitive. Both stdio and HTTP require the latest published MCP
behavior, currently 2026-07-28, while retaining the existing tool, resource,
and successful-result contracts. Compass does not add a legacy transport
fallback for lagging clients.
