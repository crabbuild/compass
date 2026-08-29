# vendor-discovery-policy Specification

## Purpose
TBD - created by archiving change vendor-skip-policy. Update Purpose after archive.

## Requirements

### Requirement: vendor source is eligible by default

Compass SHALL NOT exclude a directory solely because its name is `vendor`.

#### Scenario: discover workspace and Go vendor source

- **WHEN** default discovery scans Rust workspace-member source and Go source
  beneath `vendor/`
- **THEN** both sources are eligible for classification and publication

### Requirement: vendor exclusion remains explicit

Compass SHALL apply configured or command-line `vendor/**` exclusions through
the existing scope and ignore contracts.

#### Scenario: repository opts out of vendor source

- **WHEN** the repository configures `vendor/**` in project scope,
  `.compassignore`, or `--exclude`
- **THEN** matching vendored source is excluded deterministically

### Requirement: watcher parity

Filesystem watching SHALL use the same default vendor eligibility as discovery.

#### Scenario: vendor source changes

- **WHEN** an otherwise eligible file beneath `vendor/` changes
- **THEN** the watcher allows the event unless an explicit ignore or scope rule
  excludes it
