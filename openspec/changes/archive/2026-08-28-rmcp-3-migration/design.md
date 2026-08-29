# Design: rmcp 3.1.4 migration

## Scope boundary

This is a dependency and API migration only. It does not add tools, redesign
results, introduce SurrealDB, or change the HTTP session contract.

## API migration

Current rmcp documentation identifies `ServiceExt::serve`/`serve_server` with
the stdio transport as the supported server startup path. rmcp 3.1.4 also adds
the `ServerHandler::discover` surface and 2026 protocol models while retaining
the established list-tools/list-resources handler shape.

Compilation against the exact pin required three adaptations:

1. `ServerHandler::call_tool` now returns `CallToolResponse`; Compass wraps its
   unchanged complete `CallToolResult` with the SDK's lossless `From` conversion.
2. `ServerHandler::read_resource` likewise returns `ReadResourceResponse` and
   wraps the unchanged complete `ReadResourceResult`.
3. `StreamableHttpServerConfig::with_stateful_mode` was renamed to
   `with_legacy_session_mode`; passing the same `!options.stateless` value
   preserves the current transport choice pending the separate C-008 redesign.

The stdio `ServiceExt::serve` and `RunningService::waiting` calls did not change.
All existing duplex protocol tests pass without adaptation.

## Dependency baseline

Before migration, the selected rmcp 2.2.0 normal/build subtree contains 69
normalized package-version entries and the `compass-mcp` normal/build tree
contains 369 distinct rendered lines. After migration those counts are 70 and
370. The exact rmcp subtree delta is:

- removed: `rmcp 2.2.0`;
- added: `rmcp 3.1.4`, `base64 0.23.1`;
- no other rmcp-subtree packages changed.

Both rmcp releases declare Apache-2.0. rmcp 3.1.4 declares Rust 1.88, below
Compass's pinned 1.97.1 toolchain. The advisory gate also exposed two current
lockfile issues unrelated to the rmcp version delta: `h2 0.4.15`
(`RUSTSEC-2026-0258`) and yanked `chacha20 0.10.1`. Compatible patch updates to
`h2 0.4.16` and `chacha20 0.10.2` make advisories, bans, licenses, and sources
all pass without weakening `deny.toml`.

## Compatibility evidence

A checked-in golden derived before the upgrade records the server identity,
capabilities, ordered tool names and SHA-256 digests of their full input
schemas, and ordered resource inventory. Both the local handler view and a real
rmcp 3.1.4 `server/discover` exchange must match it after the migration.
