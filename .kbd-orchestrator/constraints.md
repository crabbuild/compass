# Compass KBD Constraint Configuration

Project-specific constraint rules for KBD. Derived from `AGENTS.md` (product
invariants, Rust conventions, tests and fixtures) and `docs/design/principles.md`.

`AGENTS.md` remains authoritative. This file encodes the machine-checkable
subset for automated verification; it does not replace the completion checklist.

---

## Blocking Constraints (prevent archiving until resolved)

```yaml
constraints:
  - id: build-passes
    severity: blocking
    description: 'Workspace compile check must pass'
    command: '<see .kbd-orchestrator/project.json build_health_command>'

  - id: tests-pass
    severity: blocking
    description: 'Native workspace tests must pass'
    command: '<see .kbd-orchestrator/project.json test_command>'

  - id: clippy-clean
    severity: blocking
    description: 'Clippy must pass with -D warnings across the workspace'
    command: '<see .kbd-orchestrator/project.json lint_command>'

  - id: fmt-clean
    severity: blocking
    description: 'Rust formatting must be clean'
    command: 'cargo fmt --all -- --check'

  - id: no-unsafe-code
    severity: blocking
    description: 'Workspace policy forbids unsafe code'
    check: "rg -n 'unsafe\\s*\\{|unsafe fn' crates/ --glob '*.rs'"

  - id: no-unwrap-expect-panic
    severity: blocking
    description: 'Workspace denies unwrap_used, expect_used, and panic. Return typed errors with actionable context.'
    check: "rg -n '\\.unwrap\\(\\)|\\.expect\\(|panic!\\(' crates/ --glob '*/src/**/*.rs'"
    note: 'Test modules may differ; verify against the crate lint configuration rather than raw grep count.'

  - id: product-boundary
    severity: blocking
    description: 'Production Compass must not reference Graphify, or use non-Compass configuration/artifact names'
    command: 'sh scripts/check_product_boundary.sh'

  - id: no-graphify-dependency
    severity: blocking
    description: 'No Graphify runtime, test, configuration, artifact, or fallback dependency may be introduced'
    note: 'Enforced by product-boundary above; also verify no new build/CI automation checks out or executes Graphify.'

  - id: local-first-preserved
    severity: blocking
    description: 'Structural extraction and graph queries must work without Python, model credentials, embeddings, a vector database, runtime grammar downloads, or Graphify'

  - id: no-secrets-or-machine-paths
    severity: blocking
    description: 'No credentials, machine-specific paths, generated graphs, local .compass/ state, compass-out/, or private repository content committed outside the repository-owned .prometheus/knowledge/wiki transcript exception'
    check: "rg -n 'sk-|api_key|API_KEY|BEGIN [A-Z ]*PRIVATE KEY' crates/ scripts/ --glob '!*.lock'"

  - id: locked-dependencies
    severity: blocking
    description: 'Build and test commands use --locked; Cargo.lock is updated when dependency resolution changes'

  - id: regression-test-present
    severity: blocking
    description: 'Behavior changes require a regression test at the lowest useful layer, plus an interface/contract test when user-visible behavior changes'

  - id: determinism-preserved
    severity: blocking
    description: 'Discovery, identities, ordering, canonical encoding, and output stay deterministic for equivalent inputs. Ambiguity is never resolved by taking the first or most convenient candidate.'
    check: "rg -n 'HashMap|HashSet' crates/ --glob '*/src/**/*.rs'"
    note: 'Prefer BTreeMap/BTreeSet or explicit sorting at contract boundaries. Hits are not automatically failures — confirm no contract boundary depends on hash iteration order.'

  - id: bounded-work
    severity: blocking
    description: 'All work over source files, graphs, archives, network responses, queries, and subprocess output remains bounded. A limit error is a distinct outcome from an empty result.'

  - id: immutable-realizations
    severity: blocking
    description: 'Published historical realizations are immutable — never rewritten in place, never silently substituted with a different realization/profile'

  - id: no-real-credentials-in-tests
    severity: blocking
    description: 'Network/provider tests use local mock servers and fixtures; subprocess work uses bounded fake runners. Never real credentials or services.'

  - id: viewer-assets-generated
    severity: blocking
    description: 'Generated viewer assets must match their packages/compass-viewer source and never be hand-edited'
    command: 'node scripts/check_viewer_assets.mjs'
```

