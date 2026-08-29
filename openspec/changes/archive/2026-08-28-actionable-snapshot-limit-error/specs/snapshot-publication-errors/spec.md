## Purpose

Make canonical graph snapshot limit failures recoverable through existing
scope controls while preserving bounded publication behavior.

## ADDED Requirements

### Requirement: Oversized canonical graphs report a resource limit

Compass SHALL reject a canonical graph whose encoded size exceeds the active
snapshot publication limit as a typed snapshot limit failure, distinct from an
empty or corrupt snapshot.

#### Scenario: Canonical graph exceeds the default limit
- **WHEN** snapshot validation observes `graph_bytes` above `MAX_GRAPH_BYTES`
- **THEN** it returns `SnapshotError::Limit` and does not publish the manifest

#### Scenario: Canonical graph byte count is empty
- **WHEN** snapshot validation observes a zero `graph_bytes` value
- **THEN** it returns a corruption failure rather than a limit failure

### Requirement: Limit remediation names shipped scope controls

The oversized-graph failure SHALL name `--exclude <pattern>` and
`.compassignore` as concrete recovery actions and SHALL NOT advertise an
override that the snapshot publication path does not yet honor.

#### Scenario: CLI renders the oversized-graph failure
- **WHEN** a CLI operation encounters the oversized canonical graph manifest
- **THEN** stderr names both scope controls and the process exits with code 1
- **AND** the message does not name `COMPASS_MAX_GRAPH_BYTES`
