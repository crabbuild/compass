# Partition contract

## ADDED Requirements

### Requirement: storage-neutral ownership

The workspace SHALL provide `compass-partition` without dependencies on
`prolly-map`, `prolly-store-sqlite`, `compass-ir`, or `compass-analysis`.

#### Scenario: inspect the dependency tree

- **WHEN** the crate dependency tree is inspected
- **THEN** none of the forbidden dependencies is reachable

### Requirement: byte compatibility

Canonical JSON and typed graph keys SHALL remain byte-identical to the existing
`compass-history` contract.

#### Scenario: use history compatibility exports

- **WHEN** existing canonical, round-trip, and diff tests run unchanged
- **THEN** all encoded bytes and reconstructed graph behavior remain identical

### Requirement: typed boundary errors

`compass-partition` SHALL expose its own error type, and `compass-history` SHALL
convert that error at its public compatibility boundary.

#### Scenario: canonical encoding fails

- **WHEN** the partition encoder cannot encode a value
- **THEN** history reports a typed `HistoryError` sourced from `PartitionError`
