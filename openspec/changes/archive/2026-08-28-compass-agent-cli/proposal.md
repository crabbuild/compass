## Why

Compass can install agent guidance, but discovery, health checks, portable export, validation, and MCP setup are scattered or implicit. A cohesive `compass agent` namespace makes those operations scriptable while preserving the established `compass install` contract.

## What Changes

- Add public `compass agent list|install|doctor|export|validate|mcp-config` subcommands with native help, stable exit behavior, and text or JSON results where applicable.
- Make `compass agent install` delegate to the existing managed installer while retaining `compass install` byte-for-byte as the compatibility entry point.
- Add deterministic export and validation for the embedded seven-skill collection without introducing the cross-harness package generators owned by C-018.
- Add bounded health checks for the running binary, graph availability and freshness, MCP protocol/configuration, and installed skill checksums.
- Reject exported or installed agent content containing unsafe absolute paths or likely literal credentials.
- Document the new namespace and its compatibility boundary.

## Capabilities

### New Capabilities

- `agent-cli`: Defines agent inventory, managed-install aliasing, diagnostics, deterministic skill export, package validation, and MCP configuration rendering.

### Modified Capabilities

None.

## Impact

The change affects public CLI parsing/help/output, the `compass-cli` installer registry and embedded assets, CLI contract tests, command reference documentation, compatibility notes, and the changelog. It adds no dependency, graph schema, MCP wire-protocol, or generated harness-package change.
