## Why

Compass currently installs one broad, canonical Agent Skill, so clients must load a large umbrella instruction set even for narrow navigation, debugging, impact, architecture, index-maintenance, or MCP-setup requests. Additive focused skills can make activation more precise while preserving the existing `compass` skill as the unchanged compatibility entry point.

## What Changes

- Add six portable Agent Skills: `compass-navigate`, `compass-debug`, `compass-change-impact`, `compass-architecture`, `compass-index-maintenance`, and `compass-mcp-setup`.
- Keep the existing `compass` skill byte-for-byte unchanged and canonical.
- Extend Compass installation to place every focused skill as a complete sibling tree alongside the umbrella skill.
- Validate skill naming, relative references, trigger discrimination, complete-tree checksums, idempotent reinstall, and preservation of unowned or modified destinations.
- Add a checked-in trigger corpus that proves existing umbrella prompts still match and focused boundary prompts select one intended skill.

## Capabilities

### New Capabilities

- `focused-agent-skills`: Defines the additive focused skill inventory, activation boundaries, portable package structure, and managed installation behavior.

### Modified Capabilities

None.

## Impact

The change affects embedded assets, build-time asset validation, and managed installation in `compass-cli`, plus installer regression tests. It adds installed files but does not remove or rename commands, change the umbrella skill, add dependencies, or alter graph/MCP machine schemas.
