# snapshot-publication-limit Specification

## Purpose
TBD - created by archiving change honor-max-graph-bytes-on-publication. Update Purpose after archive.

## Requirements

### Requirement: Publication honors the explicit graph-byte override

Compass SHALL use `COMPASS_MAX_GRAPH_BYTES` consistently for canonical graph
preflight, encoding, manifest validation, snapshot streaming, and publication.

#### Scenario: Valid explicit override

- **WHEN** the environment contains a positive raw-byte, MB, or GB value
- **THEN** snapshot publication uses that finite bound
- **AND** the actionable error advertises the override

#### Scenario: Missing or invalid override

- **WHEN** the override is absent, zero, invalid, overflowing, or cannot fit the platform
- **THEN** the effective limit is the unchanged 2 GiB default
