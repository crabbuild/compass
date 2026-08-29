# Harness lifecycle specification

## Purpose

Defines isolated installed-artifact qualification so Compass can prove native
agent packages survive complete harness lifecycles without altering user-owned
state.

## ADDED Requirements

### Requirement: Installed-artifact evidence

Each native package SHALL be qualified against the exact harness version
recorded in `distribution.toml`. Export, install, discovery, load, MCP
configuration, upgrade, and uninstall SHALL operate on the generated artifact.

#### Scenario: Complete isolated lifecycle

- **WHEN** the phase-end harness runs in isolated configuration roots
- **THEN** every stage SHALL pass without credentials, network model calls, or
  access to a source template

### Requirement: Ownership preservation

Upgrade and uninstall SHALL alter only harness-managed package files and
configuration entries. User-authored instructions and unrelated plugins SHALL
remain byte-identical.

#### Scenario: User sentinel

- **WHEN** a sentinel instruction exists beside managed harness state
- **THEN** install, upgrade, and uninstall SHALL preserve its bytes
