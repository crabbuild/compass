## Purpose

Provide portable, task-specific Compass Agent Skills while retaining the existing umbrella skill as the stable compatibility entry point.

## ADDED Requirements

### Requirement: Additive focused skill inventory
Compass SHALL distribute exactly the focused skills `compass-navigate`, `compass-debug`, `compass-change-impact`, `compass-architecture`, `compass-index-maintenance`, and `compass-mcp-setup` in addition to the canonical `compass` skill. The canonical umbrella skill SHALL remain byte-for-byte unchanged by this capability.

#### Scenario: Focused inventory is present
- **WHEN** the embedded skill inventory is validated
- **THEN** all six focused names and the canonical umbrella are present exactly once
- **AND** the canonical umbrella digest matches the pre-change digest

### Requirement: Portable complete skill trees
Every focused skill SHALL be a complete Agent Skills-compatible directory whose `SKILL.md` name matches its lower-kebab directory. Focused skill content MUST contain no absolute filesystem paths and MUST use only relative links to bundled resources.

#### Scenario: Focused package validation succeeds
- **WHEN** a release build validates the embedded focused skill trees
- **THEN** every tree satisfies the Agent Skills frontmatter, naming, path, and resource-completeness constraints

#### Scenario: Invalid portable content is rejected
- **WHEN** a focused tree contains a mismatched name, absolute path, or reference to an absent bundled resource
- **THEN** validation fails before the Compass binary is built

### Requirement: Discriminative activation metadata
Focused skill descriptions SHALL identify distinct task boundaries, and a checked-in trigger corpus SHALL select one intended focused skill for boundary prompts while retaining the umbrella fallback for broad or explicit Compass invocations.

#### Scenario: Boundary prompt selects one focused skill
- **WHEN** the trigger-discrimination corpus evaluates a task-specific prompt
- **THEN** exactly one focused skill has the highest matching activation evidence
- **AND** that skill equals the corpus expectation

#### Scenario: Umbrella invocation remains compatible
- **WHEN** the corpus evaluates an explicit `/compass` invocation or a broad multi-operation Compass request
- **THEN** no focused skill supersedes the canonical umbrella fallback

### Requirement: Managed installation of complete inventory
An installation that targets an Agent Skills-capable platform SHALL place the canonical umbrella and all six focused skills as sibling directories. Installation SHALL preflight every destination, use content checksums to identify current managed trees, be idempotent for equivalent inputs, and preserve unowned or modified destinations.

#### Scenario: First install places complete trees
- **WHEN** a user installs Compass guidance into an empty supported skill container
- **THEN** the umbrella and all six focused skill trees are installed with ownership manifests covering every managed file

#### Scenario: Equivalent reinstall is idempotent
- **WHEN** the same Compass version and consumer set are installed again
- **THEN** every checksum remains unchanged
- **AND** the installer reports the target as current

#### Scenario: Unowned focused destination blocks mutation
- **WHEN** any focused skill destination already contains an unowned or modified tree
- **THEN** installation fails before changing the umbrella, focused skills, adapters, or configuration

#### Scenario: Last consumer uninstall removes focused trees
- **WHEN** the last registered consumer uninstalls a managed skill collection
- **THEN** the umbrella and every unmodified managed focused tree are removed
- **AND** user-owned or modified trees are preserved
