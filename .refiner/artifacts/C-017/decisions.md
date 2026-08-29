# Refinement decisions — C-017

- Kept `agent_commands` presentation-only and exposed narrow immutable helpers
  from the installer rather than making registry internals public.
- Limited MCP configuration rendering to the four hosts with an explicit
  native schema: Codex, Claude, OpenCode, and generic Agent Skills.
- Limited export to the current seven embedded skills; full cross-harness
  packages remain owned by C-018.
- Used the graph manifest and bounded detection for freshness rather than file
  modification times.
- Excluded checksum-owned installation metadata from portability scanning
  because its root is intentionally machine-local, while still verifying its
  checksums and current embedded collection bytes.
- Returned the complete export manifest for JSON mode so one schema identifier
  never describes two incompatible shapes.
