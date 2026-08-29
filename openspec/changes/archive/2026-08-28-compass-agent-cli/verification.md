# Verification: Compass agent CLI namespace

## Result

PASS for the additive `compass agent` namespace and its bounded offline bundle
and diagnostic contracts. All implementation, qualification, graph-refresh,
artifact-refinement, and adversarial-review gates are complete.

## Requirement evidence

- The released binary routes exactly six nested commands and publishes nested
  help plus deterministic text and versioned JSON inventory.
- `compass agent install` passes its remaining arguments directly to the
  managed installer. Integration coverage compares successful and failing exit
  codes, stdout, stderr, and complete installed trees; manifest roots are
  normalized while every other manifest field and checksum remains compared.
- Export stages beside the destination, validates before rename, preserves an
  existing empty destination's permissions, refuses non-empty destinations, and
  emits deterministic `compass.agent-bundle/1` manifests.
- Validation is non-executing and globally bounded. It verifies manifest schema,
  transport, seven-skill inventory, checksums, path containment, symlink policy,
  portable paths, redacted credential heuristics, and exact Compass stdio or
  loopback MCP entries.
- Doctor is offline, honors `COMPASS_OUT` and immutable snapshots, reports user
  graph checks as not applicable, verifies the MCP transport's exported protocol
  constant, validates exact current managed-skill bytes, and checks native MCP
  configuration.

## Verification performed

- `cargo fmt --all -- --check`: passed.
- `cargo test -p compass-cli agent_commands --locked`: 20 passed.
- `cargo test -p compass-cli --test agent_cli --locked`: 4 passed.
- `cargo test -p compass-cli --test install_cli --locked`: 25 passed.
- `cargo clippy -p compass-cli --all-targets --all-features --locked -- -D
  warnings`: passed.
- `cargo test -p compass-cli --test compass_product --locked`: 7 passed.
- `sh scripts/check_product_boundary.sh`: passed.
- `cargo clippy --workspace --lib --bins --locked -- -D warnings`: passed after
  the final adversarial refinements.
- `cargo test --workspace --lib --bins --locked`: passed after the final
  adversarial refinements.
- `openspec validate compass-agent-cli --strict`: passed.
- `git diff --check`: passed.
- `compass update .`: passed; indexed 713 files into 121,261 nodes, 285,681
  edges, and 3,389 communities, with 68 explicitly omitted edges and zero
  identity collisions.
- Deterministic artifact-refiner fallback completed because the installed skill
  package lacks its canonical controllers and schema assets.
- Isolated K3 adversarial diff review: PASS (0 critical, 2 warnings, 3
  suggestions). Every remaining non-blocking finding was subsequently resolved
  in the implementation and regression tests.

## Compatibility and security

The namespace is additive; `compass install` remains unchanged, so no C-017
migration step is required. `CHANGELOG.md`, `COMPATIBILITY.md`, command docs,
integration guidance, embedded skill references, and `SECURITY.md` describe the
new public, portability, credential-redaction, and loopback boundaries.
