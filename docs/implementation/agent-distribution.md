# Native agent package distribution

Compass has one checked-in `distribution.toml` inventory for its Codex, Claude
Code, and OpenCode packages. `compass agent export` parses and validates that
inventory, copies the real embedded umbrella and focused skill directories,
and generates the harness-native manifests and credential-free MCP
configuration. The same inventory records the exact harness versions used by
installed-artifact qualification.

## Generated package shapes

| Harness | Native artifacts | Verified version |
| --- | --- | --- |
| Codex | `.codex-plugin/plugin.json`, `.agents/plugins/marketplace.json`, `.mcp.json`, `skills/**` | `0.146.0` |
| Claude Code | `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `.mcp.json`, `skills/**` | `2.1.251` |
| OpenCode | `package.json`, `opencode.json`, compiled `src/index.js`, `skills/**` | `1.18.23` |

Every export also contains `manifest.json`, a sorted SHA-256 inventory under
`compass.agent-bundle/1`. Generation is staged beside the destination,
validated, and published with one rename. The destination must be absent or
empty, so export never overwrites user files.

The OpenCode package depends on exactly `@opencode-ai/plugin` 1.18.25. Its
TypeScript module registers one thin MCP-configuration tool. Run
`npm run build -w @compass/opencode-plugin` after changing the module. The
native generator embeds the resulting checked-in `src/index.js`, while
`npm run typecheck:js` recompiles in memory and fails if that artifact is stale.
It contains no graph traversal, ranking, persistence, or query logic; all such
behavior stays in the native `compass` binary.

The umbrella skill digest in `tools/skillgen/mod.rs` is an intentional
compatibility pin. If that skill changes by design, regenerate its canonical
LF-normalized SHA-256 digest, review the focused-skill additive contract, and
update the pin in the same change. Release version bumps likewise update
`distribution.toml` alongside the workspace package version.

## Ownership and portability

Skills are copied as real files, never symlinked. Validation rejects symlinks,
parent traversal, absolute or machine-specific paths, likely literal
credentials, missing native manifests, checksum mismatches, and bundles that
omit the recorded harness version. Harness installation owns only files copied
into the harness plugin cache or configuration entry. Qualification places a
sentinel user instruction beside those managed files and requires it to survive
install, upgrade, and uninstall.

## Phase-end verification

Package generation and lifecycle evidence is collected only after the whole
phase implementation is complete. The verification wave runs the full Rust
integration targets, the installed-artifact harness qualification, and the
workspace JavaScript typecheck and integration suite. Source templates alone
are never accepted as lifecycle evidence.
