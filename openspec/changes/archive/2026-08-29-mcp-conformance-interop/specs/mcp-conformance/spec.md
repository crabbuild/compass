# mcp-conformance Specification

## Purpose

Defines how Compass proves current Model Context Protocol conformance across its
supported transports and records real-client interoperability without weakening
the normative protocol contract for lagging clients.

## ADDED Requirements

### Requirement: both MCP transports are conformance-gated

Compass SHALL run deterministic conformance coverage for stdio and HTTP in CI.
The HTTP leg SHALL use the official MCP conformance implementation pinned to an
exact upstream revision that understands the 2026-07-28 requirement set.

#### Scenario: qualify both transports in CI

- **WHEN** the MCP conformance job runs
- **THEN** stdio and HTTP are reported as separate required legs and any
  unbaselined failure fails the job

### Requirement: conformance evidence cannot be simulated by product fixtures

Compass SHALL NOT expose test-only tools, prompts, resources, or protocol
capabilities in the production server solely to satisfy diagnostic scenarios
for capabilities Compass does not claim.

#### Scenario: the reference suite requests an unsupported fixture capability

- **WHEN** a diagnostic scenario depends on a tool or capability Compass does
  not advertise
- **THEN** the qualification records it as not applicable rather than adding a
  production test hook or claiming a pass

### Requirement: named clients prove discovery and invocation

The interop matrix SHALL record exact tested versions of Codex CLI, Claude
Code, and OpenCode. Every passing cell SHALL prove both discovery of the
Compass MCP server and successful invocation of a real Compass tool.

#### Scenario: qualify a named client

- **WHEN** a pinned client is run with isolated stdio or HTTP configuration
- **THEN** the receipt identifies the exact client version, discovered tool,
  invoked tool, result evidence, transport, and pass/fail outcome

### Requirement: latest protocol conformance governs the wave

Compass SHALL treat the latest published MCP revision and the required stdio
and HTTP conformance legs as the normative merge gate. Both production
transports SHALL advertise only that revision and reject older negotiation. A named client that
implements an older revision SHALL be recorded as incompatible and SHALL NOT
cause Compass to restore protocol behavior removed by the latest revision.

#### Scenario: a named client uses an older MCP lifecycle

- **WHEN** discovery or invocation fails because the client requests an older
  protocol revision or removed transport lifecycle
- **THEN** the exact outcome is recorded, Compass retains the latest protocol,
  and the client limitation does not by itself hold C-010

#### Scenario: Compass fails current conformance

- **WHEN** either required conformance leg fails or a client failure exposes a
  Compass deviation from the latest published revision
- **THEN** C-010 remains incomplete and the Wave 3 merge gate is explicitly held
