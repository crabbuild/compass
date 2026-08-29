# rmcp-migration Specification

## Purpose
TBD - created by archiving change rmcp-3-migration. Update Purpose after archive.

## Requirements

### Requirement: exact rmcp 3.1.4 dependency

Compass SHALL resolve rmcp at exactly version 3.1.4 from the workspace
dependency declaration.

#### Scenario: inspect workspace dependency state

- **WHEN** Cargo metadata and the lockfile are inspected
- **THEN** rmcp resolves to 3.1.4 and member crates inherit the workspace entry

### Requirement: stdio behavior remains compatible

Compass SHALL preserve bounded, blank-line-tolerant MCP stdio operation while
using the rmcp 3.x server API.

#### Scenario: exercise the in-memory stdio-equivalent protocol

- **WHEN** a client initializes, lists tools and resources, calls a tool, reads
  a resource, and closes the duplex transport
- **THEN** all operations complete with the same public results and errors as
  the rmcp 2.2 implementation

### Requirement: discovery contract remains stable

Compass SHALL preserve the existing server identity, advertised capabilities,
ordered tool names and input schemas, and ordered resource inventory.

#### Scenario: compare migrated discovery to the rmcp 2.2 golden

- **WHEN** the normalized discovery contract is generated under rmcp 3.1.4
- **THEN** it exactly matches the checked-in rmcp 2.2 golden

### Requirement: dependency policy remains satisfied

The migration SHALL document the exact transitive package delta and SHALL pass
the repository license, source, ban, lint, and unsafe-code policies.

#### Scenario: qualify resolved dependencies

- **WHEN** dependency and workspace policy gates run
- **THEN** rmcp remains acceptably licensed, no disallowed source is introduced,
  and the workspace continues to compile without unsafe Compass code
