# Agent CLI Specification

## Purpose

Define a stable, bounded command-line contract for discovering, installing,
diagnosing, exporting, validating, and configuring Compass agent integrations.

## Requirements

### Requirement: Agent namespace and inventory
Compass SHALL expose exactly the public subcommands `agent list`, `agent install`, `agent doctor`, `agent export`, `agent validate`, and `agent mcp-config`, and SHALL provide nested help for each subcommand. `agent list` SHALL report the supported agent platforms in deterministic identifier order without changing files.

#### Scenario: List agents as text
- **WHEN** a user runs `compass agent list`
- **THEN** the command exits successfully and emits one deterministic inventory row per supported platform

#### Scenario: List agents as JSON
- **WHEN** a user runs `compass agent list --format json`
- **THEN** the command exits successfully and emits a versioned JSON object whose agents are sorted by platform identifier

#### Scenario: Reject an unknown agent subcommand
- **WHEN** a user runs `compass agent` with an unknown subcommand
- **THEN** the command exits with usage status 2 and identifies the invalid subcommand

### Requirement: Managed install compatibility
`compass agent install` SHALL delegate to the existing managed installer with the remaining arguments unchanged. The established `compass install` entry point SHALL remain available and SHALL retain byte-identical output, side effects, and exit behavior for equivalent arguments.

#### Scenario: Install through the namespace
- **WHEN** a user invokes `compass agent install` with valid installer arguments
- **THEN** the command produces the same files, output bytes, and exit status as `compass install` with those arguments

#### Scenario: Preserve legacy install failures
- **WHEN** equivalent invalid arguments are supplied to `compass agent install` and `compass install`
- **THEN** both commands produce the same error bytes and exit status

### Requirement: Bounded agent diagnostics
`compass agent doctor` SHALL report deterministic checks for the Compass binary version, MCP protocol compatibility, graph presence and freshness, installed seven-skill collection checksums, and platform MCP configuration. It SHALL bound filesystem discovery and SHALL not contact a network service or start an MCP server. The command SHALL exit 0 only when every required check passes, 1 when one or more checks fail, and 2 for invalid arguments.

#### Scenario: Healthy agent environment
- **WHEN** doctor inspects a selected platform whose graph, managed skills, protocol, and MCP configuration are current and valid
- **THEN** every check is reported as passing and the command exits 0

#### Scenario: Stale graph
- **WHEN** doctor inspects a graph whose manifest no longer matches the project source set
- **THEN** the graph-freshness check fails explicitly and the command exits 1

#### Scenario: Bad installed skill checksum
- **WHEN** doctor inspects a managed skill whose bytes no longer match its install manifest
- **THEN** the skill-checksum check fails explicitly and the command exits 1

#### Scenario: Inspect a user-scoped integration
- **WHEN** doctor is given a user configuration root instead of a project root
- **THEN** project graph checks are reported as not applicable and health is determined by the remaining user-scoped checks

### Requirement: Deterministic portable export
`compass agent export --platform <platform> --out <directory>` SHALL export the embedded seven-skill collection, a versioned manifest with sorted file checksums, and a credential-free platform-native MCP configuration. Equivalent inputs SHALL produce byte-identical file contents and manifest ordering. Export SHALL refuse to replace a non-empty destination and SHALL publish the complete output set atomically.

#### Scenario: Export an agent bundle
- **WHEN** a user exports a supported platform to a missing or empty destination
- **THEN** the destination contains all seven skills, the native MCP configuration, and a manifest whose paths and checksums are deterministically ordered

#### Scenario: Protect an existing destination
- **WHEN** a user exports to a non-empty destination
- **THEN** the command fails without modifying any existing destination entry

### Requirement: Safe bundle validation
`compass agent validate` SHALL validate a bounded bundle or managed installation without executing its content. Validation SHALL verify the versioned manifest, supported transport, the complete seven-skill inventory, file checksums, path containment, and platform configuration shape. It SHALL reject symlinks, unresolved or machine-specific absolute paths, file URLs, and likely literal credential values while allowing ordinary web URLs. Validation failures SHALL be deterministic and exit 1; invalid invocation SHALL exit 2.

#### Scenario: Validate a portable bundle
- **WHEN** a user validates an unmodified bundle produced by `agent export`
- **THEN** validation succeeds without changing the bundle

#### Scenario: Reject an absolute path
- **WHEN** a validated text file contains a Unix absolute path, a Windows absolute path, or a file URL
- **THEN** validation reports the affected relative file and exits 1

#### Scenario: Reject a credential string
- **WHEN** a validated configuration contains a likely literal secret value
- **THEN** validation reports the affected relative file without echoing the secret and exits 1

#### Scenario: Allow a web URL
- **WHEN** a validated configuration contains an ordinary HTTPS endpoint and no unsafe value
- **THEN** the URL is not classified as a Windows absolute path or a credential

### Requirement: Native MCP configuration rendering
`compass agent mcp-config` SHALL render deterministic, credential-free MCP configuration for `codex`, `claude`, `opencode`, and `agents` platforms using either `stdio` or `http` transport. Codex output SHALL use its `mcp_servers` TOML shape, Claude output SHALL use the `.mcp.json` `mcpServers` shape, OpenCode output SHALL use its `mcp` local or remote shape, and the generic agents output SHALL use a versioned JSON envelope. Stdio configuration SHALL invoke Compass with separate arguments; HTTP configuration SHALL use a loopback URL.

#### Scenario: Render stdio configuration
- **WHEN** a user requests stdio MCP configuration for a supported platform
- **THEN** the output uses that platform's native schema and launches `compass serve --transport stdio` without credentials

#### Scenario: Render HTTP configuration
- **WHEN** a user requests HTTP MCP configuration for a supported platform
- **THEN** the output uses that platform's native schema and the loopback MCP endpoint without credentials

#### Scenario: Reject unsupported configuration values
- **WHEN** a user requests an unsupported platform or transport
- **THEN** the command exits with usage status 2 and identifies the invalid value
