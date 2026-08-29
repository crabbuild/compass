# Preflight graph size estimate

## ADDED Requirements

### Requirement: Oversized graph estimates fail before extraction

Compass SHALL estimate canonical graph payload size after deterministic source
discovery and before project-wide per-file extraction. When the estimate exceeds
the canonical publication bound, Compass SHALL return a typed limit error rather
than an empty result.

#### Scenario: Calibrated fixture exceeds the bound

- **WHEN** the measured synthetic fixture is evaluated against its calibrated bound
- **THEN** estimation fails before extraction completes
- **AND** time to error is below 322 ms, 20% of the recorded 1.61 s baseline
- **AND** the error names `--exclude <pattern>` and `.compassignore`

### Requirement: Estimation is bounded and deterministic

The estimate SHALL perform bounded work, use saturating arithmetic, depend on
root-relative source metadata, and produce the same result for equivalent
inputs in different checkout locations.

#### Scenario: Equivalent source trees

- **WHEN** equivalent source files exist beneath different absolute roots
- **THEN** their estimates and limit outcomes are identical

#### Scenario: Parser-oversized source

- **WHEN** a source exceeds `max_source_bytes`
- **THEN** the estimate accounts only for its inventory/partial record
- **AND** does not amplify bytes Compass will not parse
