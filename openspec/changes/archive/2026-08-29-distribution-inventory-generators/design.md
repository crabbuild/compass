# Design

`distribution.toml` is the only package identity, harness-version, manifest
path, and plugin-API inventory. The CLI parses it at runtime from an embedded
copy, rejects unsupported schemas or non-portable paths, combines it with the
real embedded skill files, and generates harness-native JSON plus the OpenCode
TypeScript bridge. A sorted digest manifest closes over the staged output
before atomic publication.

Codex and Claude packages use their current plugin manifests and local
marketplace catalogs. OpenCode uses a workspace TypeScript package and exact
plugin API dependency; it may emit MCP configuration but contains no graph
logic. Validation is bounded and never executes bundle content.
