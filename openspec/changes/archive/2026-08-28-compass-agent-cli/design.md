## Context

See `proposal.md` for motivation and `specs/agent-cli/spec.md` for the observable contract. Compass already owns an embedded, manifest-backed seven-skill collection and a registry-driven managed installer in `compass-cli`. The new namespace must compose those primitives rather than create a second installer. Public CLI behavior is compatibility-sensitive, while filesystem content and configuration are untrusted and must remain bounded.

Current platform MCP configuration differs materially: Codex uses `mcp_servers` TOML; Claude uses an `.mcp.json` `mcpServers` wrapper; OpenCode uses a top-level `mcp` object with local command arrays or remote URLs. The generic `agents` platform has no vendor-owned configuration file, so Compass must identify its own schema.

## Goals / Non-Goals

**Goals:**

- Keep command dispatch and presentation in `compass-cli`, reusing installer registry, embedded assets, manifests, atomic writes, graph manifests, and MCP protocol constants.
- Make all enumeration, diagnostics, export, and validation deterministic and bounded.
- Give text and machine consumers stable exit semantics and versioned JSON where output is contractual.
- Validate portable content without executing it or disclosing a detected secret.

**Non-Goals:**

- Generate the full cross-harness packages, plugin manifests, or installation scripts owned by C-018.
- Contact remote services, launch a server, repair an installation, or mutate configuration during `list`, `doctor`, `validate`, or `mcp-config`.
- Change the MCP wire protocol, graph schema, installer destination rules, or existing `compass install` contract.

## Decisions

### Add a thin sibling command module

Add `agent_commands` beside `install_commands` and route only the `agent` top-level token to it. `agent install` passes its remaining argument slice directly to the existing installer. Narrow `pub(crate)` helpers expose immutable registry inventory, embedded asset bytes, install destinations, and manifest verification; registry ownership stays in `install_commands`.

This avoids duplicating parsing and rollback logic. Exposing registry internals wholesale was rejected because it would blur the installer ownership boundary.

### Use explicit versioned output contracts

JSON inventory, doctor results, export manifests, validation results, and the generic MCP configuration use explicit `compass.* /1` schema identifiers (without the separating space in serialized values). Collections and findings are sorted before rendering. Text output is derived from the same typed records.

Versioned records make future additions distinguishable. Unversioned ad hoc JSON was rejected because downstream consumers could not detect incompatible changes.

### Export the current collection as an atomic portable bundle

Export writes a sibling staging directory, creates `skills/<skill>/...`, the platform MCP configuration, and a sorted SHA-256 manifest, validates the staged result, then renames it into place. A non-empty destination is an error; an existing empty directory is removed only immediately before the same-filesystem rename and restored on failure when possible. Staging is cleaned after any error.

The bundle deliberately stops at the seven embedded skills and one MCP config. Generating platform plugin packages here was rejected because C-018 owns that surface.

### Reuse graph manifests for freshness

Doctor resolves the project root and configured `COMPASS_OUT` container, resolves `graph.json` and `manifest.json` through the current immutable snapshot, runs bounded repository detection, and asks the existing manifest implementation whether the source set is unchanged. Presence and freshness are separate checks so missing, corrupt, and stale graphs remain distinguishable. User-scoped diagnostics mark project graph checks not applicable instead of interpreting the user configuration root as a project.

Modification-time comparison was rejected because it is unreliable across copied files, clock changes, and equivalent content.

### Diagnose independently and aggregate failures

Doctor executes every bounded local check and returns a sorted result record instead of stopping at the first failure. Binary version and the compiled MCP protocol constant are intrinsic checks. Skill checks use the managed install manifest and exact file checksums. MCP configuration checks resolve the registry-owned platform path and validate shape/content without parsing credentials into output.

Live server discovery was rejected for this command because it would add process and network side effects; configuration discovery plus the compiled protocol contract provides a deterministic offline health check.

### Validate with containment, bounds, and redaction

Validation refuses symlinks, limits file count, per-file bytes, and aggregate bytes, canonicalizes the root, and ensures every visited path remains below it. It verifies the bundle schema, exact seven-skill inventory, sorted manifest entries, SHA-256 checksums, and platform config shape. Text scanning detects Unix roots, drive/UNC paths, and `file://` URLs only at token boundaries. Credential detection recognizes sensitive key names paired with non-placeholder literal values; findings name the file and rule, never the value.

Scanning every arbitrary token as a path or secret was rejected because it would misclassify HTTPS URLs and documentation prose.

### Render vendor-native MCP schemas directly

Configuration rendering is a pure function over platform and transport. Stdio uses an executable plus separate argument elements for `compass serve --transport stdio`; HTTP uses `http://127.0.0.1:8080/mcp`. Codex output is deterministic TOML. Claude and OpenCode output are pretty JSON with sorted object construction. Generic agents output uses a Compass-owned versioned JSON envelope.

A single nominally portable schema was rejected because current hosts consume different keys and command representations.

## Risks / Trade-offs

- [Doctor repository detection may be noticeable on very large projects] → Reuse the repository's existing bounded discovery and manifest comparison, and report limit failures rather than treating them as freshness.
- [Credential heuristics can produce false positives or miss novel secret names] → Restrict detection to explicit sensitive key/value shapes, redact values, and keep manifest checksum validation authoritative.
- [Atomic directory replacement differs across platforms] → Stage beside the destination, refuse non-empty destinations, use existing atomic file primitives inside the stage, and test cleanup and rollback behavior.
- [Vendor MCP schemas can evolve] → Isolate rendering functions, version Compass-owned schemas, and cover exact current shapes with contract tests and documentation.

## Migration Plan

1. Ship `compass agent` as an additive namespace while retaining the legacy installer route.
2. Document both install spellings and identify `compass install` as the compatibility entry point.
3. If rollback is required, remove the new route and help pages; no graph, storage, or installed-file migration is needed because installation continues to use the existing managed manifest format.
