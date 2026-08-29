# Agent distribution specification

## Purpose

Defines deterministic, native package exports that let supported agent
harnesses install Compass skills and MCP configuration from one canonical
inventory without duplicating graph behavior.

## ADDED Requirements

### Requirement: Canonical deterministic inventory

Compass SHALL generate every native harness package from one versioned
`distribution.toml` inventory. Equivalent inventory and embedded skills SHALL
produce byte-identical files in stable path order.

#### Scenario: Repeat generation

- **WHEN** the same Compass binary exports a platform package twice
- **THEN** every path and byte SHALL be identical

### Requirement: Native complete packages

Codex SHALL receive its plugin manifest, marketplace, `.mcp.json`, and copied
skills. Claude Code SHALL receive its plugin manifest, required marketplace,
`.mcp.json`, and copied skills. OpenCode SHALL receive its npm manifest,
configuration, JavaScript artifact generated from the checked TypeScript
plugin source, and copied skills.

#### Scenario: Native validation

- **WHEN** an exported package is validated for its declared platform
- **THEN** missing native artifacts, missing harness versions, unsafe paths,
  symlinks, credentials, and checksum mismatches SHALL fail closed

### Requirement: Thin OpenCode bridge

The OpenCode plugin MAY register MCP configuration tools but SHALL NOT contain
graph persistence, traversal, ranking, or query behavior.

#### Scenario: Bridge responsibility boundary

- **WHEN** the generated OpenCode plugin is inspected and loaded
- **THEN** it SHALL configure the native Compass MCP server without implementing
  a second graph or query engine