---

## Warning Constraints (acknowledge before archiving)

```yaml
- id: correct-ownership-boundary
  severity: warning
  description: 'Change lives in the lowest crate that owns the behavior (see the AGENTS.md ownership table). CLI and MCP layers stay thin.'

- id: extractor-resolver-separation
  severity: warning
  description: 'Per-file extractors emit evidence only; project-wide target selection belongs in compass-resolve'

- id: compatibility-documented
  severity: warning
  description: 'Incompatible user-visible changes include regression coverage, updated reference docs, a MIGRATION.md note when users must act, and a CHANGELOG.md entry when release-visible'
  note: 'Compatibility-sensitive: CLI args/help/exits, env vars, configuration, graph JSON, CompassQL, MCP schemas, output files, history formats, stable IDs.'

- id: compassql-gates
  severity: warning
  description: 'CompassQL grammar, execution, or support claims run their gates'
  command: 'cargo test -p compass-cypher --test tck --locked && cargo test -p compass-query --test opencypher_tck --locked && python3 scripts/check_compassql_support.py'

- id: code-graph-qualification
  severity: warning
  description: 'Code-graph publication, language, resolver, or viewer contract changes run the fixture release gate'
  command: './scripts/qualify_code_graph_v1.sh --fixtures-only'

- id: js-surface-verified
  severity: warning
  description: 'JavaScript, viewer, or VS Code changes typecheck and test'
  command: 'npm ci && npm run typecheck:js && npm run test:js'

- id: cli-product-contract
  severity: warning
  description: 'Public CLI or product identity changes run the product contract test'
  command: 'cargo test -p compass-cli --test compass_product --locked'

- id: ambiguity-and-negative-cases
  severity: warning
  description: 'Language/resolution tests cover ambiguity and negative cases — asserting identity, direction, occurrence/source range, provenance, multiplicity, and deterministic ordering — not just a happy-path symbol match'

- id: workspace-dependencies-reused
  severity: warning
  description: 'Dependencies come from root Cargo.toml via {name}.workspace = true; new dependencies are justified'

- id: platform-portable
  severity: warning
  description: 'Behavior stays portable across Linux, macOS, and Windows. No assumption of UTF-8 paths, Unix separators, or Unix-only process behavior without a guarded implementation and test.'

- id: no-shell-construction
  severity: warning
  description: 'Subprocess integrations pass arguments separately and bound duration plus captured output — never construct shell command strings'

- id: safe-primitives-reused
  severity: warning
  description: 'Existing bounded readers, subprocess helpers, path containment, endpoint checks, and atomic writes are reused rather than reimplemented as weaker local variants'

- id: security-docs-updated
  severity: warning
  description: 'SECURITY.md or the security/privacy design docs are updated when a trust, credential, network, path, subprocess, or disclosure boundary changes'

- id: performance-claims-qualified
  severity: warning
  description: 'PERFORMANCE.md is updated and the documented qualification is run when making performance claims. Correctness and deterministic equivalence take priority over speed.'

- id: no-unrelated-diff-noise
  severity: warning
  description: 'git diff and git status --short contain no unrelated or generated noise; user changes are preserved'
```

---

## Notes

- `docs/superpowers/` describes designs, not shipped evidence — never cite it as
  proof a behavior exists.
- Unknown graph attributes are preserved unless the relevant contract explicitly
  rejects them.
- `make test` is workspace `--lib --bins` only. `make test-all` adds
  `--all-targets --all-features` and requires the documented Python oracle setup.
- Some Makefile targets (`install`, `dist`, `release-check`) resolve binaries
  through a literal `target/` path after Cargo finishes. Inspect them before use
  so they do not silently trigger a second local build.
